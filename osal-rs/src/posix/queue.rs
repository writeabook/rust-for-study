//! Native POSIX FIFO queue using `pthread_mutex_t` + `pthread_cond_t`.
//!
//! - **Mutex**: `PosixMutex` (from `sys::mutex`).
//! - **Condvars**: `PosixCondvar` (CLOCK_MONOTONIC, from `sys::condvar`).
//! - **Timeout**: `pthread_cond_timedwait` with CLOCK_MONOTONIC absolute deadlines.

use core::cell::UnsafeCell;
use core::fmt::{Debug, Display, Formatter};
use core::marker::PhantomData;
use core::ops::Deref;

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use libc::PTHREAD_MUTEX_ERRORCHECK;

#[cfg(not(feature = "serde"))]
use crate::os::Deserialize;

#[cfg(not(feature = "serde"))]
use crate::traits::Serialize;

#[cfg(feature = "serde")]
use osal_rs_serde::{
    Deserialize, Serialize, from_bytes as serde_from_bytes, to_bytes as serde_to_bytes,
};

use super::sys::clock;
use super::sys::condvar::PosixCondvar;
use super::sys::mutex::PosixMutex;
use super::types::{QueueHandle, TickType, UBaseType};
use crate::traits::{BytesHasLen, QueueFn, QueueStreamedFn, ToTick};
use crate::utils::{Error, Result};

// ---------------------------------------------------------------------------
// StructSerde
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
    mtx: PosixMutex,
    not_empty: PosixCondvar,
    not_full: PosixCondvar,
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
        let mtx = PosixMutex::new(PTHREAD_MUTEX_ERRORCHECK).ok_or(Error::OutOfMemory)?;
        let not_empty = PosixCondvar::new().ok_or(Error::OutOfMemory)?;
        let not_full = PosixCondvar::new().ok_or(Error::OutOfMemory)?;
        Ok(Self {
            handle: mtx.raw_ptr() as QueueHandle,
            mtx,
            not_empty,
            not_full,
            buffer: UnsafeCell::new(VecDeque::with_capacity(size as usize)),
            capacity: size as usize,
            message_size: message_size as usize,
            closed: UnsafeCell::new(false),
        })
    }

    pub fn close(&self) {
        let _ = self.mtx.lock();
        let closed = unsafe { &mut *self.closed.get() };
        if !*closed {
            *closed = true;
            unsafe { &mut *self.buffer.get() }.clear();
            self.not_empty.broadcast();
            self.not_full.broadcast();
        }
        let _ = self.mtx.unlock();
    }

    #[inline]
    pub fn fetch_with_to_tick(&self, b: &mut [u8], t: impl ToTick) -> Result<()> {
        self.fetch(b, t.to_ticks())
    }
    #[inline]
    pub fn post_with_to_tick(&self, i: &[u8], t: impl ToTick) -> Result<()> {
        self.post(i, t.to_ticks())
    }
}

impl QueueFn for Queue {
    fn post(&self, item: &[u8], time: TickType) -> Result<()> {
        let _ = self.mtx.lock();
        let closed = unsafe { &*self.closed.get() };
        if *closed {
            let _ = self.mtx.unlock();
            return Err(Error::QueueClosed);
        }
        if item.len() != self.message_size {
            let _ = self.mtx.unlock();
            return Err(Error::InvalidMessageSize);
        }

        let buffer = unsafe { &mut *self.buffer.get() };
        if buffer.len() < self.capacity {
            buffer.push_back(item.to_vec());
            self.not_empty.signal();
            let _ = self.mtx.unlock();
            return Ok(());
        }
        if time == 0 {
            let _ = self.mtx.unlock();
            return Err(Error::Timeout);
        }

        if time == TickType::MAX {
            loop {
                let closed = unsafe { &*self.closed.get() };
                if *closed {
                    let _ = self.mtx.unlock();
                    return Err(Error::QueueClosed);
                }
                if buffer.len() < self.capacity {
                    buffer.push_back(item.to_vec());
                    self.not_empty.signal();
                    let _ = self.mtx.unlock();
                    return Ok(());
                }
                self.not_full.wait(&self.mtx);
            }
        }

        let deadline = clock::deadline_from_ms(time as u64);
        loop {
            let closed = unsafe { &*self.closed.get() };
            if *closed {
                let _ = self.mtx.unlock();
                return Err(Error::QueueClosed);
            }
            if buffer.len() < self.capacity {
                buffer.push_back(item.to_vec());
                self.not_empty.signal();
                let _ = self.mtx.unlock();
                return Ok(());
            }
            if !self.not_full.timedwait(&self.mtx, &deadline) {
                let _ = self.mtx.unlock();
                return Err(Error::Timeout);
            }
        }
    }

