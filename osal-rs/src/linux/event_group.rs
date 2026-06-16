//! Event group synchronization primitive for the Linux backend.
//!
//! # Overview
//!
//! Implements the `EventGroupFn` trait using `std::sync::Mutex` +
//! `std::sync::Condvar`.  Event groups allow threads to synchronize on
//! multiple independent event bits — set, clear, and wait operations with
//! timeout support.
//!
//! # Design
//!
//! - **State**: A `StdMutex<EventGroupState>` holds the current bit flags.
//! - **Blocking wait**: `wait(mask, timeout)` uses `Condvar::wait_timeout`
//!   to block until at least one bit in the mask is set, or the timeout
//!   expires.  Wait uses **OR** semantics (any bit in the mask) matching
//!   the FreeRTOS backend and the contract specification.
//! - **ISR emulation**: `set_from_isr()` / `get_from_isr()` /
//!   `clear_from_isr()` use `StdMutex::try_lock` — non-blocking, return
//!   immediately.
//! - **RAII**: `Drop` is a no-op (Rust memory is managed by the compiler).
//!
//! # Contract
//!
//! See `doc/osal-contact-zh.md` §8 for the detailed behavioural
//! specification.

use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use std::sync::{Condvar, Mutex as StdMutex, MutexGuard as StdMutexGuard, TryLockError};
use std::time::Instant;

use crate::traits::EventGroupFn;
use crate::traits::ToTick;
use crate::utils::{Error, Result};

use super::types::{EventBits, EventGroupHandle, TickType};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type EventGroupState = usize;

/// Recovers from a poisoned mutex lock.  This keeps the event group usable
/// even after a panic inside a critical section.
fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Global atomic counter for allocating unique `EventGroupHandle` values.
static NEXT_EVENT_GROUP_HANDLE: AtomicUsize = AtomicUsize::new(1);

/// Allocates the next unique event group handle.
fn next_event_group_handle() -> EventGroupHandle {
    NEXT_EVENT_GROUP_HANDLE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("Linux event group handle space exhausted") as EventGroupHandle
}

// ---------------------------------------------------------------------------
// EventGroup — multi-bit flag synchronization on stdlib primitives
// ---------------------------------------------------------------------------

/// A set of event flags for thread synchronization.
///
/// Event groups contain multiple event bits (typically 24 bits) that can be
/// manipulated independently.  Threads can wait for specific combinations of
/// bits to be set (OR semantics — any bit in the mask triggers wake-up).
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::{EventGroup, EventGroupFn};
/// use core::time::Duration;
///
/// const BIT_0: u32 = 1 << 0;
/// const BIT_1: u32 = 1 << 1;
///
/// let events = EventGroup::new().unwrap();
///
/// // Set bit 0
/// events.set(BIT_0);
///
/// // Wait for bit 0 or bit 1 with 100 ms timeout
/// let result = events.wait(BIT_0 | BIT_1, Duration::from_millis(100).to_ticks());
/// if result & (BIT_0 | BIT_1) != 0 {
///     println!("At least one bit was set");
/// }
/// ```
pub struct EventGroup {
    inner: StdMutex<EventGroupState>,
    condvar: Condvar,
    handle: EventGroupHandle,
}

// Safety: StdMutex + Condvar are Send + Sync.
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
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` — Successfully created event group.
    /// * `Err(Error::OutOfMemory)` — Creation failed.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use osal_rs::os::{EventGroup, EventGroupFn};
    ///
    /// let events = EventGroup::new().unwrap();
    /// ```
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: StdMutex::new(0),
            condvar: Condvar::new(),
            handle: next_event_group_handle(),
        })
    }

    /// Waits for specified event bits to be set with a timeout in a
    /// `ToTick`-compatible type.
    ///
    /// Convenience method that converts a `ToTick` type (e.g. `Duration`)
    /// to ticks and delegates to `wait`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use osal_rs::os::{EventGroup, EventGroupFn};
    /// use core::time::Duration;
    ///
    /// let events = EventGroup::new().unwrap();
    /// let bits = events.wait_with_to_tick(0b0001, Duration::from_secs(1));
    /// ```
    pub fn wait_with_to_tick(&self, mask: EventBits, timeout_ticks: impl ToTick) -> EventBits {
        self.wait(mask, timeout_ticks.to_ticks())
    }

    /// Non-blocking try-lock for ISR simulation paths.
    ///
    /// Recovers from poisoned mutexes; only returns `Err(())` on
    /// `TryLockError::WouldBlock`.
    fn try_lock_state(&self) -> core::result::Result<StdMutexGuard<'_, EventGroupState>, ()> {
        match self.inner.try_lock() {
            Ok(state) => Ok(state),
            Err(TryLockError::Poisoned(err)) => Ok(err.into_inner()),
            Err(TryLockError::WouldBlock) => Err(()),
        }
    }
}

