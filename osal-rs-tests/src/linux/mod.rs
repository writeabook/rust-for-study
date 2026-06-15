#[test]
fn test_run_all_tests_duration() {
    crate::common::duration_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_system() {
    crate::common::system_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_mutex() {
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
fn test_run_all_tests_queue() {
    crate::common::queue_tests::run_all_tests().unwrap();
}

#[test]
fn test_run_all_tests_thread() {
    crate::common::thread_tests::run_all_tests().unwrap();
}

// #[test]
// fn test_system_get_tick_count() {
//     crate::common::system_tests::test_system_get_tick_count().unwrap();
// }

// #[test]
// fn test_system_get_current_time() {
//     crate::common::system_tests::test_system_get_current_time().unwrap();
// }

// #[test]
// fn test_system_delay() {
//     crate::common::system_tests::test_system_delay().unwrap();
// }

// #[test]
// fn test_system_delay_until() {
//     crate::common::system_tests::test_system_delay_until().unwrap();
// }

// #[test]
// fn test_system_check_timer() {
//     crate::common::system_tests::test_system_check_timer().unwrap();
// }

// #[test]
// fn test_system_time_conversion() {
//     crate::common::system_tests::test_system_time_conversion().unwrap();
// }

// #[test]
// fn test_system_multiple_delays() {
//     crate::common::system_tests::test_system_multiple_delays().unwrap();
// }

// #[test]
// fn test_system_time_monotonic() {
//     crate::common::system_tests::test_system_time_monotonic().unwrap();
// }
