//! Thread management and synchronization for the Linux backend.
//!
//! # Overview
//!
//! This module provides a safe Rust interface for creating and managing
//! threads on Linux using `std::thread`. It implements the [`ThreadFn`]
//! trait, supporting thread creation with callbacks, thread notifications,
//! and metadata introspection.
//!
//! # Design
//!
//! - Each `Thread` wraps an `Arc<StdMutex<ThreadInner>>`, shared between
//!   the creator and the spawned OS thread.
//! - A global `AtomicUsize` counter generates unique thread IDs, which
//!   are cast to `ThreadHandle` (`*const c_void`) for API compatibility.
//! - `suspend` / `resume` are no-ops in the current version (see
//!   `doc/backend-alignment-gaps.md`).
//! - ISR variants (`notify_from_isr`) use `StdMutex::try_lock`.
//!
//! # Limitations
//!
//! - Thread priorities are informational only; Linux does not use them
//!   for scheduling.
//! - Stack high-water mark is filled with the initial `stack_depth`.
//! - `suspend` / `resume` are no-ops.


use core::fmt::{Debug, Display, Formatter};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::sync::Arc;

use std::sync::{Condvar, Mutex as StdMutex};
use std::thread::{Builder as ThreadBuilder, JoinHandle, current as current_thread};

use super::types::{BaseType, StackType, ThreadHandle, TickType, UBaseType};
use crate::traits::ThreadFn;
use crate::traits::{ThreadParam, ThreadNotification};
use crate::utils::{Bytes, DoublePtr, Error, Result};

const MAX_TASK_NAME_LEN: usize = 16;

/// Monotonic thread-ID counter.
static NEXT_THREAD_ID: AtomicUsize = AtomicUsize::new(1);

// ---------------------------------------------------------------------------
// ThreadState
// ---------------------------------------------------------------------------

/// Thread execution state enumeration.
///
/// Mirrors the FreeRTOS `eTaskState` values to keep application code
/// portable across backends.
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

// ---------------------------------------------------------------------------
// ThreadMetadata — aligned with the FreeRTOS definition
// ---------------------------------------------------------------------------

/// Metadata describing a single OSAL thread.
///
/// Field names and types match the FreeRTOS backend so that test code
/// remains backend-agnostic.
#[derive(Clone, Debug)]
pub struct ThreadMetadata {
    /// Opaque handle cast from the thread's unique numeric ID.
    pub thread: ThreadHandle,
    /// Human-readable thread name (may be truncated).
    pub name: Bytes<MAX_TASK_NAME_LEN>,
    /// Requested stack depth (bytes on Linux).
    pub stack_depth: StackType,
    /// Scheduling priority (informational on Linux).
    pub priority: UBaseType,
    /// Unique thread number (0 on Linux).
    pub thread_number: UBaseType,
    /// Current execution state.
    pub state: ThreadState,
    /// Current priority (same as `priority` on Linux).
    pub current_priority: UBaseType,
    /// Base priority (same as `priority` on Linux).
    pub base_priority: UBaseType,
    /// Run-time counter (always 0 on Linux).
    pub run_time_counter: UBaseType,
    /// Minimum remaining stack — filled with `stack_depth` (not tracked).
    pub stack_high_water_mark: StackType,
}

unsafe impl Send for ThreadMetadata {}
unsafe impl Sync for ThreadMetadata {}

