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

//! POSIX OSAL backend — native pthread implementation.
//!
//! This module provides the POSIX (host) backend for OSAL-RS, built on
//! `libc::pthread_*` primitives (`pthread_mutex`, `pthread_cond`,
//! `pthread_create`, `CLOCK_MONOTONIC`).  It is NOT a thin wrapper over
//! the Linux backend — each module has its own native implementation.
//!
//! # Architecture (NASA OSAL pattern)
//!
//! Following NASA's OSAL architecture, **POSIX is the adaptation layer**
//! and **Linux is one BSP / reference implementation**:
//!
//! ```text
//!   Application code
//!        ↓
//!   pub mod os  (unified API)
//!        ↓
//!   posix/      (POSIX adaptation — native pthread primitives)
//!        ↓
//!   posix/sys/  (thin FFI wrappers — PosixMutex, PosixCondvar, clock)
//!        ↓
//!   libc::pthread_*, clock_gettime(CLOCK_MONOTONIC)
//! ```
//!
//! # Modules
//!
//! - [`sys`] — Low-level POSIX wrappers (PosixMutex, PosixCondvar, clock,
//!   thread create/join/TLS).
//! - [`config`] — Logical tick period (`TICK_PERIOD_MS = 1`).
//! - [`types`] — Backend type aliases (TickType, handle types, etc.).
//! - [`duration`] — `ToTick` / `FromTick` with nanosecond ceiling rounding.
//! - [`system`] — System operations (monotonic timing, nanosleep delays,
//!   recursive critical-section mutex, scheduler/ISR no-ops).
//! - [`thread`] — Thread lifecycle via `pthread_create`/`pthread_join`,
//!   pthread TLS for current-thread lookup, cooperative cancellation,
//!   task notifications via PosixMutex + PosixCondvar.
//! - [`mutex`] — `RawMutex` (PTHREAD_MUTEX_RECURSIVE) and `Mutex<T>`
//!   (PTHREAD_MUTEX_ERRORCHECK + UnsafeCell<Box<T>>).
//! - [`semaphore`] — Counting semaphore via PosixMutex + PosixCondvar + count.
//! - [`event_group`] — Multi-bit event flags via PosixMutex + PosixCondvar
//!   with CLOCK_MONOTONIC deadline wait (OR semantics).
//! - [`queue`] — FIFO queue via PosixMutex + dual PosixCondvar
//!   (not_empty / not_full) with CLOCK_MONOTONIC timeouts
//! - [`timer`] - Timer service worker via pthread + detached daemon, with
//!   deadline heap, generation-based lazy invalidation, and callbacks executed
//!   outside the timer-manager lock.
//! - [`bsp`] — Board Support Package selection (platform-specific config).
//!
//! # Relationship to Linux
//!
//! The POSIX backend targets Linux through its `generic_linux` BSP
//! (`posix/bsp/generic_linux.rs`), which provides platform constants
//! (`TICK_PERIOD_MS = 1`, `TickType = u32`) and type aliases.  All
//! OSAL primitives are implemented with native pthread / libc APIs
//! (`posix/sys/`).

pub(crate) mod allocator;
pub mod bsp;
pub mod config;
pub(crate) mod duration;
pub mod event_group;
pub mod mutex;
pub mod queue;
pub mod semaphore;
pub(crate) mod sys;
pub mod system;
pub mod thread;
pub mod timer;
pub mod types;
