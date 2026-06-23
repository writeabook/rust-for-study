//! Shared OSAL contract tests.
//!
//! This module contains the portable behavioural tests that every OSAL
//! backend must pass.  The tests verify the trait contracts defined in
//! `osal-rs/src/traits/` — mutex lock/unlock, semaphore wait/signal,
//! queue post/fetch timeout, event-group set/wait/clear, thread
//! spawn/join/notify, timer start/reset/stop/delete, and system
//! delay/tick semantics.
//!
//! These tests are **backend-agnostic**: they never reference `pthread`,
//! `std::thread`, FreeRTOS C APIs, or any backend-internal types.
//! Each backend (`linux/mod.rs`, `posix/mod.rs`, `freertos/mod.rs`)
//! runs the same suite through its own `#[test]` entry points.

pub mod api_surface;
pub mod duration_tests;
pub mod event_group_tests;
pub mod mutex_tests;
pub mod queue_tests;
pub mod semaphore_tests;
pub mod system_tests;
pub mod thread_tests;
pub mod timer_tests;
