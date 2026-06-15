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
//! - **MutexGuard / MutexGuardFromIsr**: `!Send` because they wrap
//!   `StdMutexGuard`. `Drop` first clears the owner, then releases
//!   the stdlib guard (reliable, no `RawMutex::unlock` involved).
//!
//! # ISR path (host simulation only)
//!
//! Linux has no real interrupt context. `lock_from_isr` / `unlock_from_isr`
//! simulate immediate-acquisition behaviour by using `try_lock` so they
//! never block. The guard-release path may briefly acquire the internal
//! `owner` management lock—this is acceptable for host testing but does
//! **not** guarantee hard‑real‑time ISR semantics.
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
use std::sync::{Condvar, Mutex as StdMutex, MutexGuard as StdMutexGuard, TryLockError};
use std::thread::ThreadId;

use crate::traits::{MutexGuardFn, MutexFn, RawMutexFn};
use crate::utils::{Error, OsalRsBool, Result};
use super::types::MutexHandle;

// ---------------------------------------------------------------------------
// Helpers: recover from a poisoned std::sync::Mutex
// ---------------------------------------------------------------------------

fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ===========================================================================
// RawMutex — low-level recursive mutex
// ===========================================================================

pub struct RawMutex {
    inner: StdMutex<RawMutexState>,
    condvar: Condvar,
    handle: MutexHandle,
}

struct RawMutexState {
    owner: Option<ThreadId>,
    recursion: u32,
}

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
        Ok(Self { inner: StdMutex::new(RawMutexState { owner: None, recursion: 0 }), condvar: Condvar::new(), handle: 1 as MutexHandle })
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
            state.recursion = state.recursion.saturating_add(1);
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
        let mut guard = match self.inner.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return OsalRsBool::False,
        };
        let current = std::thread::current().id();
        if guard.owner == Some(current) {
            guard.recursion = guard.recursion.saturating_add(1);
            OsalRsBool::True
        } else if guard.owner.is_none() {
            guard.owner = Some(current);
            guard.recursion = 1;
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    fn unlock(&self) -> OsalRsBool {
        let mut state = recover_lock(self.inner.lock());
        let current = std::thread::current().id();
        if state.owner != Some(current) || state.recursion == 0 { return OsalRsBool::False; }
        state.recursion -= 1;
        if state.recursion == 0 { state.owner = None; self.condvar.notify_one(); }
        OsalRsBool::True
    }

    fn unlock_from_isr(&self) -> OsalRsBool {
        let mut guard = match self.inner.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return OsalRsBool::False,
        };
        let current = std::thread::current().id();
        if guard.owner != Some(current) || guard.recursion == 0 { return OsalRsBool::False; }
        guard.recursion -= 1;
        if guard.recursion == 0 {
            guard.owner = None;
            drop(guard);
            self.condvar.notify_one();
        }
        OsalRsBool::True
    }

    fn delete(&mut self) {}
}

// ===========================================================================
// Mutex<T> — high-level non-recursive typed mutex
// ===========================================================================

pub struct Mutex<T: ?Sized> {
    owner: StdMutex<Option<ThreadId>>,
    data: Box<StdMutex<T>>,
    handle: MutexHandle,
}

unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T: ?Sized> Deref for Mutex<T> {
    type Target = MutexHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl<T> Mutex<T> {
    pub fn new(data: T) -> Self {
        Self { owner: StdMutex::new(None), data: Box::new(StdMutex::new(data)), handle: 1 as MutexHandle }
    }
    pub fn new_arc(data: T) -> Arc<Self> { Arc::new(Self::new(data)) }
}

impl<T: ?Sized> Mutex<T> {

    pub fn lock_from_isr_explicit(&self) -> Result<MutexGuardFromIsr<'_, T>> {
        self.lock_from_isr()
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, T>> {
        let current = std::thread::current().id();
        {
            let owner = recover_lock(self.owner.lock());
            if *owner == Some(current) { return Err(Error::MutexLockFailed); }
        }
        let data_guard = recover_lock(self.data.lock());
        {
            let mut owner = recover_lock(self.owner.lock());
            *owner = Some(current);
        }
        Ok(MutexGuard { owner: &self.owner, data_guard: Some(data_guard), _thread_id: current })
    }

    /// Non-blocking try-lock (ISR emulation).
    ///
    /// Acquires the data lock first, then sets the owner — no rollback
    /// is needed because the owner is only committed once the data lock
    /// is held.  If the owner is already occupied, the data lock is
    /// released immediately.
    fn lock_from_isr_inner(&self) -> Result<MutexGuardFromIsr<'_, T>> {
        let current = std::thread::current().id();

        // 1. Try-lock data FIRST (non-blocking).
        let data_guard = match self.data.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(Error::MutexLockFailed),
        };

        // 2. Commit owner only after the data lock is held.
        {
            let mut owner = match self.owner.try_lock() {
                Ok(o) => o,
                Err(TryLockError::Poisoned(p)) => p.into_inner(),
                Err(TryLockError::WouldBlock) => { drop(data_guard); return Err(Error::MutexLockFailed); }
            };
            if owner.is_some() { drop(owner); drop(data_guard); return Err(Error::MutexLockFailed); }
            *owner = Some(current);
        }

        Ok(MutexGuardFromIsr { owner: &self.owner, data_guard: Some(data_guard), _thread_id: current })
    }
}

