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

#![cfg_attr(not(feature = "posix"), no_std)]

extern crate alloc;

mod common;

// Layered test modules
// api/ is NOT behind #[cfg(test)] because the FreeRTOS backend runner
// exposes pub fn run_all_tests() for use from embedded firmware.
mod api;

#[cfg(test)]
mod unit;

#[cfg(test)]
mod coverage;

#[cfg(test)]
mod port;

#[cfg(feature = "freertos")]
pub mod freertos;

#[cfg(feature = "posix")]
pub mod posix;
