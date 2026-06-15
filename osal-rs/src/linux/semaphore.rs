//! Counting semaphore for the Linux backend.
//!
//! # Overview
//!
//! Implements the `SemaphoreFn` trait using `std::sync::Mutex` +
//! `std::sync::Condvar`.  The semaphore is a **counting** semaphore with
//! a configurable maximum count, matching the FreeRTOS counting-semaphore
//! contract.
//!
//! # Design
//!
//! - **State**: A `StdMutex<SemaphoreState>` holds the current count and
//!   the maximum count.  Validation (`initial_count ≤ max_count`) happens
//!   at construction time.
//! - **Blocking wait**: `wait(timeout)` uses `Condvar::wait` or
//!   `Condvar::wait_timeout` depending on the timeout value.
//! - **ISR emulation**: `wait_from_isr()` / `signal_from_isr()` use
//!   `StdMutex::try_lock` — non-blocking, return immediately.
//! - **RAII**: `Drop` is a no-op (Rust memory is managed by the compiler).
//!
//! # Contract
//!
//! See `doc/osal-contact-zh.md` §6 for the detailed behavioural
//! specification.

use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::time::Duration;

use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Instant;

use crate::traits::SemaphoreFn;
use crate::traits::ToTick;
use super::types::{SemaphoreHandle, UBaseType};
use crate::utils::{Error, OsalRsBool, Result, MAX_DELAY};

// ---------------------------------------------------------------------------
// Semaphore — counting semaphore on stdlib primitives
// ---------------------------------------------------------------------------

/// A counting semaphore for resource management and signaling.
///
/// Maintains a count between `0` and `max_count`.  Tasks call `wait` to
/// decrement the count (blocking if the count is zero) and `signal` to
/// increment it (waking one waiter).
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::Semaphore;
/// use core::time::Duration;
///
/// // Binary semaphore
/// let sem = Semaphore::new(1, 1).unwrap();
/// if sem.wait(Duration::from_millis(100)) == OsalRsBool::True {
///     // critical section
///     sem.signal();
/// }
/// ```
pub struct Semaphore {
    inner: StdMutex<SemaphoreState>,
    condvar: Condvar,
    handle: SemaphoreHandle,
}

struct SemaphoreState {
    count: u32,
    max_count: u32,
}

// Safety: StdMutex + Condvar are Send + Sync.
unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}

impl Deref for Semaphore {
    type Target = SemaphoreHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl Semaphore {
    /// Creates a new counting semaphore.
    ///
    /// # Parameters
    ///
    /// * `max_count` — Maximum value the count can reach.
    /// * `initial_count` — Starting count (must be ≤ `max_count`).
    ///
    /// # Returns
    ///
    /// * `Ok(Semaphore)` on success.
    /// * `Err(Error::OutOfMemory)` if `initial_count > max_count`.
    pub fn new(max_count: UBaseType, initial_count: UBaseType) -> Result<Self> {
        if initial_count > max_count {
            return Err(Error::OutOfMemory);
        }
        Ok(Self {
            inner: StdMutex::new(SemaphoreState {
                count: initial_count,
                max_count,
            }),
            condvar: Condvar::new(),
            handle: 1 as SemaphoreHandle,
        })
    }

    /// Creates a counting semaphore with `max_count = UBaseType::MAX`.
    ///
    /// Convenience constructor for event-counting use cases where the
    /// upper bound is effectively unlimited.
    pub fn new_with_count(initial_count: UBaseType) -> Result<Self> {
        Self::new(UBaseType::MAX, initial_count)
    }
}

impl SemaphoreFn for Semaphore {
    /// Waits to acquire the semaphore (decrements count).
    ///
    /// # Parameters
    ///
    /// * `ticks_to_wait` — Maximum time to wait.  Accepts `Duration`
    ///   or raw ticks via the `ToTick` trait.
    ///
    ///   | Value                      | Behavior                   |
    ///   |----------------------------|----------------------------|
    ///   | `Duration::ZERO` / `0`    | Immediate try-wait         |
    ///   | finite `Duration` / ticks | Block up to that duration  |
    ///   | `MAX_DELAY` / `TickType::MAX` | Block forever         |
    ///
    /// # Returns
    ///
    /// * `True` — Semaphore acquired (count decremented).
    /// * `False` — Timeout expired, semaphore not acquired.
    fn wait(&self, ticks_to_wait: impl ToTick) -> OsalRsBool {
        let ticks = ticks_to_wait.to_ticks();
        let mut state = self.inner.lock().unwrap();

        // Fast path: count > 0 — decrement and return immediately.
        if state.count > 0 {
            state.count -= 1;
            return OsalRsBool::True;
        }

        // Count is zero — need to wait.
        if ticks == 0 {
            return OsalRsBool::False;
        }

        // Convert ticks to Duration for Condvar.
        let timeout = if ticks == UBaseType::MAX {
            MAX_DELAY
        } else {
            // ticks are in milliseconds (TICK_PERIOD_MS = 1)
            Duration::from_millis(ticks as u64)
        };

        let deadline = Instant::now() + timeout;
        loop {
            let elapsed = Instant::now();
            if elapsed >= deadline {
                // Timeout — could not acquire.
                return OsalRsBool::False;
            }
            let remaining = deadline - elapsed;

            state = self.condvar.wait_timeout(state, remaining).unwrap().0;

            if state.count > 0 {
                state.count -= 1;
                return OsalRsBool::True;
            }

            // Spurious wakeup — loop again with updated remaining time.
        }
    }

