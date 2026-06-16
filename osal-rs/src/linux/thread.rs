/***************************************************************************
 *
 * osal-rs — Linux Thread backend
 *
 * Copyright (C) 2026 Antonio Salsi <passy.linux@zresa.it>
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <https://www.gnu.org/licenses/>.
 *
 ***************************************************************************/

//! Thread management and synchronization for the Linux backend.
//!
//! # Design
//!
//! - Each `Thread` wraps an `Arc<ThreadCore>`, which holds the shared state,
//!   a `Condvar`, and a weak registry entry.
//! - A global `Mutex<ThreadRegistry>` maps `ThreadHandle` → `Weak<ThreadCore>`
//!   and `std::thread::ThreadId` → `ThreadHandle`.
//! - `Thread::new()` allocates a unique `ThreadHandle` from a global counter
//!   and registers the new core.
//! - `get_metadata_from_handle()` queries the registry.
//! - `get_current()` queries `by_os_thread_id`; falls back to lazy
//!   registration for the main/current thread.
//! - Spawn checks for duplicate starts; the child thread is responsible for
//!   transitioning to `Running` and `Deleted`; the parent never overwrites.
//! - Callback panics are caught; `state` is still set to `Deleted`.
//! - `suspend` / `resume` are no-ops.
//! - `wait_notification(TickType::MAX)` uses `Condvar::wait()` for true
//!   infinite blocking.

use core::any::Any;
use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use alloc::boxed::Box;
use std::collections::HashMap;
use alloc::string::ToString;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use std::panic::catch_unwind;
use std::panic::AssertUnwindSafe;
use std::sync::{Condvar, Mutex as StdMutex, OnceLock};
use std::thread::{Builder as ThreadBuilder, JoinHandle, ThreadId};

use super::types::{BaseType, StackType, ThreadHandle, TickType, UBaseType};
use crate::traits::{ThreadFn, ThreadParam, ThreadNotification, ToPriority, ToTick};
use crate::utils::{Bytes, DoublePtr, Error, Result};

const MAX_TASK_NAME_LEN: usize = 16;
static NEXT_THREAD_ID: AtomicUsize = AtomicUsize::new(1);

fn next_thread_id() -> usize {
    NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed)
}

fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ---------------------------------------------------------------------------
// ThreadState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThreadState {
    #[default] Running = 0,
    Ready = 1,
    Blocked = 2,
    Suspended = 3,
    Deleted = 4,
    Invalid,
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
// ThreadRegistry
// ---------------------------------------------------------------------------

struct ThreadRegistry {
    by_handle: HashMap<usize, Weak<ThreadCore>>,
    by_os_tid: HashMap<ThreadId, usize>,
}

static REGISTRY: OnceLock<StdMutex<ThreadRegistry>> = OnceLock::new();

fn registry() -> &'static StdMutex<ThreadRegistry> {
    REGISTRY.get_or_init(|| {
        StdMutex::new(ThreadRegistry {
            by_handle: HashMap::new(),
            by_os_tid: HashMap::new(),
        })
    })
}

fn register_thread(id: usize, core: &Arc<ThreadCore>) {
    let mut r = recover_lock(registry().lock());
    r.by_handle.insert(id, Arc::downgrade(core));
}

fn register_os_tid(id: usize, os_id: ThreadId) {
    let mut r = recover_lock(registry().lock());
    r.by_os_tid.insert(os_id, id);
}

fn unregister_thread(id: usize) {
    let mut r = recover_lock(registry().lock());
    r.by_handle.remove(&id);
    // also remove from by_os_tid (find key by value)
    r.by_os_tid.retain(|_, v| *v != id);
}

fn lookup_by_handle(handle: ThreadHandle) -> Option<Arc<ThreadCore>> {
    let r = recover_lock(registry().lock());
    r.by_handle.get(&(handle as usize)).and_then(|w| w.upgrade())
}

fn lookup_current() -> Option<Arc<ThreadCore>> {
    let r = recover_lock(registry().lock());
    let os_id = std::thread::current().id();
    r.by_os_tid.get(&os_id)
        .and_then(|id| r.by_handle.get(id))
        .and_then(|w| w.upgrade())
}

static MAIN_THREAD_CORE: OnceLock<Arc<ThreadCore>> = OnceLock::new();