impl EventGroupFn for EventGroup {
    /// Sets the specified event bits (OR operation).
    ///
    /// Any tasks waiting for these bits will be unblocked if their wait
    /// conditions are now satisfied (any bit in their mask is set).
    ///
    /// Reserved bits (above [`Self::MAX_MASK`]) are silently ignored.
    ///
    /// # Parameters
    ///
    /// * `bits` — The bits to set (bitwise OR with current value).
    ///
    /// # Returns
    ///
    /// The event bits value **after** the bits were set, with reserved
    /// bits masked out.
    fn set(&self, bits: EventBits) -> EventBits {
        let bits = bits & Self::MAX_MASK;

        let mut state = recover_lock(self.inner.lock());
        *state |= bits as EventGroupState;
        let current_bits = (*state as EventBits) & Self::MAX_MASK;

        drop(state);

        if bits != 0 {
            self.condvar.notify_all();
        }

        current_bits
    }

    /// Sets event bits from ISR context (non-blocking).
    ///
    /// On Linux this uses `StdMutex::try_lock`.  If the lock cannot be
    /// acquired immediately (not poisoned), the call returns
    /// `Err(Error::QueueFull)`.
    ///
    /// Reserved bits (above [`Self::MAX_MASK`]) are silently ignored.
    ///
    /// # Parameters
    ///
    /// * `bits` — The bits to set.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Bits set successfully.
    /// * `Err(Error)` — Lock busy, operation could not complete.
    fn set_from_isr(&self, bits: EventBits) -> Result<()> {
        let bits = bits & Self::MAX_MASK;

        match self.try_lock_state() {
            Ok(mut state) => {
                *state |= bits as EventGroupState;
                drop(state);

                if bits != 0 {
                    self.condvar.notify_all();
                }

                Ok(())
            }
            Err(_) => Err(Error::QueueFull),
        }
    }

    /// Gets the current value of all event bits (non-blocking).
    ///
    /// Reserved bits are masked out of the returned value.
    ///
    /// # Returns
    ///
    /// Current state of event bits with reserved bits masked.
    fn get(&self) -> EventBits {
        let state = recover_lock(self.inner.lock());
        (*state as EventBits) & Self::MAX_MASK
    }

    /// Gets event bits from ISR context (non-blocking).
    ///
    /// On Linux this uses `StdMutex::try_lock`.  If the lock cannot be
    /// acquired immediately, returns `0`.
    ///
    /// Reserved bits are masked out of the returned value.
    ///
    /// # Returns
    ///
    /// Current state of event bits with reserved bits masked, or `0` if
    /// the lock is busy (not poisoned).
    fn get_from_isr(&self) -> EventBits {
        match self.try_lock_state() {
            Ok(state) => (*state as EventBits) & Self::MAX_MASK,
            Err(_) => 0,
        }
    }

    /// Clears the specified event bits (AND NOT operation).
    ///
    /// Reserved bits (above [`Self::MAX_MASK`]) are silently ignored.
    ///
    /// # Parameters
    ///
    /// * `bits` — The bits to clear.
    ///
    /// # Returns
    ///
    /// The event bits value **after** the bits were cleared, with
    /// reserved bits masked out.
    fn clear(&self, bits: EventBits) -> EventBits {
        let bits = bits & Self::MAX_MASK;

        let mut state = recover_lock(self.inner.lock());
        *state &= !(bits as EventGroupState);
        (*state as EventBits) & Self::MAX_MASK
    }

    /// Clears event bits from ISR context (non-blocking).
    ///
    /// On Linux this uses `StdMutex::try_lock`.  If the lock cannot be
    /// acquired immediately (not poisoned), returns `Err(Error::QueueFull)`.
    ///
    /// Reserved bits (above [`Self::MAX_MASK`]) are silently ignored.
    ///
    /// # Parameters
    ///
    /// * `bits` — The bits to clear.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Bits cleared successfully.
    /// * `Err(Error)` — Lock busy, operation could not complete.
    fn clear_from_isr(&self, bits: EventBits) -> Result<()> {
        let bits = bits & Self::MAX_MASK;

        match self.try_lock_state() {
            Ok(mut state) => {
                *state &= !(bits as EventGroupState);
                Ok(())
            }
            Err(_) => Err(Error::QueueFull),
        }
    }