    /// Attempts to acquire the semaphore without blocking (ISR-friendly).
    ///
    /// On Linux this is a non-blocking try-wait using `StdMutex::try_lock`.
    ///
    /// # Returns
    ///
    /// * `True` — Semaphore acquired.
    /// * `False` — Semaphore not available (count was 0) or lock failed.
    fn wait_from_isr(&self) -> OsalRsBool {
        match self.inner.try_lock() {
            Ok(mut state) => {
                if state.count > 0 {
                    state.count -= 1;
                    OsalRsBool::True
                } else {
                    OsalRsBool::False
                }
            }
            Err(_) => OsalRsBool::False,
        }
    }

    /// Signals the semaphore (increments count, wakes one waiter).
    ///
    /// # Returns
    ///
    /// * `True` — Signal successful.
    /// * `False` — Count already at maximum.
    fn signal(&self) -> OsalRsBool {
        let mut state = self.inner.lock().unwrap();
        if state.count < state.max_count {
            state.count += 1;
            self.condvar.notify_one();
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    /// Signals the semaphore from ISR context (non-blocking).
    ///
    /// On Linux this uses `StdMutex::try_lock`.  If the lock cannot be
    /// acquired immediately the call returns `False`.
    ///
    /// # Returns
    ///
    /// * `True` — Signal successful.
    /// * `False` — Signal failed (count at max, or lock busy).
    fn signal_from_isr(&self) -> OsalRsBool {
        match self.inner.try_lock() {
            Ok(mut state) => {
                if state.count < state.max_count {
                    state.count += 1;
                    self.condvar.notify_one();
                    OsalRsBool::True
                } else {
                    OsalRsBool::False
                }
            }
            Err(_) => OsalRsBool::False,
        }
    }

    /// Destroys the semaphore.
    ///
    /// On Linux this is a no-op; memory is reclaimed when `self` is dropped.
    fn delete(&mut self) {}
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Debug for Semaphore {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => f
                .debug_struct("Semaphore")
                .field("count", &state.count)
                .field("max_count", &state.max_count)
                .finish(),
            Err(_) => f.debug_struct("Semaphore").finish_non_exhaustive(),
        }
    }
}

impl Display for Semaphore {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => write!(f, "Semaphore {{ count: {}, max: {} }}", state.count, state.max_count),
            Err(_) => write!(f, "Semaphore {{ <locked> }}"),
        }
    }
}