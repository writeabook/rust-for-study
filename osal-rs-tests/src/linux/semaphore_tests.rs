//! Linux-specific semaphore tests.
//!
//! These tests supplement the common semaphore test suite with tests
//! that exercise Linux-backend-specific behaviours: unique handles,
//! non-blocking ISR paths, infinite wait, finite timeout, and
//! signal-wake semantics.

use core::time::Duration;

use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};

/// Entry-point called from `mod.rs` to run all Linux-specific
/// semaphore tests.
pub fn run_all_tests() -> Result<()> {
    semaphore_handles_are_unique()?;
    semaphore_wait_zero_is_non_blocking()?;
    semaphore_signal_wakes_waiter()?;
    semaphore_signal_fails_when_full()?;
    semaphore_wait_times_out()?;
    semaphore_isr_paths_are_non_blocking()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handle uniqueness
// ---------------------------------------------------------------------------

/// Two new semaphore objects must have distinct handles.
fn semaphore_handles_are_unique() -> Result<()> {
    let s1 = Semaphore::new(1, 1)?;
    let s2 = Semaphore::new(1, 1)?;

    assert_ne!(*s1, *s2);
    Ok(())
}

// ---------------------------------------------------------------------------
// Non-blocking wait(0)
// ---------------------------------------------------------------------------

/// `wait(Duration::ZERO)` must return `False` immediately when the
/// count is 0.
fn semaphore_wait_zero_is_non_blocking() -> Result<()> {
    let sem = Semaphore::new(1, 0)?;

    assert_eq!(sem.wait(Duration::ZERO), OsalRsBool::False);
    Ok(())
}

// ---------------------------------------------------------------------------
// Signal wakes a blocked waiter
// ---------------------------------------------------------------------------

/// Thread A blocks on infinite wait.  Thread B calls `signal()` and
/// Thread A wakes up successfully.
fn semaphore_signal_wakes_waiter() -> Result<()> {
    use std::sync::Arc;

    // Duration::from_millis(u32::MAX) produces UBaseType::MAX ticks,
    // which triggers true infinite wait via Condvar::wait().
    let infinite = Duration::from_millis(u32::MAX as u64);

    let sem = Arc::new(Semaphore::new(1, 0)?);
    let sem_waiter = Arc::clone(&sem);

    let handle = std::thread::spawn(move || sem_waiter.wait(infinite));

    // Give the waiter time to block on the condvar.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(sem.signal(), OsalRsBool::True);

    assert_eq!(handle.join().unwrap(), OsalRsBool::True);
    Ok(())
}

// ---------------------------------------------------------------------------
// Signal when full
// ---------------------------------------------------------------------------

/// `signal()` must return `False` when the count is already at
/// `max_count`.
fn semaphore_signal_fails_when_full() -> Result<()> {
    let sem = Semaphore::new(1, 1)?;

    assert_eq!(sem.signal(), OsalRsBool::False);
    Ok(())
}

// ---------------------------------------------------------------------------
// Finite timeout
// ---------------------------------------------------------------------------

/// `wait(finite_duration)` returns `False` after the timeout expires
/// with no signal.
fn semaphore_wait_times_out() -> Result<()> {
    let sem = Semaphore::new(1, 0)?;

    assert_eq!(sem.wait(Duration::from_millis(10)), OsalRsBool::False);
    Ok(())
}

// ---------------------------------------------------------------------------
// ISR simulation paths
// ---------------------------------------------------------------------------

/// `wait_from_isr()` and `signal_from_isr()` are non-blocking and
/// follow the same counting semantics as the blocking variants.
fn semaphore_isr_paths_are_non_blocking() -> Result<()> {
    // Binary semaphore with initial count = 1.
    let sem = Semaphore::new(1, 1)?;

    // First ISR take must succeed (count 1 → 0).
    assert_eq!(sem.wait_from_isr(), OsalRsBool::True);
    // Second ISR take must fail (count is 0).
    assert_eq!(sem.wait_from_isr(), OsalRsBool::False);

    // ISR give must succeed (count 0 → 1).
    assert_eq!(sem.signal_from_isr(), OsalRsBool::True);
    // Next ISR give must fail (count is already at max).
    assert_eq!(sem.signal_from_isr(), OsalRsBool::False);

    Ok(())
}