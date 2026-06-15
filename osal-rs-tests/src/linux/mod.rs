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

mod mutex_tests;

#[test]
fn test_run_all_tests_mutex_linux() {
    crate::linux::mutex_tests::run_all_tests().unwrap();
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
fn test_run_all_tests_queue() {
    crate::common::queue_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_thread() {
    crate::common::thread_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_timer() {
    crate::common::timer_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_api_surface() {
    crate::common::api_surface::run_all_tests().unwrap();
}
