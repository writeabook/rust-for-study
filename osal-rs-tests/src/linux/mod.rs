pub mod duration_tests;

use osal_rs::utils::Result;
use osal_rs::log_info;

const TAG: &str = "LinuxTests";

/// Run all available Linux tests
pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "\n\n========================================");
    log_info!(TAG, "   Starting Linux Test Suite");
    log_info!(TAG, "========================================\n");

    duration_tests::run_all_tests()?;

    log_info!(TAG, "\n========================================");
    log_info!(TAG, "   All Linux Tests PASSED!");
    log_info!(TAG, "========================================\n");
    Ok(())
}