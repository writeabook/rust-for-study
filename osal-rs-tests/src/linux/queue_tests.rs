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

//! Linux-specific queue tests.
//!
//! These tests verify the fixed-length message contract, dual-Condvar
//! liveness, Max-delay semantics, close lifecycle, poison recovery,
//! and typed QueueStreamed round-trip correctness.  They are **not**
//! part of the cross-backend common test suite.

extern crate alloc;

use core::time::Duration;
use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use std::sync::Mutex as StdMutex;

use osal_rs::os::*;
use osal_rs::os::types::TickType;
use osal_rs::utils::{Error, Result};
use osal_rs::log_info;

#[cfg(not(feature = "serde"))]
use osal_rs::os::{Serialize as OsalSerialize, Deserialize as OsalDeserialize};

#[cfg(feature = "serde")]
use osal_rs_serde::{Serialize as OsalSerialize, Deserialize as OsalDeserialize};

use osal_rs::os::BytesHasLen;

const TAG: &str = "LinuxQueueTests";

// ===========================================================================
// Test type for non-serde QueueStreamed round-trip tests
// ===========================================================================

#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TestMessage { bytes: [u8; 6] }

#[cfg(not(feature = "serde"))]
impl TestMessage {
    fn new(id: u32, value: i16) -> Self {
        let mut bytes = [0u8; 6];
        bytes[..4].copy_from_slice(&id.to_le_bytes());
        bytes[4..].copy_from_slice(&value.to_le_bytes());
        Self { bytes }
    }
    fn id(&self) -> u32 { u32::from_le_bytes([self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3]]) }
    fn value(&self) -> i16 { i16::from_le_bytes([self.bytes[4], self.bytes[5]]) }
}

#[cfg(not(feature = "serde"))]
impl BytesHasLen for TestMessage { fn len(&self) -> usize { self.bytes.len() } }

#[cfg(not(feature = "serde"))]
impl OsalSerialize for TestMessage { fn to_bytes(&self) -> &[u8] { &self.bytes } }

#[cfg(not(feature = "serde"))]
impl OsalDeserialize for TestMessage {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let arr: [u8; 6] = bytes.try_into().map_err(|_| Error::InvalidMessageSize)?;
        Ok(Self { bytes: arr })
    }
}

// ===========================================================================
// Test type for serde QueueStreamed round-trip tests
// ===========================================================================

#[cfg(feature = "serde")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, OsalSerialize, OsalDeserialize)]
struct SerdeTestMessage { id: u32, value: i16 }

#[cfg(feature = "serde")]
impl BytesHasLen for SerdeTestMessage { fn len(&self) -> usize { 6 } }

// ===========================================================================
// Broken test type
// ===========================================================================

#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Copy)]
struct BrokenMessage([u8; 8]);

#[cfg(not(feature = "serde"))]
impl BytesHasLen for BrokenMessage { fn len(&self) -> usize { 6 } }

#[cfg(not(feature = "serde"))]
impl OsalSerialize for BrokenMessage { fn to_bytes(&self) -> &[u8] { &self.0 } }

#[cfg(not(feature = "serde"))]
impl OsalDeserialize for BrokenMessage {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let arr: [u8; 8] = bytes.try_into().map_err(|_| Error::InvalidMessageSize)?;
        Ok(Self(arr))
    }
}

// ===========================================================================
// Phase 2: Multi-producer/consumer — sentinel-based, uniqueness-verified
// ===========================================================================