impl Default for ThreadMetadata {
    fn default() -> Self {
        ThreadMetadata {
            thread: null_mut(),
            name: Bytes::new(),
            stack_depth: 0,
            priority: 0,
            thread_number: 0,
            state: ThreadState::Invalid,
            current_priority: 0,
            base_priority: 0,
            run_time_counter: 0,
            stack_high_water_mark: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// ThreadInner — shared state between creator & OS thread
// ---------------------------------------------------------------------------

struct ThreadInner {
    /// Unique numeric ID (also serves as the `ThreadHandle`).
    id: usize,
    /// OS-level join handle (None before spawn / after join).
    handle: Option<JoinHandle<()>>,
    /// Task-notification value.
    notification_value: u32,
    /// Whether an unread notification is pending.
    notification_pending: bool,
    /// Current thread state.
    state: ThreadState,
    /// Thread name.
    name: Bytes<MAX_TASK_NAME_LEN>,
    /// Requested stack depth.
    stack_depth: StackType,
    /// Scheduling priority (informational).
    priority: UBaseType,
}

impl ThreadInner {
    fn new(name: &str, stack_depth: StackType, priority: UBaseType) -> Self {
        Self {
            id: NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed),
            handle: None,
            notification_value: 0,
            notification_pending: false,
            state: ThreadState::Suspended,
            name: Bytes::from_str(name),
            stack_depth,
            priority,
        }
    }

    fn to_metadata(&self) -> ThreadMetadata {
        ThreadMetadata {
            thread: self.id as ThreadHandle,
            name: self.name.clone(),
            stack_depth: self.stack_depth,
            priority: self.priority,
            thread_number: 0,
            state: self.state,
            current_priority: self.priority,
            base_priority: self.priority,
            run_time_counter: 0,
            stack_high_water_mark: self.stack_depth,
        }
    }
}

// ---------------------------------------------------------------------------
// Thread
// ---------------------------------------------------------------------------

/// A Linux OSAL thread wrapper.
///
/// Implements the [`ThreadFn`] trait using `std::thread` primitives.
#[derive(Clone)]
pub struct Thread {
    inner: Arc<StdMutex<ThreadInner>>,
    condvar: Arc<Condvar>,
}

unsafe impl Send for Thread {}
unsafe impl Sync for Thread {}

impl Thread {
    /// Creates a new uninitialized thread.
    ///
    /// The thread must be started with [`spawn`](Thread::spawn) or
    /// [`spawn_simple`](Thread::spawn_simple).
    pub fn new(name: &str, stack_depth: StackType, priority: UBaseType) -> Self {
        Self {
            inner: Arc::new(StdMutex::new(ThreadInner::new(
                name,
                stack_depth,
                priority,
            ))),
            condvar: Arc::new(Condvar::new()),
        }
    }

    // -- helper: spawn the OS thread for `spawn` ---------------------------

    fn spawn_inner<F>(&mut self, param: Option<ThreadParam>, callback: F) -> Result<Self>
    where
        F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam>,
        F: Send + Sync + 'static,
    {
        let inner = Arc::clone(&self.inner);
        let inner_for_thread = Arc::clone(&self.inner);
        let condvar = Arc::clone(&self.condvar);

        let thread_name = {
            let guard = inner.lock().unwrap();
            guard.name.to_string()
        };

        let handle = ThreadBuilder::new()
            .name(thread_name.clone())
            .spawn(move || {
                // Set state to Running.
                {
                    let mut guard = inner_for_thread.lock().unwrap();
                    guard.state = ThreadState::Running;
                }

                // Build a boxed trait object for the callback.
                let boxed_self: Box<dyn ThreadFn> = Box::new(Thread {
                    inner: Arc::clone(&inner_for_thread),
                    condvar: Arc::clone(&condvar),
                });

                let _ = callback(boxed_self, param);

                // Mark as deleted.
                let mut guard = inner_for_thread.lock().unwrap();
                guard.state = ThreadState::Deleted;
                drop(guard);
                condvar.notify_all();
            })
            .map_err(|_| Error::OutOfMemory)?;

        {
            let mut guard = inner.lock().unwrap();
            guard.handle = Some(handle);
            guard.state = ThreadState::Ready;
        }

        Ok(Self {
            inner: Arc::clone(&self.inner),
            condvar: Arc::clone(&self.condvar),
        })
    }

    // -- helper: spawn the OS thread for `spawn_simple` --------------------

    fn spawn_simple_inner<F>(&mut self, callback: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let inner = Arc::clone(&self.inner);
        let inner_for_thread = Arc::clone(&self.inner);
        let condvar = Arc::clone(&self.condvar);

        let thread_name = {
            let guard = inner.lock().unwrap();
            guard.name.to_string()
        };

        let handle = ThreadBuilder::new()
            .name(thread_name.clone())
            .spawn(move || {
                {
                    let mut guard = inner_for_thread.lock().unwrap();
                    guard.state = ThreadState::Running;
                }

                callback();

                let mut guard = inner_for_thread.lock().unwrap();
                guard.state = ThreadState::Deleted;
                drop(guard);
                condvar.notify_all();
            })
            .map_err(|_| Error::OutOfMemory)?;

        {
            let mut guard = inner.lock().unwrap();
            guard.handle = Some(handle);
            guard.state = ThreadState::Ready;
        }

        Ok(Self {
            inner: Arc::clone(&self.inner),
            condvar: Arc::clone(&self.condvar),
        })
    }
}

// ---------------------------------------------------------------------------
// ThreadFn implementation
// ---------------------------------------------------------------------------

impl ThreadFn for Thread {
    fn spawn<F>(&mut self, param: Option<ThreadParam>, callback: F) -> Result<Self>
    where
        F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam>,
        F: Send + Sync + 'static,
    {
        self.spawn_inner(param, callback)
    }

    fn spawn_simple<F>(&mut self, callback: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.spawn_simple_inner(callback)
    }

    fn delete(&self) {
        let mut guard = self.inner.lock().unwrap();
        guard.state = ThreadState::Deleted;
        drop(guard);
        self.condvar.notify_all();
    }

    fn suspend(&self) {
        // no-op: Linux user space cannot atomically suspend a thread.
        // See doc/backend-alignment-gaps.md §21.
    }

    fn resume(&self) {
        // no-op: Linux user space cannot atomically resume a thread.
        // See doc/backend-alignment-gaps.md §21.
    }

    fn join(&self, _retval: DoublePtr) -> Result<i32> {
        let mut guard = self.inner.lock().unwrap();

        // Take the join handle if present, leaving None behind.
        let handle = guard.handle.take();
        drop(guard); // release the lock before joining

        if let Some(h) = handle {
            h.join().map_err(|_| Error::Unhandled("thread panicked"))?;
        }

        self.inner.lock().unwrap().state = ThreadState::Deleted;
        Ok(0)
    }

    fn get_metadata(&self) -> ThreadMetadata {
        let guard = self.inner.lock().unwrap();
        guard.to_metadata()
    }

    fn get_current() -> Self {
        // Create a fresh Thread object representing the calling thread.
        let name = {
            let ct = current_thread();
            ct.name().unwrap_or("unknown").to_string()
        };

        let mut thread_inner = ThreadInner::new(&name, 0, 0);
        thread_inner.state = ThreadState::Running;
        thread_inner.name = Bytes::from_str(&name);

        Thread {
            inner: Arc::new(StdMutex::new(thread_inner)),
            condvar: Arc::new(Condvar::new()),
        }
    }

    fn notify(&self, notification: ThreadNotification) -> Result<()> {
        let mut guard = self.inner.lock().unwrap();

        let (action, value) = notification.into();

        match action {
            // NoAction
            0 => {}
            // SetBits
            1 => guard.notification_value |= value,
            // Increment
            2 => guard.notification_value = guard.notification_value.wrapping_add(1),
            // SetValueWithOverwrite
            3 => guard.notification_value = value,
            // SetValueWithoutOverwrite
            4 => {
                if guard.notification_pending {
                    return Err(Error::QueueFull);
                }
                guard.notification_value = value;
            }
            _ => {}
        }

        guard.notification_pending = true;
        drop(guard);
        self.condvar.notify_all();
        Ok(())
    }

    fn notify_from_isr(
        &self,
        notification: ThreadNotification,
        higher_priority_task_woken: &mut BaseType,
    ) -> Result<()> {
        // Try-lock variant — non-blocking.
        let mut guard = match self.inner.try_lock() {
            Ok(g) => g,
            Err(_) => return Err(Error::QueueFull),
        };

        let (action, value) = notification.into();

        match action {
            0 => {}
            1 => guard.notification_value |= value,
            2 => guard.notification_value = guard.notification_value.wrapping_add(1),
            3 => guard.notification_value = value,
            4 => {
                if guard.notification_pending {
                    return Err(Error::QueueFull);
                }
                guard.notification_value = value;
            }
            _ => {}
        }

        guard.notification_pending = true;
        *higher_priority_task_woken = 1;
        drop(guard);
        self.condvar.notify_all();
        Ok(())
    }

    fn wait_notification(
        &self,
        bits_to_clear_on_entry: u32,
        bits_to_clear_on_exit: u32,
        timeout_ticks: TickType,
    ) -> Result<u32> {
        let mut guard = self.inner.lock().unwrap();

        // Clear entry bits.
        guard.notification_value &= !bits_to_clear_on_entry;

        // Fast path: notification already pending.
        if guard.notification_pending {
            let val = guard.notification_value;
            guard.notification_value &= !bits_to_clear_on_exit;
            guard.notification_pending = false;
            return Ok(val);
        }

        // Zero timeout — return immediately (no notification).
        if timeout_ticks == 0 {
            return Err(Error::Timeout);
        }

        // Convert timeout.
        let timeout = if timeout_ticks == TickType::MAX {
            Duration::from_secs(u64::MAX / 1_000) // effectively infinite
        } else {
            Duration::from_millis(timeout_ticks as u64)
        };

        let deadline = std::time::Instant::now() + timeout;
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                return Err(Error::Timeout);
            }
            let remaining = deadline - now;

            let (new_guard, wait_result) = self.condvar.wait_timeout(guard, remaining).unwrap();
            guard = new_guard;

            if wait_result.timed_out() && !guard.notification_pending {
                return Err(Error::Timeout);
            }

            if guard.notification_pending {
                let val = guard.notification_value;
                guard.notification_value &= !bits_to_clear_on_exit;
                guard.notification_pending = false;
                return Ok(val);
            }

            // Spurious wakeup — loop.
        }
    }
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Debug for Thread {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(guard) => f
                .debug_struct("Thread")
                .field("id", &guard.id)
                .field("name", &guard.name)
                .field("stack_depth", &guard.stack_depth)
                .field("priority", &guard.priority)
                .field("state", &guard.state)
                .field("has_handle", &guard.handle.is_some())
                .finish(),
            Err(_) => f.debug_struct("Thread").finish_non_exhaustive(),
        }
    }
}

impl Display for Thread {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(guard) => write!(
                f,
                "Thread {{ id: {}, name: {}, priority: {}, stack_depth: {} }}",
                guard.id, guard.name, guard.priority, guard.stack_depth
            ),
            Err(_) => write!(f, "Thread {{ <locked> }}"),
        }
    }
}