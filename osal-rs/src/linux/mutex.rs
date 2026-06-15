//! Recursive mutex synchronization primitives for the Linux backend.
//!
//! # Overview
//!
//! Implements the `RawMutexFn` and `MutexFn` traits for the Linux backend.
//!
//! # Design
//!
//! - **RawMutex**: Low-level recursive mutex. Uses `StdMutex<State>`
//!   plus `Condvar`. Supports recursion—the same thread may call `lock()`
//!   multiple times. Each `lock()` increments a recursion counter; each
//!   `unlock()` decrements it. The mutex is released when the counter
//!   reaches zero.
//!
//! - **Mutex\<T\>**: High-level type-safe non-recursive mutex. Built
//!   on `std::sync::Mutex<T>` for data-protection and a separate
//!   `StdMutex<Option<ThreadId>>` to detect and reject recursive
//!   locking. Does **not** use `UnsafeCell`—`Deref`/`DerefMut` directly
//!   forward to the stdlib guard.
//!
//! - **Guard**: Guarantees `!Send` by holding a `ThreadId` field.
//!   `Drop` first clears the owner, then releases the stdlib guard.
//!
//! # Poison handling
//!
//! A helper `recover_lock()` transparently unpacks poisoned stdlib
//! mutexes so the OSAL synchronization primitive remains usable after
//! a panic in another thread. The recovered mutex's guarded data may
//! be inconsistent—the caller is responsible for higher-level
//! validation.

use core::fmt::{Debug, Display, Formatter};
use core::ops::{Deref, DerefMut};

use alloc::sync::Arc;
use std::sync::{Condvar, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::thread::ThreadId;

use crate::traits::{MutexGuardFn, MutexFn, RawMutexFn};
use crate::utils::{Error, OsalRsBool, Result};
use super::types::MutexHandle;

// ---------------------------------------------------------------------------
// Helper: recover from a poisoned std::sync::Mutex
// ---------------------------------------------------------------------------

/// Unpack a `LockResult`—if the mutex was poisoned, recover its inner
/// value so the synchronization primitive remains usable.  The caller
/// must be aware that the guarded data may be logically inconsistent.
fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ===========================================================================
// RawMutex — low-level recursive mutex
// ===========================================================================

/// Low-level recursive mutex.
///
/// Uses `StdMutex<State>` + `Condvar`. Same thread may lock multiple
/// times; each lock must be matched by an unlock.  No data reference is
/// exposed—only `OsalRsBool` return values.
pub struct RawMutex {
    inner: StdMutex<RawMutexState>,
    condvar: Condvar,
    /// Dummy handle for API surface compatibility (Deref target).
    handle: MutexHandle,
}

struct RawMutexState {
    owner: Option<ThreadId>,
    recursion: u32,
}

// Safety: StdMutex + Condvar are Send + Sync.
unsafe impl Send for RawMutex {}
unsafe impl Sync for RawMutex {}

impl Debug for RawMutex {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RawMutex").finish_non_exhaustive()
    }
}

impl Display for RawMutex {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "RawMutex")
    }
}

impl RawMutex {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: StdMutex::new(RawMutexState { owner: None, recursion: 0 }),
            condvar: Condvar::new(),
            handle: 1 as MutexHandle,
        })
    }
}

impl Deref for RawMutex {
    type Target = MutexHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl RawMutexFn for RawMutex {

    fn lock(&self) -> OsalRsBool {
        let mut state = recover_lock(self.inner.lock());
        let current = std::thread::current().id();

        if state.owner == Some(current) {
            state.recursion += 1;
            OsalRsBool::True
        } else {
            while state.owner.is_some() {
                state = recover_lock(self.condvar.wait(state));
            }
            state.owner = Some(current);
            state.recursion = 1;
            OsalRsBool::True
        }
    }

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

    fn unlock(&self) -> OsalRsBool {
        let mut state = recover_lock(self.inner.lock());
        let current = std::thread::current().id();
        if state.owner != Some(current) || state.recursion == 0 {
            return OsalRsBool::False;
        }
        state.recursion -= 1;
        if state.recursion == 0 {
            state.owner = None;
            self.condvar.notify_one();
        }
        OsalRsBool::True
    }

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
                    drop(state);
                    self.condvar.notify_one();
                }
                OsalRsBool::True
            }
            Err(_) => OsalRsBool::False,
        }
    }

    fn delete(&mut self) {}
}

