//! Port bring-up and smoke tests.
//!
//! These tests verify that a specific backend can be built and minimally
//! exercised. They must **not** duplicate full API contract tests (those
//! live in `crate::api`).

#[cfg(feature = "posix")]
mod posix_smoke_tests;

#[cfg(feature = "freertos")]
mod freertos_smoke_tests;
