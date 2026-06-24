//! Semaphore boundary and edge-case tests.
//!
//! These tests cover edge behaviour that every backend should handle,
//! but which is not part of the normal success-path contract verified
//! in `api/semaphore_tests.rs`.
//!
//! ISR-path tests live in backend-specific port smoke tests, not here.

use core::time::Duration;

use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};

pub fn run_all_tests() -> Result<()> {
    semaphore_wait_zero_is_non_blocking()?;
    semaphore_signal_fails_when_full()?;
    semaphore_wait_times_out()?;
    Ok(())
}

fn semaphore_wait_zero_is_non_blocking() -> Result<()> {
    let sem = Semaphore::new(1, 0)?;
    assert_eq!(sem.wait(Duration::ZERO), OsalRsBool::False);
    Ok(())
}

fn semaphore_signal_fails_when_full() -> Result<()> {
    let sem = Semaphore::new(1, 1)?;
    assert_eq!(sem.signal(), OsalRsBool::False);
    Ok(())
}

fn semaphore_wait_times_out() -> Result<()> {
    let sem = Semaphore::new(1, 0)?;
    assert_eq!(sem.wait(Duration::from_millis(10)), OsalRsBool::False);
    Ok(())
}
