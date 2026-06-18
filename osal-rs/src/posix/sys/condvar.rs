//! Safe wrapper around `libc::pthread_cond_t` with CLOCK_MONOTONIC.

use core::cell::UnsafeCell;
use libc::{pthread_cond_init, pthread_cond_destroy, pthread_cond_wait, pthread_cond_timedwait, pthread_cond_signal, pthread_cond_broadcast};
use libc::{pthread_cond_t, pthread_condattr_t, pthread_condattr_init, pthread_condattr_setclock, pthread_condattr_destroy};
use libc::{pthread_mutex_t, timespec, CLOCK_MONOTONIC};

pub struct PosixCondvar {
    raw: UnsafeCell<pthread_cond_t>,
}

unsafe impl Send for PosixCondvar {}
unsafe impl Sync for PosixCondvar {}

impl PosixCondvar {
    pub fn new() -> Option<Self> {
        let mut attr: pthread_condattr_t = unsafe { core::mem::zeroed() };
        if unsafe { pthread_condattr_init(&mut attr) } != 0 { return None; }
        if unsafe { pthread_condattr_setclock(&mut attr, CLOCK_MONOTONIC) } != 0 {
            unsafe { pthread_condattr_destroy(&mut attr) };
            return None;
        }
        let mut cond: pthread_cond_t = unsafe { core::mem::zeroed() };
        let ret = unsafe { pthread_cond_init(&mut cond, &attr) };
        unsafe { pthread_condattr_destroy(&mut attr) };
        if ret == 0 { Some(Self { raw: UnsafeCell::new(cond) }) } else { None }
    }

    pub fn wait(&self, mtx: &super::mutex::PosixMutex) {
        unsafe { pthread_cond_wait(self.raw.get(), mtx.raw_ptr()) };
    }

    pub fn timedwait(&self, mtx: &super::mutex::PosixMutex, deadline: &timespec) -> bool {
        (unsafe { pthread_cond_timedwait(self.raw.get(), mtx.raw_ptr(), deadline) }) == 0
    }

    pub fn signal(&self) {
        unsafe { pthread_cond_signal(self.raw.get()) };
    }

    pub fn broadcast(&self) {
        unsafe { pthread_cond_broadcast(self.raw.get()) };
    }
}

impl Drop for PosixCondvar {
    fn drop(&mut self) {
        unsafe { pthread_cond_destroy(self.raw.get()) };
    }
}
