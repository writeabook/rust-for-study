//! Event group boundary and edge-case tests.
//!
//! These tests cover edge behaviour that every backend should handle,
//! but which is not part of the normal success-path contract verified
//! in `api/event_group_tests.rs`.
//!
//! ISR-path tests live in backend-specific port smoke tests, not here.

use osal_rs::os::types::TickType;
use osal_rs::os::*;
use osal_rs::utils::Result;

pub fn run_all_tests() -> Result<()> {
    event_group_wait_zero_is_non_blocking()?;
    event_group_reserved_bits_are_masked()?;
    event_group_clear_reserved_bits_is_noop()?;
    Ok(())
}

fn event_group_wait_zero_is_non_blocking() -> Result<()> {
    let events = EventGroup::new()?;
    let result = events.wait(0b0001, 0 as TickType);
    // No bits were set; wait(0) must return immediately with 0.
    assert_eq!(result & 0b0001, 0);
    Ok(())
}

fn event_group_reserved_bits_are_masked() -> Result<()> {
    let events = EventGroup::new()?;
    // Setting bits above MAX_MASK must be silently masked.
    let reserved = !EventGroup::MAX_MASK;
    events.set(reserved);
    let bits = events.get();
    assert_eq!(bits & reserved, 0);
    Ok(())
}

fn event_group_clear_reserved_bits_is_noop() -> Result<()> {
    let events = EventGroup::new()?;
    events.set(0b0001);
    let reserved = !EventGroup::MAX_MASK;
    // Clearing reserved bits must not affect valid bits.
    let after = events.clear(reserved);
    assert_eq!(after & 0b0001, 0b0001);
    Ok(())
}
