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

//! POSIX backend configuration.
//!
//! POSIX does not provide an RTOS configuration model like FreeRTOSConfig.h.
//! The OSAL POSIX backend therefore defines a small set of logical backend
//! constants directly in Rust.
//!
//! At this stage, the POSIX configuration is intentionally minimal. Additional
//! values such as priority ranges, default stack size, and task name limits
//! should be added later when `posix/thread.rs` and `posix/types.rs` are fully
//! decoupled from the Linux backend.

/// Tick period in milliseconds.
///
/// POSIX itself does not define an RTOS tick. The OSAL POSIX backend uses a
/// logical tick to provide a stable timing abstraction for APIs that accept
/// tick counts or `core::time::Duration`.
///
/// With a value of `1`, one OSAL tick represents one millisecond of monotonic
/// wall-clock time.
///
/// This value must remain constant for the entire process lifetime.
pub const TICK_PERIOD_MS: u64 = 1;
