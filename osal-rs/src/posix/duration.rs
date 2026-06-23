//! POSIX backend duration conversions.
//!
//! POSIX does not provide an RTOS tick by itself. The OSAL POSIX backend
//! defines a logical tick period through `posix::config`, then uses that
//! period to convert between `core::time::Duration` and OSAL tick counts.
//!
//! # Conversion Rules
//!
//! - **Duration → ticks**: ceiling rounding at nanosecond granularity so
//!   that a non-zero sub-tick duration always waits for at least one tick.
//! - **Ticks → Duration**: `tick × TICK_PERIOD_MS`, saturated on overflow.
//!
//! # Examples
//!
//! ```ignore
//! use core::time::Duration;
//! use osal_rs::os::{ToTick, FromTick};
//!
//! // 1 ms = 1 tick (when TICK_PERIOD_MS = 1)
//! let d = Duration::from_micros(1500);
//! assert_eq!(d.to_ticks(), 2);   // ceiling: 1.5 ms → 2 ticks
//! ```

use core::time::Duration;

use crate::traits::{FromTick, ToTick};

use super::config::TICK_PERIOD_MS;
use super::types::TickType;

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Converts a `Duration` to a POSIX logical tick count with ceiling rounding.
///
/// Nanosecond-level rounding guarantees that a non-zero duration shorter than
/// one tick period still waits for at least one tick.
///
/// # Overflow
///
/// If the resulting tick count exceeds `TickType::MAX`, the function returns
/// `TickType::MAX`.
#[inline]
fn duration_to_ticks(duration: Duration) -> TickType {
    let period_ms = TICK_PERIOD_MS as u128;

    if period_ms == 0 {
        return TickType::MAX;
    }

    let period_ns = period_ms.saturating_mul(1_000_000);

    if period_ns == 0 {
        return TickType::MAX;
    }

    let duration_ns = duration.as_nanos();

    if duration_ns == 0 {
        return 0;
    }

    // Round up so that non-zero sub-tick durations still wait
    // for at least one OSAL tick.
    let ticks = duration_ns.saturating_add(period_ns - 1) / period_ns;

    ticks.min(TickType::MAX as u128) as TickType
}

/// Converts a POSIX tick count back to a `Duration`.
///
/// Multiplies `tick` by [`TICK_PERIOD_MS`], saturating at `u64::MAX`
/// milliseconds.
#[inline]
fn ticks_to_duration(tick: TickType) -> Duration {
    let millis = (tick as u128).saturating_mul(TICK_PERIOD_MS as u128);
    let millis = millis.min(u64::MAX as u128) as u64;

    Duration::from_millis(millis)
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

impl ToTick for TickType {
    /// Identity conversion: a raw tick count is already in ticks.
    #[inline]
    fn to_ticks(&self) -> TickType {
        *self
    }
}

impl ToTick for Duration {
    /// Converts a `Duration` into a POSIX logical tick count with ceiling
    /// rounding.
    ///
    /// # Behavior
    ///
    /// | Duration            | Ticks (TICK_PERIOD_MS = 1) |
    /// |---------------------|----------------------------|
    /// | 0 ns                | 0                          |
    /// | 1 ns                | 1                          |
    /// | 999 µs              | 1                          |
    /// | 1 ms                | 1                          |
    /// | 1.5 ms              | 2                          |
    /// | 100 ms              | 100                        |
    /// | Huge Duration       | `TickType::MAX`            |
    #[inline]
    fn to_ticks(&self) -> TickType {
        duration_to_ticks(*self)
    }
}

impl FromTick for Duration {
    /// Overwrites `self` with a [`Duration`] representing the given tick
    /// count.
    ///
    /// Multiplies `tick` by [`TICK_PERIOD_MS`] and converts the result to
    /// milliseconds.  Saturated multiplication prevents panics on overflow.
    #[inline]
    fn ticks(&mut self, tick: TickType) {
        *self = ticks_to_duration(tick);
    }
}
