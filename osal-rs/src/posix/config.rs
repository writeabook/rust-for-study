//! POSIX backend configuration.
//!
//! POSIX does not provide an RTOS configuration model like FreeRTOSConfig.h.
//! The OSAL POSIX backend therefore defines a small set of logical backend
//! constants directly in Rust.
//!
//! At this stage, the POSIX configuration is intentionally minimal. Additional
//! values such as priority ranges, default stack size, and task name limits
//! should be added later when `posix/thread.rs` and `posix/types.rs` are fully
//! decoupled from the Linux backend.

/// Tick period in milliseconds.
///
/// POSIX itself does not define an RTOS tick. The OSAL POSIX backend uses a
/// logical tick to provide a stable timing abstraction for APIs that accept
/// tick counts or `core::time::Duration`.
///
/// With a value of `1`, one OSAL tick represents one millisecond of monotonic
/// wall-clock time.
///
/// This value must remain constant for the entire process lifetime.
pub const TICK_PERIOD_MS: u64 = 1;
