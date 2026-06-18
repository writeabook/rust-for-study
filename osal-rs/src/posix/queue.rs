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

//! Native POSIX FIFO queue using `pthread_mutex_t` + `pthread_cond_t`.
//!
//! # Design
//!
//! Unlike the Linux backend (which uses `std::sync::Mutex` + `Condvar`
//! with poison recovery), this module uses native pthread primitives
//! directly — no poison overhead, no recover_lock boilerplate.
//!
//! - **Mutex**: `pthread_mutex_t` guards the internal buffer, capacity,
//!   and closed flag.
//! - **Condition variables**: `not_empty` wakes consumers when a producer
//!   pushes; `not_full` wakes producers when a consumer pops.
//! - **Timeout**: `pthread_cond_timedwait` with `CLOCK_MONOTONIC` absolute
//!   deadlines.
//! - **ISR emulation**: `pthread_mutex_trylock` for non-blocking send/recv.
//!
//! # Note on mq_*
//!
//! POSIX message queues (`mq_open/send/receive`) were considered but
//! rejected due to system-wide limits (`/proc/sys/fs/mqueue/msg_max`),
//! root-required tuning, and priority-ordered (non-FIFO) default behaviour.

use core::cell::UnsafeCell;
use core::fmt::{Debug, Display, Formatter};
use core::marker::PhantomData;
use core::ops::Deref;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

#[cfg(not(feature = "serde"))]
use crate::os::Deserialize;

#[cfg(not(feature = "serde"))]
use crate::traits::Serialize;

#[cfg(feature = "serde")]
use osal_rs_serde::{Deserialize, Serialize, from_bytes as serde_from_bytes, to_bytes as serde_to_bytes};

use crate::traits::{BytesHasLen, QueueFn, QueueStreamedFn, ToTick};
use super::ffi::{self, PthreadCond, PthreadMutex};
use super::types::{QueueHandle, TickType, UBaseType};
use crate::utils::{Error, Result};

// ---------------------------------------------------------------------------
// StructSerde — helper trait bound
// ---------------------------------------------------------------------------

#[cfg(not(feature = "serde"))]
pub trait StructSerde: Serialize + BytesHasLen + Deserialize {}

#[cfg(not(feature = "serde"))]
impl<T> StructSerde for T where T: Serialize + BytesHasLen + Deserialize {}

#[cfg(feature = "serde")]
pub trait StructSerde: Serialize + Deserialize + BytesHasLen {}

#[cfg(feature = "serde")]
impl<T> StructSerde for T where T: Serialize + Deserialize + BytesHasLen {}

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

pub struct Queue {
    mtx: *mut PthreadMutex,
    not_empty: *mut PthreadCond,
    not_full: *mut PthreadCond,
    buffer: UnsafeCell<VecDeque<Vec<u8>>>,
    capacity: usize,
    message_size: usize,
    closed: UnsafeCell<bool>,
    handle: QueueHandle,
}

unsafe impl Send for Queue {}
unsafe impl Sync for Queue {}

impl Queue {
    pub fn new(size: UBaseType, message_size: UBaseType) -> Result<Self> {
        if size == 0 || message_size == 0 {
            return Err(Error::OutOfMemory);
        }
        let mtx = ffi::create_mutex(ffi::PTHREAD_MUTEX_ERRORCHECK)
            .ok_or(Error::OutOfMemory)?;
        let not_empty = ffi::create_cond_monotonic()
            .ok_or(Error::OutOfMemory)?;
        let not_full = ffi::create_cond_monotonic()
            .ok_or(Error::OutOfMemory)?;
        Ok(Self {
            mtx,
            not_empty,
            not_full,
            buffer: UnsafeCell::new(VecDeque::with_capacity(size as usize)),
            capacity: size as usize,
            message_size: message_size as usize,
            closed: UnsafeCell::new(false),
            handle: mtx as QueueHandle,
        })
    }

