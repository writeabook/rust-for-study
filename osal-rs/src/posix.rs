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

#[cfg(feature = "std")]
pub mod config;

#[cfg(feature = "std")]
pub(crate) mod duration;

#[cfg(feature = "std")]
pub mod event_group;

#[cfg(feature = "std")]
pub mod mutex;

#[cfg(feature = "std")]
pub mod queue;

#[cfg(feature = "std")]
pub mod semaphore;

#[cfg(feature = "std")]
pub mod system;

#[cfg(feature = "std")]
pub mod thread;

#[cfg(feature = "std")]
pub mod timer;

#[cfg(feature = "std")]
pub mod types;