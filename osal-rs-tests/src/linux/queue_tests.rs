//! Linux-specific queue tests.
//!
//! These tests verify the fixed-length message contract enforced by the
//! Linux Queue backend, as well as typed `QueueStreamed` round-trip
//! correctness. They are **not** part of the cross-backend common test
//! suite.

extern crate alloc;

use core::time::Duration;
use alloc::sync::Arc;
use std::sync::Mutex as StdMutex;

use osal_rs::os::*;
use osal_rs::os::types::TickType;
use osal_rs::utils::{Error, Result};
use osal_rs::log_info;

// In non-serde mode the osal_rs::traits::{Serialize, Deserialize, BytesHasLen}
// are re-exported via `osal_rs::os::*`.  In serde mode the serialization
// traits come from osal_rs_serde, but BytesHasLen is still needed from the
// osal_rs traits module.  We import it via the `os` re-export.
#[cfg(not(feature = "serde"))]
use osal_rs::os::{Serialize as OsalSerialize, Deserialize as OsalDeserialize};

#[cfg(feature = "serde")]
use osal_rs_serde::{Serialize as OsalSerialize, Deserialize as OsalDeserialize};

use osal_rs::os::BytesHasLen;

const TAG: &str = "LinuxQueueTests";

// ===========================================================================
// Test type for non-serde QueueStreamed round-trip tests
// ===========================================================================

/// A safe, fixed-size test message backed by a `[u8; 6]` array.
///
/// Uses explicit little-endian encoding in `new()` and `deserialize()` so
/// that the test is portable and does not depend on Rust struct layout.
#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TestMessage {
    bytes: [u8; 6],
}

#[cfg(not(feature = "serde"))]
impl TestMessage {
    fn new(id: u32, value: i16) -> Self {
        let mut bytes = [0u8; 6];
        bytes[..4].copy_from_slice(&id.to_le_bytes());
        bytes[4..].copy_from_slice(&value.to_le_bytes());
        Self { bytes }
    }

    fn id(&self) -> u32 {
        u32::from_le_bytes([self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3]])
    }

    fn value(&self) -> i16 {
        i16::from_le_bytes([self.bytes[4], self.bytes[5]])
    }
}

#[cfg(not(feature = "serde"))]
impl BytesHasLen for TestMessage {
    fn len(&self) -> usize {
        self.bytes.len()
    }
}

#[cfg(not(feature = "serde"))]
impl OsalSerialize for TestMessage {
    fn to_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(not(feature = "serde"))]
impl OsalDeserialize for TestMessage {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let arr: [u8; 6] = bytes
            .try_into()
            .map_err(|_| Error::InvalidMessageSize)?;
        Ok(Self { bytes: arr })
    }
}

// ===========================================================================
// Test type for serde QueueStreamed round-trip tests
// ===========================================================================

#[cfg(feature = "serde")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, OsalSerialize, OsalDeserialize)]
struct SerdeTestMessage {
    id: u32,
    value: i16,
}

#[cfg(feature = "serde")]
impl BytesHasLen for SerdeTestMessage {
    fn len(&self) -> usize { 6 }
}

// ===========================================================================
// Broken test type — len() disagrees with to_bytes()
// ===========================================================================

#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Copy)]
struct BrokenMessage([u8; 8]);

#[cfg(not(feature = "serde"))]
impl BytesHasLen for BrokenMessage {
    fn len(&self) -> usize { 6 } // intentionally wrong
}

#[cfg(not(feature = "serde"))]
impl OsalSerialize for BrokenMessage {
    fn to_bytes(&self) -> &[u8] {
        &self.0 // 8 bytes
    }
}

#[cfg(not(feature = "serde"))]
impl OsalDeserialize for BrokenMessage {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let arr: [u8; 8] = bytes
            .try_into()
            .map_err(|_| Error::InvalidMessageSize)?;
        Ok(Self(arr))
    }
}

// ===========================================================================
// Raw Queue fixed-length contract tests
// ===========================================================================