impl<T: ?Sized> MutexFn<T> for Mutex<T> {
    type Guard<'a> = MutexGuard<'a, T> where Self: 'a, T: 'a;
    type GuardFromIsr<'a> = MutexGuardFromIsr<'a, T> where Self: 'a, T: 'a;

    fn lock(&self) -> Result<Self::Guard<'_>> { self.lock_inner() }
    fn lock_from_isr(&self) -> Result<Self::GuardFromIsr<'_>> { self.lock_from_isr_inner() }

    fn into_inner(self) -> Result<T> where Self: Sized, T: Sized { Ok(recover_lock(self.data.into_inner())) }
    fn get_mut(&mut self) -> &mut T { recover_lock(self.data.get_mut()) }
}

impl<T: ?Sized> Debug for Mutex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result { f.debug_struct("Mutex").finish_non_exhaustive() }
}
impl<T: ?Sized> Display for Mutex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result { write!(f, "Mutex") }
}

// ===========================================================================
// MutexGuard
// ===========================================================================

pub struct MutexGuard<'a, T: ?Sized + 'a> {
    owner: &'a StdMutex<Option<ThreadId>>,
    data_guard: Option<StdMutexGuard<'a, T>>,
    _thread_id: ThreadId,
}

impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T { self.data_guard.as_deref().expect("guard already released") }
}
impl<'a, T: ?Sized> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T { self.data_guard.as_deref_mut().expect("guard already released") }
}
impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        { let mut owner = recover_lock(self.owner.lock()); *owner = None; }
        drop(self.data_guard.take());
    }
}
impl<'a, T: ?Sized> MutexGuardFn<'a, T> for MutexGuard<'a, T> {
    fn update(&mut self, t: &T) where T: Clone { **self = t.clone(); }
}

// ===========================================================================
// MutexGuardFromIsr
// ===========================================================================

pub struct MutexGuardFromIsr<'a, T: ?Sized + 'a> {
    owner: &'a StdMutex<Option<ThreadId>>,
    data_guard: Option<StdMutexGuard<'a, T>>,
    _thread_id: ThreadId,
}

impl<'a, T: ?Sized> Deref for MutexGuardFromIsr<'a, T> {
    type Target = T;
    fn deref(&self) -> &T { self.data_guard.as_deref().expect("guard already released") }
}
impl<'a, T: ?Sized> DerefMut for MutexGuardFromIsr<'a, T> {
    fn deref_mut(&mut self) -> &mut T { self.data_guard.as_deref_mut().expect("guard already released") }
}
impl<'a, T: ?Sized> Drop for MutexGuardFromIsr<'a, T> {
    fn drop(&mut self) {
        { let mut owner = recover_lock(self.owner.lock()); *owner = None; }
        drop(self.data_guard.take());
    }
}
impl<'a, T: ?Sized> MutexGuardFn<'a, T> for MutexGuardFromIsr<'a, T> {
    fn update(&mut self, t: &T) where T: Clone { **self = t.clone(); }
}

// ===========================================================================
// MutexGuard and MutexGuardFromIsr are `!Send` because they contain
// `std::sync::MutexGuard`, whose ownership must be released on the
// acquiring thread.
//
// `_thread_id` is retained for diagnostics (not required for safety).
// ===========================================================================
