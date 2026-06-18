/***************************************************************************
 *
 * osal-rs
 * Copyright (C) 2026 Antonio Salsi <passy.linux@zresa.it>
 *
 * ... LGPL-2.1 header ...
 *
 ***************************************************************************/

//! Counting semaphore using `pthread_mutex_t` + `pthread_cond_t` + count.
//! Replaces `sem_t` — solves max_count race and CLOCK_REALTIME dependency.

use core::cell::UnsafeCell;
use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;

use super::sys::clock;
use super::sys::condvar::PosixCondvar;
use super::sys::mutex::PosixMutex;
use super::types::{SemaphoreHandle, UBaseType};
use crate::traits::SemaphoreFn;
use crate::traits::ToTick;
use crate::utils::{Error, OsalRsBool, Result};

use libc::PTHREAD_MUTEX_ERRORCHECK;

pub struct Semaphore {
    mtx: PosixMutex,
    cond: PosixCondvar,
    count: UnsafeCell<u32>,
    max_count: u32,
    handle: SemaphoreHandle,
}

unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}

impl Semaphore {
    pub fn new(max_count: UBaseType, initial_count: UBaseType) -> Result<Self> {
        if initial_count > max_count { return Err(Error::OutOfMemory); }
        let mtx = PosixMutex::new(PTHREAD_MUTEX_ERRORCHECK).ok_or(Error::OutOfMemory)?;
        let cond = PosixCondvar::new().ok_or(Error::OutOfMemory)?;
        let handle = mtx.raw_ptr() as SemaphoreHandle;
        Ok(Self { mtx, cond, count: UnsafeCell::new(initial_count), max_count, handle })
    }

    pub fn new_with_count(initial_count: UBaseType) -> Result<Self> {
        Self::new(UBaseType::MAX, initial_count)
    }

    fn count(&self) -> &mut u32 { unsafe { &mut *self.count.get() } }
}

impl SemaphoreFn for Semaphore {
    fn wait(&self, ticks_to_wait: impl ToTick) -> OsalRsBool {
        let ticks = ticks_to_wait.to_ticks();
        let _ = self.mtx.lock();

        if *self.count() > 0 { *self.count() -= 1; let _ = self.mtx.unlock(); return OsalRsBool::True; }
        if ticks == 0 { let _ = self.mtx.unlock(); return OsalRsBool::False; }

        if ticks == UBaseType::MAX {
            loop {
                self.cond.wait(&self.mtx);
                if *self.count() > 0 { *self.count() -= 1; let _ = self.mtx.unlock(); return OsalRsBool::True; }
            }
        }

        let deadline = clock::deadline_from_ms(ticks as u64);
        loop {
            if !self.cond.timedwait(&self.mtx, &deadline) { let _ = self.mtx.unlock(); return OsalRsBool::False; }
            if *self.count() > 0 { *self.count() -= 1; let _ = self.mtx.unlock(); return OsalRsBool::True; }
        }
    }

    fn wait_from_isr(&self) -> OsalRsBool {
        if !self.mtx.try_lock() { return OsalRsBool::False; }
        if *self.count() > 0 { *self.count() -= 1; let _ = self.mtx.unlock(); OsalRsBool::True }
        else { let _ = self.mtx.unlock(); OsalRsBool::False }
    }

    fn signal(&self) -> OsalRsBool {
        let _ = self.mtx.lock();
        if *self.count() < self.max_count {
            *self.count() += 1;
            self.cond.signal();
            let _ = self.mtx.unlock();
            OsalRsBool::True
        } else {
            let _ = self.mtx.unlock();
            OsalRsBool::False
        }
    }

    fn signal_from_isr(&self) -> OsalRsBool { self.signal() }

    fn delete(&mut self) {}
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        let _ = self.mtx.lock();
        self.cond.broadcast();
        let _ = self.mtx.unlock();
    }
}

impl Deref for Semaphore {
    type Target = SemaphoreHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl Debug for Semaphore {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let _ = self.mtx.lock();
        let c = *self.count();
        let _ = self.mtx.unlock();
        f.debug_struct("Semaphore").field("count", &c).field("max", &self.max_count).finish()
    }
}
impl Display for Semaphore {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let _ = self.mtx.lock();
        let c = *self.count();
        let _ = self.mtx.unlock();
        write!(f, "Semaphore {{ count: {c} }}")
    }
}
