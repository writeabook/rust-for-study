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
//! - **State**: A `StdMutex<EventBits>` holds the current bit flags.
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
use core::time::Duration;

use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Instant;

use crate::traits::EventGroupFn;
use crate::traits::ToTick;
use super::types::{EventBits, EventGroupHandle, TickType};
use crate::utils::{Error, Result, MAX_DELAY};

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
    inner: StdMutex<usize>,
    condvar: Condvar,
    /// Dummy handle for API surface compatibility (Deref target).
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
            handle: 1 as EventGroupHandle,
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
}

impl EventGroupFn for EventGroup {
    /// Sets the specified event bits (OR operation).
    ///
    /// Any tasks waiting for these bits will be unblocked if their wait
    /// conditions are now satisfied (any bit in their mask is set).
    ///
    /// # Parameters
    ///
    /// * `bits` — The bits to set (bitwise OR with current value).
    ///
    /// # Returns
    ///
    /// The event bits value **after** the bits were set.
    /// This is a snapshot at return time; if another task clears some bits
    /// between the set and the return, the returned value may not contain
    /// all requested bits.
    fn set(&self, bits: EventBits) -> EventBits {
        let mut state = self.inner.lock().unwrap();
        *state |= bits as usize;
        let current_bits = *state;
        // Wake ALL waiting threads — any of them may now have their
        // condition satisfied (OR semantics).
        self.condvar.notify_all();
        current_bits as EventBits
    }

    /// Sets event bits from ISR context (non-blocking).
    ///
    /// On Linux this uses `StdMutex::try_lock`.  If the lock cannot be
    /// acquired immediately, the call returns `Err(Error::QueueFull)`.
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
        match self.inner.try_lock() {
            Ok(mut state) => {
                *state |= bits as usize;
                self.condvar.notify_all();
                Ok(())
            }
            Err(_) => Err(Error::QueueFull),
        }
    }

    /// Gets the current value of all event bits (non-blocking).
    ///
    /// # Returns
    ///
    /// Current state of all event bits.
    fn get(&self) -> EventBits {
        let state = self.inner.lock().unwrap();
        *state as EventBits
    }

    /// Gets event bits from ISR context (non-blocking).
    ///
    /// On Linux this uses `StdMutex::try_lock`.  If the lock cannot be
    /// acquired immediately, returns `0`.
    ///
    /// # Returns
    ///
    /// Current state of all event bits, or `0` if the lock is busy.
    fn get_from_isr(&self) -> EventBits {
        match self.inner.try_lock() {
            Ok(state) => *state as EventBits,
            Err(_) => 0,
        }
    }

    /// Clears the specified event bits (AND NOT operation).
    ///
    /// # Parameters
    ///
    /// * `bits` — The bits to clear.
    ///
    /// # Returns
    ///
    /// The event bits value **after** the bits were cleared.
    /// This is a snapshot at return time; if another task sets some bits
    /// between the clear and the return, the returned value may not reflect
    /// the cleared state.
    fn clear(&self, bits: EventBits) -> EventBits {
        let mut state = self.inner.lock().unwrap();
        *state &= !(bits as usize);
        let current_bits = *state;
        current_bits as EventBits
    }

    /// Clears event bits from ISR context (non-blocking).
    ///
    /// On Linux this uses `StdMutex::try_lock`.  If the lock cannot be
    /// acquired immediately, returns `Err(Error::QueueFull)`.
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
        match self.inner.try_lock() {
            Ok(mut state) => {
                *state &= !(bits as usize);
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
    /// The event bits value when the function returns.  Check if any bit
    /// in the mask is set to determine if the wait succeeded:
    ///
    /// ```ignore
    /// let result = events.wait(mask, timeout);
    /// if result & mask != 0 {
    ///     // Success: at least one bit was set
    /// }
    /// ```
    fn wait(&self, mask: EventBits, timeout_ticks: TickType) -> EventBits {
        let mask_usize = mask as usize;
        let mut state = self.inner.lock().unwrap();

        // Fast path: condition already satisfied.
        if *state & mask_usize != 0 {
            return *state as EventBits;
        }

        // Zero timeout — return immediately with current value.
        if timeout_ticks == 0 {
            return *state as EventBits;
        }

        // Convert ticks to Duration for Condvar.
        let timeout = if timeout_ticks == TickType::MAX {
            MAX_DELAY
        } else {
            // ticks are in milliseconds (TICK_PERIOD_MS = 1)
            Duration::from_millis(timeout_ticks as u64)
        };

        let deadline = Instant::now() + timeout;
        loop {
            let elapsed = Instant::now();
            if elapsed >= deadline {
                // Timeout — return current bits (may be zero).
                return *state as EventBits;
            }
            let remaining = deadline - elapsed;

            state = self.condvar.wait_timeout(state, remaining).unwrap().0;

            if *state & mask_usize != 0 {
                return *state as EventBits;
            }

            // Spurious wakeup — loop again with updated remaining time.
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
                .field("bits", &format_args!("0x{:08X}", *state))
                .finish(),
            Err(_) => f.debug_struct("EventGroup").finish_non_exhaustive(),
        }
    }
}

impl Display for EventGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => write!(f, "EventGroup {{ bits: 0x{:08X} }}", *state),
            Err(_) => write!(f, "EventGroup {{ <locked> }}"),
        }
    }
}