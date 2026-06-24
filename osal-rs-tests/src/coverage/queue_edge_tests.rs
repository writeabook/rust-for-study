//! Queue boundary and edge-case tests.
//!
//! These tests cover parameter validation, timeout behaviour, close
//! semantics, and typed QueueStreamed edge cases.  Normal success-path
//! queue tests live in `api/queue_tests.rs`.

use core::time::Duration;

use osal_rs::os::types::TickType;
use osal_rs::os::*;
use osal_rs::utils::{Error, Result};

// --- helpers ---

fn queue_filled_to_capacity(n: usize) -> Queue {
    let q = Queue::new(n as _, 4).unwrap();
    let data = [1u8, 2, 3, 4];
    for _ in 0..n {
        q.post(&data, 0).unwrap();
    }
    q
}

// ===========================================================================
// Parameter validation / message size
// ===========================================================================

pub fn run_all_tests() -> Result<()> {
    queue_exact_message_size_round_trip()?;
    queue_post_too_short_rejected()?;
    queue_post_too_long_rejected()?;
    queue_fetch_buffer_too_short_does_not_consume()?;
    queue_fetch_buffer_too_long_rejected()?;
    queue_fetch_timeout_returns_error()?;
    queue_post_timeout_when_full_returns_error()?;
    queue_close_is_idempotent()?;
    queue_close_blocked_consumer_wakes()?;
    queue_close_blocked_producer_wakes()?;
    queue_all_ops_fail_after_close()?;
    queue_send_to_full_queue_returns_full()?;
    queue_receive_from_empty_queue_times_out()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Message size
// ---------------------------------------------------------------------------

fn queue_exact_message_size_round_trip() -> Result<()> {
    let q = Queue::new(4, 4)?;
    let data = [1u8, 2, 3, 4];
    let mut buf = [0u8; 4];
    q.post(&data, 0)?;
    q.fetch(&mut buf, 0)?;
    assert_eq!(buf, data);
    Ok(())
}

fn queue_post_too_short_rejected() -> Result<()> {
    let q = Queue::new(4, 4)?;
    let too_short = [1u8, 2];
    let err = q.post(&too_short, 0).unwrap_err();
    assert_eq!(err, Error::InvalidMessageSize);
    Ok(())
}

fn queue_post_too_long_rejected() -> Result<()> {
    let q = Queue::new(4, 4)?;
    let too_long = [1u8, 2, 3, 4, 5];
    let err = q.post(&too_long, 0).unwrap_err();
    assert_eq!(err, Error::InvalidMessageSize);
    Ok(())
}

fn queue_fetch_buffer_too_short_does_not_consume() -> Result<()> {
    let q = Queue::new(4, 4)?;
    let data = [1u8, 2, 3, 4];
    q.post(&data, 0)?;
    let mut too_short = [0u8; 2];
    let err = q.fetch(&mut too_short, 0).unwrap_err();
    assert_eq!(err, Error::InvalidMessageSize);
    // The message must still be retrievable.
    let mut ok_buf = [0u8; 4];
    q.fetch(&mut ok_buf, 0)?;
    assert_eq!(ok_buf, data);
    Ok(())
}

fn queue_fetch_buffer_too_long_rejected() -> Result<()> {
    let q = Queue::new(4, 4)?;
    let data = [1u8, 2, 3, 4];
    q.post(&data, 0)?;
    let mut too_long = [0u8; 8];
    let err = q.fetch(&mut too_long, 0).unwrap_err();
    assert_eq!(err, Error::InvalidMessageSize);
    Ok(())
}

// ---------------------------------------------------------------------------
// Timeout
// ---------------------------------------------------------------------------

fn queue_fetch_timeout_returns_error() -> Result<()> {
    let q = Queue::new(4, 4)?;
    let mut buf = [0u8; 4];
    let err = q.fetch(&mut buf, 1 as TickType).unwrap_err();
    assert_eq!(err, Error::QueueReceiveTimeout);
    Ok(())
}

fn queue_post_timeout_when_full_returns_error() -> Result<()> {
    let q = queue_filled_to_capacity(4);
    let data = [1u8, 2, 3, 4];
    let err = q.post(&data, 1 as TickType).unwrap_err();
    assert_eq!(err, Error::QueueSendTimeout);
    Ok(())
}

// ---------------------------------------------------------------------------
// Close sematics
// ---------------------------------------------------------------------------

fn queue_close_is_idempotent() -> Result<()> {
    let q = Queue::new(4, 4)?;
    q.close();
    q.close(); // second close must not panic
    Ok(())
}

fn queue_close_blocked_consumer_wakes() -> Result<()> {
    use std::sync::Arc;
    let q = Arc::new(Queue::new(4, 4)?);
    let q2 = q.clone();
    let handle = std::thread::spawn(move || {
        let mut buf = [0u8; 4];
        let _ = q2.fetch(&mut buf, u32::MAX);
    });
    std::thread::sleep(Duration::from_millis(20));
    q.close();
    handle.join().unwrap();
    Ok(())
}

fn queue_close_blocked_producer_wakes() -> Result<()> {
    use std::sync::Arc;
    let q = Arc::new(queue_filled_to_capacity(4));
    let q2 = q.clone();
    let handle = std::thread::spawn(move || {
        let data = [1u8, 2, 3, 4];
        let _ = q2.post(&data, u32::MAX);
    });
    std::thread::sleep(Duration::from_millis(20));
    q.close();
    handle.join().unwrap();
    Ok(())
}

fn queue_all_ops_fail_after_close() -> Result<()> {
    let q = Queue::new(4, 4)?;
    q.close();
    let data = [1u8; 4];
    let mut buf = [0u8; 4];
    assert_eq!(q.post(&data, 0).unwrap_err(), Error::QueueClosed);
    assert_eq!(q.fetch(&mut buf, 0).unwrap_err(), Error::QueueClosed);
    Ok(())
}

// ---------------------------------------------------------------------------
// Full / empty
// ---------------------------------------------------------------------------

fn queue_send_to_full_queue_returns_full() -> Result<()> {
    let q = queue_filled_to_capacity(4);
    let data = [1u8; 4];
    let err = q.post(&data, 0).unwrap_err();
    assert!(err == Error::QueueFull || err == Error::QueueSendTimeout);
    Ok(())
}

fn queue_receive_from_empty_queue_times_out() -> Result<()> {
    let q = Queue::new(4, 4)?;
    let mut buf = [0u8; 4];
    let err = q.fetch(&mut buf, 0).unwrap_err();
    assert!(err == Error::QueueReceiveTimeout);
    Ok(())
}