    fn post_from_isr(&self, item: &[u8]) -> Result<()> {
        if !self.mtx.try_lock() {
            return Err(Error::Timeout);
        }
        let closed = unsafe { &*self.closed.get() };
        if *closed {
            let _ = self.mtx.unlock();
            return Err(Error::QueueClosed);
        }
        if item.len() != self.message_size {
            let _ = self.mtx.unlock();
            return Err(Error::InvalidMessageSize);
        }
        let buffer = unsafe { &mut *self.buffer.get() };
        if buffer.len() < self.capacity {
            buffer.push_back(item.to_vec());
            self.not_empty.signal();
            let _ = self.mtx.unlock();
            Ok(())
        } else {
            let _ = self.mtx.unlock();
            Err(Error::Timeout)
        }
    }

    fn fetch(&self, out: &mut [u8], time: TickType) -> Result<()> {
        let _ = self.mtx.lock();
        let closed = unsafe { &*self.closed.get() };
        if *closed {
            let _ = self.mtx.unlock();
            return Err(Error::QueueClosed);
        }
        if out.len() != self.message_size {
            let _ = self.mtx.unlock();
            return Err(Error::InvalidMessageSize);
        }

        let buffer = unsafe { &mut *self.buffer.get() };
        if let Some(item) = buffer.pop_front() {
            out.copy_from_slice(&item);
            self.not_full.signal();
            let _ = self.mtx.unlock();
            return Ok(());
        }
        if time == 0 {
            let _ = self.mtx.unlock();
            return Err(Error::Timeout);
        }

        if time == TickType::MAX {
            loop {
                let closed = unsafe { &*self.closed.get() };
                if *closed {
                    let _ = self.mtx.unlock();
                    return Err(Error::QueueClosed);
                }
                if let Some(item) = buffer.pop_front() {
                    out.copy_from_slice(&item);
                    self.not_full.signal();
                    let _ = self.mtx.unlock();
                    return Ok(());
                }
                self.not_empty.wait(&self.mtx);
            }
        }

        let deadline = clock::deadline_from_ms(time as u64);
        loop {
            let closed = unsafe { &*self.closed.get() };
            if *closed {
                let _ = self.mtx.unlock();
                return Err(Error::QueueClosed);
            }
            if let Some(item) = buffer.pop_front() {
                out.copy_from_slice(&item);
                self.not_full.signal();
                let _ = self.mtx.unlock();
                return Ok(());
            }
            if !self.not_empty.timedwait(&self.mtx, &deadline) {
                let _ = self.mtx.unlock();
                return Err(Error::Timeout);
            }
        }
    }

    fn fetch_from_isr(&self, out: &mut [u8]) -> Result<()> {
        if !self.mtx.try_lock() {
            return Err(Error::Timeout);
        }
        let closed = unsafe { &*self.closed.get() };
        if *closed {
            let _ = self.mtx.unlock();
            return Err(Error::QueueClosed);
        }
        if out.len() != self.message_size {
            let _ = self.mtx.unlock();
            return Err(Error::InvalidMessageSize);
        }
        let buffer = unsafe { &mut *self.buffer.get() };
        if let Some(item) = buffer.pop_front() {
            out.copy_from_slice(&item);
            self.not_full.signal();
            let _ = self.mtx.unlock();
            Ok(())
        } else {
            let _ = self.mtx.unlock();
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
    }
}

impl Deref for Queue {
    type Target = QueueHandle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Debug for Queue {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let _ = self.mtx.lock();
        let len = unsafe { &*self.buffer.get() }.len();
        let _ = self.mtx.unlock();
        f.debug_struct("Queue")
            .field("len", &len)
            .field("capacity", &self.capacity)
            .finish()
    }
}
impl Display for Queue {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Queue {{ capacity: {} }}", self.capacity)
    }
}

