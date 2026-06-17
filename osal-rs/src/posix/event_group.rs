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

//! Event group stub for the POSIX backend (experimental / unimplemented).
//!
//! **WARNING:** This module provides no real event group semantics.
//! All operations are stubs that return default values. This backend
//! should not be used in production.

use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::ptr::null;

use crate::posix::types::{EventBits, EventGroupHandle, TickType};
use crate::traits::{EventGroupFn, ToTick};
use crate::utils::{Error, Result};

pub struct EventGroup(EventGroupHandle);

unsafe impl Send for EventGroup {}
unsafe impl Sync for EventGroup {}

impl EventGroup {
	pub const MAX_MASK: EventBits = EventBits::MAX >> 8;

	pub fn wait_with_to_tick(&self, mask: EventBits, timeout_ticks: impl ToTick) -> EventBits {
		self.wait(mask, timeout_ticks.to_ticks())
	}

	pub fn new() -> Result<Self> {
		Ok(Self(null()))
	}
}

impl EventGroupFn for EventGroup {
	fn set(&self, _bits: EventBits) -> EventBits {
		todo!("POSIX EventGroup::set not implemented")
	}

	fn set_from_isr(&self, _bits: EventBits) -> Result<()> {
		todo!("POSIX EventGroup::set_from_isr not implemented")
	}

	fn get(&self) -> EventBits {
		todo!("POSIX EventGroup::get not implemented")
	}

	fn get_from_isr(&self) -> EventBits {
		todo!("POSIX EventGroup::get_from_isr not implemented")
	}

	fn clear(&self, _bits: EventBits) -> EventBits {
		todo!("POSIX EventGroup::clear not implemented")
	}

	fn clear_from_isr(&self, _bits: EventBits) -> Result<()> {
		todo!("POSIX EventGroup::clear_from_isr not implemented")
	}

	fn wait(&self, _mask: EventBits, _timeout_ticks: TickType) -> EventBits {
		todo!("POSIX EventGroup::wait not implemented")
	}

	fn delete(&mut self) {
		self.0 = null();
	}
}

impl Drop for EventGroup {
	fn drop(&mut self) {
		self.0 = null();
	}
}

impl Deref for EventGroup {
	type Target = EventGroupHandle;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl Debug for EventGroup {
	fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
		write!(f, "EventGroup {{ handle: {:?} }}", self.0)
	}
}

impl Display for EventGroup {
	fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
		write!(f, "EventGroup {{ handle: {:?} }}", self.0)
	}
}