// ===========================================================================
// Mutex<T> — high-level non-recursive typed mutex
// ===========================================================================

/// A non-recursive mutual-exclusion wrapper protecting data of type `T`.
///
/// Built on `std::sync::Mutex<T>` for data protection and a separate
/// `StdMutex<Option<ThreadId>>` to detect recursive lock attempts.
///
/// # Non-recursive
///
/// Calling `lock()` while already holding a guard returns
/// `Err(Error::MutexLockFailed)`.  Use [`RawMutex`] if recursion is
/// needed (but no data access is required).
pub struct Mutex<T: ?Sized> {
    /// Tracks which thread currently holds the typed lock.
    owner: StdMutex<Option<ThreadId>>,
    /// Actual mutual exclusion on `T` (boxed to support `?Sized`).
    data: Box<StdMutex<T>>,
    /// Dummy handle for API surface compatibility (Deref target).
    handle: MutexHandle,
}

// Safety: StdMutex provides actual mutual exclusion.
// handle is *const c_void (not Send+Sync), so we manually impl.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T: ?Sized> Deref for Mutex<T> {
    type Target = MutexHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl<T> Mutex<T> {
    pub fn new(data: T) -> Self {
        Self {
            owner: StdMutex::new(None),
            data: Box::new(StdMutex::new(data)),
            handle: 1 as MutexHandle,
        }
    }

    pub fn new_arc(data: T) -> Arc<Self> {
        Arc::new(Self::new(data))
    }
}

impl<T: ?Sized> Mutex<T> {

    /// ISR-specific lock alias (API surface compatibility).
    pub fn lock_from_isr_explicit(&self) -> Result<MutexGuardFromIsr<'_, T>> {
        self.lock_from_isr()
    }

    // -- internal helpers --------------------------------------------------

    /// Acquire the typed lock (blocking).  Returns an error if the
    /// current thread already holds the lock—recursive acquisition of
    /// the typed mutex is not permitted.
    fn lock_inner(&self) -> Result<MutexGuard<'_, T>> {
        let current = std::thread::current().id();

        // 1. Check / set owner — detect recursive locking.
        {
            let mut owner = recover_lock(self.owner.lock());
            if *owner == Some(current) {
                return Err(Error::MutexLockFailed);
            }
            // Do NOT set owner yet — we must hold the data lock first.
        }

        // 2. Acquire data lock.
        let data_guard = recover_lock(self.data.lock());

        // 3. Commit owner.
        {
            let mut owner = recover_lock(self.owner.lock());
            *owner = Some(current);
        }

        Ok(MutexGuard {
            owner: &self.owner,
            data_guard: Some(data_guard),
            _thread_id: current,
        })
    }

    /// Non-blocking try-lock (ISR emulation).
    fn lock_from_isr_inner(&self) -> Result<MutexGuardFromIsr<'_, T>> {
        let current = std::thread::current().id();

<<<<<<< HEAD
        // 1. Check and set owner (non-blocking).
=======
        // 1. Check and set owner atomically (non-blocking).
>>>>>>> c6c728dd23f4b5240085ef909d2bc69bce834bc0
        {
            let mut owner = match self.owner.try_lock() {
                Ok(o) => o,
                Err(_) => return Err(Error::MutexLockFailed),
            };
            // Reject if any thread (current or other) already holds it.
            if owner.is_some() {
                return Err(Error::MutexLockFailed);
            }
            *owner = Some(current);
        }

        // 2. Try-lock data.
        let data_guard = match self.data.try_lock() {
            Ok(g) => g,
            Err(_) => {
<<<<<<< HEAD
                // Rollback owner — use blocking lock() for reliability
                // since this is an error-recovery path not performance-critical.
                let mut owner = recover_lock(self.owner.lock());
                *owner = None;
=======
                // Rollback owner.
                if let Ok(mut o) = self.owner.try_lock() { *o = None; }
>>>>>>> c6c728dd23f4b5240085ef909d2bc69bce834bc0
                return Err(Error::MutexLockFailed);
            }
        };

        Ok(MutexGuardFromIsr {
            owner: &self.owner,
            data_guard: Some(data_guard),
            _thread_id: current,
        })
    }
}

