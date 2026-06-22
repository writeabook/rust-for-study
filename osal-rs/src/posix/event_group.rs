//! POSIX backend event group — multi-bit flag synchronization.
//!
//! Backed by `PosixMutex` + `PosixCondvar` with `CLOCK_MONOTONIC` deadlines.
//! Wait uses OR semantics (any bit in the mask triggers wake-up), matching
//! the FreeRTOS and Linux backend contracts.
//!
//! ISR-like methods (`_from_isr`) use non-blocking `try_lock`; they are
//! host-simulation paths — POSIX user space has no real ISR context.

use core::cell::UnsafeCell;
use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};

use libc::PTHREAD_MUTEX_ERRORCHECK;

use crate::traits::{EventGroupFn, ToTick};
use crate::utils::{Error, Result};

use super::sys::clock;
use super::sys::condvar::PosixCondvar;
use super::sys::mutex::PosixMutex;
use super::types::{EventBits, EventGroupHandle, TickType};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type EventGroupState = usize;

static NEXT_EVENT_GROUP_HANDLE: AtomicUsize = AtomicUsize::new(1);

fn next_event_group_handle() -> EventGroupHandle {
    NEXT_EVENT_GROUP_HANDLE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("POSIX event group handle space exhausted") as EventGroupHandle
}

// ---------------------------------------------------------------------------
// RAII lock guard
// ---------------------------------------------------------------------------

struct EventGroupLockGuard<'a> {
    mutex: &'a PosixMutex,
}

impl<'a> EventGroupLockGuard<'a> {
    fn lock(mutex: &'a PosixMutex) -> Option<Self> {
        if mutex.lock() {
            Some(Self { mutex })
        } else {
            None
        }
    }

    fn try_lock(mutex: &'a PosixMutex) -> Option<Self> {
        if mutex.try_lock() {
            Some(Self { mutex })
        } else {
            None
        }
    }
}

impl Drop for EventGroupLockGuard<'_> {
    fn drop(&mut self) {
        assert!(
            self.mutex.unlock(),
            "failed to unlock POSIX event-group mutex"
        );
    }
}

// ---------------------------------------------------------------------------
// EventGroup
// ---------------------------------------------------------------------------

pub struct EventGroup {
    mutex: PosixMutex,
    condvar: PosixCondvar,
    state: UnsafeCell<EventGroupState>,
    handle: EventGroupHandle,
}

unsafe impl Send for EventGroup {}
unsafe impl Sync for EventGroup {}

impl Deref for EventGroup {
    type Target = EventGroupHandle;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl EventGroup {
    /// Maximum usable event bits mask.
    ///
    /// Matches the FreeRTOS backend: the top 8 bits are reserved for
    /// internal use, leaving 24 usable bits (56 on 64-bit platforms).
    pub const MAX_MASK: EventBits = EventBits::MAX >> 8;

    /// Creates a new event group with all bits initially cleared (0).
    pub fn new() -> Result<Self> {
        let mutex =
            PosixMutex::new(PTHREAD_MUTEX_ERRORCHECK).ok_or(Error::OutOfMemory)?;
        let condvar = PosixCondvar::new().ok_or(Error::OutOfMemory)?;

        Ok(Self {
            mutex,
            condvar,
            state: UnsafeCell::new(0),
            handle: next_event_group_handle(),
        })
    }

    /// Convenience: waits with a `ToTick`-compatible timeout.
    pub fn wait_with_to_tick(
        &self,
        mask: EventBits,
        timeout_ticks: impl ToTick,
    ) -> EventBits {
        self.wait(mask, timeout_ticks.to_ticks())
    }

    // -- internal helpers --------------------------------------------------

    #[inline]
    fn state_mut(&self) -> &mut EventGroupState {
        unsafe { &mut *self.state.get() }
    }

    #[inline]
    fn current_bits(&self) -> EventBits {
        (*self.state_mut() as EventBits) & Self::MAX_MASK
    }
}

// ---------------------------------------------------------------------------
// EventGroupFn
// ---------------------------------------------------------------------------

impl EventGroupFn for EventGroup {
    fn set(&self, bits: EventBits) -> EventBits {
        let bits = bits & Self::MAX_MASK;
        let _guard = EventGroupLockGuard::lock(&self.mutex)
            .expect("failed to lock POSIX event-group mutex");

        let state = self.state_mut();
        *state |= bits as EventGroupState;

        let current = (*state as EventBits) & Self::MAX_MASK;

        if bits != 0 {
            self.condvar.broadcast();
        }

        current
    }

