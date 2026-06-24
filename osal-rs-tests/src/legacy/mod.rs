//! Linux legacy backend regression tests.
//!
//! These tests exercise Linux-backend-specific behaviour (ISR host
//! simulation, std::sync::Mutex poison recovery, QueueStreamed,
//! cooperative cancellation, critical-section implementation details,
//! etc.).  They are **not** portable OSAL API contract tests and
//! should not be expanded.
//!
//! The Linux backend itself is transitional and may be removed once
//! the POSIX backend fully covers host functionality.

#[cfg(test)]
mod linux_legacy_extended_tests;