pub fn test_queue_exact_message_size() -> Result<()> {
    log_info!(TAG, "Starting test_queue_exact_message_size");
    let queue = Queue::new(2, 4)?;

    queue.post(&[1, 2, 3, 4], 0)?;

    let mut received = [0u8; 4];
    queue.fetch(&mut received, 0)?;

    assert_eq!(received, [1, 2, 3, 4]);
    log_info!(TAG, "test_queue_exact_message_size PASSED");
    Ok(())
}

pub fn test_queue_post_too_short_rejected() -> Result<()> {
    log_info!(TAG, "Starting test_queue_post_too_short_rejected");
    let queue = Queue::new(2, 4)?;

    let result = queue.post(&[1, 2, 3], 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    // Verify queue state unchanged — still empty
    let mut received = [0u8; 4];
    assert_eq!(queue.fetch(&mut received, 0), Err(Error::Timeout));

    log_info!(TAG, "test_queue_post_too_short_rejected PASSED");
    Ok(())
}

pub fn test_queue_post_too_long_rejected() -> Result<()> {
    log_info!(TAG, "Starting test_queue_post_too_long_rejected");
    let queue = Queue::new(2, 4)?;

    let result = queue.post(&[1, 2, 3, 4, 5], 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    let mut received = [0u8; 4];
    assert_eq!(queue.fetch(&mut received, 0), Err(Error::Timeout));

    log_info!(TAG, "test_queue_post_too_long_rejected PASSED");
    Ok(())
}

pub fn test_queue_fetch_buffer_too_short_does_not_consume() -> Result<()> {
    log_info!(TAG, "Starting test_queue_fetch_buffer_too_short_does_not_consume");
    let queue = Queue::new(2, 4)?;

    queue.post(&[10, 20, 30, 40], 0)?;

    let mut short_buf = [0u8; 3];
    assert_eq!(queue.fetch(&mut short_buf, 0), Err(Error::InvalidMessageSize));

    // Message must still be in the queue
    let mut correct_buf = [0u8; 4];
    queue.fetch(&mut correct_buf, 0)?;
    assert_eq!(correct_buf, [10, 20, 30, 40]);

    log_info!(TAG, "test_queue_fetch_buffer_too_short_does_not_consume PASSED");
    Ok(())
}

pub fn test_queue_fetch_buffer_too_long_rejected() -> Result<()> {
    log_info!(TAG, "Starting test_queue_fetch_buffer_too_long_rejected");
    let queue = Queue::new(2, 4)?;

    queue.post(&[1, 2, 3, 4], 0)?;

    let mut long_buf = [0u8; 5];
    assert_eq!(queue.fetch(&mut long_buf, 0), Err(Error::InvalidMessageSize));

    let mut correct_buf = [0u8; 4];
    queue.fetch(&mut correct_buf, 0)?;
    assert_eq!(correct_buf, [1, 2, 3, 4]);

    log_info!(TAG, "test_queue_fetch_buffer_too_long_rejected PASSED");
    Ok(())
}

// ===========================================================================
// ISR path length contract tests
// ===========================================================================

pub fn test_queue_isr_post_too_short() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_post_too_short");
    let queue = Queue::new(2, 4)?;

    assert_eq!(queue.post_from_isr(&[1, 2]), Err(Error::InvalidMessageSize));

    let mut buf = [0u8; 4];
    assert_eq!(queue.fetch_from_isr(&mut buf), Err(Error::Timeout));

    log_info!(TAG, "test_queue_isr_post_too_short PASSED");
    Ok(())
}

pub fn test_queue_isr_post_too_long() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_post_too_long");
    let queue = Queue::new(2, 4)?;

    assert_eq!(queue.post_from_isr(&[1, 2, 3, 4, 5, 6]), Err(Error::InvalidMessageSize));

    let mut buf = [0u8; 4];
    assert_eq!(queue.fetch_from_isr(&mut buf), Err(Error::Timeout));

    log_info!(TAG, "test_queue_isr_post_too_long PASSED");
    Ok(())
}

