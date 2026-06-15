//! FIFO queue for inter-thread communication — Linux backend.
//!
//! # Overview
//!
//! Implements the `QueueFn` trait using `std::sync::Mutex<QueueInner>` +
//! `std::sync::Condvar`.  The queue is a bounded FIFO that stores messages
//! as `Vec<u8>` copies, matching the FreeRTOS queue contract.
//!
//! # Design
//!
//! - **State**: A `StdMutex<QueueInner>` holds a `VecDeque<Vec<u8>>` buffer,
//!   capacity, message size, and a closed flag.
//! - **Blocking send/receive**: `post(timeout)` and `fetch(timeout)` use
//!   `Condvar::wait` / `wait_timeout` with deadline loops.
//! - **ISR emulation**: `post_from_isr()` / `fetch_from_isr()` use
//!   `StdMutex::try_lock` — non-blocking, return immediately.
//! - **RAII**: `Drop` calls `delete()` which sets the closed flag and
//!   notifies all waiters.
//!
//! # Fixed-length message contract
//!
//! Each slot in the queue stores exactly `message_size` bytes.  Senders
//! MUST provide a slice of length `message_size`; receivers MUST provide
//! a buffer of length `message_size`.  Violations return
//! [`Error::InvalidMessageSize`] immediately, without modifying queue state.
//!
//! # Contract
//!
//! See `doc/osal-contact-zh.md` §7 for the detailed behavioural
//! specification.

use core::fmt::{Debug, Display};
use core::marker::PhantomData;
use core::ops::Deref;
use core::time::Duration;

use alloc::vec::Vec;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Instant;

#[cfg(not(feature = "serde"))]
use crate::os::Deserialize;

#[cfg(not(feature = "serde"))]
use crate::traits::{BytesHasLen, Serialize};

#[cfg(feature = "serde")]
use osal_rs_serde::{Deserialize, Serialize, from_bytes as serde_from_bytes, to_bytes as serde_to_bytes};

use crate::traits::{QueueFn, QueueStreamedFn, ToTick};
use super::types::{QueueHandle, TickType, UBaseType};
use crate::utils::{Error, Result, MAX_DELAY};

// ---------------------------------------------------------------------------
// StructSerde — helper trait bound (mirrors freertos/queue.rs)
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
// QueueInner — shared state behind the mutex
// ---------------------------------------------------------------------------

struct QueueInner {
    buffer: VecDeque<Vec<u8>>,
    capacity: usize,
    message_size: usize,
    closed: bool,
}

// ---------------------------------------------------------------------------
// Helper: validate message / buffer size against the queue's message_size
// ---------------------------------------------------------------------------

#[inline]
fn validate_message_size(expected: usize, actual: usize) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::InvalidMessageSize)
    }
}

// ---------------------------------------------------------------------------
// Queue — byte-based FIFO queue
// ---------------------------------------------------------------------------

/// A FIFO queue for byte-based message passing.
///
/// Provides a thread-safe queue implementation for sending and receiving
/// raw byte slices between threads. Supports both blocking and ISR-safe
/// operations.
///
/// Internally uses `VecDeque<Vec<u8>>` with a `Mutex` + `Condvar` for
/// thread-safe, blocking FIFO semantics.
///
/// # Fixed-length messages
///
/// The queue enforces that every posted slice and every receive buffer
/// has exactly the length specified by `message_size` at creation time.
/// Providing a different length returns `Error::InvalidMessageSize`
/// without modifying queue state.
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::{Queue, QueueFn};
/// use core::time::Duration;
///
/// // Create a queue with 10 slots, each 32 bytes
/// let queue = Queue::new(10, 32).unwrap();
///
/// // Send data
/// let data = [1u8; 32];
/// queue.post(&data, Duration::from_millis(100).to_ticks()).unwrap();
///
/// // Receive data
/// let mut buffer = [0u8; 32];
/// queue.fetch(&mut buffer, Duration::from_millis(100).to_ticks()).unwrap();
/// ```
pub struct Queue {
    inner: StdMutex<QueueInner>,
    condvar: Condvar,
    handle: QueueHandle,
}