pub fn test_queue_multi_producer_consumer() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_multi_producer_consumer");

    const PRODUCERS: usize = 4;
    const CONSUMERS: usize = 4;
    const ITERS: u32 = 250;
    const STOP: u64 = u64::MAX; // sentinel

    let queue = Arc::new(Queue::new(4, 8)?);
    let received = Arc::new(StdMutex::new(BTreeSet::new()));

    // Producers
    let mut producers = vec![];
    for pid in 0..PRODUCERS {
        let q = Arc::clone(&queue);
        producers.push(thread::spawn(move || {
            for seq in 0..ITERS {
                let msg: u64 = ((pid as u64) << 32) | (seq as u64);
                q.post(&msg.to_le_bytes(), TickType::MAX).unwrap();
            }
        }));
    }

    // Consumers — spawned first so they drain the queue concurrently
    let mut consumers = vec![];
    for _ in 0..CONSUMERS {
        let q = Arc::clone(&queue);
        let r = Arc::clone(&received);
        consumers.push(thread::spawn(move || {
            let mut buf = [0u8; 8];
            loop {
                match q.fetch(&mut buf, TickType::MAX) {
                    Ok(()) => {
                        let val = u64::from_le_bytes(buf);
                        if val == STOP { break; }
                        r.lock().unwrap().insert(val);
                    }
                    Err(Error::QueueClosed) => break,
                    Err(_) => break,
                }
            }
        }));
    }

    for p in producers { p.join().unwrap(); }

    // Send sentinels after producers finish, consumers are already draining
    for _ in 0..CONSUMERS {
        queue.post(&STOP.to_le_bytes(), TickType::MAX)?;
    }

    for c in consumers { c.join().unwrap(); }
    queue.close();

    let received = received.lock().unwrap();
    let total = received.len();

    // Verify total
    let expected_total = PRODUCERS * ITERS as usize;
    assert_eq!(total, expected_total, "expected {} messages, got {}", expected_total, total);

    // Verify per-producer each sequence is present (uniqueness)
    for pid in 0..PRODUCERS {
        for seq in 0..ITERS {
            let msg: u64 = ((pid as u64) << 32) | (seq as u64);
            assert!(received.contains(&msg), "missing message (pid={}, seq={})", pid, seq);
        }
    }

    log_info!(TAG, "test_queue_multi_producer_consumer PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Indefinite wait wake-up tests
// ===========================================================================

pub fn test_queue_max_fetch_woken_by_post() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_max_fetch_woken_by_post");
    let queue = Arc::new(Queue::new(1, 4)?);

    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 4];
        q.fetch(&mut buf, TickType::MAX)
    });

    thread::sleep(Duration::from_millis(20));
    queue.post(&[1, 2, 3, 4], 0)?;

    let result = handle.join().unwrap();
    assert_eq!(result, Ok(()));
    log_info!(TAG, "test_queue_max_fetch_woken_by_post PASSED");
    Ok(())
}

