//! POSIX backend event group — currently delegates to the Linux reference
//! implementation.  See `crate::linux::event_group` for documentation.

pub use crate::linux::event_group::*;
