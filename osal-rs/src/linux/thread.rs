//! Thread state and metadata types for the Linux backend.
//!
//! # Overview
//!
//! Defines the `ThreadState` enumeration and `ThreadMetadata` struct
//! used by [`SystemState`](super::system::SystemState) and by the
//! future thread-management module.
//!
//! # Thread States
//!
//! The state machine mirrors FreeRTOS task states:
//!
//! ```text
//! Created → Ready → Running → Suspended
//!                   ↕          ↕
//!                 Blocked → Deleted → Invalid
//! ```
//!
//! # Stub Limitations
//!
//! In the current stub phase these types are purely informational.
//! A real thread scheduler is not yet implemented. The default state
//! for new threads is [`ThreadState::Running`].

/// Thread execution state enumeration.
///
/// Mirrors the FreeRTOS `eTaskState` values to keep application code
/// portable across backends.
///
/// # Variants
///
/// | Variant     | FreeRTOS equivalent | Meaning                              |
/// |-------------|---------------------|--------------------------------------|
/// | `Running`   | `eRunning`          | Currently executing on the CPU       |
/// | `Ready`     | `eReady`            | Ready to run but not scheduled yet   |
/// | `Blocked`   | `eBlocked`          | Waiting for an event or timeout      |
/// | `Suspended` | `eSuspended`        | Explicitly suspended by API call     |
/// | `Deleted`   | `eDeleted`          | Task has been deleted                |
/// | `Invalid`   | `eInvalid`          | Handle does not refer to a valid task|
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::ThreadState;
///
/// let state = ThreadState::Running;
/// assert_eq!(state, ThreadState::Running);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThreadState {
    /// Task is currently executing.
    #[default]
    Running = 0,

    /// Task is ready to execute but waiting for the scheduler.
    Ready = 1,

    /// Task is blocked waiting for an event or resource.
    Blocked = 2,

    /// Task has been suspended via `suspend()`.
    Suspended = 3,

    /// Task has been deleted and its resources freed.
    Deleted = 4,

    /// The handle is invalid or the task no longer exists.
    Invalid,
}

/// Metadata describing a single OSAL thread.
///
/// Used by [`SystemState`](super::system::SystemState) for thread
/// enumeration and introspection.
///
/// # Fields
///
/// * `name` — Human-readable thread name (may be truncated).
/// * `state` — Current execution state of the thread.
/// * `priority` — Scheduling priority (informational on Linux).
/// * `stack_size` — Requested stack size in bytes.
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::{ThreadMetadata, ThreadState};
///
/// let meta = ThreadMetadata {
///     name: "idle",
///     state: ThreadState::Ready,
///     priority: 0,
///     stack_size: 512,
/// };
/// assert_eq!(meta.name, "idle");
/// ```
#[derive(Debug, Clone, Default)]
pub struct ThreadMetadata {
    /// Human-readable thread name.
    pub name: &'static str,

    /// Current execution state.
    pub state: ThreadState,

    /// Scheduling priority (higher = more CPU time).
    pub priority: u32,

    /// Stack size requested at creation time (bytes).
    pub stack_size: u32,
}