pub(crate) fn ensure_main_thread_registered() {
    let mut r = recover_lock(registry().lock());
    if r.by_handle.is_empty() {
        let os_id = std::thread::current().id();
        let id = next_thread_id();
        let core = MAIN_THREAD_CORE.get_or_init(|| {
            Arc::new(ThreadCore {
                id,
                inner: StdMutex::new(ThreadInner {
                    id,
                    name: Bytes::from_str("main"),
                    stack_depth: 0,
                    priority: 1,
                    state: ThreadState::Running,
                    join_handle: None,
                    spawn_started: false,
                    joined: false,
                    panic_payload: false,
                    notification_value: 0,
                    notification_pending: false,
                    waiting_notification: false,
                }),
                condvar: Condvar::new(),
            })
        });
        // Use the stored Arc so the Weak stays alive
        r.by_handle.insert(id, Arc::downgrade(core));
        r.by_os_tid.insert(os_id, id);
    }
}

pub(crate) fn count_registered_threads() -> usize {
    ensure_main_thread_registered();
    let mut r = recover_lock(registry().lock());
    r.by_handle.retain(|_, w| w.strong_count() > 0);
    r.by_handle.len()
}

pub(crate) fn snapshot_registered_threads() -> Vec<ThreadMetadata> {
    ensure_main_thread_registered();
    let r = recover_lock(registry().lock());
    let mut tasks = Vec::new();
    for weak in r.by_handle.values() {
        if let Some(core) = weak.upgrade() {
            let inner = recover_lock(core.inner.lock());
            tasks.push(ThreadMetadata {
                thread: core.id as ThreadHandle,
                name: inner.name.clone(),
                stack_depth: inner.stack_depth,
                priority: inner.priority,
                thread_number: 0,
                state: inner.state,
                current_priority: inner.priority,
                base_priority: inner.priority,
                run_time_counter: 0,
                stack_high_water_mark: inner.stack_depth,
            });
        }
    }
    tasks
}

pub(crate) fn current_thread_state() -> ThreadState {
    if let Some(core) = lookup_current() {
        recover_lock(core.inner.lock()).state
    } else {
        ThreadState::Running
    }
}

// ---------------------------------------------------------------------------
// ThreadInner
// ---------------------------------------------------------------------------

struct ThreadInner {
    id: usize,
    name: Bytes<MAX_TASK_NAME_LEN>,
    stack_depth: StackType,
    priority: UBaseType,

    state: ThreadState,
    join_handle: Option<JoinHandle<()>>,
    spawn_started: bool,
    joined: bool,
    panic_payload: bool,

    notification_value: u32,
    notification_pending: bool,
    waiting_notification: bool,
}

impl ThreadInner {
    fn new(id: usize, name: &str, stack_depth: StackType, priority: UBaseType) -> Self {
        Self {
            id,
            name: Bytes::from_str(name),
            stack_depth,
            priority,
            state: ThreadState::Suspended,
            join_handle: None,
            spawn_started: false,
            joined: false,
            panic_payload: false,
            notification_value: 0,
            notification_pending: false,
            waiting_notification: false,
        }
    }
}

// ---------------------------------------------------------------------------
// ThreadCore
// ---------------------------------------------------------------------------

struct ThreadCore {
    id: usize,
    inner: StdMutex<ThreadInner>,
    condvar: Condvar,
}

unsafe impl Send for ThreadCore {}
unsafe impl Sync for ThreadCore {}

// ---------------------------------------------------------------------------
// Thread
// ---------------------------------------------------------------------------

pub struct Thread {
    core: Arc<ThreadCore>,
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
        Self { core: Arc::clone(&self.core), handle: self.handle }
    }
}

impl Thread {
    // -- constructors -------------------------------------------------------

    pub fn new(name: &str, stack_depth: StackType, priority: UBaseType) -> Self {
        let id = next_thread_id();
        let handle = id as ThreadHandle;
        let core = Arc::new(ThreadCore {
            id,
            inner: StdMutex::new(ThreadInner::new(id, name, stack_depth, priority)),
            condvar: Condvar::new(),
        });
        register_thread(id, &core);
        Self { core, handle }
    }

    pub fn new_with_handle(_handle: ThreadHandle, name: &str, stack_depth: StackType, priority: UBaseType) -> Result<Self> {
        Ok(Self::new(name, stack_depth, priority))
    }

    pub fn new_with_to_priority(name: &str, stack_depth: StackType, priority: impl ToPriority) -> Self {
        Self::new(name, stack_depth, priority.to_priority())
    }

    pub fn new_with_handle_and_to_priority(handle: ThreadHandle, name: &str, stack_depth: StackType, priority: impl ToPriority) -> Result<Self> {
        Self::new_with_handle(handle, name, stack_depth, priority.to_priority())
    }

    // -- static helper ------------------------------------------------------

    pub fn get_metadata_from_handle(handle: ThreadHandle) -> ThreadMetadata {
        if let Some(core) = lookup_by_handle(handle) {
            let inner = recover_lock(core.inner.lock());
            ThreadMetadata {
                thread: core.id as ThreadHandle,
                name: inner.name.clone(),
                stack_depth: inner.stack_depth,
                priority: inner.priority,
                thread_number: 0,
                state: inner.state,
                current_priority: inner.priority,
                base_priority: inner.priority,
                run_time_counter: 0,
                stack_high_water_mark: inner.stack_depth,
            }
        } else {
            ThreadMetadata::default()
        }
    }

