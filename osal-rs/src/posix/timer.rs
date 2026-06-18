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

//! POSIX backend timer — delegates to the Linux reference implementation.
//!
//! A Timer Service Thread + deadline heap + lazy invalidation implementation
//! (FreeRTOS Timer Service Task pattern) is under development — see
//! `refactor/posix-arch` branch history.  The one-shot path works correctly;
//! periodic auto-reload has a clock-synchronisation issue between
//! CLOCK_MONOTONIC and pthread_cond_timedwait that needs deeper debugging.
//!
//! # Future direction
//!
//! Re-visit once the mutex / semaphore / queue native POSIX implementations
//! are stable and the condvar clock attribute issue is resolved.

pub use crate::linux::timer::*;
