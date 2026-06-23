//! FIFO queue for inter-thread communication — Linux backend.
//!
//! # Overview
//!
//! Implements the `QueueFn` trait using `std::sync::Mutex<QueueInner>` +
//! two `Condvar`s. The queue is a bounded FIFO that stores messages as
//! `Vec<u8>` copies.
//!
//! # Design
//!
//! - **Dual Condvars**: `not_empty` for consumers; `not_full` for
//!   producers. Post success wakes a consumer on `not_empty`; fetch
//!   success wakes a producer on `not_full`.
//! - **Blocking send/receive**: `post(timeout)` and `fetch(timeout)` use
//!   `Condvar::wait` (MAX = indefinite) or `wait_timeout` (finite).
//! - **ISR emulation**: `post_from_isr()` / `fetch_from_isr()` use
//!   `StdMutex::try_lock` — non-blocking. `TryLockError::Poisoned` is
//!   recovered transparently.
//! - **Close lifecycle**: `close(&self)` can be called through `Arc`.
//!   `delete(&mut self)` and `Drop` delegate to `close()`.
//! - **RAII Poison recovery**: `recover_lock()` unwraps poisoned mutexes
//!   so that the Queue remains usable after a panic.
//!
//! # Fixed-length message contract
//!
//! Each slot stores exactly `message_size` bytes. Senders and receivers
//! must use slices of that exact length.

use core::fmt::{Debug, Display};
use core::marker::PhantomData;
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use alloc::vec::Vec;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex as StdMutex, TryLockError};
use std::time::Instant;

#[cfg(not(feature = "serde"))]
use crate::os::Deserialize;

#[cfg(not(feature = "serde"))]
use crate::traits::Serialize;

#[cfg(feature = "serde")]
use osal_rs_serde::{
    Deserialize, Serialize, from_bytes as serde_from_bytes, to_bytes as serde_to_bytes,
};

use super::types::{QueueHandle, TickType, UBaseType};
use crate::traits::{BytesHasLen, QueueFn, QueueStreamedFn, ToTick};
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
// QueueInner
// ---------------------------------------------------------------------------

struct QueueInner {
    buffer: VecDeque<Vec<u8>>,
    capacity: usize,
    message_size: usize,
    closed: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn validate_message_size(expected: usize, actual: usize) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::InvalidMessageSize)
    }
}

fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

static NEXT_QUEUE_HANDLE: AtomicUsize = AtomicUsize::new(1);

fn next_queue_handle() -> QueueHandle {
    NEXT_QUEUE_HANDLE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("Linux queue handle space exhausted") as QueueHandle
}

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

pub struct Queue {
    inner: StdMutex<QueueInner>,
    not_empty: Condvar,
    not_full: Condvar,
    handle: QueueHandle,
}

unsafe impl Send for Queue {}
unsafe impl Sync for Queue {}

