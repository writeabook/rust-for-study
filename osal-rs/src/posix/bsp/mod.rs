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
//!   osal_rs::os  (unified API)
//!        ↓
//!   posix/  (OSAL trait → POSIX API mapping)
//!   ├── sys/                   pthread / clock / condvar wrappers
//!   └── bsp/generic_linux      Linux host constants & type aliases
//!        ↓
//!   Linux kernel / glibc / musl
//! ```
//!
//! # Current BSP: generic-linux
//!
//! `generic_linux` is the default and currently the only BSP target.
//! It defines `TICK_PERIOD_MS = 1` (1 tick = 1 ms), `TickType = u32`,
//! and opaque handle aliases (`*const c_void`) for trait compatibility.
//!
//! # Adding a new BSP (future work)
//!
//! To add a new BSP target (e.g., macOS, FreeBSD):
//!
//! 1. Create `posix/bsp/generic_<target>.rs` with the same constants and types
//! 2. Add a feature flag (e.g., `bsp-macos`) in `Cargo.toml`
//! 3. Update `posix/config.rs` and `posix/types.rs` to conditionally
//!    re-export from the correct BSP
//!
//! # Relationship to NASA OSAL
//!
//! | NASA OSAL                     | osal-rs                               |
//! |-------------------------------|---------------------------------------|
//! | `src/os/shared/`              | `crate::traits`                       |
//! | `src/os/posix/`               | `crate::posix` (adaptation)           |
//! | `src/bsp/generic-linux/`      | `crate::posix::bsp::generic_linux`    |
//! | `src/os/rtems/`               | `crate::freertos` (embedded)          |

pub mod generic_linux;
