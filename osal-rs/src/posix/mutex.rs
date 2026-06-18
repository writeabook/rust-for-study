/***************************************************************************
 *
 * osal-rs
 * Copyright (C) 2026 Antonio Salsi <passy.linux@zresa.it>
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <https://www.gnu.org/licenses/>.
 *
 ***************************************************************************/

//! Native POSIX mutex implementation using `pthread_mutex_t`.
//!
//! # Design
//!
//! Unlike the Linux backend (which uses `std::sync::Mutex` + `Condvar`
//! with manual recursion tracking), this module delegates to the kernel's
//! `pthread_mutex_t` for both recursive and error-checking semantics:
//!
//! - **RawMutex** — `PTHREAD_MUTEX_RECURSIVE`, allowing the same thread
//!   to lock the mutex multiple times without deadlocking.
//! - **Mutex\<T\>** — `PTHREAD_MUTEX_ERRORCHECK`, detecting and rejecting
//!   recursive locking by returning `Error::MutexLockFailed`.
//!
//! # ISR path
//!
//! `lock_from_isr` / `unlock_from_isr` use `pthread_mutex_trylock` so
//! they never block, matching the expected ISR semantics for host testing.
//!
//! # Future work
//!
//! - `PTHREAD_PRIO_INHERIT` for real priority-inheritance behaviour.

use core::cell::UnsafeCell;
use core::fmt::{Debug, Display, Formatter};
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr;

use alloc::boxed::Box;
use alloc::sync::Arc;

use super::ffi::{self, PTHREAD_MUTEX_ERRORCHECK, PTHREAD_MUTEX_RECURSIVE, PthreadMutex};
use super::types::MutexHandle;
use crate::traits::{MutexFn, MutexGuardFn, RawMutexFn};
use crate::utils::{Error, OsalRsBool, Result};

// ---------------------------------------------------------------------------
// RawMutex — recursive, PTHREAD_MUTEX_RECURSIVE
// ---------------------------------------------------------------------------

pub struct RawMutex {
    mtx: *mut PthreadMutex,
    handle: MutexHandle,
}

unsafe impl Send for RawMutex {}
unsafe impl Sync for RawMutex {}

impl RawMutex {
    pub fn new() -> Result<Self> {
        let mtx = ffi::create_mutex(PTHREAD_MUTEX_RECURSIVE)
            .ok_or(Error::OutOfMemory)?;
        Ok(Self { mtx, handle: mtx as MutexHandle })
    }
}