    fn set_from_isr(&self, bits: EventBits) -> Result<()> {
        let bits = bits & Self::MAX_MASK;
        let Some(_guard) = EventGroupLockGuard::try_lock(&self.mutex) else {
            return Err(Error::QueueFull);
        };

        let state = self.state_mut();
        *state |= bits as EventGroupState;

        if bits != 0 {
            self.condvar.broadcast();
        }

        Ok(())
    }

    fn get(&self) -> EventBits {
        let _guard = EventGroupLockGuard::lock(&self.mutex)
            .expect("failed to lock POSIX event-group mutex");

        self.current_bits()
    }

    fn get_from_isr(&self) -> EventBits {
        let Some(_guard) = EventGroupLockGuard::try_lock(&self.mutex) else {
            return 0;
        };

        self.current_bits()
    }

    fn clear(&self, bits: EventBits) -> EventBits {
        let bits = bits & Self::MAX_MASK;
        let _guard = EventGroupLockGuard::lock(&self.mutex)
            .expect("failed to lock POSIX event-group mutex");

        let state = self.state_mut();
        *state &= !(bits as EventGroupState);

        (*state as EventBits) & Self::MAX_MASK
    }

    fn clear_from_isr(&self, bits: EventBits) -> Result<()> {
        let bits = bits & Self::MAX_MASK;
        let Some(_guard) = EventGroupLockGuard::try_lock(&self.mutex) else {
            return Err(Error::QueueFull);
        };

        let state = self.state_mut();
        *state &= !(bits as EventGroupState);

        Ok(())
    }

    fn wait(&self, mask: EventBits, timeout_ticks: TickType) -> EventBits {
        let mask = mask & Self::MAX_MASK;
        let mask_state = mask as EventGroupState;

        let _guard = EventGroupLockGuard::lock(&self.mutex)
            .expect("failed to lock POSIX event-group mutex");

        // Empty mask — nothing to wait for.
        if mask == 0 {
            return self.current_bits();
        }

        // Fast path: condition already satisfied.
        if (*self.state_mut() & mask_state) != 0 {
            return self.current_bits();
        }

        // Zero timeout — return immediately.
        if timeout_ticks == 0 {
            return self.current_bits();
        }

        // True infinite wait.
        if timeout_ticks == TickType::MAX {
            loop {
                self.condvar.wait(&self.mutex);

                if (*self.state_mut() & mask_state) != 0 {
                    return self.current_bits();
                }
            }
        }

        // Finite wait with CLOCK_MONOTONIC absolute deadline.
        let timeout_ms = timeout_ticks as u64;
        let deadline = clock::deadline_from_ms(timeout_ms);

        loop {
            let signaled = self.condvar.timedwait(&self.mutex, &deadline);

            if (*self.state_mut() & mask_state) != 0 {
                return self.current_bits();
            }

            if !signaled {
                return self.current_bits();
            }
        }
    }

    fn delete(&mut self) {}
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Debug for EventGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        if let Some(_guard) = EventGroupLockGuard::try_lock(&self.mutex) {
            f.debug_struct("EventGroup")
                .field(
                    "bits",
                    &format_args!("0x{:08X}", self.current_bits()),
                )
                .field("handle", &self.handle)
                .finish()
        } else {
            f.debug_struct("EventGroup")
                .field("handle", &self.handle)
                .finish_non_exhaustive()
        }
    }
}

impl Display for EventGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        if let Some(_guard) = EventGroupLockGuard::try_lock(&self.mutex) {
            write!(
                f,
                "EventGroup {{ bits: 0x{:08X}, handle: {} }}",
                self.current_bits(),
                self.handle as usize
            )
        } else {
            write!(
                f,
                "EventGroup {{ handle: {}, locked: true }}",
                self.handle as usize
            )
        }
    }
}