// Safety: StdMutex + Condvar are Send + Sync.
unsafe impl Send for Queue {}
unsafe impl Sync for Queue {}

impl Deref for Queue {
    type Target = QueueHandle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Queue {
    /// Creates a new queue.
    ///
    /// # Parameters
    ///
    /// * `size` — Maximum number of messages the queue can hold.
    /// * `message_size` — Size in bytes of each message.
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` — Successfully created queue.
    /// * `Err(Error::OutOfMemory)` — Invalid size or message_size (zero).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use osal_rs::os::{Queue, QueueFn};
    ///
    /// // Queue for 5 messages of 16 bytes each
    /// let queue = Queue::new(5, 16).unwrap();
    /// ```
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
            condvar: Condvar::new(),
            handle: 1 as QueueHandle,
        })
    }

    // -----------------------------------------------------------------------
    // Convenience methods with ToTick conversion
    // -----------------------------------------------------------------------

    /// Receives data from the queue with a convertible timeout.
    ///
    /// This is a convenience method that accepts any type implementing `ToTick`
    /// (like `Duration`) and converts it to ticks before calling `fetch()`.
    ///
    /// # Arguments
    ///
    /// * `buffer` — Mutable slice to receive data into.
    /// * `time` — Timeout value (e.g., `Duration::from_millis(100)`).
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Data successfully received.
    /// * `Err(Error::Timeout)` — No data available within timeout.
    #[inline]
    pub fn fetch_with_to_tick(&self, buffer: &mut [u8], time: impl ToTick) -> Result<()> {
        self.fetch(buffer, time.to_ticks())
    }

    /// Sends data to the queue with a convertible timeout.
    ///
    /// This is a convenience method that accepts any type implementing `ToTick`
    /// (like `Duration`) and converts it to ticks before calling `post()`.
    ///
    /// # Arguments
    ///
    /// * `item` — Slice of data to send.
    /// * `time` — Timeout value (e.g., `Duration::from_millis(100)`).
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Data successfully sent.
    /// * `Err(Error::Timeout)` — Queue full, could not send within timeout.
    #[inline]
    pub fn post_with_to_tick(&self, item: &[u8], time: impl ToTick) -> Result<()> {
        self.post(item, time.to_ticks())
    }
}

// ---------------------------------------------------------------------------
// QueueFn trait implementation
// ---------------------------------------------------------------------------

impl QueueFn for Queue {
    /// Receives data from the queue, blocking until data is available or timeout.
    ///
    /// This function blocks the calling thread until data is available or the
    /// specified timeout expires.
    ///
    /// # Arguments
    ///
    /// * `buffer` — Mutable byte slice to receive data into. Must have length
    ///   equal to the queue's `message_size`.
    /// * `time` — Timeout in system ticks (0 = no wait, MAX = wait forever).
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Data successfully received into buffer.
    /// * `Err(Error::Timeout)` — No data available within timeout period.
    /// * `Err(Error::InvalidMessageSize)` — `buffer.len() != message_size`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use osal_rs::os::{Queue, QueueFn};
    ///
    /// let queue = Queue::new(5, 16).unwrap();
    /// let mut buffer = [0u8; 16];
    ///
    /// // Wait up to 1000 ticks for data
    /// match queue.fetch(&mut buffer, 1000) {
    ///     Ok(()) => println!("Received data: {:?}", buffer),
    ///     Err(_) => println!("Timeout"),
    /// }
    /// ```
    fn fetch(&self, buffer: &mut [u8], time: TickType) -> Result<()> {
        let mut state = self.inner.lock().unwrap();

        validate_message_size(state.message_size, buffer.len())?;

        // Fast path: data available — dequeue and return.
        if let Some(item) = state.buffer.pop_front() {
            debug_assert_eq!(item.len(), state.message_size);
            buffer.copy_from_slice(&item);
            self.condvar.notify_one(); // wake a potential blocked sender
            return Ok(());
        }

        // Queue is empty — check if we should block.
        if time == 0 {
            return Err(Error::Timeout);
        }

        // Convert ticks to Duration for Condvar.
        let timeout = if time == TickType::MAX {
            MAX_DELAY
        } else {
            // ticks are in milliseconds (TICK_PERIOD_MS = 1)
            Duration::from_millis(time as u64)
        };

        let deadline = Instant::now() + timeout;
        loop {
            let elapsed = Instant::now();
            if elapsed >= deadline {
                return Err(Error::Timeout);
            }
            let remaining = deadline - elapsed;

            state = self.condvar.wait_timeout(state, remaining).unwrap().0;

            // Check if queue was closed (deletion).
            if state.closed {
                return Err(Error::Timeout);
            }

            if let Some(item) = state.buffer.pop_front() {
                debug_assert_eq!(item.len(), state.message_size);
                buffer.copy_from_slice(&item);
                self.condvar.notify_one(); // wake a potential blocked sender
                return Ok(());
            }

            // Spurious wakeup — loop again with updated remaining time.
        }
    }

