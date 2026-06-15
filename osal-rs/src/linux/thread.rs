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

use core::any::Any;
use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::sync::Arc;

use std::sync::{Condvar, Mutex as StdMutex};
use std::thread::{Builder as ThreadBuilder, JoinHandle, current as current_thread};

use super::types::{BaseType, StackType, ThreadHandle, TickType, UBaseType};
use crate::traits::{ThreadFn, ThreadParam, ThreadNotification, ToPriority, ToTick};
use crate::utils::{Bytes, DoublePtr, Error, Result};

const MAX_TASK_NAME_LEN: usize = 16;
static NEXT_THREAD_ID: AtomicUsize = AtomicUsize::new(1);
static DUMMY_METADATA_ID: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// ThreadState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThreadState {
    #[default] Running = 0,
    Ready = 1,  Blocked = 2,  Suspended = 3,  Deleted = 4,  Invalid,
}

// ---------------------------------------------------------------------------
// ThreadMetadata
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ThreadMetadata {
    pub thread: ThreadHandle,
    pub name: Bytes<MAX_TASK_NAME_LEN>,
    pub stack_depth: StackType,
    pub priority: UBaseType,
    pub thread_number: UBaseType,
    pub state: ThreadState,
    pub current_priority: UBaseType,
    pub base_priority: UBaseType,
    pub run_time_counter: UBaseType,
    pub stack_high_water_mark: StackType,
}

unsafe impl Send for ThreadMetadata {}
unsafe impl Sync for ThreadMetadata {}

impl Default for ThreadMetadata {
    fn default() -> Self {
        ThreadMetadata {
            thread: core::ptr::null_mut(),
            name: Bytes::new(), stack_depth: 0, priority: 0,
            thread_number: 0, state: ThreadState::Invalid,
            current_priority: 0, base_priority: 0,
            run_time_counter: 0, stack_high_water_mark: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// ThreadInner
// ---------------------------------------------------------------------------

struct ThreadInner {
    id: usize,
    handle: Option<JoinHandle<()>>,
    notification_value: u32,
    notification_pending: bool,
    state: ThreadState,
    name: Bytes<MAX_TASK_NAME_LEN>,
    stack_depth: StackType,
    priority: UBaseType,
}

impl ThreadInner {
    fn new(name: &str, stack_depth: StackType, priority: UBaseType) -> Self {
        Self {
            id: NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed),
            handle: None, notification_value: 0, notification_pending: false,
            state: ThreadState::Suspended, name: Bytes::from_str(name),
            stack_depth, priority,
        }
    }
    fn to_metadata(&self) -> ThreadMetadata {
        ThreadMetadata {
            thread: self.id as ThreadHandle, name: self.name.clone(),
            stack_depth: self.stack_depth, priority: self.priority,
            thread_number: 0, state: self.state,
            current_priority: self.priority, base_priority: self.priority,
            run_time_counter: 0, stack_high_water_mark: self.stack_depth,
        }
    }
}

// ---------------------------------------------------------------------------
// Thread
// ---------------------------------------------------------------------------

pub struct Thread {
    inner: Arc<StdMutex<ThreadInner>>,
    condvar: Arc<Condvar>,
    handle: ThreadHandle,
}

unsafe impl Send for Thread {}
unsafe impl Sync for Thread {}

impl Deref for Thread {
    type Target = ThreadHandle;
    fn deref(&self) -> &Self::Target { &self.handle }
}

impl Clone for Thread {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner), condvar: Arc::clone(&self.condvar), handle: self.handle }
    }
}

impl Thread {

    pub fn new(name: &str, stack_depth: StackType, priority: UBaseType) -> Self {
        Self {
            inner: Arc::new(StdMutex::new(ThreadInner::new(name, stack_depth, priority))),
            condvar: Arc::new(Condvar::new()),
            handle: 1 as ThreadHandle,
        }
    }

    /// Creates a thread from an existing handle (API surface compat).
    pub fn new_with_handle(_handle: ThreadHandle, name: &str, stack_depth: StackType, priority: UBaseType) -> Result<Self> {
        Ok(Self { inner: Arc::new(StdMutex::new(ThreadInner::new(name, stack_depth, priority))),
            condvar: Arc::new(Condvar::new()), handle: _handle })
    }