    pub fn close(&self) {
        unsafe { ffi::pthread_mutex_lock(self.mtx) };
        let closed = unsafe { &mut *self.closed.get() };
        if !*closed {
            *closed = true;
            unsafe { &mut *self.buffer.get() }.clear();
            unsafe { ffi::pthread_cond_broadcast(self.not_empty) };
            unsafe { ffi::pthread_cond_broadcast(self.not_full) };
        }
        unsafe { ffi::pthread_mutex_unlock(self.mtx) };
    }

    #[inline]
    pub fn fetch_with_to_tick(&self, buffer: &mut [u8], time: impl ToTick) -> Result<()> {
        self.fetch(buffer, time.to_ticks())
    }

    #[inline]
    pub fn post_with_to_tick(&self, item: &[u8], time: impl ToTick) -> Result<()> {
        self.post(item, time.to_ticks())
    }
}

impl QueueFn for Queue {
    fn post(&self, item: &[u8], time: TickType) -> Result<()> {
        unsafe { ffi::pthread_mutex_lock(self.mtx) };

        let closed = unsafe { &*self.closed.get() };
        if *closed { unsafe { ffi::pthread_mutex_unlock(self.mtx) }; return Err(Error::QueueClosed); }

        if item.len() != self.message_size {
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            return Err(Error::InvalidMessageSize);
        }

        let buffer = unsafe { &mut *self.buffer.get() };

        // Fast path
        if buffer.len() < self.capacity {
            buffer.push_back(item.to_vec());
            unsafe { ffi::pthread_cond_signal(self.not_empty) };
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            return Ok(());
        }

        if time == 0 {
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            return Err(Error::Timeout);
        }

        // Finite or infinite wait
        if time == TickType::MAX {
            loop {
                let closed = unsafe { &*self.closed.get() };
                if *closed { unsafe { ffi::pthread_mutex_unlock(self.mtx) }; return Err(Error::QueueClosed); }
                if buffer.len() < self.capacity {
                    buffer.push_back(item.to_vec());
                    unsafe { ffi::pthread_cond_signal(self.not_empty) };
                    unsafe { ffi::pthread_mutex_unlock(self.mtx) };
                    return Ok(());
                }
                unsafe { ffi::pthread_cond_wait(self.not_full, self.mtx) };
            }
        }

        // Finite wait
        let now = ffi::realtime_monotonic();
        let deadline = ffi::timespec_add_ms(&now, time as u64);

        loop {
            let closed = unsafe { &*self.closed.get() };
            if *closed { unsafe { ffi::pthread_mutex_unlock(self.mtx) }; return Err(Error::QueueClosed); }
            if buffer.len() < self.capacity {
                buffer.push_back(item.to_vec());
                unsafe { ffi::pthread_cond_signal(self.not_empty) };
                unsafe { ffi::pthread_mutex_unlock(self.mtx) };
                return Ok(());
            }
            let ret = unsafe { ffi::pthread_cond_timedwait(self.not_full, self.mtx, &deadline) };
            if ret != 0 { // ETIMEDOUT
                unsafe { ffi::pthread_mutex_unlock(self.mtx) };
                return Err(Error::Timeout);
            }
        }
    }

    fn post_from_isr(&self, item: &[u8]) -> Result<()> {
        if unsafe { ffi::pthread_mutex_trylock(self.mtx) } != 0 {
            return Err(Error::Timeout);
        }

        let closed = unsafe { &*self.closed.get() };
        if *closed { unsafe { ffi::pthread_mutex_unlock(self.mtx) }; return Err(Error::QueueClosed); }

        if item.len() != self.message_size {
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            return Err(Error::InvalidMessageSize);
        }

        let buffer = unsafe { &mut *self.buffer.get() };
        if buffer.len() < self.capacity {
            buffer.push_back(item.to_vec());
            unsafe { ffi::pthread_cond_signal(self.not_empty) };
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            Ok(())
        } else {
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            Err(Error::Timeout)
        }
    }