impl Deref for Queue {
    type Target = QueueHandle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Queue {
    pub fn new(size: UBaseType, message_size: UBaseType) -> Result<Self> {
        if size == 0 || message_size == 0 {
            return Err(Error::OutOfMemory);
        }
        Ok(Self {
            inner: StdMutex::new(QueueInner {
                buffer: VecDeque::with_capacity(size as usize),
                capacity: size as usize,
                message_size: message_size as usize,
                closed: false,
            }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
            handle: next_queue_handle(),
        })
    }

    /// Close the queue, waking all blocked threads.
    ///
    /// Idempotent — calling multiple times is safe. After closing,
    /// all send/receive operations return [`Error::QueueClosed`].
    pub fn close(&self) {
        let mut state = recover_lock(self.inner.lock());
        if state.closed {
            return;
        }
        state.closed = true;
        state.buffer.clear();
        drop(state);
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }

    #[inline]
    pub fn fetch_with_to_tick(&self, buffer: &mut [u8], time: impl ToTick) -> Result<()> {
        self.fetch(buffer, time.to_ticks())
    }

    #[inline]
    pub fn post_with_to_tick(&self, item: &[u8], time: impl ToTick) -> Result<()> {
        self.post(item, time.to_ticks())
    }

    // ISR helper: try_lock with poison recovery
    fn try_lock_state(&self) -> Result<std::sync::MutexGuard<'_, QueueInner>> {
        match self.inner.try_lock() {
            Ok(state) => Ok(state),
            Err(TryLockError::Poisoned(err)) => Ok(err.into_inner()),
            Err(TryLockError::WouldBlock) => Err(Error::Timeout),
        }
    }
}

// ---------------------------------------------------------------------------
// QueueFn
// ---------------------------------------------------------------------------

impl QueueFn for Queue {
    fn post(&self, item: &[u8], time: TickType) -> Result<()> {
        let mut state = recover_lock(self.inner.lock());

        // closed check first
        if state.closed {
            return Err(Error::QueueClosed);
        }

        validate_message_size(state.message_size, item.len())?;

        // Fast path
        if state.buffer.len() < state.capacity {
            state.buffer.push_back(item.to_vec());
            drop(state);
            self.not_empty.notify_one();
            return Ok(());
        }

        if time == 0 {
            return Err(Error::Timeout);
        }

        // Indefinite wait
        if time == TickType::MAX {
            loop {
                if state.closed {
                    return Err(Error::QueueClosed);
                }
                if state.buffer.len() < state.capacity {
                    state.buffer.push_back(item.to_vec());
                    drop(state);
                    self.not_empty.notify_one();
                    return Ok(());
                }
                state = recover_lock(self.not_full.wait(state));
            }
        }

        // Finite wait
        let timeout = Duration::from_millis(time as u64);
        let deadline = Instant::now().checked_add(timeout).ok_or(Error::Timeout)?;

        loop {
            if state.closed {
                return Err(Error::QueueClosed);
            }
            if state.buffer.len() < state.capacity {
                state.buffer.push_back(item.to_vec());
                drop(state);
                self.not_empty.notify_one();
                return Ok(());
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout);
            }
            let remaining = deadline - now;

            let (next_state, timeout_result) =
                recover_lock(self.not_full.wait_timeout(state, remaining));
            state = next_state;

            if timeout_result.timed_out() && state.buffer.len() >= state.capacity && !state.closed {
                return Err(Error::Timeout);
            }
        }
    }

    fn post_from_isr(&self, item: &[u8]) -> Result<()> {
        let mut state = self.try_lock_state()?;

        if state.closed {
            return Err(Error::QueueClosed);
        }

        validate_message_size(state.message_size, item.len())?;

        if state.buffer.len() < state.capacity {
            state.buffer.push_back(item.to_vec());
            drop(state);
            self.not_empty.notify_one();
            Ok(())
        } else {
            Err(Error::Timeout)
        }
    }

    fn fetch(&self, buffer: &mut [u8], time: TickType) -> Result<()> {
        let mut state = recover_lock(self.inner.lock());

        if state.closed {
            return Err(Error::QueueClosed);
        }

        validate_message_size(state.message_size, buffer.len())?;

        // Fast path
        if let Some(item) = state.buffer.pop_front() {
            debug_assert_eq!(item.len(), state.message_size);
            buffer.copy_from_slice(&item);
            drop(state);
            self.not_full.notify_one();
            return Ok(());
        }

        if time == 0 {
            return Err(Error::Timeout);
        }

        // Indefinite wait
        if time == TickType::MAX {
            loop {
                if state.closed {
                    return Err(Error::QueueClosed);
                }
                if let Some(item) = state.buffer.pop_front() {
                    debug_assert_eq!(item.len(), state.message_size);
                    buffer.copy_from_slice(&item);
                    drop(state);
                    self.not_full.notify_one();
                    return Ok(());
                }
                state = recover_lock(self.not_empty.wait(state));
            }
        }

        // Finite wait
        let timeout = Duration::from_millis(time as u64);
        let deadline = Instant::now().checked_add(timeout).ok_or(Error::Timeout)?;

        loop {
            if state.closed {
                return Err(Error::QueueClosed);
            }
            if let Some(item) = state.buffer.pop_front() {
                debug_assert_eq!(item.len(), state.message_size);
                buffer.copy_from_slice(&item);
                drop(state);
                self.not_full.notify_one();
                return Ok(());
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout);
            }
            let remaining = deadline - now;

            let (next_state, timeout_result) =
                recover_lock(self.not_empty.wait_timeout(state, remaining));
            state = next_state;

            if timeout_result.timed_out() && state.buffer.is_empty() && !state.closed {
                return Err(Error::Timeout);
            }
        }
    }

    fn fetch_from_isr(&self, buffer: &mut [u8]) -> Result<()> {
        let mut state = self.try_lock_state()?;

        if state.closed {
            return Err(Error::QueueClosed);
        }

        validate_message_size(state.message_size, buffer.len())?;

        if let Some(item) = state.buffer.pop_front() {
            debug_assert_eq!(item.len(), state.message_size);
            buffer.copy_from_slice(&item);
            drop(state);
            self.not_full.notify_one();
            Ok(())
        } else {
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

impl Debug for Queue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => f
                .debug_struct("Queue")
                .field("len", &state.buffer.len())
                .field("capacity", &state.capacity)
                .field("message_size", &state.message_size)
                .field("closed", &state.closed)
                .finish(),
            Err(_) => f.debug_struct("Queue").finish_non_exhaustive(),
        }
    }
}

