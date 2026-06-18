//! POSIX backend test module.
//!
//! These tests verify that the POSIX backend (currently a thin wrapper
//! over the Linux reference implementation) correctly exposes all OSAL
//! APIs and passes behavioural tests.
//!
//! # Structure
//!
//! - Common tests (from `crate::common`) — cross-backend behaviour
//!   assertions shared with FreeRTOS and Linux.
//! - POSIX-specific tests (future) — native POSIX primitives
//!   (`pthread_mutex_t`, `sem_open`, `mq_open`, …) will be tested
//!   here as individual modules are replaced.

// ---------------------------------------------------------------------------
// Common cross-backend tests
// ---------------------------------------------------------------------------

#[test]
fn test_run_all_tests_duration() {
    crate::common::duration_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_system() {
    crate::common::system_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_mutex_common() {
    crate::common::mutex_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_semaphore() {
    crate::common::semaphore_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_event_group() {
    crate::common::event_group_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_queue_common() {
    crate::common::queue_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_thread() {
    crate::common::thread_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_timer_common() {
    crate::common::timer_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_api_surface() {
    crate::common::api_surface::run_all_tests().unwrap();
}
