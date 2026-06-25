//! Type definitions for the POSIX OSAL backend.
//!
//! Re-exports platform type aliases from the active BSP target.
//! Currently the only BSP is `generic_linux`.

pub use super::bsp::generic_linux::{
    BaseType, EventBits, EventGroupHandle, MutexHandle, QueueHandle, SemaphoreHandle, StackType,
    ThreadHandle, TickType, TimerHandle, UBaseType,
};
