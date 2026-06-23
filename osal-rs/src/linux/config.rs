//! Configuration constants for the Linux OSAL backend.
//!
//! # Overview
//!
//! These constants control the behaviour of the Linux backend. Unlike
//! the FreeRTOS backend, which derives its configuration from the
//! C toolchain and `FreeRTOSConfig.h`, the Linux backend defines all
//! constants inline using safe Rust.
//!
//! # Tick Period
//!
//! The tick period maps one OSAL tick to one millisecond of wall-clock
//! time. This is the same default used by the POSIX backend and is chosen
//! to keep duration ↔ tick conversions simple and predictable.
//!
//! # Examples
//!
//! ```ignore
//! use osal_rs::os::traits::ToTick;
//! use core::time::Duration;
//!
//! // 100 ms → 100 ticks (when TICK_PERIOD_MS = 1)
//! let ticks = Duration::from_millis(100).to_ticks();
//! assert_eq!(ticks, 100);
//! ```

/// Tick period in milliseconds.
///
/// Determines how many wall-clock milliseconds one OSAL tick represents.
///
/// With a value of `1`, one tick equals one millisecond, making
/// duration ↔ tick conversions identity operations (modulo rounding).
///
/// # Safety
///
/// Changing this value at runtime is not supported. It must remain
/// stable across the entire process lifetime to ensure consistent
/// timing behaviour.
pub const TICK_PERIOD_MS: u64 = 1;
