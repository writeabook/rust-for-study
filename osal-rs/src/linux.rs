//! Linux OSAL backend module.
//!
//! This module provides the Linux (developer workstation) backend for the
//! OSAL-RS abstraction layer. It uses safe Rust standard library primitives
//! (`std::thread`, `std::sync`, `std::time`) instead of direct FFI, enabling
//! development, testing, and CI on a host machine without an RTOS.
//!
//! # Overview
//!
//! The Linux backend targets development and test workflows:
//!
//! - **Development**: Write and debug OSAL-based application logic on a
//!   Linux workstation before deploying to an embedded target.
//! - **Testing and CI**: Run the full OSAL test suite via `cargo test`
//!   without hardware or emulators.
//! - **Simulation**: Exercise multi-threading, synchronization, and timing
//!   contracts with standard OS primitives.
//!
//! # Design
//!
//! - No FFI — uses `std::thread::Builder`, `std::sync::Mutex`,
//!   `std::sync::Condvar`, `std::time::Instant`.
//! - Tick resolution: 1 ms (see [`config`] for `TICK_PERIOD_MS`).
//! - Does not simulate real-time scheduling guarantees; best-effort only.
//! - ISR APIs (`_from_isr` functions) are non-blocking fallbacks.
//!
//! # Modules
//!
//! - [`config`] — Backend-wide constants (tick period, feature flags).
//! - [`types`] — Type aliases matching the FreeRTOS type layer
//!   (`TickType`, handle types, etc.).
//! - [`duration`] — `ToTick` / `FromTick` impls for `core::time::Duration`.
//! - [`system`] — System-level operations (time, delays, scheduler stubs).
//! - [`thread`] — Thread state and metadata types.
//!
//! # Platform Support
//!
//! Any Linux kernel with `glibc` or `musl` and Rust `std` support.
//! Real-time behaviour requires `SCHED_FIFO` / `SCHED_RR` (future
//! extensions).

pub mod config;
pub mod types;
pub(crate) mod duration;
pub mod system;
pub mod mutex;
pub mod semaphore;
pub mod event_group;
pub mod thread;
pub mod queue;
pub mod timer;