pub fn test_queue_isr_fetch_buffer_too_short() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_fetch_buffer_too_short");
    let queue = Queue::new(2, 4)?;

    queue.post_from_isr(&[7, 8, 9, 10])?;

    let mut short_buf = [0u8; 2];
    assert_eq!(queue.fetch_from_isr(&mut short_buf), Err(Error::InvalidMessageSize));

    let mut correct_buf = [0u8; 4];
    queue.fetch_from_isr(&mut correct_buf)?;
    assert_eq!(correct_buf, [7, 8, 9, 10]);

    log_info!(TAG, "test_queue_isr_fetch_buffer_too_short PASSED");
    Ok(())
}

pub fn test_queue_isr_fetch_buffer_too_long() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_fetch_buffer_too_long");
    let queue = Queue::new(2, 4)?;

    queue.post_from_isr(&[3, 4, 5, 6])?;

    let mut long_buf = [0u8; 8];
    assert_eq!(queue.fetch_from_isr(&mut long_buf), Err(Error::InvalidMessageSize));

    let mut correct_buf = [0u8; 4];
    queue.fetch_from_isr(&mut correct_buf)?;
    assert_eq!(correct_buf, [3, 4, 5, 6]);

    log_info!(TAG, "test_queue_isr_fetch_buffer_too_long PASSED");
    Ok(())
}

// ===========================================================================
// Error propagation tests (no more silent Timeout conversion)
// ===========================================================================

pub fn test_queue_propagates_underlying_error() -> Result<()> {
    log_info!(TAG, "Starting test_queue_propagates_underlying_error");
    let queue = Queue::new(2, 4)?;

    let mut buf = [0u8; 4];
    assert_eq!(queue.fetch(&mut buf, 0), Err(Error::Timeout));

    let mut wrong_buf = [0u8; 8];
    assert_eq!(queue.fetch(&mut wrong_buf, 0), Err(Error::InvalidMessageSize));

    log_info!(TAG, "test_queue_propagates_underlying_error PASSED");
    Ok(())
}

// ===========================================================================
// Non-serde QueueStreamed round-trip tests
// ===========================================================================

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_round_trip");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(4, 6)?;

    let msg = TestMessage::new(7, -12);
    queue.post(&msg, 0)?;

    let mut received = TestMessage::new(0, 0);
    queue.fetch(&mut received, 0)?;

    assert_eq!(received, msg);
    assert_eq!(received.id(), 7);
    assert_eq!(received.value(), -12);
    log_info!(TAG, "test_queue_streamed_round_trip PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_fifo() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_fifo");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(4, 6)?;

    let messages: Vec<_> = (0..4)
        .map(|i| TestMessage::new(i, -(i as i16)))
        .collect();

    for msg in &messages {
        queue.post(msg, 0)?;
    }

    for expected in &messages {
        let mut received = TestMessage::new(0, 0);
        queue.fetch(&mut received, 0)?;
        assert_eq!(received, *expected, "FIFO order mismatch");
    }

    log_info!(TAG, "test_queue_streamed_fifo PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_wrong_message_size() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_wrong_message_size");
    // TestMessage serialization is 6 bytes, but queue expects 8
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(2, 8)?;

    let msg = TestMessage::new(1, 2);
    let result = queue.post(&msg, 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    log_info!(TAG, "test_queue_streamed_wrong_message_size PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_isr_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_isr_round_trip");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(2, 6)?;

    let msg = TestMessage::new(42, 99);
    queue.post_from_isr(&msg)?;

    let mut received = TestMessage::new(0, 0);
    queue.fetch_from_isr(&mut received)?;

    assert_eq!(received, msg);
    log_info!(TAG, "test_queue_streamed_isr_round_trip PASSED");
    Ok(())
}

// ===========================================================================
// len() vs to_bytes() consistency test
// ===========================================================================

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_broken_len_consistency() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_broken_len_consistency");
    // BrokenMessage: len() = 6, to_bytes() = 8
    let queue: QueueStreamed<BrokenMessage> = QueueStreamed::new(2, 8)?;

    let msg = BrokenMessage([1, 2, 3, 4, 5, 6, 7, 8]);

    // post() must detect the inconsistency and reject
    let result = queue.post(&msg, 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    // post_from_isr() must also detect it
    let result_isr = queue.post_from_isr(&msg);
    assert_eq!(result_isr, Err(Error::InvalidMessageSize));

    log_info!(TAG, "test_queue_streamed_broken_len_consistency PASSED");
    Ok(())
}