    /// Creates a thread with a priority that implements `ToPriority`.
    pub fn new_with_to_priority(name: &str, stack_depth: StackType, priority: impl ToPriority) -> Self {
        Self::new(name, stack_depth, priority.to_priority())
    }

    /// Creates a thread from an existing handle with ToPriority.
    pub fn new_with_handle_and_to_priority(handle: ThreadHandle, name: &str, stack_depth: StackType, priority: impl ToPriority) -> Result<Self> {
        Self::new_with_handle(handle, name, stack_depth, priority.to_priority())
    }

    /// Gets metadata from a raw handle (API surface compat).
    pub fn get_metadata_from_handle(_handle: ThreadHandle) -> ThreadMetadata {
        let id = DUMMY_METADATA_ID.fetch_add(1, Ordering::Relaxed);
        ThreadMetadata {
            thread: id as ThreadHandle,
            name: Bytes::from_str("handle"), stack_depth: 0, priority: 1,
            thread_number: 0, state: ThreadState::Running,
            current_priority: 1, base_priority: 1,
            run_time_counter: 0, stack_high_water_mark: 0,
        }
    }

    /// Gets metadata from a Thread reference (static method).
    pub fn get_metadata(thread: &Thread) -> ThreadMetadata {
        thread.get_metadata()
    }

    /// Waits for notification with ToTick conversion.
    pub fn wait_notification_with_to_tick(&self, bits_to_clear_on_entry: u32, bits_to_clear_on_exit: u32, timeout: impl ToTick) -> Result<u32> {
        self.wait_notification(bits_to_clear_on_entry, bits_to_clear_on_exit, timeout.to_ticks())
    }

    // -- internal helpers --------------------------------------------------

    fn spawn_inner<F>(&mut self, param: Option<ThreadParam>, callback: F) -> Result<Self>
    where F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam> + Send + Sync + 'static,
    {
        let inner = Arc::clone(&self.inner);
        let inner_for_thread = Arc::clone(&self.inner);
        let condvar = Arc::clone(&self.condvar);
        let thread_name = { let g = inner.lock().unwrap(); g.name.to_string() };

        let handle = ThreadBuilder::new().name(thread_name.clone()).spawn(move || {
            { let mut g = inner_for_thread.lock().unwrap(); g.state = ThreadState::Running; }
            let boxed_self: Box<dyn ThreadFn> = Box::new(Thread {
                inner: Arc::clone(&inner_for_thread), condvar: Arc::clone(&condvar), handle: 1 as ThreadHandle,
            });
            let _ = callback(boxed_self, param);
            let mut g = inner_for_thread.lock().unwrap(); g.state = ThreadState::Deleted; drop(g);
            condvar.notify_all();
        }).map_err(|_| Error::OutOfMemory)?;

        {
            let mut g = inner.lock().unwrap(); g.handle = Some(handle); g.state = ThreadState::Ready;
        }
        Ok(Self { inner: Arc::clone(&self.inner), condvar: Arc::clone(&self.condvar), handle: self.handle })
    }

    fn spawn_simple_inner<F>(&mut self, callback: F) -> Result<Self>
    where F: Fn() + Send + Sync + 'static,
    {
        let inner = Arc::clone(&self.inner);
        let inner_for_thread = Arc::clone(&self.inner);
        let condvar = Arc::clone(&self.condvar);
        let thread_name = { let g = inner.lock().unwrap(); g.name.to_string() };

        let handle = ThreadBuilder::new().name(thread_name.clone()).spawn(move || {
            { let mut g = inner_for_thread.lock().unwrap(); g.state = ThreadState::Running; }
            callback();
            let mut g = inner_for_thread.lock().unwrap(); g.state = ThreadState::Deleted; drop(g);
            condvar.notify_all();
        }).map_err(|_| Error::OutOfMemory)?;

        {
            let mut g = inner.lock().unwrap(); g.handle = Some(handle); g.state = ThreadState::Ready;
        }
        Ok(Self { inner: Arc::clone(&self.inner), condvar: Arc::clone(&self.condvar), handle: self.handle })
    }
}

// ---------------------------------------------------------------------------
// ThreadFn implementation
// ---------------------------------------------------------------------------

