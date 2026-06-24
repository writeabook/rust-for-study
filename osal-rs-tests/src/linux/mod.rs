//! Linux backend test runner.
//!
//! Each `#[test]` function executes a shared OSAL contract test suite
//! from `crate::api` against the Linux reference backend.
//!
//! Linux-backend-specific smoke tests (handle uniqueness, poison
//! recovery, ISR emulation, etc.) live in
//! `crate::port::linux_legacy_smoke_tests`.

#[test]
fn test_run_all_tests_duration() {
    crate::api::duration_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_system() {
    crate::api::system_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_mutex_common() {
    crate::api::mutex_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_semaphore() {
    crate::api::semaphore_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_event_group() {
    crate::api::event_group_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_queue_common() {
    crate::api::queue_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_thread() {
    crate::api::thread_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_timer_common() {
    crate::api::timer_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_api_surface() {
    crate::api::api_surface::run_all_tests().unwrap();
}
