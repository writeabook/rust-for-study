//! Recursive mutex synchronization primitives for the Linux backend.
//!
//! # Overview
//!
//! Implements the `RawMutexFn` and `MutexFn` traits using safe Rust
//! standard-library primitives (`std::sync::Mutex`, `std::sync::Condvar`,
//! `std::thread::ThreadId`). The mutex is **recursive** — the same thread
//! may acquire it multiple times without deadlocking, matching the
//! FreeRTOS recursive-mutex contract.
//!
//! # Design
//!
//! - **Recursion**: A thread-ID field and a recursion counter track
//!   ownership.  `lock()` increments the counter when called by the
//!   owning thread; otherwise it blocks on a `Condvar`.
//! - **Blocking**: `lock()` waits indefinitely via `Condvar::wait`,
//!   matching the OSAL contract for `lock()` with `MAX_DELAY`.
//! - **ISR emulation**: `lock_from_isr()` / `unlock_from_isr()` are
//!   non-blocking try-lock wrappers — they use `try_lock` on the inner
//!   `std::sync::Mutex` and return immediately.
//! - **RAII**: `MutexGuard` and `MutexGuardFromIsr` release exactly one
//!   recursion level on drop.
//!
//! # Contract
//!
//! See `doc/osal-contact-zh.md` §5 for the detailed behavioural
//! specification.

use core::cell::UnsafeCell;
use core::fmt::{Debug, Display, Formatter};
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use alloc::sync::Arc;
use std::sync::{Condvar, Mutex as StdMutex};
use std::thread::ThreadId;

use crate::traits::{MutexGuardFn, RawMutexFn, MutexFn};
use crate::utils::{Error, OsalRsBool, Result};

// ---------------------------------------------------------------------------
// RawMutex — recursive mutex built on stdlib primitives
// ---------------------------------------------------------------------------

/// Low-level recursive mutex for the Linux backend.
///
/// Uses `std::sync::Mutex<State>` + `std::sync::Condvar` to provide
/// blocking, recursive mutual exclusion without FFI.
///
/// # Recursion
///
/// The same thread may call `lock()` multiple times.  Each `lock()`
/// increments an internal recursion counter; each `unlock()` decrements
/// it.  The mutex is only released for other threads when the counter
/// reaches zero.
pub struct RawMutex {
    inner: StdMutex<RawMutexState>,
    condvar: Condvar,
}

/// Internal state protected by the inner stdlib mutex.
struct RawMutexState {
    /// `Some(id)` when a thread holds the mutex; `None` when free.
    owner: Option<ThreadId>,
    /// Number of times the owning thread has locked (≥ 1 when owned).
    recursion: u32,
}

// Safety: the inner stdlib `Mutex` provides Send + Sync.
unsafe impl Send for RawMutex {}
unsafe impl Sync for RawMutex {}

impl RawMutex {
    /// Creates a new recursive mutex in the unlocked state.
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: StdMutex::new(RawMutexState {
                owner: None,
                recursion: 0,
            }),
            condvar: Condvar::new(),
        })
    }
}

impl RawMutexFn for RawMutex {
    /// Blocks until the mutex can be acquired.
    ///
    /// If called by the thread that already owns the mutex the
    /// recursion counter is incremented and the call returns
    /// immediately.
    fn lock(&self) -> OsalRsBool {
        let mut state = self.inner.lock().unwrap();
        let current = std::thread::current().id();

        if state.owner == Some(current) {
            // Recursive acquisition by the owning thread.
            state.recursion += 1;
            OsalRsBool::True
        } else {
            // Wait until the mutex becomes free.
            while state.owner.is_some() {
                state = self.condvar.wait(state).unwrap();
            }
            state.owner = Some(current);
            state.recursion = 1;
            OsalRsBool::True
        }
    }

    /// Non-blocking try-lock (ISR emulation).
    ///
    /// Returns `True` if the mutex was immediately available,
    /// `False` otherwise.  Does not block.
    fn lock_from_isr(&self) -> OsalRsBool {
        match self.inner.try_lock() {
            Ok(mut state) => {
                let current = std::thread::current().id();
                if state.owner == Some(current) {
                    state.recursion += 1;
                    OsalRsBool::True
                } else if state.owner.is_none() {
                    state.owner = Some(current);
                    state.recursion = 1;
                    OsalRsBool::True
                } else {
                    OsalRsBool::False
                }
            }
            Err(_) => OsalRsBool::False,
        }
    }

    /// Releases one level of recursion.
    ///
    /// If the recursion counter reaches zero the mutex is made
    /// available for other threads and one waiter is woken.
    fn unlock(&self) -> OsalRsBool {
        let mut state = self.inner.lock().unwrap();
        let current = std::thread::current().id();

        if state.owner != Some(current) || state.recursion == 0 {
            // Not owned by us, or already fully released.
            return OsalRsBool::False;
        }

        state.recursion -= 1;
        if state.recursion == 0 {
            state.owner = None;
            self.condvar.notify_one();
        }

        OsalRsBool::True
    }

    /// Releases the mutex from ISR context (non-blocking).
    ///
    /// Identical to `unlock()` on Linux since there is no distinct ISR
    /// context, but provided for API compatibility.
    fn unlock_from_isr(&self) -> OsalRsBool {
        match self.inner.try_lock() {
            Ok(mut state) => {
                let current = std::thread::current().id();
                if state.owner != Some(current) || state.recursion == 0 {
                    return OsalRsBool::False;
                }
                state.recursion -= 1;
                if state.recursion == 0 {
                    state.owner = None;
                    // We already hold the inner lock; unlock + notify.
                    drop(state);
                    self.condvar.notify_one();
                }
                OsalRsBool::True
            }
            Err(_) => OsalRsBool::False,
        }
    }

