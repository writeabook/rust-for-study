//! Low-level POSIX system wrappers.
//!
//! Each module wraps a `libc` type with safe(r) Rust APIs.
//! `unsafe` is concentrated here; higher-level `posix/*.rs` modules
//! should use these wrappers and avoid raw FFI.

pub mod clock;
pub mod condvar;
pub mod mutex;