// ===========================================================================
// Serde QueueStreamed round-trip tests
// ===========================================================================

#[cfg(feature = "serde")]
pub fn test_queue_streamed_serde_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_serde_round_trip");
    let queue: QueueStreamed<SerdeTestMessage> = QueueStreamed::new(4, 6)?;

    let msg = SerdeTestMessage { id: 99, value: -1 };
    queue.post(&msg, 0)?;

    let mut received = SerdeTestMessage { id: 0, value: 0 };
    queue.fetch(&mut received, 0)?;

    assert_eq!(received, msg);
    log_info!(TAG, "test_queue_streamed_serde_round_trip PASSED");
    Ok(())
}

#[cfg(feature = "serde")]
pub fn test_queue_streamed_serde_fifo() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_serde_fifo");
    let queue: QueueStreamed<SerdeTestMessage> = QueueStreamed::new(4, 6)?;

    let messages: Vec<_> = (0..4)
        .map(|i| SerdeTestMessage { id: 10 + i, value: (i as i16 * 10) })
        .collect();

    for msg in &messages {
        queue.post(msg, 0)?;
    }

    for expected in &messages {
        let mut received = SerdeTestMessage { id: 0, value: 0 };
        queue.fetch(&mut received, 0)?;
        assert_eq!(received, *expected, "serde FIFO order mismatch");
    }

    log_info!(TAG, "test_queue_streamed_serde_fifo PASSED");
    Ok(())
}

#[cfg(feature = "serde")]
pub fn test_queue_streamed_serde_isr_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_serde_isr_round_trip");
    let queue: QueueStreamed<SerdeTestMessage> = QueueStreamed::new(2, 6)?;

    let msg = SerdeTestMessage { id: 42, value: -9 };
    queue.post_from_isr(&msg)?;

    let mut received = SerdeTestMessage { id: 0, value: 0 };
    queue.fetch_from_isr(&mut received)?;

    assert_eq!(received, msg);
    log_info!(TAG, "test_queue_streamed_serde_isr_round_trip PASSED");
    Ok(())
}

// ===========================================================================
// Close lifecycle tests
// ===========================================================================

pub fn test_queue_close_wakes_blocked_consumer() -> Result<()> {
    log_info!(TAG, "Starting test_queue_close_wakes_blocked_consumer");
    use std::thread;
    let queue = Arc::new(Queue::new(1, 4)?);

    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 4];
        q.fetch(&mut buf, TickType::MAX)
    });

    thread::sleep(Duration::from_millis(10));
    queue.close();
    assert_eq!(handle.join().unwrap(), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_close_wakes_blocked_consumer PASSED");
    Ok(())
}

pub fn test_queue_close_wakes_blocked_producer() -> Result<()> {
    log_info!(TAG, "Starting test_queue_close_wakes_blocked_producer");
    use std::thread;
    let queue = Arc::new(Queue::new(1, 4)?);
    queue.post(&[1u8; 4], 0)?;

    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || {
        q.post(&[2u8; 4], TickType::MAX)
    });

    thread::sleep(Duration::from_millis(10));
    queue.close();
    assert_eq!(handle.join().unwrap(), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_close_wakes_blocked_producer PASSED");
    Ok(())
}

