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

//! BSP (Board Support Package) selection for the POSIX backend.
//!
//! Following NASA OSAL's three-layer architecture, the BSP layer provides
//! **platform-specific configuration** while the POSIX module provides
//! **OS adaptation** (mapping OSAL traits to POSIX APIs).
//!
//! # Architecture
//!
//! ```text
//!   Application code
//!        ↓
//!   pub mod os (unified API)
//!        ↓
//!   posix/ (POSIX adaptation layer — OS trait → POSIX API mapping)
//!        ↓
//!   posix/bsp/ (BSP selection — platform config: types, tick period)
//!        ↓
//!   linux/ (reference host implementation — Rust std primitives)
//!        ↓
//!   Linux kernel / glibc / musl
//! ```
//!
//! # Current BSP: Linux
//!
//! The Linux BSP is the default and currently the only BSP target.
//! It re-uses `crate::linux::config` (TICK_PERIOD_MS = 1) and
//! `crate::linux::types` (TickType = u32, etc.) as its platform
//! configuration.
//!
//! # Adding a new BSP (future work)
//!
//! To add a new BSP target (e.g., macOS, FreeBSD):
//!
//! 1. Create `posix/bsp/<target>.rs` exporting `pub mod config; pub mod types;`
//! 2. Add a feature flag (e.g., `bsp-macos`) in `Cargo.toml`
//! 3. Update `posix/config.rs` and `posix/types.rs` to conditionally
//!    re-export from the correct BSP
//!
//! # Relationship to NASA OSAL
//!
//! | NASA OSAL                | osal-rs                          |
//! |--------------------------|----------------------------------|
//! | `src/os/shared/`         | `crate::linux/` (reference impl) |
//! | `src/os/posix/`          | `crate::posix/` (adaptation)     |
//! | `src/bsp/generic-linux/` | `crate::posix::bsp` (BSP select) |
//! | `src/os/rtems/`          | `crate::freertos/` (embedded)    |
