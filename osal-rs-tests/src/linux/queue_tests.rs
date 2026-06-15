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
//! Linux Queue backend. They use `std::thread` and are therefore **not**
//! part of the cross-backend common test suite.

extern crate alloc;

use osal_rs::os::*;
use osal_rs::utils::{Error, Result};
use osal_rs::{log_debug, log_info};

const TAG: &str = "LinuxQueueTests";

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

    // Too short — must be rejected
    let result = queue.post(&[1, 2, 3], 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    // Verify queue state unchanged — still empty
    let mut received = [0u8; 4];
    let fetch_result = queue.fetch(&mut received, 0);
    assert!(fetch_result.is_err()); // queue is empty

    log_info!(TAG, "test_queue_post_too_short_rejected PASSED");
    Ok(())
}

pub fn test_queue_post_too_long_rejected() -> Result<()> {
    log_info!(TAG, "Starting test_queue_post_too_long_rejected");
    let queue = Queue::new(2, 4)?;

    let result = queue.post(&[1, 2, 3, 4, 5], 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    // Verify queue state unchanged
    let mut received = [0u8; 4];
    let fetch_result = queue.fetch(&mut received, 0);
    assert!(fetch_result.is_err()); // queue is empty

    log_info!(TAG, "test_queue_post_too_long_rejected PASSED");
    Ok(())
}

pub fn test_queue_fetch_buffer_too_short_does_not_consume() -> Result<()> {
    log_info!(TAG, "Starting test_queue_fetch_buffer_too_short_does_not_consume");
    let queue = Queue::new(2, 4)?;

    // Send correct 4-byte message
    queue.post(&[10, 20, 30, 40], 0)?;

    // Try to fetch with 3-byte buffer — must fail
    let mut short_buf = [0u8; 3];
    let result = queue.fetch(&mut short_buf, 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

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
    let result = queue.fetch(&mut long_buf, 0);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    // Message must still be in the queue
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

    let result = queue.post_from_isr(&[1, 2]);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    // Queue still empty
    let mut buf = [0u8; 4];
    assert!(queue.fetch_from_isr(&mut buf).is_err());

    log_info!(TAG, "test_queue_isr_post_too_short PASSED");
    Ok(())
}

pub fn test_queue_isr_post_too_long() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_post_too_long");
    let queue = Queue::new(2, 4)?;

    let result = queue.post_from_isr(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    let mut buf = [0u8; 4];
    assert!(queue.fetch_from_isr(&mut buf).is_err());

    log_info!(TAG, "test_queue_isr_post_too_long PASSED");
    Ok(())
}

pub fn test_queue_isr_fetch_buffer_too_short() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_fetch_buffer_too_short");
    let queue = Queue::new(2, 4)?;

    queue.post_from_isr(&[7, 8, 9, 10])?;

    let mut short_buf = [0u8; 2];
    let result = queue.fetch_from_isr(&mut short_buf);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    // Message still there
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
    let result = queue.fetch_from_isr(&mut long_buf);
    assert_eq!(result, Err(Error::InvalidMessageSize));

    // Message still there
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

    // Normal: empty queue with timeout=0 returns Timeout
    assert_eq!(queue.fetch(&mut buf, 0), Err(Error::Timeout));

    // Wrong buffer size returns InvalidMessageSize, NOT Timeout
    let mut wrong_buf = [0u8; 8];
    assert_eq!(queue.fetch(&mut wrong_buf, 0), Err(Error::InvalidMessageSize));

    log_info!(TAG, "test_queue_propagates_underlying_error PASSED");
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
    log_info!(TAG, "========== All Linux-Specific Queue Tests PASSED ==========");
    Ok(())
}