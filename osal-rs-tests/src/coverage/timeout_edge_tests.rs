//! Timeout-related boundary and edge-case tests.
//!
//! Tests that verify timeout behaviour: zero timeout does not block,
//! short timeouts return within a reasonable upper bound, and
//! conversions of extreme duration values do not overflow.

use core::time::Duration;

use osal_rs::os::*;
use osal_rs::os::types::TickType;
use osal_rs::utils::{OsalRsBool, Result};

pub fn run_all_tests() -> Result<()> {
    timeout_zero_semaphore_wait_does_not_block()?;
    timeout_short_semaphore_wait_returns_after_expiry()?;
    timeout_max_delay_does_not_overflow()?;
    Ok(())
}

fn timeout_zero_semaphore_wait_does_not_block() -> Result<()> {
    let sem = Semaphore::new(1, 0)?;
    let start = System::get_current_time_us();
    let _ = sem.wait(Duration::ZERO);
    let elapsed = System::get_current_time_us() - start;
    // Zero-timeout wait must return quickly (well under 500ms).
    assert!(elapsed < Duration::from_millis(500));
    Ok(())
}

fn timeout_short_semaphore_wait_returns_after_expiry() -> Result<()> {
    let sem = Semaphore::new(1, 0)?;
    let start = System::get_current_time_us();
    let result = sem.wait(Duration::from_millis(20));
    let elapsed = System::get_current_time_us() - start;
    // Must time out (no signal was posted).
    assert_eq!(result, OsalRsBool::False);
    // Should return within 5 seconds — very loose upper bound.
    assert!(elapsed < Duration::from_secs(5));
    Ok(())
}

fn timeout_max_delay_does_not_overflow() -> Result<()> {
    let tick = System::get_us_from_tick(&Duration::MAX);
    // The conversion must produce a finite tick value.
    assert!(tick > 0);
    Ok(())
}
