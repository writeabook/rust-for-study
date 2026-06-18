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

//! POSIX OSAL backend module.
//!
//! This module provides the POSIX (host) backend for the OSAL-RS abstraction
//! layer.  It is currently built on top of the Linux reference implementation
//! (`crate::linux`), which uses safe Rust standard library primitives.  Each
//! sub-module re-exports the corresponding Linux implementation.
//!
//! # Architecture (NASA OSAL pattern)
//!
//! Following NASA's OSAL architecture, **POSIX is the adaptation layer** and
//! **Linux is one BSP / reference implementation** of that layer:
//!
//! ```text
//!   Application code
//!        ↓
//!   pub mod os  (unified API)
//!        ↓
//!   posix/      (POSIX adaptation layer — thin wrapper)
//!        ↓
//!   linux/      (reference host implementation — full code)
//!        ↓
//!   std::thread, std::sync::Mutex, std::sync::Condvar, ...
//! ```
//!
//! # Design
//!
//! - Currently all sub-modules delegate to the Linux backend via `pub use`.
//! - Individual modules can be replaced with native POSIX primitives
//!   (`pthread_mutex_t`, `sem_open`, `mq_open`, `timer_create`, …) in
//!   future phases **without affecting the Linux backend**.
//! - The Linux backend remains independently usable via
//!   `--features linux,std`.
//!
//! # Modules
//!
//! - [`config`] — Backend-wide constants (tick period, feature flags).
//! - [`types`] — Type aliases (TickType, handle types, etc.).
//! - [`duration`] — `ToTick` / `FromTick` impls for `core::time::Duration`.
//! - [`system`] — System-level operations (time, delays, critical sections).
//! - [`thread`] — Thread state, metadata, registry, and cooperative cancellation.
//! - [`mutex`] — `RawMutex` (recursive) and `Mutex<T>` (non-recursive).
//! - [`semaphore`] — Binary and counting semaphores.
//! - [`event_group`] — Multi-bit event-group synchronization.
//! - [`queue`] — Raw `Queue` and type-safe `QueueStreamed<T>`.
//! - [`timer`] — Periodic and one-shot software timers.
//! - [`bsp`] — Board Support Package selection (platform-specific config).

pub mod bsp;
pub mod config;
pub(crate) mod duration;
pub mod event_group;
pub(crate) mod ffi;
pub mod mutex;
pub mod queue;
pub mod semaphore;
pub mod system;
pub mod thread;
pub mod timer;
pub mod types;
