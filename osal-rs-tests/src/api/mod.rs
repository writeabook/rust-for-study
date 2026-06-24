//! OSAL public API contract tests.
//!
//! Tests in this module verify the OSAL public API behavior that every
//! supported backend must satisfy. They are backend-agnostic — they only
//! use `osal_rs::os::*` and never reference backend-internal types.