pub fn test_queue_max_post_woken_by_fetch() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_max_post_woken_by_fetch");
    let queue = Arc::new(Queue::new(1, 4)?);
    queue.post(&[9, 8, 7, 6], 0)?;

    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || {
        q.post(&[1, 2, 3, 4], TickType::MAX)
    });

    thread::sleep(Duration::from_millis(20));
    let mut buf = [0u8; 4];
    queue.fetch(&mut buf, 0)?; // drain
    assert_eq!(handle.join().unwrap(), Ok(()));
    log_info!(TAG, "test_queue_max_post_woken_by_fetch PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Finite timeout
// ===========================================================================

pub fn test_queue_fetch_timeout() -> Result<()> {
    log_info!(TAG, "Starting test_queue_fetch_timeout");
    let queue = Queue::new(1, 4)?;
    let mut buf = [0u8; 4];
    let start = std::time::Instant::now();
    assert_eq!(queue.fetch(&mut buf, 50), Err(Error::Timeout));
    assert!(start.elapsed() >= Duration::from_millis(40)); // ~50ms
    log_info!(TAG, "test_queue_fetch_timeout PASSED");
    Ok(())
}

pub fn test_queue_post_timeout() -> Result<()> {
    log_info!(TAG, "Starting test_queue_post_timeout");
    let queue = Queue::new(1, 4)?;
    queue.post(&[1, 2, 3, 4], 0)?;
    let start = std::time::Instant::now();
    assert_eq!(queue.post(&[2, 3, 4, 5], 50), Err(Error::Timeout));
    assert!(start.elapsed() >= Duration::from_millis(40));
    log_info!(TAG, "test_queue_post_timeout PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Close lifecycle
// ===========================================================================

pub fn test_queue_close_wakes_blocked_consumer() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_close_wakes_blocked_consumer");
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
    use std::thread;
    log_info!(TAG, "Starting test_queue_close_wakes_blocked_producer");
    let queue = Arc::new(Queue::new(1, 4)?);
    queue.post(&[1u8; 4], 0)?;
    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || { q.post(&[2u8; 4], TickType::MAX) });
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
    queue.close(); queue.close(); queue.close();
    assert_eq!(queue.post(&[1u8; 4], 0), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_close_idempotent PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_closed() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_closed");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(2, 6)?;
    queue.close();
    let msg = TestMessage::new(1, 2);
    assert_eq!(queue.post(&msg, 0), Err(Error::QueueClosed));
    assert_eq!(queue.fetch(&mut TestMessage::new(0, 0), 0), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_streamed_closed PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Handle
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
// Raw Queue contract tests (Phase 1)
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
    assert_eq!(queue.post(&[1, 2, 3], 0), Err(Error::InvalidMessageSize));
    let mut received = [0u8; 4];
    assert_eq!(queue.fetch(&mut received, 0), Err(Error::Timeout));
    log_info!(TAG, "test_queue_post_too_short_rejected PASSED");
    Ok(())
}

pub fn test_queue_post_too_long_rejected() -> Result<()> {
    log_info!(TAG, "Starting test_queue_post_too_long_rejected");
    let queue = Queue::new(2, 4)?;
    assert_eq!(queue.post(&[1, 2, 3, 4, 5], 0), Err(Error::InvalidMessageSize));
    let mut received = [0u8; 4];
    assert_eq!(queue.fetch(&mut received, 0), Err(Error::Timeout));
    log_info!(TAG, "test_queue_post_too_long_rejected PASSED");
    Ok(())
}

pub fn test_queue_fetch_buffer_too_short_does_not_consume() -> Result<()> {
    log_info!(TAG, "Starting test_queue_fetch_buffer_too_short_does_not_consume");
    let queue = Queue::new(2, 4)?;
    queue.post(&[10, 20, 30, 40], 0)?;
    assert_eq!(queue.fetch(&mut [0u8; 3], 0), Err(Error::InvalidMessageSize));
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
    assert_eq!(queue.fetch(&mut [0u8; 5], 0), Err(Error::InvalidMessageSize));
    let mut correct_buf = [0u8; 4];
    queue.fetch(&mut correct_buf, 0)?;
    assert_eq!(correct_buf, [1, 2, 3, 4]);
    log_info!(TAG, "test_queue_fetch_buffer_too_long_rejected PASSED");
    Ok(())
}

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
    assert_eq!(queue.fetch_from_isr(&mut [0u8; 2]), Err(Error::InvalidMessageSize));
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
    assert_eq!(queue.fetch_from_isr(&mut [0u8; 8]), Err(Error::InvalidMessageSize));
    let mut correct_buf = [0u8; 4];
    queue.fetch_from_isr(&mut correct_buf)?;
    assert_eq!(correct_buf, [3, 4, 5, 6]);
    log_info!(TAG, "test_queue_isr_fetch_buffer_too_long PASSED");
    Ok(())
}

pub fn test_queue_propagates_underlying_error() -> Result<()> {
    log_info!(TAG, "Starting test_queue_propagates_underlying_error");
    let queue = Queue::new(2, 4)?;
    assert_eq!(queue.fetch(&mut [0u8; 4], 0), Err(Error::Timeout));
    assert_eq!(queue.fetch(&mut [0u8; 8], 0), Err(Error::InvalidMessageSize));
    log_info!(TAG, "test_queue_propagates_underlying_error PASSED");
    Ok(())
}

// ===========================================================================
// Non-serde QueueStreamed
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
    let messages: Vec<_> = (0..4).map(|i| TestMessage::new(i, -(i as i16))).collect();
    for msg in &messages { queue.post(msg, 0)?; }
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
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(2, 8)?;
    assert_eq!(queue.post(&TestMessage::new(1, 2), 0), Err(Error::InvalidMessageSize));
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

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_broken_len_consistency() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_broken_len_consistency");
    let queue: QueueStreamed<BrokenMessage> = QueueStreamed::new(2, 8)?;
    let msg = BrokenMessage([1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(queue.post(&msg, 0), Err(Error::InvalidMessageSize));
    assert_eq!(queue.post_from_isr(&msg), Err(Error::InvalidMessageSize));
    log_info!(TAG, "test_queue_streamed_broken_len_consistency PASSED");
    Ok(())
}

// ===========================================================================
// Serde QueueStreamed
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
    let messages: Vec<_> = (0..4).map(|i| SerdeTestMessage { id: 10 + i, value: (i as i16 * 10) }).collect();
    for msg in &messages { queue.post(msg, 0)?; }
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
// Run all tests
// ===========================================================================

pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "========== Running Linux-Specific Queue Tests ==========");

    // Phase 1: Raw Queue contract
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

    // Phase 2: Close lifecycle
    test_queue_close_wakes_blocked_consumer()?;
    test_queue_close_wakes_blocked_producer()?;
    test_queue_all_ops_fail_after_close()?;
    test_queue_close_idempotent()?;

    // Phase 2: Handle
    test_queue_unique_handles()?;

    // Phase 2: MAX delay
    test_queue_max_fetch_woken_by_post()?;
    test_queue_max_post_woken_by_fetch()?;

    // Phase 2: Finite timeout
    test_queue_fetch_timeout()?;
    test_queue_post_timeout()?;

    // Phase 2: Multi-producer/consumer liveness
    test_queue_multi_producer_consumer()?;

    // QueueStreamed close
    #[cfg(not(feature = "serde"))]
    { test_queue_streamed_closed()?; }

    // Non-serde QueueStreamed
    #[cfg(not(feature = "serde"))]
    {
        test_queue_streamed_round_trip()?;
        test_queue_streamed_fifo()?;
        test_queue_streamed_wrong_message_size()?;
        test_queue_streamed_isr_round_trip()?;
        test_queue_streamed_broken_len_consistency()?;
    }

    // Serde QueueStreamed
    #[cfg(feature = "serde")]
    {
        test_queue_streamed_serde_round_trip()?;
        test_queue_streamed_serde_fifo()?;
        test_queue_streamed_serde_isr_round_trip()?;
    }

    log_info!(TAG, "========== All Linux-Specific Queue Tests PASSED ==========");
    Ok(())
}