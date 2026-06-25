//! POSIX backend configuration.
//!
//! Re-exports platform constants from the active BSP target.
//! Currently the only BSP is `generic_linux`.

pub use super::bsp::generic_linux::TICK_PERIOD_MS;