impl Display for Queue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => write!(
                f,
                "Queue {{ len: {}, capacity: {}, msg_size: {} }}",
                state.buffer.len(),
                state.capacity,
                state.message_size
            ),
            Err(_) => write!(f, "Queue {{ <locked> }}"),
        }
    }
}

// ---------------------------------------------------------------------------
// QueueStreamed<T>
// ---------------------------------------------------------------------------

pub struct QueueStreamed<T: StructSerde>(Queue, PhantomData<T>);

unsafe impl<T: StructSerde> Send for QueueStreamed<T> {}
unsafe impl<T: StructSerde> Sync for QueueStreamed<T> {}

impl<T> QueueStreamed<T>
where
    T: StructSerde,
{
    #[inline]
    pub fn new(size: UBaseType, message_size: UBaseType) -> Result<Self> {
        Ok(Self(Queue::new(size, message_size)?, PhantomData))
    }

    #[allow(dead_code)]
    #[inline]
    fn fetch_with_to_tick(&self, buffer: &mut T, time: impl ToTick) -> Result<()> {
        self.fetch(buffer, time.to_ticks())
    }

    #[inline]
    #[allow(dead_code)]
    fn post_with_to_tick(&self, item: &T, time: impl ToTick) -> Result<()> {
        self.post(item, time.to_ticks())
    }
}

// Non-serde QueueStreamedFn
#[cfg(not(feature = "serde"))]
impl<T> QueueStreamedFn<T> for QueueStreamed<T>
where
    T: StructSerde,
{
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

    fn delete(&mut self) {
        self.0.delete()
    }
}

// Serde QueueStreamedFn
#[cfg(feature = "serde")]
impl<T> QueueStreamedFn<T> for QueueStreamed<T>
where
    T: StructSerde,
{
    fn post(&self, item: &T, time: TickType) -> Result<()> {
        let mut buf_bytes = vec![0u8; item.len()];
        let written = serde_to_bytes(item, &mut buf_bytes)
            .map_err(|_| Error::Unhandled("Serialization error"))?;
        if written != buf_bytes.len() {
            return Err(Error::InvalidMessageSize);
        }
        self.0.post(&buf_bytes, time)
    }

    fn post_from_isr(&self, item: &T) -> Result<()> {
        let mut buf_bytes = vec![0u8; item.len()];
        let written = serde_to_bytes(item, &mut buf_bytes)
            .map_err(|_| Error::Unhandled("Serialization error"))?;
        if written != buf_bytes.len() {
            return Err(Error::InvalidMessageSize);
        }
        self.0.post_from_isr(&buf_bytes)
    }

    fn fetch(&self, buffer: &mut T, time: TickType) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch(&mut buf_bytes, time)?;
        *buffer =
            serde_from_bytes(&buf_bytes).map_err(|_| Error::Unhandled("Deserialization error"))?;
        Ok(())
    }

    fn fetch_from_isr(&self, buffer: &mut T) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch_from_isr(&mut buf_bytes)?;
        *buffer =
            serde_from_bytes(&buf_bytes).map_err(|_| Error::Unhandled("Deserialization error"))?;
        Ok(())
    }

    fn delete(&mut self) {
        self.0.delete()
    }
}

// Trait impls
impl<T> QueueStreamed<T>
where
    T: StructSerde,
{
    /// Close the underlying queue (see [`Queue::close`]).
    pub fn close(&self) {
        self.0.close();
    }
}

impl<T> Deref for QueueStreamed<T>
where
    T: StructSerde,
{
    type Target = Queue;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> Debug for QueueStreamed<T>
where
    T: StructSerde,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QueueStreamed")
            .field("inner", &self.0)
            .finish()
    }
}

impl<T> Display for QueueStreamed<T>
where
    T: StructSerde,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "QueueStreamed {{ inner: {} }}", self.0)
    }
}
