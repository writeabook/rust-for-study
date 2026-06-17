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

//! Semaphore stub for the POSIX backend (experimental / unimplemented).
//!
//! **WARNING:** This module provides no real semaphore semantics.
//! All operations are stubs that succeed without synchronization.
//! This backend should not be used in production.

use core::fmt::{Debug, Display};
use core::ops::Deref;
use core::ptr::null;

use crate::posix::types::{SemaphoreHandle, UBaseType};
use crate::traits::{SemaphoreFn, ToTick};
use crate::utils::{OsalRsBool, Result};

pub struct Semaphore(SemaphoreHandle);

unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}

impl Semaphore {
	pub fn new(_max_count: UBaseType, _initial_count: UBaseType) -> Result<Self> {
		Ok(Self(null()))
	}

	pub fn new_with_count(_initial_count: UBaseType) -> Result<Self> {
		Ok(Self(null()))
	}
}

impl SemaphoreFn for Semaphore {
	fn wait(&self, _ticks_to_wait: impl ToTick) -> OsalRsBool {
		todo!("POSIX Semaphore::wait not implemented")
	}

	fn wait_from_isr(&self) -> OsalRsBool {
		todo!("POSIX Semaphore::wait_from_isr not implemented")
	}

	fn signal(&self) -> OsalRsBool {
		todo!("POSIX Semaphore::signal not implemented")
	}

	fn signal_from_isr(&self) -> OsalRsBool {
		todo!("POSIX Semaphore::signal_from_isr not implemented")
	}

	fn delete(&mut self) {
		self.0 = null();
	}
}

impl Drop for Semaphore {
	fn drop(&mut self) {
		self.0 = null();
	}
}

impl Deref for Semaphore {
	type Target = SemaphoreHandle;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl Debug for Semaphore {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		f.debug_struct("Semaphore")
			.field("handle", &self.0)
			.finish()
	}
}

impl Display for Semaphore {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		write!(f, "Semaphore {{ handle: {:?} }}", self.0)
	}
}