    /// Receives data from the queue without blocking (ISR-friendly).
    ///
    /// On Linux this is a non-blocking try-receive using `StdMutex::try_lock`.
    ///
    /// # Arguments
    ///
    /// * `buffer` — Mutable byte slice to receive data into. Must have length
    ///   equal to the queue's `message_size`.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Data successfully received.
    /// * `Err(Error::Timeout)` — Queue is empty or lock busy.
    /// * `Err(Error::InvalidMessageSize)` — `buffer.len() != message_size`.
    fn fetch_from_isr(&self, buffer: &mut [u8]) -> Result<()> {
        match self.inner.try_lock() {
            Ok(mut state) => {
                validate_message_size(state.message_size, buffer.len())?;

                if let Some(item) = state.buffer.pop_front() {
                    debug_assert_eq!(item.len(), state.message_size);
                    buffer.copy_from_slice(&item);
                    self.condvar.notify_one();
                    Ok(())
                } else {
                    Err(Error::Timeout)
                }
            }
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Sends data to the back of the queue, blocking until space is available.
    ///
    /// This function blocks the calling thread until space becomes available
    /// or the timeout expires.
    ///
    /// # Arguments
    ///
    /// * `item` — Byte slice to send. Must have length equal to the queue's
    ///   `message_size`.
    /// * `time` — Timeout in system ticks (0 = no wait, MAX = wait forever).
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Data successfully sent.
    /// * `Err(Error::Timeout)` — Queue full, could not send within timeout.
    /// * `Err(Error::InvalidMessageSize)` — `item.len() != message_size`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use osal_rs::os::{Queue, QueueFn};
    ///
    /// let queue = Queue::new(5, 16).unwrap();
    /// let data = [0xAA; 16];
    ///
    /// // Wait up to 1000 ticks to send
    /// queue.post(&data, 1000)?;
    /// ```
    fn post(&self, item: &[u8], time: TickType) -> Result<()> {
        let mut state = self.inner.lock().unwrap();

        validate_message_size(state.message_size, item.len())?;

        // Fast path: queue not full — push and return.
        if state.buffer.len() < state.capacity {
            state.buffer.push_back(item.to_vec());
            self.condvar.notify_one(); // wake a potential blocked receiver
            return Ok(());
        }

        // Queue is full — check if we should block.
        if time == 0 {
            return Err(Error::Timeout);
        }

        // Convert ticks to Duration for Condvar.
        let timeout = if time == TickType::MAX {
            MAX_DELAY
        } else {
            Duration::from_millis(time as u64)
        };

        let deadline = Instant::now() + timeout;
        loop {
            let elapsed = Instant::now();
            if elapsed >= deadline {
                return Err(Error::Timeout);
            }
            let remaining = deadline - elapsed;

            state = self.condvar.wait_timeout(state, remaining).unwrap().0;

            // Check if queue was closed (deletion).
            if state.closed {
                return Err(Error::Timeout);
            }

            if state.buffer.len() < state.capacity {
                state.buffer.push_back(item.to_vec());
                self.condvar.notify_one(); // wake a potential blocked receiver
                return Ok(());
            }

            // Spurious wakeup — loop again.
        }
    }

    /// Sends data to the queue without blocking (ISR-friendly).
    ///
    /// On Linux this uses `StdMutex::try_lock`. If the lock cannot be
    /// acquired immediately the call returns `Err(Error::Timeout)`.
    ///
    /// # Arguments
    ///
    /// * `item` — Byte slice to send. Must have length equal to the queue's
    ///   `message_size`.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Data successfully sent.
    /// * `Err(Error::Timeout)` — Queue is full or lock busy.
    /// * `Err(Error::InvalidMessageSize)` — `item.len() != message_size`.
    fn post_from_isr(&self, item: &[u8]) -> Result<()> {
        match self.inner.try_lock() {
            Ok(mut state) => {
                validate_message_size(state.message_size, item.len())?;

                if state.buffer.len() < state.capacity {
                    state.buffer.push_back(item.to_vec());
                    self.condvar.notify_one();
                    Ok(())
                } else {
                    Err(Error::Timeout)
                }
            }
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Deletes the queue and frees its resources.
    ///
    /// Sets the closed flag and notifies all waiting threads so they can
    /// unblock. Memory is reclaimed when `self` is dropped (RAII).
    ///
    /// # Safety
    ///
    /// Ensure no threads are blocked on this queue before deletion.
    /// Calling this while tasks are waiting will cause them to be woken
    /// with an error.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use osal_rs::os::{Queue, QueueFn};
    ///
    /// let mut queue = Queue::new(5, 16).unwrap();
    /// // Use the queue...
    /// queue.delete();
    /// ```
    fn delete(&mut self) {
        if let Ok(mut state) = self.inner.lock() {
            state.closed = true;
            self.condvar.notify_all();
        }
    }
}

// ---------------------------------------------------------------------------
// Trait impls for Queue
// ---------------------------------------------------------------------------

impl Drop for Queue {
    fn drop(&mut self) {
        self.delete();
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
// QueueStreamed<T> — type-safe FIFO queue
// ---------------------------------------------------------------------------

/// A type-safe FIFO queue for message passing.
///
/// Unlike [`Queue`], which works with raw byte slices, `QueueStreamed` provides
/// a type-safe interface for sending and receiving structured data. The type must
/// implement serialization traits.
///
/// # Type Parameters
///
/// * `T` — The message type. Must implement `StructSerde`
///   (`Serialize + BytesHasLen + Deserialize`).
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::{QueueStreamed, QueueStreamedFn};
/// use core::time::Duration;
///
/// #[derive(Debug, Clone, Copy)]
/// struct Message {
///     id: u32,
///     value: i16,
/// }
///
/// // Assuming Message implements the required traits
/// let queue: QueueStreamed<Message> = QueueStreamed::new(10, size_of::<Message>()).unwrap();
///
/// // Send a message
/// let msg = Message { id: 1, value: 42 };
/// queue.post_with_to_tick(&msg, Duration::from_millis(100)).unwrap();
///
/// // Receive a message
/// let mut received = Message { id: 0, value: 0 };
/// queue.fetch_with_to_tick(&mut received, Duration::from_millis(100)).unwrap();
/// assert_eq!(received.id, 1);
/// assert_eq!(received.value, 42);
/// ```
pub struct QueueStreamed<T: StructSerde>(Queue, PhantomData<T>);

unsafe impl<T: StructSerde> Send for QueueStreamed<T> {}
unsafe impl<T: StructSerde> Sync for QueueStreamed<T> {}

impl<T> QueueStreamed<T>
where
    T: StructSerde,
{
    /// Creates a new type-safe queue.
    ///
    /// # Parameters
    ///
    /// * `size` — Maximum number of messages.
    /// * `message_size` — Size of each message (typically `size_of::<T>()`).
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` — Successfully created queue.
    /// * `Err(Error)` — Creation failed.
    #[inline]
    pub fn new(size: UBaseType, message_size: UBaseType) -> Result<Self> {
        Ok(Self(Queue::new(size, message_size)?, PhantomData))
    }

    /// Receives a typed message with a convertible timeout.
    ///
    /// This is a convenience method that accepts any type implementing `ToTick`.
    ///
    /// # Arguments
    ///
    /// * `buffer` — Mutable reference to receive the message into.
    /// * `time` — Timeout value (e.g., `Duration::from_millis(100)`).
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully received and deserialized.
    /// * `Err(Error)` — Timeout or deserialization error.
    #[allow(dead_code)]
    #[inline]
    fn fetch_with_to_tick(&self, buffer: &mut T, time: impl ToTick) -> Result<()> {
        self.fetch(buffer, time.to_ticks())
    }

    /// Sends a typed message with a convertible timeout.
    ///
    /// This is a convenience method that accepts any type implementing `ToTick`.
    ///
    /// # Arguments
    ///
    /// * `item` — Reference to the message to send.
    /// * `time` — Timeout value (e.g., `Duration::from_millis(100)`).
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully serialized and sent.
    /// * `Err(Error)` — Timeout or serialization error.
    #[inline]
    #[allow(dead_code)]
    fn post_with_to_tick(&self, item: &T, time: impl ToTick) -> Result<()> {
        self.post(item, time.to_ticks())
    }
}

// ---------------------------------------------------------------------------
// QueueStreamedFn impl — without serde feature (custom traits)
// ---------------------------------------------------------------------------

#[cfg(not(feature = "serde"))]
impl<T> QueueStreamedFn<T> for QueueStreamed<T>
where
    T: StructSerde,
{
    /// Receives a typed message from the queue (without serde feature).
    ///
    /// Deserializes the message from bytes using the custom serialization traits.
    ///
    /// # Arguments
    ///
    /// * `buffer` — Mutable reference to receive the deserialized message.
    /// * `time` — Timeout in system ticks.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully received and deserialized.
    /// * `Err(Error::Timeout)` — Queue empty or timeout.
    /// * `Err(Error::InvalidMessageSize)` — Buffer length mismatch.
    /// * `Err(Error)` — Deserialization error.
    fn fetch(&self, buffer: &mut T, time: TickType) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch(&mut buf_bytes, time)?;
        *buffer = T::from_bytes(&buf_bytes)?;
        Ok(())
    }

    /// Receives a typed message from ISR context (without serde feature).
    ///
    /// ISR-safe version that does not block. Deserializes the message from bytes.
    ///
    /// # Arguments
    ///
    /// * `buffer` — Mutable reference to receive the deserialized message.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully received and deserialized.
    /// * `Err(Error::Timeout)` — Queue is empty.
    /// * `Err(Error::InvalidMessageSize)` — Buffer length mismatch.
    /// * `Err(Error)` — Deserialization error.
    fn fetch_from_isr(&self, buffer: &mut T) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch_from_isr(&mut buf_bytes)?;
        *buffer = T::from_bytes(&buf_bytes)?;
        Ok(())
    }

    /// Sends a typed message to the queue (without serde feature).
    ///
    /// Serializes the message to bytes using the custom serialization traits.
    ///
    /// # Arguments
    ///
    /// * `item` — Reference to the message to send.
    /// * `time` — Timeout in system ticks.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully serialized and sent.
    /// * `Err(Error::Timeout)` — Queue full.
    /// * `Err(Error::InvalidMessageSize)` — Message size mismatch.
    /// * `Err(Error)` — Serialization error.
    #[inline]
    fn post(&self, item: &T, time: TickType) -> Result<()> {
        self.0.post(&item.to_bytes(), time)
    }

    /// Sends a typed message from ISR context (without serde feature).
    ///
    /// ISR-safe version that does not block. Serializes the message to bytes.
    ///
    /// # Arguments
    ///
    /// * `item` — Reference to the message to send.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully serialized and sent.
    /// * `Err(Error::Timeout)` — Queue is full.
    /// * `Err(Error::InvalidMessageSize)` — Message size mismatch.
    /// * `Err(Error)` — Serialization error.
    #[inline]
    fn post_from_isr(&self, item: &T) -> Result<()> {
        self.0.post_from_isr(&item.to_bytes())
    }

    /// Deletes the typed queue.
    ///
    /// Delegates to the underlying byte queue's delete method.
    #[inline]
    fn delete(&mut self) {
        self.0.delete()
    }
}

// ---------------------------------------------------------------------------
// QueueStreamedFn impl — with serde feature
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
impl<T> QueueStreamedFn<T> for QueueStreamed<T>
where
    T: StructSerde,
{
    /// Receives a typed message from the queue (with serde feature).
    ///
    /// Deserializes the message from bytes using the serde framework.
    ///
    /// # Arguments
    ///
    /// * `buffer` — Mutable reference to receive the deserialized message.
    /// * `time` — Timeout in system ticks.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully received and deserialized.
    /// * `Err(Error::Timeout)` — Queue empty or timeout.
    /// * `Err(Error::InvalidMessageSize)` — Buffer length mismatch.
    /// * `Err(Error::Unhandled)` — Deserialization error.
    fn fetch(&self, buffer: &mut T, time: TickType) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch(&mut buf_bytes, time)?;
        *buffer = serde_from_bytes(&buf_bytes)
            .map_err(|_| Error::Unhandled("Deserialization error"))?;
        Ok(())
    }

    /// Receives a typed message from ISR context (with serde feature).
    ///
    /// ISR-safe version that does not block. Deserializes using serde.
    ///
    /// # Arguments
    ///
    /// * `buffer` — Mutable reference to receive the deserialized message.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully received and deserialized.
    /// * `Err(Error::Timeout)` — Queue is empty.
    /// * `Err(Error::InvalidMessageSize)` — Buffer length mismatch.
    /// * `Err(Error::Unhandled)` — Deserialization error.
    fn fetch_from_isr(&self, buffer: &mut T) -> Result<()> {
        let mut buf_bytes = vec![0u8; buffer.len()];
        self.0.fetch_from_isr(&mut buf_bytes)?;
        *buffer = serde_from_bytes(&buf_bytes)
            .map_err(|_| Error::Unhandled("Deserialization error"))?;
        Ok(())
    }

    /// Sends a typed message to the queue (with serde feature).
    ///
    /// Serializes the message to bytes using the serde framework.
    ///
    /// # Arguments
    ///
    /// * `item` — Reference to the message to send.
    /// * `time` — Timeout in system ticks.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully serialized and sent.
    /// * `Err(Error::Timeout)` — Queue full.
    /// * `Err(Error::InvalidMessageSize)` — Message size mismatch.
    /// * `Err(Error::Unhandled)` — Serialization error.
    fn post(&self, item: &T, time: TickType) -> Result<()> {
        let mut buf_bytes = vec![0u8; item.len()];
        let written = serde_to_bytes(item, &mut buf_bytes)
            .map_err(|_| Error::Unhandled("Serialization error"))?;
        if written != buf_bytes.len() {
            return Err(Error::InvalidMessageSize);
        }
        self.0.post(&buf_bytes, time)
    }

    /// Sends a typed message from ISR context (with serde feature).
    ///
    /// ISR-safe version that does not block. Serializes using serde.
    ///
    /// # Arguments
    ///
    /// * `item` — Reference to the message to send.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — Message successfully serialized and sent.
    /// * `Err(Error::Timeout)` — Queue is full.
    /// * `Err(Error::InvalidMessageSize)` — Message size mismatch.
    /// * `Err(Error::Unhandled)` — Serialization error.
    fn post_from_isr(&self, item: &T) -> Result<()> {
        let mut buf_bytes = vec![0u8; item.len()];
        let written = serde_to_bytes(item, &mut buf_bytes)
            .map_err(|_| Error::Unhandled("Serialization error"))?;
        if written != buf_bytes.len() {
            return Err(Error::InvalidMessageSize);
        }
        self.0.post_from_isr(&buf_bytes)
    }

    /// Deletes the typed queue (serde version).
    ///
    /// Delegates to the underlying byte queue's delete method.
    #[inline]
    fn delete(&mut self) {
        self.0.delete()
    }
}

// ---------------------------------------------------------------------------
// Trait impls for QueueStreamed
// ---------------------------------------------------------------------------

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