impl RawMutexFn for RawMutex {
    fn lock(&self) -> OsalRsBool {
        if unsafe { ffi::pthread_mutex_lock(self.mtx) } == 0 {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    fn lock_from_isr(&self) -> OsalRsBool {
        if unsafe { ffi::pthread_mutex_trylock(self.mtx) } == 0 {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    fn unlock(&self) -> OsalRsBool {
        if unsafe { ffi::pthread_mutex_unlock(self.mtx) } == 0 {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    fn unlock_from_isr(&self) -> OsalRsBool {
        if unsafe { ffi::pthread_mutex_unlock(self.mtx) } == 0 {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    fn delete(&mut self) {
        if !self.mtx.is_null() {
            unsafe { ffi::destroy_mutex(self.mtx) };
            self.mtx = core::ptr::null_mut();
        }
    }
}

impl Drop for RawMutex {
    fn drop(&mut self) {
        if !self.mtx.is_null() {
            unsafe { ffi::destroy_mutex(self.mtx) };
        }
    }
}

impl Deref for RawMutex {
    type Target = MutexHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl Debug for RawMutex {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RawMutex")
            .field("mtx", &self.mtx)
            .finish()
    }
}

impl Display for RawMutex {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "RawMutex {{ mtx: {:?} }}", self.mtx)
    }
}

// ===========================================================================
// Mutex<T> — non-recursive, PTHREAD_MUTEX_ERRORCHECK
// ===========================================================================

pub struct Mutex<T: ?Sized> {
    mtx: *mut PthreadMutex,
    data: UnsafeCell<Box<T>>,
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
        let mtx = ffi::create_mutex(PTHREAD_MUTEX_ERRORCHECK)
            .expect("POSIX Mutex<T>: failed to create pthread_mutex_t");
        Self { mtx, data: UnsafeCell::new(Box::new(data)), handle: mtx as MutexHandle }
    }

    pub fn new_arc(data: T) -> Arc<Self> {
        Arc::new(Self::new(data))
    }
}

impl<T: ?Sized> Mutex<T> {
    pub fn lock_from_isr_explicit(&self) -> Result<MutexGuardFromIsr<'_, T>> {
        self.lock_from_isr()
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, T>> {
        let ret = unsafe { ffi::pthread_mutex_lock(self.mtx) };
        if ret == 0 {
            Ok(MutexGuard { mutex: self, _phantom: PhantomData })
        } else {
            Err(Error::MutexLockFailed)
        }
    }

    fn lock_from_isr_inner(&self) -> Result<MutexGuardFromIsr<'_, T>> {
        let ret = unsafe { ffi::pthread_mutex_trylock(self.mtx) };
        if ret == 0 {
            Ok(MutexGuardFromIsr { mutex: self, _phantom: PhantomData })
        } else {
            Err(Error::MutexLockFailed)
        }
    }
}

impl<T: ?Sized> MutexFn<T> for Mutex<T> {
    type Guard<'a> = MutexGuard<'a, T> where Self: 'a, T: 'a;
    type GuardFromIsr<'a> = MutexGuardFromIsr<'a, T> where Self: 'a, T: 'a;

    fn lock(&self) -> Result<Self::Guard<'_>> { self.lock_inner() }
    fn lock_from_isr(&self) -> Result<Self::GuardFromIsr<'_>> { self.lock_from_isr_inner() }

    fn into_inner(self) -> Result<T>
    where
        Self: Sized,
        T: Sized,
    {
        let mtx = self.mtx;
        // Extract the Box<T> before forget to avoid dropping it
        let data_ptr: *mut Box<T> = self.data.get();
        // Prevent Drop from running (would double-destroy mtx and drop Box<T>)
        core::mem::forget(self);
        // Read the Box<T> — we now own it
        let boxed = unsafe { ptr::read(data_ptr) };
        unsafe { ffi::destroy_mutex(mtx) };
        Ok(*boxed)
    }

    fn get_mut(&mut self) -> &mut T {
        self.data.get_mut().as_mut()
    }
}

impl<T: ?Sized> Drop for Mutex<T> {
    fn drop(&mut self) {
        if !self.mtx.is_null() {
            unsafe { ffi::destroy_mutex(self.mtx) };
        }
    }
}

impl<T: ?Sized> Debug for Mutex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mutex")
            .field("mtx", &self.mtx)
            .field("handle", &self.handle)
            .finish()
    }
}

impl<T: ?Sized> Display for Mutex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Mutex {{ mtx: {:?} }}", self.mtx)
    }
}

// ===========================================================================
// MutexGuard — RAII wrapper, drops the lock on Drop
// ===========================================================================

pub struct MutexGuard<'a, T: ?Sized + 'a> {
    mutex: &'a Mutex<T>,
    _phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }.as_ref()
    }
}

impl<'a, T: ?Sized> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }.as_mut()
    }
}

impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        unsafe { ffi::pthread_mutex_unlock(self.mutex.mtx) };
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

// ===========================================================================
// MutexGuardFromIsr — ISR-safe RAII wrapper
// ===========================================================================

pub struct MutexGuardFromIsr<'a, T: ?Sized + 'a> {
    mutex: &'a Mutex<T>,
    _phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Deref for MutexGuardFromIsr<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }.as_ref()
    }
}

impl<'a, T: ?Sized> DerefMut for MutexGuardFromIsr<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }.as_mut()
    }
}

impl<'a, T: ?Sized> Drop for MutexGuardFromIsr<'a, T> {
    fn drop(&mut self) {
        unsafe { ffi::pthread_mutex_unlock(self.mutex.mtx) };
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
