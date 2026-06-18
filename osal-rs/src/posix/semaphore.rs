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

//! Native POSIX semaphore implementation using `sem_t`.
//!
//! # Design
//!
//! Unlike the Linux backend (which uses `StdMutex<State>` + `Condvar` with
//! manual count tracking), this module delegates to the kernel's `sem_t`
//! for wait/wake semantics:
//!
//! - **Blocking wait**: `sem_wait()` / `sem_timedwait()` — kernel-managed,
//!   automatically unblocks the highest-priority waiter.
//! - **Non-blocking wait**: `sem_trywait()` — returns immediately if count
//!   is zero.
//! - **Signal**: `sem_post()` — increments count, wakes one waiter via
//!   the kernel scheduler.
//! - **Max-count enforcement**: Since POSIX `sem_t` has no built-in maximum,
//!   we store `max_count` separately and check with `sem_getvalue()` before
//!   calling `sem_post()`.
//!
//! # ISR path
//!
//! `wait_from_isr()` / `signal_from_isr()` mirror the task-level path using
//! `sem_trywait()` / `sem_post()` — neither blocks, matching ISR expectations
//! for host testing.

use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::time::Duration;

use std::time::{SystemTime, UNIX_EPOCH};

use super::ffi::{self, SemT, Timespec};
use super::types::{SemaphoreHandle, UBaseType};
use crate::traits::SemaphoreFn;
use crate::traits::ToTick;
use crate::utils::{Error, OsalRsBool, Result};

// ---------------------------------------------------------------------------
// Semaphore — counting semaphore on POSIX sem_t
// ---------------------------------------------------------------------------

pub struct Semaphore {
    sem: *mut SemT,
    max_count: u32,
    handle: SemaphoreHandle,
}

unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}

impl Semaphore {
    pub fn new(max_count: UBaseType, initial_count: UBaseType) -> Result<Self> {
        if initial_count > max_count {
            return Err(Error::OutOfMemory);
        }
        let sem = ffi::create_sem(initial_count)
            .ok_or(Error::OutOfMemory)?;
        Ok(Self { sem, max_count, handle: sem as SemaphoreHandle })
    }

    pub fn new_with_count(initial_count: UBaseType) -> Result<Self> {
        Self::new(UBaseType::MAX, initial_count)
    }
}

impl SemaphoreFn for Semaphore {
    fn wait(&self, ticks_to_wait: impl ToTick) -> OsalRsBool {
        let ticks = ticks_to_wait.to_ticks();

        // Zero ticks: try-wait only
        if ticks == 0 {
            return if unsafe { ffi::sem_trywait(self.sem) } == 0 {
                OsalRsBool::True
            } else {
                OsalRsBool::False
            };
        }

        // Infinite wait
        if ticks == UBaseType::MAX {
            loop {
                if unsafe { ffi::sem_wait(self.sem) } == 0 {
                    return OsalRsBool::True;
                }
                // EINTR — retry
            }
        }

        // Finite wait: sem_timedwait with absolute deadline (CLOCK_REALTIME)
        let timeout = Duration::from_millis(ticks as u64);
        let deadline = match SystemTime::now().checked_add(timeout) {
            Some(d) => d,
            None => return OsalRsBool::False,
        };
        let since_epoch = match deadline.duration_since(UNIX_EPOCH) {
            Ok(d) => d,
            Err(_) => return OsalRsBool::False,
        };
        let abs_ts = Timespec {
            tv_sec: since_epoch.as_secs() as i64,
            tv_nsec: since_epoch.subsec_nanos() as i64,
        };

        if unsafe { ffi::sem_timedwait(self.sem, &abs_ts) } == 0 {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    fn wait_from_isr(&self) -> OsalRsBool {
        if unsafe { ffi::sem_trywait(self.sem) } == 0 {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    fn signal(&self) -> OsalRsBool {
        // Check max_count before posting
        let mut current: i32 = 0;
        if unsafe { ffi::sem_getvalue(self.sem, &mut current) } != 0 {
            return OsalRsBool::False;
        }
        if (current as u32) >= self.max_count {
            return OsalRsBool::False;
        }
        if unsafe { ffi::sem_post(self.sem) } == 0 {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    fn signal_from_isr(&self) -> OsalRsBool {
        self.signal()
    }

    fn delete(&mut self) {
        if !self.sem.is_null() {
            unsafe { ffi::destroy_sem(self.sem) };
            self.sem = core::ptr::null_mut();
        }
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        if !self.sem.is_null() {
            unsafe { ffi::destroy_sem(self.sem) };
        }
    }
}

impl Deref for Semaphore {
    type Target = SemaphoreHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl Debug for Semaphore {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let mut current: i32 = 0;
        let _ = unsafe { ffi::sem_getvalue(self.sem, &mut current) };
        f.debug_struct("Semaphore")
            .field("count", &current)
            .field("max_count", &self.max_count)
            .field("handle", &self.handle)
            .finish()
    }
}

impl Display for Semaphore {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let mut current: i32 = 0;
        let _ = unsafe { ffi::sem_getvalue(self.sem, &mut current) };
        write!(
            f,
            "Semaphore {{ count: {}, max: {} }}",
            current, self.max_count
        )
    }
}
