/***************************************************************************
 *                                                                         *
 * osal-rs                                                                 *
 * Copyright (C) 2026 Antonio Salsi <passy.linux@zresa.it>                  *
 *                                                                         *
 * This library is free software; you can redistribute it and/or            *
 * modify it under the terms of the GNU Lesser General Public               *
 * License as published by the Free Software Foundation; either             *
 * version 2.1 of the License, or (at your option) any later version.       *
 *                                                                         *
 * This library is distributed in the hope that it will be useful,          *
 * but WITHOUT ANY WARRANTY; without even the implied warranty of           *
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU         *
 * Lesser General Public License for more details.                          *
 *                                                                         *
 * You should have received a copy of the GNU Lesser General Public         *
 * License along with this library; if not, see <https://www.gnu.org/licenses/>.
 *                                                                         *
 ***************************************************************************/

//! Thin wrappers around `libc::pthread_*` for the POSIX backend.
//!
//! These functions isolate the `unsafe` FFI calls.  Higher-level modules
//! (`posix/thread.rs`) should use this module and avoid calling `libc`
//! directly.

use core::ffi::c_void;

use libc::{
    pthread_attr_destroy, pthread_attr_init, pthread_attr_setstacksize, pthread_create,
    pthread_detach, pthread_equal, pthread_getspecific, pthread_join, pthread_key_create,
    pthread_key_delete, pthread_key_t, pthread_self, pthread_setspecific, pthread_t,
};

// ---------------------------------------------------------------------------
// Thread handle
// ---------------------------------------------------------------------------

/// Opaque POSIX thread handle.
pub type PosixThread = pthread_t;

/// Creates a new thread.
///
/// # Safety
///
/// `entry` and `arg` must be valid for the lifetime of the created thread.
#[inline]
pub unsafe fn create(
    stack_size: Option<usize>,
    entry: extern "C" fn(*mut c_void) -> *mut c_void,
    arg: *mut c_void,
) -> Option<PosixThread> {
    let mut attr: libc::pthread_attr_t = core::mem::zeroed();

    if libc::pthread_attr_init(&mut attr) != 0 {
        return None;
    }

    if let Some(sz) = stack_size {
        if sz > 0 {
            // Clamp to PTHREAD_STACK_MIN so that the OSAL contract
            // (stack_depth is advisory, not a hard guarantee) still
            // holds when tests use artificially small stack values.
            let clamped = sz.max(libc::PTHREAD_STACK_MIN as usize);
            if libc::pthread_attr_setstacksize(&mut attr, clamped) != 0 {
                libc::pthread_attr_destroy(&mut attr);
                return None;
            }
        }
    }

    let mut thread: pthread_t = core::mem::zeroed();

    let ret = libc::pthread_create(&mut thread, &attr, entry, arg);

    libc::pthread_attr_destroy(&mut attr);

    if ret == 0 { Some(thread) } else { None }
}

/// Waits for `thread` to terminate.
///
/// Returns `true` on success.
#[inline]
pub unsafe fn join(thread: PosixThread) -> bool {
    libc::pthread_join(thread, core::ptr::null_mut()) == 0
}

/// Detaches a POSIX thread.
///
/// Detached threads cannot be joined and their resources are automatically
/// reclaimed on exit.  This is suitable for daemon-like service threads
/// whose lifetime is tied to the process.
///
/// Returns `true` on success.
#[inline]
pub unsafe fn detach(thread: PosixThread) -> bool {
    pthread_detach(thread) == 0
}

/// Returns the calling thread's handle.
#[inline]
pub fn current() -> PosixThread {
    unsafe { pthread_self() }
}

/// Compares two thread handles for equality.
#[inline]
pub fn equal(a: PosixThread, b: PosixThread) -> bool {
    unsafe { pthread_equal(a, b) != 0 }
}

// ---------------------------------------------------------------------------
// TLS key
// ---------------------------------------------------------------------------

/// Creates a thread-local storage key.
///
/// The associated destructor (if any) is called with the stored value when a
/// thread exits.  Pass `None` if no cleanup is needed.
#[inline]
pub fn key_create(destructor: Option<unsafe extern "C" fn(*mut c_void)>) -> Option<pthread_key_t> {
    let mut key: pthread_key_t = 0;

    if unsafe { pthread_key_create(&mut key, destructor) } == 0 {
        Some(key)
    } else {
        None
    }
}

/// Deletes a TLS key.
///
/// Returns `true` on success.
#[inline]
pub unsafe fn key_delete(key: pthread_key_t) -> bool {
    pthread_key_delete(key) == 0
}

/// Reads the value associated with a TLS key for the calling thread.
#[inline]
pub unsafe fn key_get(key: pthread_key_t) -> *mut c_void {
    pthread_getspecific(key)
}

/// Stores a value for a TLS key for the calling thread.
///
/// Returns `true` on success.
#[inline]
pub unsafe fn key_set(key: pthread_key_t, value: *mut c_void) -> bool {
    pthread_setspecific(key, value) == 0
}