    /// Waits for specified event bits to be set (OR semantics).
    ///
    /// Blocks the calling thread until **any** of the bits in `mask`
    /// are set, or until the timeout expires.  The bits are **not**
    /// cleared automatically.
    ///
    /// Reserved bits in the mask are silently ignored.  If the resulting
    /// mask is 0, the current bits are returned immediately without
    /// blocking.
    ///
    /// # Parameters
    ///
    /// * `mask` — Bit mask of bits to wait for (waits for ANY bit in mask).
    /// * `timeout_ticks` — Maximum time to wait in ticks:
    ///   * `0` — Non-blocking, returns immediately.
    ///   * `TickType::MAX` — Block forever.
    ///   * finite value — Block up to that many ticks.
    ///
    /// # Returns
    ///
    /// The event bits value (with reserved bits masked) when the
    /// function returns.  Check if any bit in the mask is set to
    /// determine if the wait succeeded:
    ///
    /// ```ignore
    /// let result = events.wait(mask, timeout);
    /// if result & mask != 0 {
    ///     // Success: at least one bit was set
    /// }
    /// ```
    fn wait(&self, mask: EventBits, timeout_ticks: TickType) -> EventBits {
        let mask = mask & Self::MAX_MASK;
        let mask_usize = mask as EventGroupState;

        let mut state = recover_lock(self.inner.lock());

        // Mask == 0: return current bits immediately; nothing to wait for.
        if mask == 0 {
            return (*state as EventBits) & Self::MAX_MASK;
        }

        // Fast path: condition already satisfied.
        if *state & mask_usize != 0 {
            return (*state as EventBits) & Self::MAX_MASK;
        }

        // Zero timeout — return immediately with current value.
        if timeout_ticks == 0 {
            return (*state as EventBits) & Self::MAX_MASK;
        }

        // True infinite wait: block forever until signaled.
        if timeout_ticks == TickType::MAX {
            loop {
                state = recover_lock(self.condvar.wait(state));

                if *state & mask_usize != 0 {
                    return (*state as EventBits) & Self::MAX_MASK;
                }
            }
        }

        // Finite wait with deadline loop.
        let timeout = Duration::from_millis(timeout_ticks as u64);
        let deadline = match Instant::now().checked_add(timeout) {
            Some(deadline) => deadline,
            None => return (*state as EventBits) & Self::MAX_MASK,
        };

        loop {
            let now = Instant::now();

            if now >= deadline {
                return (*state as EventBits) & Self::MAX_MASK;
            }

            let remaining = deadline - now;

            let (next_state, timeout_result) =
                recover_lock(self.condvar.wait_timeout(state, remaining));

            state = next_state;

            if *state & mask_usize != 0 {
                return (*state as EventBits) & Self::MAX_MASK;
            }

            if timeout_result.timed_out() {
                return (*state as EventBits) & Self::MAX_MASK;
            }
        }
    }

    /// Destroys the event group.
    ///
    /// On Linux this is a no-op; memory is reclaimed when `self` is dropped.
    fn delete(&mut self) {}
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Debug for EventGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => f
                .debug_struct("EventGroup")
                .field(
                    "bits",
                    &format_args!("0x{:08X}", (*state as EventBits) & Self::MAX_MASK),
                )
                .field("handle", &self.handle)
                .finish(),
            Err(TryLockError::Poisoned(err)) => {
                let state = err.into_inner();
                f.debug_struct("EventGroup")
                    .field(
                        "bits",
                        &format_args!("0x{:08X}", (*state as EventBits) & Self::MAX_MASK),
                    )
                    .field("handle", &self.handle)
                    .field("poisoned", &true)
                    .finish()
            }
            Err(TryLockError::WouldBlock) => f
                .debug_struct("EventGroup")
                .field("handle", &self.handle)
                .finish_non_exhaustive(),
        }
    }
}

impl Display for EventGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => write!(
                f,
                "EventGroup {{ bits: 0x{:08X}, handle: {} }}",
                (*state as EventBits) & Self::MAX_MASK,
                self.handle as usize,
            ),
            Err(TryLockError::Poisoned(err)) => {
                let state = err.into_inner();
                write!(
                    f,
                    "EventGroup {{ bits: 0x{:08X}, handle: {}, poisoned: true }}",
                    (*state as EventBits) & Self::MAX_MASK,
                    self.handle as usize,
                )
            }
            Err(TryLockError::WouldBlock) => write!(
                f,
                "EventGroup {{ handle: {}, locked: true }}",
                self.handle as usize,
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal poison-recovery tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// After the internal mutex is poisoned, subsequent operations should
    /// still work without panicking.
    #[test]
    fn event_group_recovers_from_poisoned_lock() {
        let events = EventGroup::new().unwrap();

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = events.inner.lock().unwrap();
            panic!("poison event group mutex");
        }));

        // All operations must continue to work after poison recovery.
        assert_eq!(events.get(), 0);

        let bits = events.set(0b0001);
        assert_ne!(bits & 0b0001, 0);

        let waited = events.wait(0b0001, 0);
        assert_ne!(waited & 0b0001, 0);

        assert!(events.clear_from_isr(0b0001).is_ok());
        assert_eq!(events.get_from_isr() & 0b0001, 0);

        assert!(events.set_from_isr(0b0010).is_ok());
        assert_ne!(events.get() & 0b0010, 0);
    }
}