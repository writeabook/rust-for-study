//! Configuration constants for the Linux OSAL backend.
//!
//! On Linux, 1 tick = 1 millisecond (TICK_PERIOD_MS = 1).
//! These defaults match the POSIX backend.

/// Tick period in milliseconds (1 tick = 1 ms on Linux).
pub const TICK_PERIOD_MS: u64 = 1;