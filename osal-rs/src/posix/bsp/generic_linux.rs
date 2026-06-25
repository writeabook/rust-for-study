//! Generic Linux BSP configuration for the POSIX backend.
//!
//! This module provides the platform-level constants and type aliases
//! needed when the POSIX backend runs on a Linux host (glibc / musl).
//! It is the equivalent of NASA OSAL's `src/bsp/generic-linux/`.

// ---------------------------------------------------------------------------
// Platform configuration
// ---------------------------------------------------------------------------

/// Tick period in milliseconds.
///
/// With a value of `1`, one OSAL tick represents one millisecond of
/// monotonic wall-clock time.  This must remain constant for the
/// entire process lifetime.
pub const TICK_PERIOD_MS: u64 = 1;

// ---------------------------------------------------------------------------
// Platform type aliases
// ---------------------------------------------------------------------------

use core::ffi::c_void;

/// System tick count type (`u32`, wrapping).
pub type TickType = u32;

/// Signed base type for return values and comparisons (`i32`).
pub type BaseType = i32;

/// Unsigned base type for sizes, counts, and non-negative values (`u32`).
pub type UBaseType = u32;

/// Stack size unit (bytes on Linux).
pub type StackType = u32;

/// Event-group bitmask type.
pub type EventBits = TickType;

// ---------------------------------------------------------------------------
// Opaque handle types
// ---------------------------------------------------------------------------

/// Opaque handle to a POSIX backend thread.
pub type ThreadHandle = *const c_void;

/// Opaque handle to a POSIX backend queue.
pub type QueueHandle = *const c_void;

/// Opaque handle to a POSIX backend semaphore.
pub type SemaphoreHandle = *const c_void;

/// Opaque handle to a POSIX backend mutex.
pub type MutexHandle = *const c_void;

/// Opaque handle to a POSIX backend event group.
pub type EventGroupHandle = *const c_void;

/// Opaque handle to a POSIX backend software timer.
pub type TimerHandle = *const c_void;