    fn fetch(&self, buffer_out: &mut [u8], time: TickType) -> Result<()> {
        unsafe { ffi::pthread_mutex_lock(self.mtx) };

        let closed = unsafe { &*self.closed.get() };
        if *closed { unsafe { ffi::pthread_mutex_unlock(self.mtx) }; return Err(Error::QueueClosed); }

        if buffer_out.len() != self.message_size {
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            return Err(Error::InvalidMessageSize);
        }

        let buffer = unsafe { &mut *self.buffer.get() };

        // Fast path
        if let Some(item) = buffer.pop_front() {
            buffer_out.copy_from_slice(&item);
            unsafe { ffi::pthread_cond_signal(self.not_full) };
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            return Ok(());
        }

        if time == 0 {
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            return Err(Error::Timeout);
        }

        // Finite or infinite wait
        if time == TickType::MAX {
            loop {
                let closed = unsafe { &*self.closed.get() };
                if *closed { unsafe { ffi::pthread_mutex_unlock(self.mtx) }; return Err(Error::QueueClosed); }
                if let Some(item) = buffer.pop_front() {
                    buffer_out.copy_from_slice(&item);
                    unsafe { ffi::pthread_cond_signal(self.not_full) };
                    unsafe { ffi::pthread_mutex_unlock(self.mtx) };
                    return Ok(());
                }
                unsafe { ffi::pthread_cond_wait(self.not_empty, self.mtx) };
            }
        }

        // Finite wait
        let now = ffi::realtime_monotonic();
        let deadline = ffi::timespec_add_ms(&now, time as u64);

        loop {
            let closed = unsafe { &*self.closed.get() };
            if *closed { unsafe { ffi::pthread_mutex_unlock(self.mtx) }; return Err(Error::QueueClosed); }
            if let Some(item) = buffer.pop_front() {
                buffer_out.copy_from_slice(&item);
                unsafe { ffi::pthread_cond_signal(self.not_full) };
                unsafe { ffi::pthread_mutex_unlock(self.mtx) };
                return Ok(());
            }
            let ret = unsafe { ffi::pthread_cond_timedwait(self.not_empty, self.mtx, &deadline) };
            if ret != 0 {
                unsafe { ffi::pthread_mutex_unlock(self.mtx) };
                return Err(Error::Timeout);
            }
        }
    }

    fn fetch_from_isr(&self, buffer_out: &mut [u8]) -> Result<()> {
        if unsafe { ffi::pthread_mutex_trylock(self.mtx) } != 0 {
            return Err(Error::Timeout);
        }

        let closed = unsafe { &*self.closed.get() };
        if *closed { unsafe { ffi::pthread_mutex_unlock(self.mtx) }; return Err(Error::QueueClosed); }

        if buffer_out.len() != self.message_size {
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            return Err(Error::InvalidMessageSize);
        }

        let buffer = unsafe { &mut *self.buffer.get() };
        if let Some(item) = buffer.pop_front() {
            buffer_out.copy_from_slice(&item);
            unsafe { ffi::pthread_cond_signal(self.not_full) };
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            Ok(())
        } else {
            unsafe { ffi::pthread_mutex_unlock(self.mtx) };
            Err(Error::Timeout)
        }
    }

    fn delete(&mut self) {
        self.close();
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        self.close();
        if !self.mtx.is_null() { unsafe { ffi::destroy_mutex(self.mtx) }; }
        if !self.not_empty.is_null() { unsafe { ffi::destroy_cond(self.not_empty) }; }
        if !self.not_full.is_null() { unsafe { ffi::destroy_cond(self.not_full) }; }
    }
}

impl Deref for Queue {
    type Target = QueueHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl Debug for Queue {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let len = unsafe {
            ffi::pthread_mutex_lock(self.mtx);
            let l = (*self.buffer.get()).len();
            ffi::pthread_mutex_unlock(self.mtx);
            l
        };
        f.debug_struct("Queue")
            .field("len", &len)
            .field("capacity", &self.capacity)
            .field("message_size", &self.message_size)
            .finish()
    }
}

impl Display for Queue {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let len = unsafe {
            ffi::pthread_mutex_lock(self.mtx);
            let l = (*self.buffer.get()).len();
            ffi::pthread_mutex_unlock(self.mtx);
            l
        };
        write!(f, "Queue {{ len: {}, capacity: {}, msg_size: {} }}",
            len, self.capacity, self.message_size)
    }
}

