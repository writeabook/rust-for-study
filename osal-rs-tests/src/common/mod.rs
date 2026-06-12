pub mod duration_tests;
pub mod system_tests;
pub mod mutex_tests;

// The following modules require types that are only implemented
// in the FreeRTOS backend. They will be enabled for Linux backend
// as each module is implemented.
#[cfg(feature = "freertos")]
pub mod api_surface;
#[cfg(feature = "freertos")]
pub mod event_group_tests;
#[cfg(feature = "freertos")]
pub mod queue_tests;
#[cfg(feature = "freertos")]
pub mod semaphore_tests;
#[cfg(feature = "freertos")]
pub mod thread_tests;
#[cfg(feature = "freertos")]
pub mod timer_tests;
