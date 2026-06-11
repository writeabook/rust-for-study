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

//! System-level operations stub for Linux backend.
//!
//! Placeholder until the full implementation is developed.

use alloc::vec::Vec;
use crate::linux::thread::{ThreadMetadata, ThreadState};

#[derive(Debug, Clone)]
pub struct SystemState {
    pub tasks: Vec<ThreadMetadata>,
    pub total_run_time: u32,
}

impl core::ops::Deref for SystemState {
    type Target = [ThreadMetadata];

    fn deref(&self) -> &Self::Target {
        &self.tasks
    }
}

use crate::linux::types::TickType;
use core::time::Duration;

pub struct System;

impl System {
    #[allow(dead_code)]
    pub fn get_current_time_us() -> Duration {
        Duration::ZERO
    }

    #[allow(dead_code)]
    pub fn get_tick_count() -> TickType {
        0
    }
}
