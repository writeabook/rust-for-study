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

//! Type definitions for the Linux OSAL backend.
//!
//! These are placeholder type aliases needed for trait code compilation
//! during initial development. Will be refined as the backend is implemented.

use core::ffi::c_void;

/// System tick count type.
pub type TickType = u32;

/// Signed base type for return values and comparisons.
pub type BaseType = i32;

/// Unsigned base type for sizes and counts.
pub type UBaseType = u32;

/// Stack size type.
pub type StackType = u32;

/// Event bits type (24 usable bits, top 8 reserved).
pub type EventBits = TickType;

/// Opaque handle to a thread (placeholder).
pub type ThreadHandle = *const c_void;

/// Opaque handle to a queue (placeholder).
pub type QueueHandle = *const c_void;

/// Opaque handle to a semaphore (placeholder).
pub type SemaphoreHandle = *const c_void;

/// Opaque handle to a mutex (placeholder).
pub type MutexHandle = *const c_void;

/// Opaque handle to an event group (placeholder).
pub type EventGroupHandle = *const c_void;

/// Opaque handle to a timer (placeholder).
pub type TimerHandle = *const c_void;