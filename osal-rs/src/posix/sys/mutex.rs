//! Safe wrapper around `libc::pthread_mutex_t`.

use core::cell::UnsafeCell;
use libc::{PTHREAD_MUTEX_ERRORCHECK, PTHREAD_MUTEX_RECURSIVE};
use libc::{
    pthread_mutex_destroy, pthread_mutex_init, pthread_mutex_lock, pthread_mutex_trylock,
    pthread_mutex_unlock,
};
use libc::{
    pthread_mutex_t, pthread_mutexattr_destroy, pthread_mutexattr_init, pthread_mutexattr_settype,
    pthread_mutexattr_t,
};

pub struct PosixMutex {
    raw: UnsafeCell<pthread_mutex_t>,
}

unsafe impl Send for PosixMutex {}
unsafe impl Sync for PosixMutex {}

impl PosixMutex {
    pub fn new(kind: i32) -> Option<Self> {
        let mut attr: pthread_mutexattr_t = unsafe { core::mem::zeroed() };
        if unsafe { pthread_mutexattr_init(&mut attr) } != 0 {
            return None;
        }
        if unsafe { pthread_mutexattr_settype(&mut attr, kind) } != 0 {
            unsafe { pthread_mutexattr_destroy(&mut attr) };
            return None;
        }
        let mut mtx: pthread_mutex_t = unsafe { core::mem::zeroed() };
        let ret = unsafe { pthread_mutex_init(&mut mtx, &attr) };
        unsafe { pthread_mutexattr_destroy(&mut attr) };
        if ret == 0 {
            Some(Self {
                raw: UnsafeCell::new(mtx),
            })
        } else {
            None
        }
    }

    pub fn lock(&self) -> bool {
        (unsafe { pthread_mutex_lock(self.raw.get()) }) == 0
    }

    pub fn try_lock(&self) -> bool {
        (unsafe { pthread_mutex_trylock(self.raw.get()) }) == 0
    }

    pub fn unlock(&self) -> bool {
        (unsafe { pthread_mutex_unlock(self.raw.get()) }) == 0
    }

    pub fn raw_ptr(&self) -> *mut pthread_mutex_t {
        self.raw.get()
    }
}

impl Drop for PosixMutex {
    fn drop(&mut self) {
        unsafe { pthread_mutex_destroy(self.raw.get()) };
    }
}