    pub fn get_metadata(thread: &Thread) -> ThreadMetadata {
        thread.get_metadata()
    }

    pub fn wait_notification_with_to_tick(&self, bits_clear_entry: u32, bits_clear_exit: u32, timeout: impl ToTick) -> Result<u32> {
        self.wait_notification(bits_clear_entry, bits_clear_exit, timeout.to_ticks())
    }

    // -- spawn helpers ------------------------------------------------------

    fn spawn_inner<F>(&mut self, param: Option<ThreadParam>, callback: F) -> Result<Self>
    where
        F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam> + Send + Sync + 'static,
    {
        let mut g = recover_lock(self.core.inner.lock());
        if g.spawn_started || g.join_handle.is_some() {
            return Err(Error::ThreadAlreadyStarted);
        }
        g.spawn_started = true;
        g.state = ThreadState::Ready;
        drop(g);

        let core = Arc::clone(&self.core);
        let id = self.core.id;

        let handle = ThreadBuilder::new()
            .name(format!("osal-{}", id))
            .spawn(move || {
                register_os_tid(id, std::thread::current().id());
                {
                    let mut g = recover_lock(core.inner.lock());
                    g.state = ThreadState::Running;
                }

                let boxed_self: Box<dyn ThreadFn> = Box::new(Thread {
                    core: Arc::clone(&core),
                    handle: id as ThreadHandle,
                });

                let result = catch_unwind(AssertUnwindSafe(|| callback(boxed_self, param)));

                let mut g = recover_lock(core.inner.lock());
                match result {
                    Ok(_) => {}
                    Err(_) => { g.panic_payload = true; }
                }
                g.state = ThreadState::Deleted;
                drop(g);
                core.condvar.notify_all();
            })
            .map_err(|_| Error::OutOfMemory)?;

        let mut g = recover_lock(self.core.inner.lock());
        g.join_handle = Some(handle);
        // Do NOT overwrite state — child thread owns Running/Deleted
        drop(g);

        Ok(Self { core: Arc::clone(&self.core), handle: self.handle })
    }

    fn spawn_simple_inner<F>(&mut self, callback: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let wrapper = move |_t: Box<dyn ThreadFn>, _p: Option<ThreadParam>| -> Result<ThreadParam> {
            callback();
            Ok(Arc::new(()))
        };
        self.spawn_inner(None, wrapper)
    }
}

// ---------------------------------------------------------------------------
// ThreadFn implementation
// ---------------------------------------------------------------------------

