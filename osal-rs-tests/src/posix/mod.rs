//! POSIX backend test runner.
//!
//! The tests in this module primarily execute the shared OSAL contract
//! tests from `crate::common` against the POSIX backend. Backend-specific
//! POSIX tests should be limited to implementation details that are not part
//! of the portable OSAL contract, such as pthread stack-size clamping or
//! CLOCK_MONOTONIC timeout behavior.

// ---------------------------------------------------------------------------
// Shared OSAL contract tests
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
