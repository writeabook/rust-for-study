//! Tick ↔ Duration conversion for the Linux backend.
//!
//! # Overview
//!
//! Implements the [`ToTick`] and [`FromTick`] traits for
//! `core::time::Duration`, enabling the same duration-to-tick
//! conversion API that the FreeRTOS backend provides.
//!
//! # Conversion Rules
//!
//! - **Duration → ticks**: `to_ticks()` divides the duration's
//!   millisecond value by [`TICK_PERIOD_MS`]. Zero-period fallback
//!   returns [`TickType::MAX`] to avoid division-by-zero panics.
//! - **Ticks → Duration**: `ticks()` multiplies ticks by
//!   [`TICK_PERIOD_MS`], saturating on overflow.
//!
//! # Examples
//!
//! ```ignore
//! use core::time::Duration;
//! use osal_rs::os::{ToTick, FromTick};
//!
//! let d = Duration::from_millis(100);
//! assert_eq!(d.to_ticks(), 100);       // duration → ticks
//!
//! let mut d2 = Duration::ZERO;
//! d2.ticks(500);
//! assert_eq!(d2.as_millis(), 500);     // ticks → duration
//! ```

use core::time::Duration;

use super::config::TICK_PERIOD_MS;
use super::types::TickType;
use crate::traits::{FromTick, ToTick};

#[cfg(not(feature = "posix"))]
impl ToTick for TickType {
    /// Identity conversion: a raw tick count is already in ticks.
    fn to_ticks(&self) -> TickType {
        *self
    }
}

#[cfg(not(feature = "posix"))]
impl ToTick for Duration {
    /// Converts a `Duration` into an OSAL tick count.
    ///
    /// Divides the total milliseconds by [`TICK_PERIOD_MS`].
    /// When the tick period is zero (invalid configuration) the
    /// function returns [`TickType::MAX`] rather than panicking.
    ///
    /// Very large durations are saturated to [`TickType::MAX`]
    /// rather than truncated.
    ///
    /// # Returns
    ///
    /// Number of whole OSAL ticks represented by this duration.
    /// Fractional-tick remainders are truncated (integer division).
    fn to_ticks(&self) -> TickType {
        let millis = self.as_millis();
        let millis = millis.min(TickType::MAX as u128) as TickType;
        let period = TICK_PERIOD_MS as TickType;

        if period == 0 {
            TickType::MAX
        } else {
            millis / period
        }
    }
}

#[cfg(not(feature = "posix"))]
impl FromTick for Duration {
    /// Overwrites `self` with a `Duration` representing the given
    /// tick count.
    ///
    /// Multiplies `tick` by [`TICK_PERIOD_MS`] and converts the
    /// result to milliseconds. Saturated multiplication prevents
    /// panics on overflow.
    ///
    /// # Parameters
    ///
    /// * `tick` — OSAL tick count to convert.
    fn ticks(&mut self, tick: TickType) {
        *self = Duration::from_millis(tick.saturating_mul(TICK_PERIOD_MS as TickType) as u64);
    }
}
