//! OSAL public API contract tests.
//!
//! Tests in this module verify the OSAL public API behavior that every
//! supported backend must satisfy. They are backend-agnostic — they only
//! use `osal_rs::os::*` and never reference backend-internal types.

pub mod api_surface;
pub mod duration_tests;
pub mod event_group_tests;
pub mod mutex_tests;
pub mod queue_tests;
pub mod semaphore_tests;
pub mod system_tests;
pub mod thread_tests;
pub mod timer_tests;
