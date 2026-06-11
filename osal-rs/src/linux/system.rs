//! System-level operations stub for the Linux backend.
//!
//! # Overview
//!
//! Provides the `System` struct and `SystemState` snapshot type required
//! by the [`SystemFn`] trait. The current implementation is a **stub**
//! — it satisfies trait signatures so the crate compiles, but returns
//! placeholder values. A full implementation based on `std::time::Instant`
//! and OS-level thread enumeration will replace this stub.
//!
//! # Stub Limitations (v0.1)
//!
//! | Method                  | Current behaviour                              |
//! |-------------------------|------------------------------------------------|
//! | `get_tick_count()`      | Returns `0`                                    |
//! | `get_current_time_us()` | Returns `Duration::ZERO`                       |
//! | `delay()`               | Not yet implemented                            |
//! | `delay_until()`         | Not yet implemented                            |
//! | `check_timer()`         | Not yet implemented                            |
//! | `critical_section_*()`  | Not yet implemented                            |
//! | `start()` / `stop()`    | No-op (documented)                             |
//!
//! # Future Implementation
//!
//! A future version will use `std::time::Instant::now()` as the monotonic
//! clock source, `std::thread::sleep` for delays, and a static `Mutex<…>`
//! for critical sections. See `doc/osal-contact-zh.md` §3–4 for the
//! detailed behavioural contract.

use alloc::vec::Vec;
use crate::linux::thread::{ThreadMetadata, ThreadState};

/// Snapshot of system-wide thread state.
///
/// Captures metadata for every thread known to the OSAL runtime at the
/// moment of collection, together with an aggregate run-time counter.
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::{System, SystemFn};
///
/// let state = System::get_all_thread();
/// println!("Tasks: {}, runtime: {}", state.tasks.len(), state.total_run_time);
/// for meta in &state.tasks {
///     println!("  {} — priority {}", meta.name, meta.priority);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct SystemState {
    /// Metadata for each tracked thread.
    pub tasks: Vec<ThreadMetadata>,
    /// Accumulated run-time across all threads (milliseconds).
    pub total_run_time: u32,
}

impl core::ops::Deref for SystemState {
    type Target = [ThreadMetadata];

    fn deref(&self) -> &Self::Target {
        &self.tasks
    }
}

use crate::linux::types::TickType;
use core::time::Duration;

/// System-level operations.
///
/// Static methods mirroring the FreeRTOS `System` API. Most methods
/// are stubs in v0.1 and will be filled in as the Linux backend
/// matures.
pub struct System;

impl System {
    /// Returns the current OSAL tick count.
    ///
    /// **Stub** — currently always returns `0`. Will be backed by
    /// `std::time::Instant` in the full implementation.
    #[allow(dead_code)]
    pub fn get_tick_count() -> TickType {
        0
    }

    /// Returns the current monotonic time as a `Duration`.
    ///
    /// **Stub** — currently always returns `Duration::ZERO`.
    #[allow(dead_code)]
    pub fn get_current_time_us() -> Duration {
        Duration::ZERO
    }
}