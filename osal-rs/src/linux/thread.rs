//! Thread types stub for Linux backend.
//!
//! Placeholder until the full implementation is developed.

/// Thread execution state enumeration.
///
/// Mirrors the states defined in the OSAL traits layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThreadState {
    #[default]
    Running = 0,
    Ready = 1,
    Blocked = 2,
    Suspended = 3,
    Deleted = 4,
    Invalid,
}

/// Metadata about a thread.
#[derive(Debug, Clone, Default)]
pub struct ThreadMetadata {
    pub name: &'static str,
    pub state: ThreadState,
    pub priority: u32,
    pub stack_size: u32,
}