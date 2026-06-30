use core::ffi::c_int;

// #[path = "../examples/portable_osal_integration_demo.rs"]
#[path = "../examples/typed_message_queue_demo.rs"]
mod portable_demo;

#[cfg(feature = "freertos")]
#[unsafe(no_mangle)]
pub extern "C" fn rust_demo_entry() -> c_int {
    match portable_demo::freertos_demo_entry() {
        Ok(_) => 0,
        Err(_) => -1,
    }
}
