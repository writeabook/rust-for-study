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

//! Thread types stub for Linux backend.
//!
//! Placeholder until the full implementation is developed.

/// Thread execution state enumeration.
///
/// Mirrors the states defined in the OSAL traits layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThreadState {
    #[default]
    Running = 0,
    Ready = 1,
    Blocked = 2,
    Suspended = 3,
    Deleted = 4,
    Invalid,
}

/// Metadata about a thread.
#[derive(Debug, Clone, Default)]
pub struct ThreadMetadata {
    pub name: &'static str,
    pub state: ThreadState,
    pub priority: u32,
    pub stack_size: u32,
}