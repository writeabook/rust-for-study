//! POSIX backend thread management — currently delegates to the Linux
//! reference implementation.  See `crate::linux::thread` for documentation.

pub use crate::linux::thread::*;
