pub mod duration_tests;
pub mod system_tests;
pub mod mutex_tests;
pub mod semaphore_tests;
pub mod event_group_tests;
pub mod queue_tests;
pub mod thread_tests;

// The following modules require types that are only implemented
// in the FreeRTOS backend. They will be enabled for Linux backend
// as each module is implemented.
#[cfg(feature = "freertos")]
pub mod api_surface;
#[cfg(any(feature = "freertos", feature = "linux"))]
pub mod timer_tests;
