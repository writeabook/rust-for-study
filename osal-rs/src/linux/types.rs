//! Type definitions for the Linux OSAL backend.
//!
//! # Overview
//!
//! These type aliases mirror the FreeRTOS type layer so that traits and
//! application code remain backend-agnostic. The Linux backend uses
//! the same integer widths (`u32` / `i32`) as a typical 32-bit
//! FreeRTOS configuration.
//!
//! # Handle Types
//!
//! Handle types are opaque pointer wrappers (`*const c_void`) in the
//! initial stub phase. As modules are implemented they will be replaced
//! with real Rust types (`Arc<Inner>`, numeric IDs, or `Box<dyn …>`).
//! Until then the pointer types satisfy trait signatures and allow the
//! crate to compile.
//!
//! # Generated Types
//!
//! | OSAL type          | Linux mapping  | Notes                             |
//! |--------------------|----------------|-----------------------------------|
//! | `TickType`         | `u32`          | System tick counter               |
//! | `BaseType`         | `i32`          | Signed return / comparison type   |
//! | `UBaseType`        | `u32`          | Unsigned size / count type        |
//! | `StackType`        | `u32`          | Stack size unit                   |
//! | `EventBits`        | `TickType`     | Event-group bitmask (24 bits)     |
//! | `ThreadHandle`     | `*const c_void`| Opaque thread reference           |
//! | `QueueHandle`      | `*const c_void`| Opaque queue reference            |
//! | `SemaphoreHandle`  | `*const c_void`| Opaque semaphore reference        |
//! | `MutexHandle`      | `*const c_void`| Opaque mutex reference            |
//! | `EventGroupHandle` | `*const c_void`| Opaque event-group reference      |
//! | `TimerHandle`      | `*const c_void`| Opaque timer reference            |
//!
//! # Examples
//!
//! ```ignore
//! use osal_rs::os::types::TickType;
//!
//! let ticks: TickType = 1000;
//! println!("{} ticks", ticks);
//! ```

use core::ffi::c_void;

/// System tick count type.
///
/// Incremented by the RTOS tick interrupt (or, on Linux, derived from
/// `std::time::Instant`). Overflow behaviour follows standard unsigned
/// wrapping.
pub type TickType = u32;

/// Signed base type for function return values and comparisons.
///
/// Used by FreeRTOS-compatible APIs that return `pdPASS` / `pdFAIL`
/// or error codes.
pub type BaseType = i32;

/// Unsigned base type for sizes, counts, and non-negative values.
pub type UBaseType = u32;

/// Stack size type.
///
/// Represents the size of a thread's stack in architecture-specific
/// units (bytes on Linux, words on some FreeRTOS ports).
pub type StackType = u32;

/// Event bits type.
///
/// Holds an event-group bitmask. The top 8 bits (24–31) are reserved
/// for internal use by the RTOS; application code should only use
/// bits 0–23.
pub type EventBits = TickType;

/// Opaque handle to a thread.
///
/// Placeholder pointer type. Will be replaced with a real Rust handle
/// when the thread module is fully implemented.
pub type ThreadHandle = *const c_void;

/// Opaque handle to a queue.
///
/// Placeholder pointer type. Will be replaced when the queue module
/// is implemented.
pub type QueueHandle = *const c_void;

/// Opaque handle to a semaphore.
///
/// Placeholder pointer type. Will be replaced when the semaphore
/// module is implemented.
pub type SemaphoreHandle = *const c_void;

/// Opaque handle to a mutex.
///
/// Placeholder pointer type. Will be replaced when the mutex module
/// is implemented.
pub type MutexHandle = *const c_void;

/// Opaque handle to an event group.
///
/// Placeholder pointer type. Will be replaced when the event-group
/// module is implemented.
pub type EventGroupHandle = *const c_void;

/// Opaque handle to a software timer.
///
/// Placeholder pointer type. Will be replaced when the timer module
/// is implemented.
pub type TimerHandle = *const c_void;