//! Boundary, error-path, and defensive tests.
//!
//! Tests in this module complement the API contract tests in `crate::api`
//! by covering parameter validation, edge cases, resource lifecycle, and
//! timeout behaviour.

pub mod event_group_edge_tests;
pub mod queue_edge_tests;
pub mod resource_lifecycle_tests;
pub mod semaphore_edge_tests;
pub mod timeout_edge_tests;
