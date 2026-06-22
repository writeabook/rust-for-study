/***************************************************************************
 *                                                                         *
 * osal-rs                                                                 *
 * Copyright (C) 2026 Antonio Salsi <passy.linux@zresa.it>                  *
 *                                                                         *
 * This library is free software; you can redistribute it and/or            *
 * modify it under the terms of the GNU Lesser General Public               *
 * License as published by the Free Software Foundation; either             *
 * version 2.1 of the License, or (at your option) any later version.       *
 *                                                                         *
 * This library is distributed in the hope that it will be useful,          *
 * but WITHOUT ANY WARRANTY; without even the implied warranty of           *
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU         *
 * Lesser General Public License for more details.                          *
 *                                                                         *
 * You should have received a copy of the GNU Lesser General Public         *
 * License along with this library; if not, see <https://www.gnu.org/licenses/>.
 *                                                                         *
 ***************************************************************************/

//! Type definitions for the POSIX OSAL backend.
//!
//! These aliases mirror the FreeRTOS type layer so that traits and application
//! code remain backend-agnostic.
//!
//! POSIX does not define RTOS-style tick or handle types. The OSAL POSIX
//! backend therefore defines a small set of logical types that can be mapped
//! to pthread/libc-backed implementations over time.
//!
//! At this stage, handle types intentionally remain opaque pointers. Concrete
//! POSIX modules should manage their own safe Rust wrappers internally and only
//! expose these aliases when a trait or compatibility layer needs an opaque
//! backend handle.

use core::ffi::c_void;

/// System tick count type.
///
/// The POSIX backend uses a logical OSAL tick, currently configured through
/// `crate::posix::config::TICK_PERIOD_MS`. Overflow behavior follows normal
/// unsigned wrapping semantics.
pub type TickType = u32;

/// Signed base type for function return values and comparisons.
///
/// This mirrors the FreeRTOS-style `BaseType_t` role used by portable OSAL
/// APIs.
pub type BaseType = i32;

/// Unsigned base type for sizes, counts, and non-negative values.
pub type UBaseType = u32;

/// Stack size type.
///
/// For the POSIX backend this represents a stack-size unit used by the OSAL
/// thread abstraction. When `posix/thread.rs` is fully migrated to pthreads,
/// this value can be interpreted as bytes for `pthread_attr_setstacksize`.
pub type StackType = u32;

/// Event bits type.
///
/// Holds an event-group bitmask. The top bits may be reserved by OSAL
/// implementations for internal bookkeeping, so application code should use
/// masks exposed by the event-group abstraction rather than relying on every
/// bit being available.
pub type EventBits = TickType;

/// Opaque handle to a POSIX backend thread.
///
/// This remains an opaque pointer during the transition period. The concrete
/// `Thread` wrapper owns and manages the actual thread state.
pub type ThreadHandle = *const c_void;

/// Opaque handle to a POSIX backend queue.
///
/// This remains an opaque pointer during the transition period. The concrete
/// `Queue` wrapper owns and manages the actual queue state.
pub type QueueHandle = *const c_void;

/// Opaque handle to a POSIX backend semaphore.
///
/// This remains an opaque pointer during the transition period. The concrete
/// `Semaphore` wrapper owns and manages the actual semaphore state.
pub type SemaphoreHandle = *const c_void;

/// Opaque handle to a POSIX backend mutex.
///
/// This remains an opaque pointer during the transition period. The concrete
/// `Mutex` wrapper owns and manages the actual mutex state.
pub type MutexHandle = *const c_void;

/// Opaque handle to a POSIX backend event group.
///
/// This remains an opaque pointer during the transition period. The concrete
/// `EventGroup` wrapper owns and manages the actual event-group state.
pub type EventGroupHandle = *const c_void;

/// Opaque handle to a POSIX backend software timer.
///
/// This remains an opaque pointer during the transition period. The concrete
/// `Timer` wrapper owns and manages the actual timer state.
pub type TimerHandle = *const c_void;
