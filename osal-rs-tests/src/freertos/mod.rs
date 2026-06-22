/***************************************************************************
 *
 * osal-rs
 * Copyright (C) 2026 Antonio Salsi <passy.linux@zresa.it>
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <https://www.gnu.org/licenses/>.
 *
 ***************************************************************************/

use osal_rs::log_info;
use osal_rs::utils::Result;

const TAG: &str = "FreeRTOSTests";

/// Run all available FreeRTOS tests
pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "\n\n========================================");
    log_info!(TAG, "   Starting FreeRTOS Test Suite");
    log_info!(TAG, "========================================\n");

    crate::common::duration_tests::run_all_tests()?;
    crate::common::event_group_tests::run_all_tests()?;
    crate::common::mutex_tests::run_all_tests()?;
    crate::common::queue_tests::run_all_tests()?;
    crate::common::semaphore_tests::run_all_tests()?;
    crate::common::system_tests::run_all_tests()?;
    crate::common::thread_tests::run_all_tests()?;
    crate::common::timer_tests::run_all_tests()?;
    crate::common::api_surface::run_all_tests()?;

    log_info!(TAG, "\n========================================");
    log_info!(TAG, "   All FreeRTOS Tests PASSED!");
    log_info!(TAG, "========================================\n");
    Ok(())
}
