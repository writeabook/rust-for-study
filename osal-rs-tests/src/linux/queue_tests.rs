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
//! These tests verify the fixed-length message contract enforced by the
//! Linux Queue backend, as well as typed `QueueStreamed` round-trip
//! correctness. They are **not** part of the cross-backend common test
//! suite.

extern crate alloc;

use osal_rs::os::*;
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

#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TestMessage {
    id: u32,
    value: i16,
}

#[cfg(not(feature = "serde"))]
impl BytesHasLen for TestMessage {
    fn len(&self) -> usize { 6 }
}

#[cfg(not(feature = "serde"))]
impl OsalSerialize for TestMessage {
    fn to_bytes(&self) -> &[u8] {
        // Safety: TestMessage is a plain struct with a known layout.
        // This is only used in host (Linux) tests.
        unsafe {
            core::slice::from_raw_parts(
                self as *const Self as *const u8,
                6,
            )
        }
    }
}

#[cfg(not(feature = "serde"))]
impl OsalDeserialize for TestMessage {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 6 {
            return Err(Error::InvalidMessageSize);
        }
        Ok(TestMessage {
            id: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            value: i16::from_le_bytes([bytes[4], bytes[5]]),
        })
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

    let msg = TestMessage { id: 7, value: -12 };
    queue.post(&msg, 0)?;

    let mut received = TestMessage { id: 0, value: 0 };
    queue.fetch(&mut received, 0)?;

    assert_eq!(received, msg);
    log_info!(TAG, "test_queue_streamed_round_trip PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_fifo() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_fifo");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(4, 6)?;

    let messages: Vec<_> = (0..4)
        .map(|i| TestMessage { id: i, value: -(i as i16) })
        .collect();

    for msg in &messages {
        queue.post(msg, 0)?;
    }

    for expected in &messages {
        let mut received = TestMessage { id: 0, value: 0 };
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

    let msg = TestMessage { id: 1, value: 2 };
    let result = queue.post(&msg, 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    log_info!(TAG, "test_queue_streamed_wrong_message_size PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_isr_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_isr_round_trip");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(2, 6)?;

    let msg = TestMessage { id: 42, value: 99 };
    queue.post_from_isr(&msg)?;

    let mut received = TestMessage { id: 0, value: 0 };
    queue.fetch_from_isr(&mut received)?;

    assert_eq!(received, msg);
    log_info!(TAG, "test_queue_streamed_isr_round_trip PASSED");
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

// ===========================================================================
// Run all tests
// ===========================================================================

pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "========== Running Linux-Specific Queue Tests ==========");

    // Raw Queue length-contract tests
    test_queue_exact_message_size()?;
    test_queue_post_too_short_rejected()?;
    test_queue_post_too_long_rejected()?;
    test_queue_fetch_buffer_too_short_does_not_consume()?;
    test_queue_fetch_buffer_too_long_rejected()?;

    // ISR path length-contract tests
    test_queue_isr_post_too_short()?;
    test_queue_isr_post_too_long()?;
    test_queue_isr_fetch_buffer_too_short()?;
    test_queue_isr_fetch_buffer_too_long()?;

    // Error propagation
    test_queue_propagates_underlying_error()?;

    // Non-serde QueueStreamed tests
    #[cfg(not(feature = "serde"))]
    {
        test_queue_streamed_round_trip()?;
        test_queue_streamed_fifo()?;
        test_queue_streamed_wrong_message_size()?;
        test_queue_streamed_isr_round_trip()?;
    }

    // Serde QueueStreamed tests
    #[cfg(feature = "serde")]
    {
        test_queue_streamed_serde_round_trip()?;
        test_queue_streamed_serde_fifo()?;
    }

    log_info!(TAG, "========== All Linux-Specific Queue Tests PASSED ==========");
    Ok(())
}