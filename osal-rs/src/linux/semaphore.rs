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
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use std::sync::{Condvar, Mutex as StdMutex, MutexGuard as StdMutexGuard, TryLockError};
use std::time::Instant;

use super::types::{SemaphoreHandle, UBaseType};
use crate::traits::SemaphoreFn;
use crate::traits::ToTick;
use crate::utils::{Error, OsalRsBool, Result};

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Recovers from a poisoned mutex lock.  This keeps the semaphore usable even
/// after a panic inside a critical section.
fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Global atomic counter for allocating unique `SemaphoreHandle` values.
static NEXT_SEMAPHORE_HANDLE: AtomicUsize = AtomicUsize::new(1);

/// Allocates the next unique semaphore handle.
fn next_semaphore_handle() -> SemaphoreHandle {
    NEXT_SEMAPHORE_HANDLE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("Linux semaphore handle space exhausted") as SemaphoreHandle
}

// Safety: StdMutex + Condvar are Send + Sync.
unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}

impl Deref for Semaphore {
    type Target = SemaphoreHandle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
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
            handle: next_semaphore_handle(),
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

impl Semaphore {
    /// Non-blocking try-lock for ISR simulation paths.
    ///
    /// Recovers from poisoned mutexes; only returns `Err(())` on
    /// `TryLockError::WouldBlock`.
    fn try_lock_state(&self) -> core::result::Result<StdMutexGuard<'_, SemaphoreState>, ()> {
        match self.inner.try_lock() {
            Ok(state) => Ok(state),
            Err(TryLockError::Poisoned(err)) => Ok(err.into_inner()),
            Err(TryLockError::WouldBlock) => Err(()),
        }
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
    ///   | Value                   | Behavior                   |
    ///   |-------------------------|----------------------------|
    ///   | `0`                     | Immediate try-wait         |
    ///   | finite ticks            | Block up to that duration  |
    ///   | `UBaseType::MAX`        | Block forever              |
    ///
    /// # Returns
    ///
    /// * `True` — Semaphore acquired (count decremented).
    /// * `False` — Timeout expired, semaphore not acquired.
    fn wait(&self, ticks_to_wait: impl ToTick) -> OsalRsBool {
        let ticks = ticks_to_wait.to_ticks();
        let mut state = recover_lock(self.inner.lock());

        // Fast path: count > 0 — decrement and return immediately.
        if state.count > 0 {
            state.count -= 1;
            return OsalRsBool::True;
        }

        // Count is zero — need to wait.
        if ticks == 0 {
            return OsalRsBool::False;
        }

        // Infinite wait: block forever until signaled.
        if ticks == UBaseType::MAX {
            loop {
                state = recover_lock(self.condvar.wait(state));

                if state.count > 0 {
                    state.count -= 1;
                    return OsalRsBool::True;
                }
            }
        }

        // Finite wait with deadline loop.
        let timeout = Duration::from_millis(ticks as u64);
        let deadline = match Instant::now().checked_add(timeout) {
            Some(deadline) => deadline,
            None => return OsalRsBool::False,
        };

        loop {
            let now = Instant::now();

            if now >= deadline {
                return OsalRsBool::False;
            }

            let remaining = deadline - now;

            let (next_state, timeout_result) =
                recover_lock(self.condvar.wait_timeout(state, remaining));

            state = next_state;

            if state.count > 0 {
                state.count -= 1;
                return OsalRsBool::True;
            }

            if timeout_result.timed_out() {
                return OsalRsBool::False;
            }
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
        match self.try_lock_state() {
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
        let mut state = recover_lock(self.inner.lock());
        if state.count < state.max_count {
            state.count += 1;
            drop(state);
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
        match self.try_lock_state() {
            Ok(mut state) => {
                if state.count < state.max_count {
                    state.count += 1;
                    drop(state);
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
                .field("handle", &self.handle)
                .finish(),
            Err(TryLockError::Poisoned(err)) => {
                let state = err.into_inner();
                f.debug_struct("Semaphore")
                    .field("count", &state.count)
                    .field("max_count", &state.max_count)
                    .field("handle", &self.handle)
                    .field("poisoned", &true)
                    .finish()
            }
            Err(TryLockError::WouldBlock) => f.debug_struct("Semaphore").finish_non_exhaustive(),
        }
    }
}

impl Display for Semaphore {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => write!(
                f,
                "Semaphore {{ count: {}, max: {} }}",
                state.count, state.max_count
            ),
            Err(TryLockError::Poisoned(err)) => {
                let state = err.into_inner();
                write!(
                    f,
                    "Semaphore {{ count: {}, max: {}, poisoned: true }}",
                    state.count, state.max_count
                )
            }
            Err(TryLockError::WouldBlock) => write!(f, "Semaphore {{ <locked> }}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal poison-recovery tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// After the internal mutex is poisoned, subsequent `wait`, `signal`,
    /// and ISR operations should still work.
    #[test]
    fn semaphore_remains_usable_after_poison() {
        let sem = Arc::new(Semaphore::new(2, 1).unwrap());

        // Poison the mutex by locking it and panicking.
        let s = Arc::clone(&sem);
        let handle = std::thread::spawn(move || {
            let _guard = s.inner.lock().unwrap();
            panic!("intentional poison");
        });
        // Wait for the panic to poison the lock.
        let _ = handle.join();

        // After poison: recover_lock is used everywhere, so these must not panic.
        assert_eq!(sem.wait(Duration::ZERO), OsalRsBool::True); // count 1→0
        assert_eq!(sem.wait(Duration::ZERO), OsalRsBool::False); // count 0
        assert_eq!(sem.signal(), OsalRsBool::True); // 0→1
        assert_eq!(sem.signal(), OsalRsBool::True); // 1→2
        assert_eq!(sem.signal(), OsalRsBool::False); // already max(2)

        assert_eq!(sem.wait_from_isr(), OsalRsBool::True); // 2→1
        assert_eq!(sem.signal_from_isr(), OsalRsBool::True); // 1→2
    }
}