// ---------------------------------------------------------------------------
// QueueStreamed<T>
// ---------------------------------------------------------------------------

pub struct QueueStreamed<T: StructSerde>(Queue, PhantomData<T>);

unsafe impl<T: StructSerde> Send for QueueStreamed<T> {}
unsafe impl<T: StructSerde> Sync for QueueStreamed<T> {}

impl<T> QueueStreamed<T> where T: StructSerde {
    pub fn new(size: UBaseType, message_size: UBaseType) -> Result<Self> {
        Ok(Self(Queue::new(size, message_size)?, PhantomData))
    }

    pub fn close(&self) { self.0.close(); }

    #[allow(dead_code)]
    fn fetch_with_to_tick(&self, buffer: &mut T, time: impl ToTick) -> Result<()> {
        self.fetch(buffer, time.to_ticks())
    }

    #[allow(dead_code)]
    fn post_with_to_tick(&self, item: &T, time: impl ToTick) -> Result<()> {
        self.post(item, time.to_ticks())
    }
}

#[cfg(not(feature = "serde"))]
impl<T> QueueStreamedFn<T> for QueueStreamed<T> where T: StructSerde {
    fn post(&self, item: &T, time: TickType) -> Result<()> {
        let bytes = item.to_bytes();
        if bytes.len() != item.len() { return Err(Error::InvalidMessageSize); }
        self.0.post(bytes, time)
    }
    fn post_from_isr(&self, item: &T) -> Result<()> {
        let bytes = item.to_bytes();
        if bytes.len() != item.len() { return Err(Error::InvalidMessageSize); }
        self.0.post_from_isr(bytes)
    }
    fn fetch(&self, buffer: &mut T, time: TickType) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch(&mut buf_bytes, time)?;
        *buffer = T::from_bytes(&buf_bytes)?;
        Ok(())
    }
    fn fetch_from_isr(&self, buffer: &mut T) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch_from_isr(&mut buf_bytes)?;
        *buffer = T::from_bytes(&buf_bytes)?;
        Ok(())
    }
    fn delete(&mut self) { self.0.delete() }
}

#[cfg(feature = "serde")]
impl<T> QueueStreamedFn<T> for QueueStreamed<T> where T: StructSerde {
    fn post(&self, item: &T, time: TickType) -> Result<()> {
        let mut buf_bytes = vec![0u8; item.len()];
        let written = serde_to_bytes(item, &mut buf_bytes).map_err(|_| Error::Unhandled("Serialization error"))?;
        if written != buf_bytes.len() { return Err(Error::InvalidMessageSize); }
        self.0.post(&buf_bytes, time)
    }
    fn post_from_isr(&self, item: &T) -> Result<()> {
        let mut buf_bytes = vec![0u8; item.len()];
        let written = serde_to_bytes(item, &mut buf_bytes).map_err(|_| Error::Unhandled("Serialization error"))?;
        if written != buf_bytes.len() { return Err(Error::InvalidMessageSize); }
        self.0.post_from_isr(&buf_bytes)
    }
    fn fetch(&self, buffer: &mut T, time: TickType) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch(&mut buf_bytes, time)?;
        *buffer = serde_from_bytes(&buf_bytes).map_err(|_| Error::Unhandled("Deserialization error"))?;
        Ok(())
    }
    fn fetch_from_isr(&self, buffer: &mut T) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch_from_isr(&mut buf_bytes)?;
        *buffer = serde_from_bytes(&buf_bytes).map_err(|_| Error::Unhandled("Deserialization error"))?;
        Ok(())
    }
    fn delete(&mut self) { self.0.delete() }
}

impl<T> Deref for QueueStreamed<T> where T: StructSerde {
    type Target = Queue;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<T> Debug for QueueStreamed<T> where T: StructSerde {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QueueStreamed").field("inner", &self.0).finish()
    }
}

impl<T> Display for QueueStreamed<T> where T: StructSerde {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "QueueStreamed {{ inner: {} }}", self.0)
    }
}
