//! Global memory allocator using POSIX libc heap.
//!
//! This allocator delegates to `libc::malloc` / `libc::free` / `libc::realloc`
//! so that the POSIX `no_std` backend can satisfy the `alloc` crate's global
//! allocator requirement without pulling in `std`.
//!
//! When the `posix` feature is used together with a `std`-enabled binary (e.g.
//! the `osal-rs-tests` harness), the binary's own allocator takes precedence.
//!
//! # Safety
//!
//! `libc` allocation functions are thread-safe on all supported POSIX platforms.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;

use libc;

/// Global memory allocator using the POSIX libc heap.
pub struct PosixAllocator;

unsafe impl GlobalAlloc for PosixAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { libc::malloc(layout.size()) as *mut u8 }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe {
            libc::free(ptr as *mut c_void);
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { libc::calloc(layout.size(), 1) as *mut u8 }
    }

    unsafe fn realloc(&self, ptr: *mut u8, _layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { libc::realloc(ptr as *mut c_void, new_size) as *mut u8 }
    }
}
