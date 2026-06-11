use osal_rs::utils::Result;

/// Run all currently-implemented Linux backend tests
pub fn run_all_tests() -> Result<()> {
    crate::common::duration_tests::run_all_tests()?;
    // TODO: Add more modules as Linux backend implements them:
    //   crate::common::mutex_tests::run_all_tests()?;
    //   crate::common::semaphore_tests::run_all_tests()?;
    //   crate::common::queue_tests::run_all_tests()?;
    //   crate::common::event_group_tests::run_all_tests()?;
    //   crate::common::system_tests::run_all_tests()?;
    //   crate::common::thread_tests::run_all_tests()?;
    //   crate::common::timer_tests::run_all_tests()?;
    Ok(())
}

#[test]
fn test_duration_to_ticks() {
    crate::common::duration_tests::test_duration_to_ticks().unwrap();
}

#[test]
fn test_duration_from_ticks() {
    crate::common::duration_tests::test_duration_from_ticks().unwrap();
}

#[test]
fn test_duration_conversion_roundtrip() {
    crate::common::duration_tests::test_duration_conversion_roundtrip().unwrap();
}

#[test]
fn test_duration_zero() {
    crate::common::duration_tests::test_duration_zero().unwrap();
}

#[test]
fn test_duration_one_second() {
    crate::common::duration_tests::test_duration_one_second().unwrap();
}

#[test]
fn test_duration_microseconds() {
    crate::common::duration_tests::test_duration_microseconds().unwrap();
}

#[test]
fn test_duration_large_value() {
    crate::common::duration_tests::test_duration_large_value().unwrap();
}

#[test]
fn test_run_all_tests() {
    run_all_tests().unwrap();
}