impl ThreadFn for Thread {
    fn spawn<F>(&mut self, param: Option<ThreadParam>, callback: F) -> Result<Self>
    where F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam> + Send + Sync + 'static,
    { self.spawn_inner(param, callback) }

    fn spawn_simple<F>(&mut self, callback: F) -> Result<Self>
    where F: Fn() + Send + Sync + 'static,
    { self.spawn_simple_inner(callback) }

    fn delete(&self) {
        let mut g = self.inner.lock().unwrap(); g.state = ThreadState::Deleted; drop(g);
        self.condvar.notify_all();
    }

    fn suspend(&self) {}
    fn resume(&self) {}

    fn join(&self, _retval: DoublePtr) -> Result<i32> {
        let h = { let mut g = self.inner.lock().unwrap(); g.handle.take() };
        if let Some(jh) = h { jh.join().map_err(|_| Error::Unhandled("thread panicked"))?; }
        self.inner.lock().unwrap().state = ThreadState::Deleted; Ok(0)
    }

    fn get_metadata(&self) -> ThreadMetadata { self.inner.lock().unwrap().to_metadata() }

    fn get_current() -> Self {
        let name = current_thread().name().unwrap_or("unknown").to_string();
        let mut ti = ThreadInner::new(&name, 0, 0); ti.state = ThreadState::Running; ti.name = Bytes::from_str(&name);
        Thread { inner: Arc::new(StdMutex::new(ti)), condvar: Arc::new(Condvar::new()), handle: 1 as ThreadHandle }
    }

    fn notify(&self, notification: ThreadNotification) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let (action, value) = notification.into();
        match action { 0=>{}, 1=>g.notification_value|=value, 2=>g.notification_value=g.notification_value.wrapping_add(1),
            3=>g.notification_value=value, 4=>{if g.notification_pending{return Err(Error::QueueFull);} g.notification_value=value;}, _=>{} }
        g.notification_pending = true; drop(g); self.condvar.notify_all(); Ok(())
    }

    fn notify_from_isr(&self, notification: ThreadNotification, hpw: &mut BaseType) -> Result<()> {
        let mut g = match self.inner.try_lock() { Ok(g)=>g, Err(_)=>return Err(Error::QueueFull) };
        let (action, value) = notification.into();
        match action { 0=>{}, 1=>g.notification_value|=value, 2=>g.notification_value=g.notification_value.wrapping_add(1),
            3=>g.notification_value=value, 4=>{if g.notification_pending{return Err(Error::QueueFull);} g.notification_value=value;}, _=>{} }
        g.notification_pending = true; *hpw = 1; drop(g); self.condvar.notify_all(); Ok(())
    }

    fn wait_notification(&self, bits_clear_entry: u32, bits_clear_exit: u32, timeout_ticks: TickType) -> Result<u32> {
        let mut g = self.inner.lock().unwrap(); g.notification_value &= !bits_clear_entry;
        if g.notification_pending { let v = g.notification_value; g.notification_value &= !bits_clear_exit; g.notification_pending = false; return Ok(v); }
        if timeout_ticks == 0 { return Err(Error::Timeout); }
        let timeout = if timeout_ticks == TickType::MAX { Duration::from_secs(u64::MAX/1_000) } else { Duration::from_millis(timeout_ticks as u64) };
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let now = std::time::Instant::now();
            if now >= deadline { return Err(Error::Timeout); }
            let (new_g, wr) = self.condvar.wait_timeout(g, deadline - now).unwrap(); g = new_g;
            if wr.timed_out() && !g.notification_pending { return Err(Error::Timeout); }
            if g.notification_pending { let v = g.notification_value; g.notification_value &= !bits_clear_exit; g.notification_pending = false; return Ok(v); }
        }
    }
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Debug for Thread {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(g) => f.debug_struct("Thread").field("id",&g.id).field("name",&g.name)
                .field("priority",&g.priority).field("state",&g.state).finish(),
            Err(_) => f.debug_struct("Thread").finish_non_exhaustive(),
        }
    }
}

impl Display for Thread {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(g) => write!(f, "Thread {{ id: {}, name: {}, priority: {} }}", g.id, g.name, g.priority),
            Err(_) => write!(f, "Thread {{ <locked> }}"),
        }
    }
}