// ---------------------------------------------------------------------------
// QueueStreamed<T>
// ---------------------------------------------------------------------------

pub struct QueueStreamed<T: StructSerde>(Queue, PhantomData<T>);
unsafe impl<T: StructSerde> Send for QueueStreamed<T> {}
unsafe impl<T: StructSerde> Sync for QueueStreamed<T> {}

impl<T: StructSerde> QueueStreamed<T> {
    pub fn new(size: UBaseType, msg_size: UBaseType) -> Result<Self> {
        Ok(Self(Queue::new(size, msg_size)?, PhantomData))
    }
    pub fn close(&self) {
        self.0.close();
    }
}

#[cfg(not(feature = "serde"))]
impl<T: StructSerde> QueueStreamedFn<T> for QueueStreamed<T> {
    fn post(&self, item: &T, time: TickType) -> Result<()> {
        let bytes = item.to_bytes();
        if bytes.len() != item.len() {
            return Err(Error::InvalidMessageSize);
        }
        self.0.post(bytes, time)
    }
    fn post_from_isr(&self, item: &T) -> Result<()> {
        let bytes = item.to_bytes();
        if bytes.len() != item.len() {
            return Err(Error::InvalidMessageSize);
        }
        self.0.post_from_isr(bytes)
    }
    fn fetch(&self, buffer: &mut T, time: TickType) -> Result<()> {
        let mut buf = vec![0u8; buffer.len()];
        self.0.fetch(&mut buf, time)?;
        *buffer = T::from_bytes(&buf)?;
        Ok(())
    }
    fn fetch_from_isr(&self, buffer: &mut T) -> Result<()> {
        let mut buf = vec![0u8; buffer.len()];
        self.0.fetch_from_isr(&mut buf)?;
        *buffer = T::from_bytes(&buf)?;
        Ok(())
    }
    fn delete(&mut self) {
        self.0.delete()
    }
}

#[cfg(feature = "serde")]
impl<T: StructSerde> QueueStreamedFn<T> for QueueStreamed<T> {
    fn post(&self, item: &T, time: TickType) -> Result<()> {
        let mut buf = vec![0u8; item.len()];
        let w = serde_to_bytes(item, &mut buf).map_err(|_| Error::Unhandled("ser"))?;
        if w != buf.len() {
            return Err(Error::InvalidMessageSize);
        }
        self.0.post(&buf, time)
    }
    fn post_from_isr(&self, item: &T) -> Result<()> {
        let mut buf = vec![0u8; item.len()];
        let w = serde_to_bytes(item, &mut buf).map_err(|_| Error::Unhandled("ser"))?;
        if w != buf.len() {
            return Err(Error::InvalidMessageSize);
        }
        self.0.post_from_isr(&buf)
    }
    fn fetch(&self, buffer: &mut T, time: TickType) -> Result<()> {
        let mut buf = vec![0u8; buffer.len()];
        self.0.fetch(&mut buf, time)?;
        *buffer = serde_from_bytes(&buf).map_err(|_| Error::Unhandled("de"))?;
        Ok(())
    }
    fn fetch_from_isr(&self, buffer: &mut T) -> Result<()> {
        let mut buf = vec![0u8; buffer.len()];
        self.0.fetch_from_isr(&mut buf)?;
        *buffer = serde_from_bytes(&buf).map_err(|_| Error::Unhandled("de"))?;
        Ok(())
    }
    fn delete(&mut self) {
        self.0.delete()
    }
}

impl<T: StructSerde> Deref for QueueStreamed<T> {
    type Target = Queue;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T: StructSerde> Debug for QueueStreamed<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QueueStreamed").field("q", &self.0).finish()
    }
}
impl<T: StructSerde> Display for QueueStreamed<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "QueueStreamed {{ {} }}", self.0)
    }
}
