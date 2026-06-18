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

//! Minimal POSIX pthread FFI bindings.
//!
//! Provides the `pthread_mutex_*` family needed by the POSIX backend mutex
//! module without pulling in the `libc` crate.

use core::alloc::Layout;
use core::ptr;

// ---------------------------------------------------------------------------
// Opaque pthread_mutex_t — sized for the largest common platform (64 bytes)
// ---------------------------------------------------------------------------

/// Opaque `pthread_mutex_t` storage.
///
/// Sized to 64 bytes, which covers all common platforms:
/// - Linux x86_64:   40 bytes
/// - Linux aarch64:  48 bytes
/// - macOS x86_64:   64 bytes
/// - macOS aarch64:  64 bytes
#[repr(C, align(8))]
pub(crate) struct PthreadMutex {
    _opaque: [u8; 64],
}

// ---------------------------------------------------------------------------
// Attribute constants
// ---------------------------------------------------------------------------

/// `PTHREAD_MUTEX_RECURSIVE` — allows the same thread to lock the mutex
/// multiple times without deadlocking.
pub(crate) const PTHREAD_MUTEX_RECURSIVE: i32 = 1;   // Linux; also works on macOS via NP

/// `PTHREAD_MUTEX_ERRORCHECK` — returns `EDEADLK` if the same thread
/// attempts to lock a mutex it already holds.
pub(crate) const PTHREAD_MUTEX_ERRORCHECK: i32 = 2;

// ---------------------------------------------------------------------------
// FFI declarations
// ---------------------------------------------------------------------------

unsafe extern "C" {
    /// `int pthread_mutex_init(pthread_mutex_t *m, const pthread_mutexattr_t *attr);`
    pub(crate) fn pthread_mutex_init(m: *mut PthreadMutex, attr: *const ()) -> i32;

    /// `int pthread_mutex_destroy(pthread_mutex_t *m);`
    pub(crate) fn pthread_mutex_destroy(m: *mut PthreadMutex) -> i32;

    /// `int pthread_mutex_lock(pthread_mutex_t *m);`
    pub(crate) fn pthread_mutex_lock(m: *mut PthreadMutex) -> i32;

    /// `int pthread_mutex_trylock(pthread_mutex_t *m);`
    pub(crate) fn pthread_mutex_trylock(m: *mut PthreadMutex) -> i32;

    /// `int pthread_mutex_unlock(pthread_mutex_t *m);`
    pub(crate) fn pthread_mutex_unlock(m: *mut PthreadMutex) -> i32;

    /// `int pthread_mutexattr_init(pthread_mutexattr_t *attr);`
    pub(crate) fn pthread_mutexattr_init(attr: *mut PthreadMutexAttr) -> i32;

    /// `int pthread_mutexattr_destroy(pthread_mutexattr_t *attr);`
    pub(crate) fn pthread_mutexattr_destroy(attr: *mut PthreadMutexAttr) -> i32;

    /// `int pthread_mutexattr_settype(pthread_mutexattr_t *attr, int kind);`
    pub(crate) fn pthread_mutexattr_settype(attr: *mut PthreadMutexAttr, kind: i32) -> i32;
}

// ---------------------------------------------------------------------------
// Opaque pthread_mutexattr_t — 8 bytes on most platforms
// ---------------------------------------------------------------------------

#[repr(C, align(8))]
pub(crate) struct PthreadMutexAttr {
    _opaque: [u8; 8],
}

// ---------------------------------------------------------------------------
// Heap allocation + init helpers
// ---------------------------------------------------------------------------

/// Allocates and initialises a `PthreadMutex` with the given type attribute.
///
/// Returns a pointer to a heap-allocated, initialised mutex, or `None` if
/// allocation or initialisation fails.
pub(crate) fn create_mutex(mutex_type: i32) -> Option<*mut PthreadMutex> {
    let layout = Layout::new::<PthreadMutex>();
    // SAFETY: layout has non-zero size; allocation may fail
    let ptr = unsafe { alloc::alloc::alloc(layout) as *mut PthreadMutex };
    if ptr.is_null() {
        return None;
    }

    let mut attr_ptr: *mut PthreadMutexAttr = ptr::null_mut();

    let mut attr: PthreadMutexAttr = PthreadMutexAttr { _opaque: [0u8; 8] };
    let attr_init_ret = unsafe { pthread_mutexattr_init(&mut attr) };
    if attr_init_ret == 0 {
        let settype_ret = unsafe { pthread_mutexattr_settype(&mut attr, mutex_type) };
        if settype_ret == 0 {
            attr_ptr = &mut attr;
        }
    }

    let init_ret = unsafe { pthread_mutex_init(ptr, attr_ptr as *const ()) };

    if attr_ptr != ptr::null_mut() {
        unsafe { pthread_mutexattr_destroy(&mut attr) };
    }

    if init_ret == 0 {
        Some(ptr)
    } else {
        unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) };
        None
    }
}

/// Destroys and deallocates a `PthreadMutex`.
pub(crate) unsafe fn destroy_mutex(ptr: *mut PthreadMutex) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        pthread_mutex_destroy(ptr);
        alloc::alloc::dealloc(ptr as *mut u8, Layout::new::<PthreadMutex>());
    }
}

// ===========================================================================
// POSIX semaphore (sem_t) bindings
// ===========================================================================

/// Opaque `sem_t` storage.
///
/// Sized to 48 bytes to cover all common platforms
/// (Linux x86_64/aarch64: 32 bytes, macOS: varies).
#[repr(C, align(8))]
pub(crate) struct SemT {
    _opaque: [u8; 48],
}

/// `struct timespec` for `sem_timedwait`.
#[repr(C)]
pub(crate) struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

unsafe extern "C" {
    /// `int sem_init(sem_t *sem, int pshared, unsigned int value);`
    pub(crate) fn sem_init(sem: *mut SemT, pshared: i32, value: u32) -> i32;

    /// `int sem_destroy(sem_t *sem);`
    pub(crate) fn sem_destroy(sem: *mut SemT) -> i32;

    /// `int sem_wait(sem_t *sem);`
    pub(crate) fn sem_wait(sem: *mut SemT) -> i32;

    /// `int sem_trywait(sem_t *sem);`
    pub(crate) fn sem_trywait(sem: *mut SemT) -> i32;

    /// `int sem_timedwait(sem_t *sem, const struct timespec *abs_timeout);`
    pub(crate) fn sem_timedwait(sem: *mut SemT, abs_timeout: *const Timespec) -> i32;

    /// `int sem_post(sem_t *sem);`
    pub(crate) fn sem_post(sem: *mut SemT) -> i32;

    /// `int sem_getvalue(sem_t *sem, int *sval);`
    pub(crate) fn sem_getvalue(sem: *mut SemT, sval: *mut i32) -> i32;
}

/// Allocates and initialises a `sem_t` with the given initial value.
///
/// Uses `sem_init` (process-local, `pshared = 0`).
pub(crate) fn create_sem(initial_value: u32) -> Option<*mut SemT> {
    let layout = Layout::new::<SemT>();
    let ptr = unsafe { alloc::alloc::alloc(layout) as *mut SemT };
    if ptr.is_null() {
        return None;
    }
    let ret = unsafe { sem_init(ptr, 0, initial_value) };
    if ret == 0 {
        Some(ptr)
    } else {
        unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) };
        None
    }
}

/// Destroys and deallocates a `sem_t`.
pub(crate) unsafe fn destroy_sem(ptr: *mut SemT) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        sem_destroy(ptr);
        alloc::alloc::dealloc(ptr as *mut u8, Layout::new::<SemT>());
    }
}