impl ThreadFn for Thread {
    fn spawn<F>(&mut self, param: Option<ThreadParam>, callback: F) -> Result<Self>
    where
        F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam> + Send + Sync + 'static,
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
        let mut g = recover_lock(self.core.inner.lock());
        g.state = ThreadState::Deleted;
        unregister_thread(self.core.id);
        drop(g);
        self.core.condvar.notify_all();
    }

    fn suspend(&self) {}
    fn resume(&self) {}

    fn join(&self, _retval: DoublePtr) -> Result<i32> {
        let jh = {
            let mut g = recover_lock(self.core.inner.lock());
            if g.joined { return Err(Error::ThreadAlreadyJoined); }
            if !g.spawn_started { return Err(Error::ThreadNotStarted); }
            g.joined = true;
            g.join_handle.take()
        };

        if let Some(jh) = jh {
            let _ = jh.join(); // wait for the OS thread (panic already caught)
            let mut g = recover_lock(self.core.inner.lock());
            g.state = ThreadState::Deleted;
            if g.panic_payload { Err(Error::ThreadJoinFailed) } else { Ok(0) }
        } else {
            Err(Error::ThreadNotStarted)
        }
    }

    fn get_metadata(&self) -> ThreadMetadata {
        let inner = recover_lock(self.core.inner.lock());
        ThreadMetadata {
            thread: self.core.id as ThreadHandle,
            name: inner.name.clone(),
            stack_depth: inner.stack_depth,
            priority: inner.priority,
            thread_number: 0,
            state: inner.state,
            current_priority: inner.priority,
            base_priority: inner.priority,
            run_time_counter: 0,
            stack_high_water_mark: inner.stack_depth,
        }
    }

    fn get_current() -> Self {
        if let Some(core) = lookup_current() {
            return Self {
                core: Arc::clone(&core),
                handle: core.id as ThreadHandle,
            };
        }

        // Lazy registration for main / non-OSAL threads
        let id = next_thread_id();
        let handle = id as ThreadHandle;
        let core = Arc::new(ThreadCore {
            id,
            inner: StdMutex::new(ThreadInner {
                id,
                name: Bytes::from_str("main"),
                stack_depth: 0,
                priority: 1,
                state: ThreadState::Running,
                join_handle: None,
                spawn_started: false,
                joined: false,
                panic_payload: false,
                notification_value: 0,
                notification_pending: false,
                waiting_notification: false,
            }),
            condvar: Condvar::new(),
        });
        register_thread(id, &core);
        register_os_tid(id, std::thread::current().id());
        Self { core, handle }
    }

    fn notify(&self, notification: ThreadNotification) -> Result<()> {
        let mut g = recover_lock(self.core.inner.lock());
        let (action, value) = notification.into();
        match action {
            0 => {},
            1 => g.notification_value |= value,
            2 => g.notification_value = g.notification_value.wrapping_add(1),
            3 => g.notification_value = value,
            4 => {
                if g.notification_pending { return Err(Error::QueueFull); }
                g.notification_value = value;
            }
            _ => {}
        }
        g.notification_pending = true;
        drop(g);
        self.core.condvar.notify_all();
        Ok(())
    }

    fn notify_from_isr(&self, notification: ThreadNotification, hpw: &mut BaseType) -> Result<()> {
        let mut g = match self.core.inner.try_lock() {
            Ok(g) => g,
            Err(_) => { *hpw = 0; return Err(Error::QueueFull); }
        };
        let (action, value) = notification.into();
        match action {
            0 => {},
            1 => g.notification_value |= value,
            2 => g.notification_value = g.notification_value.wrapping_add(1),
            3 => g.notification_value = value,
            4 => {
                if g.notification_pending { return Err(Error::QueueFull); }
                g.notification_value = value;
            }
            _ => {}
        }
        let was_waiting = g.waiting_notification || g.state == ThreadState::Blocked;
        g.notification_pending = true;
        *hpw = if was_waiting { 1 } else { 0 };
        drop(g);
        self.core.condvar.notify_all();
        Ok(())
    }

    fn wait_notification(&self, bits_clear_entry: u32, bits_clear_exit: u32, timeout_ticks: TickType) -> Result<u32> {
        let mut g = recover_lock(self.core.inner.lock());
        g.notification_value &= !bits_clear_entry;

        // Fast path: already pending
        if g.notification_pending {
            let v = g.notification_value;
            g.notification_value &= !bits_clear_exit;
            g.notification_pending = false;
            return Ok(v);
        }

        if timeout_ticks == 0 {
            return Err(Error::Timeout);
        }

        // Infinite wait
        if timeout_ticks == TickType::MAX {
            g.waiting_notification = true;
            g.state = ThreadState::Blocked;
            loop {
                g = recover_lock(self.core.condvar.wait(g));
                if g.state == ThreadState::Deleted {
                    return Err(Error::Timeout);
                }
                if g.notification_pending {
                    let v = g.notification_value;
                    g.notification_value &= !bits_clear_exit;
                    g.notification_pending = false;
                    g.waiting_notification = false;
                    g.state = ThreadState::Running;
                    return Ok(v);
                }
            }
        }

        // Finite timeout
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_millis(timeout_ticks as u64))
            .ok_or(Error::Timeout)?;

        g.waiting_notification = true;
        g.state = ThreadState::Blocked;

        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                g.waiting_notification = false;
                g.state = ThreadState::Running;
                return Err(Error::Timeout);
            }

            let remaining = deadline - now;
            let (new_g, wait_result) = recover_lock(self.core.condvar.wait_timeout(g, remaining));
            g = new_g;

            if g.state == ThreadState::Deleted {
                return Err(Error::Timeout);
            }

            if g.notification_pending {
                let v = g.notification_value;
                g.notification_value &= !bits_clear_exit;
                g.notification_pending = false;
                g.waiting_notification = false;
                g.state = ThreadState::Running;
                return Ok(v);
            }

            if wait_result.timed_out() {
                g.waiting_notification = false;
                g.state = ThreadState::Running;
                return Err(Error::Timeout);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Debug for Thread {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.core.inner.try_lock() {
            Ok(g) => f.debug_struct("Thread")
                .field("id", &self.core.id)
                .field("name", &g.name)
                .field("priority", &g.priority)
                .field("state", &g.state)
                .finish(),
            Err(_) => f.debug_struct("Thread").finish_non_exhaustive(),
        }
    }
}

impl Display for Thread {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.core.inner.try_lock() {
            Ok(g) => write!(f, "Thread {{ id: {}, name: {}, priority: {} }}", self.core.id, g.name, g.priority),
            Err(_) => write!(f, "Thread {{ <locked> }}"),
        }
    }
}