pub fn test_queue_all_ops_fail_after_close() -> Result<()> {
    log_info!(TAG, "Starting test_queue_all_ops_fail_after_close");
    let queue = Queue::new(2, 4)?;
    queue.post(&[1u8; 4], 0)?;
    queue.close();

    assert_eq!(queue.post(&[2u8; 4], 0), Err(Error::QueueClosed));
    assert_eq!(queue.fetch(&mut [0u8; 4], 0), Err(Error::QueueClosed));
    assert_eq!(queue.post_from_isr(&[3u8; 4]), Err(Error::QueueClosed));
    assert_eq!(queue.fetch_from_isr(&mut [0u8; 4]), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_all_ops_fail_after_close PASSED");
    Ok(())
}

pub fn test_queue_close_idempotent() -> Result<()> {
    log_info!(TAG, "Starting test_queue_close_idempotent");
    let queue = Queue::new(1, 4)?;
    queue.close();
    queue.close();
    queue.close();
    // ops still return QueueClosed, not panic
    assert_eq!(queue.post(&[1u8; 4], 0), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_close_idempotent PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Handle uniqueness
// ===========================================================================

pub fn test_queue_unique_handles() -> Result<()> {
    log_info!(TAG, "Starting test_queue_unique_handles");
    let q1 = Queue::new(1, 4)?;
    let q2 = Queue::new(1, 4)?;
    assert_ne!(*q1, *q2, "different queues must have different handles");
    log_info!(TAG, "test_queue_unique_handles PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Multi-producer/consumer liveness
// ===========================================================================

pub fn test_queue_multi_producer_consumer() -> Result<()> {
    log_info!(TAG, "Starting test_queue_multi_producer_consumer");
    use std::thread;
    let queue = Arc::new(Queue::new(4, 4)?);
    const THREADS: usize = 4;
    const ITERS: u32 = 250;

    let mut producers = vec![];
    for id in 0..THREADS {
        let q = Arc::clone(&queue);
        producers.push(thread::spawn(move || {
            for i in 0..ITERS {
                let data = (id as u32).to_le_bytes();
                q.post(&data, TickType::MAX).unwrap();
            }
        }));
    }

    let mut consumers = vec![];
    let total = Arc::new(StdMutex::new(0u32));
    for _ in 0..THREADS {
        let q = Arc::clone(&queue);
        let t = Arc::clone(&total);
        consumers.push(thread::spawn(move || {
            let mut buf = [0u8; 4];
            loop {
                match q.fetch(&mut buf, TickType::MAX) {
                    Ok(()) => {
                        *t.lock().unwrap() += 1;
                    }
                    Err(Error::QueueClosed) => break,
                    Err(_) => break,
                }
            }
        }));
    }

    for p in producers { p.join().unwrap(); }
    queue.close();
    for c in consumers { c.join().unwrap(); }

    assert_eq!(*total.lock().unwrap(), THREADS as u32 * ITERS);
    log_info!(TAG, "test_queue_multi_producer_consumer PASSED");
    Ok(())
}

// ===========================================================================
// Run all tests
// ===========================================================================

pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "========== Running Linux-Specific Queue Tests ==========");

    test_queue_exact_message_size()?;
    test_queue_post_too_short_rejected()?;
    test_queue_post_too_long_rejected()?;
    test_queue_fetch_buffer_too_short_does_not_consume()?;
    test_queue_fetch_buffer_too_long_rejected()?;
    test_queue_isr_post_too_short()?;
    test_queue_isr_post_too_long()?;
    test_queue_isr_fetch_buffer_too_short()?;
    test_queue_isr_fetch_buffer_too_long()?;
    test_queue_propagates_underlying_error()?;

    // Phase 2
    test_queue_close_wakes_blocked_consumer()?;
    test_queue_close_wakes_blocked_producer()?;
    test_queue_all_ops_fail_after_close()?;
    test_queue_close_idempotent()?;
    test_queue_unique_handles()?;
    test_queue_multi_producer_consumer()?;

    #[cfg(not(feature = "serde"))]
    {
        test_queue_streamed_round_trip()?;
        test_queue_streamed_fifo()?;
        test_queue_streamed_wrong_message_size()?;
        test_queue_streamed_isr_round_trip()?;
        test_queue_streamed_broken_len_consistency()?;
    }

    #[cfg(feature = "serde")]
    {
        test_queue_streamed_serde_round_trip()?;
        test_queue_streamed_serde_fifo()?;
        test_queue_streamed_serde_isr_round_trip()?;
    }

    log_info!(TAG, "========== All Linux-Specific Queue Tests PASSED ==========");
    Ok(())
}
