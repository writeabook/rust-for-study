//! Linux-specific event group tests.
//!
//! These tests supplement the common event group test suite with tests
//! that exercise Linux-backend-specific behaviours: unique handles,
//! non-blocking ISR paths, infinite wait, reserved-bit masking,
//! and signal-wake semantics.

use osal_rs::os::*;
use osal_rs::os::types::TickType;
use osal_rs::utils::Result;

/// Entry-point called from `mod.rs` to run all Linux-specific
/// event group tests.
pub fn run_all_tests() -> Result<()> {
    event_group_handles_are_unique()?;
    event_group_wait_zero_is_non_blocking()?;
    event_group_wait_max_blocks_until_set()?;
    event_group_finite_wait_wakes_before_timeout()?;
    event_group_isr_paths_are_non_blocking()?;
    event_group_reserved_bits_are_masked()?;
    event_group_clear_reserved_bits_is_noop()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handle uniqueness
// ---------------------------------------------------------------------------

/// Two new event group objects must have distinct handles.
fn event_group_handles_are_unique() -> Result<()> {
    let e1 = EventGroup::new()?;
    let e2 = EventGroup::new()?;

    assert_ne!(*e1, *e2);
    Ok(())
}

// ---------------------------------------------------------------------------
// Non-blocking wait(0)
// ---------------------------------------------------------------------------

/// `wait(mask, 0)` must return immediately when no bits are set.
fn event_group_wait_zero_is_non_blocking() -> Result<()> {
    let events = EventGroup::new()?;

    let result = events.wait(0b0001, 0 as TickType);

    // No bits were set; bit 0 should be 0.
    assert_eq!(result & 0b0001, 0);
    Ok(())
}

// ---------------------------------------------------------------------------
// Infinite wait with wake-up
// ---------------------------------------------------------------------------

/// Thread A blocks on `wait(mask, TickType::MAX)`.  Thread B sets a
/// matching bit and Thread A wakes up successfully.
///
/// Uses a `Barrier` to ensure the waiter thread has truly begun
/// blocking before the signal is sent, making the test rigorous.
fn event_group_wait_max_blocks_until_set() -> Result<()> {
    use std::sync::{Arc, Barrier};

    let barrier = Arc::new(Barrier::new(2));
    let events = Arc::new(EventGroup::new()?);
    let waiter_events = Arc::clone(&events);
    let waiter_barrier = Arc::clone(&barrier);

    let handle = std::thread::spawn(move || {
        waiter_barrier.wait(); // signal: waiter about to enter wait()
        waiter_events.wait(0b0010, TickType::MAX)
    });

    barrier.wait(); // both threads ready; waiter will now block
    // Brief yield to let the waiter thread acquire the lock and block on the condvar.
    std::thread::sleep(std::time::Duration::from_millis(10));

    events.set(0b0010);

    let result = handle.join().unwrap();
    assert_ne!(result & 0b0010, 0);
    Ok(())
}

// ---------------------------------------------------------------------------
// Finite wait with successful wake-up
// ---------------------------------------------------------------------------

/// Thread A blocks on `wait(mask, finite_ticks)`.  Thread B sets a
/// matching bit before the timeout expires, and Thread A wakes up
/// successfully (exercises the `wait_timeout` wake-up path).
fn event_group_finite_wait_wakes_before_timeout() -> Result<()> {
    use std::sync::Arc;

    let events = Arc::new(EventGroup::new()?);
    let waiter_events = Arc::clone(&events);

    let handle = std::thread::spawn(move || waiter_events.wait(0b0100, 200 as TickType));

    // Give the waiter time to block on the condvar.
    std::thread::sleep(std::time::Duration::from_millis(20));
    events.set(0b0100);

    let result = handle.join().unwrap();
    assert_ne!(result & 0b0100, 0);

    Ok(())
}

// ---------------------------------------------------------------------------
// ISR simulation paths
// ---------------------------------------------------------------------------

/// `set_from_isr()`, `get_from_isr()`, and `clear_from_isr()` must be
/// non-blocking and follow correct bit-manipulation semantics.
fn event_group_isr_paths_are_non_blocking() -> Result<()> {
    let events = EventGroup::new()?;

    // Set a bit from ISR.
    assert!(events.set_from_isr(0b0001).is_ok());
    // Read it back.
    assert_ne!(events.get_from_isr() & 0b0001, 0);

    // Clear it from ISR.
    assert!(events.clear_from_isr(0b0001).is_ok());
    // Should be gone.
    assert_eq!(events.get_from_isr() & 0b0001, 0);

    Ok(())
}

// ---------------------------------------------------------------------------
// Reserved bit masking
// ---------------------------------------------------------------------------

/// Reserved bits (above `MAX_MASK`) must be silently ignored by `set()`,
/// `get()`, and `wait()`.
fn event_group_reserved_bits_are_masked() -> Result<()> {
    let events = EventGroup::new()?;

    let reserved_bits = !EventGroup::MAX_MASK;

    // Setting reserved bits must be a no-op for those bits.
    let result = events.set(reserved_bits);
    assert_eq!(result & reserved_bits, 0);

    // get() must not expose reserved bits.
    assert_eq!(events.get() & reserved_bits, 0);

    Ok(())
}

// ---------------------------------------------------------------------------
// Clearing reserved bits is a no-op
// ---------------------------------------------------------------------------

/// Clearing reserved bits must not affect usable bits.
fn event_group_clear_reserved_bits_is_noop() -> Result<()> {
    let events = EventGroup::new()?;

    // Set some usable bits.
    events.set(0b1111);

    // Clear reserved bits — usable bits must stay.
    let reserved_bits = !EventGroup::MAX_MASK;
    let result = events.clear(reserved_bits);

    // The usable bits (0b1111) must still be set.
    assert_eq!(result & 0b1111, 0b1111);

    Ok(())
}