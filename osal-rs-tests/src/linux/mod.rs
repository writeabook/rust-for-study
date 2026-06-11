use osal_rs::utils::Result;

#[test]
fn test_run_all_tests() {
    crate::common::duration_tests::run_all_tests().unwrap();
}