    /// Destroys the mutex and releases its resources.
    ///
    /// On Linux this is effectively a no-op since all memory is
    /// managed by Rust.  The inner state is consumed by dropping
    /// `self`.
    fn delete(&mut self) {
        // Resources are released when this `RawMutex` is dropped.
    }
}

// ---------------------------------------------------------------------------
// Mutex<T> — type-safe data wrapper
// ---------------------------------------------------------------------------

/// A mutual-exclusion primitive protecting shared data.
///
/// Wraps a [`RawMutex`] together with an `UnsafeCell`-protected value.
/// Provides RAII guards (`MutexGuard`, `MutexGuardFromIsr`) that
/// release the lock automatically on drop.
///
/// # Type Parameters
///
/// * `T` — The type of data protected by this mutex.
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::Mutex;
///
/// let mutex = Mutex::new(0u32);
/// {
///     let mut guard = mutex.lock().unwrap();
///     *guard += 1;
/// }
/// ```
pub struct Mutex<T: ?Sized> {
    inner: RawMutex,
    data: UnsafeCell<T>,
}

// Safety: Send + Sync when T is Send.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Creates a new mutex wrapping `data`.
    ///
    /// The mutex starts in the unlocked state.
    pub fn new(data: T) -> Self
    where
        T: Sized,
    {
        Self {
            inner: RawMutex::new().unwrap(),
            data: UnsafeCell::new(data),
        }
    }

    /// Creates a new mutex wrapped in an `Arc` for sharing.
    pub fn new_arc(data: T) -> Arc<Self> {
        Arc::new(Self::new(data))
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Acquires the mutex from ISR context, returning an
    /// ISR-specific guard type.
    ///
    /// On Linux this is a non-blocking try-lock.
    pub fn lock_from_isr_explicit(&self) -> Result<MutexGuardFromIsr<'_, T>> {
        match self.inner.lock_from_isr() {
            OsalRsBool::True => Ok(MutexGuardFromIsr {
                mutex: self,
                _phantom: PhantomData,
            }),
            OsalRsBool::False => Err(Error::MutexLockFailed),
        }
    }
}

impl<T: ?Sized> MutexFn<T> for Mutex<T> {
    type Guard<'a> = MutexGuard<'a, T> where Self: 'a, T: 'a;
    type GuardFromIsr<'a> = MutexGuardFromIsr<'a, T> where Self: 'a, T: 'a;

    /// Acquires the mutex, blocking until it becomes available.
    ///
    /// Returns a RAII guard that provides access to the protected data
    /// and releases the lock when dropped.
    fn lock(&self) -> Result<Self::Guard<'_>> {
        match self.inner.lock() {
            OsalRsBool::True => Ok(MutexGuard {
                mutex: self,
                _phantom: PhantomData,
            }),
            OsalRsBool::False => Err(Error::MutexLockFailed),
        }
    }

    /// Attempts to acquire the mutex without blocking (ISR-friendly).
    fn lock_from_isr(&self) -> Result<Self::GuardFromIsr<'_>> {
        self.lock_from_isr_explicit()
    }

    /// Consumes the mutex and returns the inner data.
    fn into_inner(self) -> Result<T>
    where
        Self: Sized,
        T: Sized,
    {
        Ok(self.data.into_inner())
    }

    /// Returns a mutable reference to the inner data.
    ///
    /// Since the caller holds `&mut self`, exclusive access is
    /// guaranteed at compile time — no locking is required.
    fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }
}

// Debug & Display
impl<T: ?Sized> Debug for Mutex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mutex").finish_non_exhaustive()
    }
}

impl<T: ?Sized> Display for Mutex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Mutex")
    }
}

// ---------------------------------------------------------------------------
// MutexGuard — RAII lock for normal context
// ---------------------------------------------------------------------------

/// RAII guard returned by [`Mutex::lock`].
///
/// Releases one recursion level of the underlying mutex when dropped.
pub struct MutexGuard<'a, T: ?Sized + 'a> {
    mutex: &'a Mutex<T>,
    _phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T: ?Sized> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.inner.unlock();
    }
}

impl<'a, T: ?Sized> MutexGuardFn<'a, T> for MutexGuard<'a, T> {
    fn update(&mut self, t: &T)
    where
        T: Clone,
    {
        **self = t.clone();
    }
}

// ---------------------------------------------------------------------------
// MutexGuardFromIsr — non-blocking ISR variant
// ---------------------------------------------------------------------------

/// RAII guard returned by [`Mutex::lock_from_isr`] and
/// [`Mutex::lock_from_isr_explicit`].
///
/// Uses ISR-safe unlock on drop.
pub struct MutexGuardFromIsr<'a, T: ?Sized + 'a> {
    mutex: &'a Mutex<T>,
    _phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Deref for MutexGuardFromIsr<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T: ?Sized> DerefMut for MutexGuardFromIsr<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T: ?Sized> Drop for MutexGuardFromIsr<'a, T> {
    fn drop(&mut self) {
        self.mutex.inner.unlock_from_isr();
    }
}

impl<'a, T: ?Sized> MutexGuardFn<'a, T> for MutexGuardFromIsr<'a, T> {
    fn update(&mut self, t: &T)
    where
        T: Clone,
    {
        **self = t.clone();
    }
}