impl<T: ?Sized> MutexFn<T> for Mutex<T> {
    type Guard<'a> = MutexGuard<'a, T> where Self: 'a, T: 'a;
    type GuardFromIsr<'a> = MutexGuardFromIsr<'a, T> where Self: 'a, T: 'a;

    fn lock(&self) -> Result<Self::Guard<'_>> {
        self.lock_inner()
    }

    fn lock_from_isr(&self) -> Result<Self::GuardFromIsr<'_>> {
        self.lock_from_isr_inner()
    }

    fn into_inner(self) -> Result<T>
    where Self: Sized, T: Sized,
    {
        Ok(recover_lock(self.data.into_inner()))
    }

    fn get_mut(&mut self) -> &mut T {
        recover_lock(self.data.get_mut())
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

// ===========================================================================
// MutexGuard — RAII lock for normal context
// ===========================================================================

/// RAII guard returned by [`Mutex::lock`].
///
/// Provides `Deref` / `DerefMut` access to `T`, and automatically
/// releases the lock on drop.  **Not `Send`** — the guard is tied to
/// the thread that acquired it.
pub struct MutexGuard<'a, T: ?Sized + 'a> {
    owner: &'a StdMutex<Option<ThreadId>>,
    data_guard: Option<StdMutexGuard<'a, T>>,
    /// Identifies the acquiring thread for debug diagnostics.
    /// The guard is `!Send` because it wraps `StdMutexGuard`.
    _thread_id: ThreadId,
}

impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.data_guard.as_deref().expect("guard already released")
    }
}

impl<'a, T: ?Sized> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data_guard.as_deref_mut().expect("guard already released")
    }
}

impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        // 1. Clear owner.
        {
            let mut owner = recover_lock(self.owner.lock());
            *owner = None;
        }
        // 2. Release data mutex.
        drop(self.data_guard.take());
    }
}

impl<'a, T: ?Sized> MutexGuardFn<'a, T> for MutexGuard<'a, T> {
    fn update(&mut self, t: &T) where T: Clone {
        **self = t.clone();
    }
}

// ===========================================================================
// MutexGuardFromIsr — non-blocking ISR variant
// ===========================================================================

/// RAII guard returned by [`Mutex::lock_from_isr`].
///
/// Drop reliably releases the lock (owner → data order).
/// `!Send` because it contains a `StdMutexGuard<'a, T>`.
pub struct MutexGuardFromIsr<'a, T: ?Sized + 'a> {
    owner: &'a StdMutex<Option<ThreadId>>,
    data_guard: Option<StdMutexGuard<'a, T>>,
    _thread_id: ThreadId,
}

impl<'a, T: ?Sized> Deref for MutexGuardFromIsr<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.data_guard.as_deref().expect("guard already released")
    }
}

impl<'a, T: ?Sized> DerefMut for MutexGuardFromIsr<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data_guard.as_deref_mut().expect("guard already released")
    }
}

impl<'a, T: ?Sized> Drop for MutexGuardFromIsr<'a, T> {
    fn drop(&mut self) {
        // 1. Clear owner.
        {
            let mut owner = recover_lock(self.owner.lock());
            *owner = None;
        }
        // 2. Release data mutex.
        drop(self.data_guard.take());
    }
}

impl<'a, T: ?Sized> MutexGuardFn<'a, T> for MutexGuardFromIsr<'a, T> {
    fn update(&mut self, t: &T) where T: Clone {
        **self = t.clone();
    }
}

// ===========================================================================
// Compile-time safety: guards must not be Send
// ===========================================================================
// The _thread_id: ThreadId field ensures that neither MutexGuard nor
// MutexGuardFromIsr implements Send (ThreadId is !Send on most
// platforms).  This is verified by static_assertions tests.