//! POSIX backend thread management.
//!
//! Thread lifecycle is built on `pthread_create` / `pthread_join` through the
//! POSIX sys thread layer.  Thread state and notifications use `PosixMutex` +
//! `PosixCondvar` (with `CLOCK_MONOTONIC` deadlines).  Current-thread lookup
//! uses `pthread_key_t` TLS.
//!
//! # Cooperative cancellation
//!
//! `delete()` does **not** call `pthread_cancel`.  It sets a flag and wakes
//! blocked waiters.  Long-running callbacks must poll `is_delete_requested()`
//! / `is_cancellation_requested()` and return naturally.  Call `join()` after
//! the callback exits to reclaim the pthread.
//!
//! # Limitations (host-simulation)
//!
//! - `suspend` / `resume` are no-ops (POSIX has no portable thread suspend).
//! - Priority is stored as metadata but not mapped to `pthread_setschedparam`.
//! - `_from_isr` methods use non-blocking `try_lock` — host simulation only.

use core::any::Any;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use libc::PTHREAD_MUTEX_ERRORCHECK;

use super::config::TICK_PERIOD_MS;
use super::sys::clock;
use super::sys::condvar::PosixCondvar;
use super::sys::mutex::PosixMutex;
use super::sys::thread as sys_thread;
use super::types::{BaseType, StackType, ThreadHandle, TickType, UBaseType};

use crate::traits::{ThreadFn, ThreadNotification, ThreadParam, ToPriority, ToTick};
use crate::utils::{Bytes, DoublePtr, Error, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_TASK_NAME_LEN: usize = 16;

static NEXT_THREAD_ID: AtomicUsize = AtomicUsize::new(1);

fn next_thread_id() -> usize {
    NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// ThreadState
// ---------------------------------------------------------------------------

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
// Registry
// ---------------------------------------------------------------------------

struct ThreadRegistry {
    by_handle: BTreeMap<usize, Weak<ThreadCore>>,
}

struct RegistryCell {
    mutex: PosixMutex,
    inner: UnsafeCell<ThreadRegistry>,
}

unsafe impl Send for RegistryCell {}
unsafe impl Sync for RegistryCell {}

/// pthread_once-based global init for the thread registry.
///
/// Initialisation failure is fatal (`expect` aborts).  The cell is leaked
/// (process-lifetime), matching OSAL global-singleton semantics.
///
/// # Safety
///
/// `pthread_once` guarantees exactly-once semantics.  The raw pointer deref
/// is safe because the pointer is written exactly once before any read.
static mut REGISTRY_ONCE: libc::pthread_once_t = libc::PTHREAD_ONCE_INIT;
static mut REGISTRY_PTR: *const RegistryCell = core::ptr::null();

extern "C" fn init_registry() {
    let cell = Box::new(RegistryCell {
        mutex: PosixMutex::new(PTHREAD_MUTEX_ERRORCHECK).expect("POSIX thread registry mutex"),
        inner: UnsafeCell::new(ThreadRegistry {
            by_handle: BTreeMap::new(),
        }),
    });
    unsafe {
        REGISTRY_PTR = Box::into_raw(cell);
    }
}

fn registry() -> &'static RegistryCell {
    unsafe {
        libc::pthread_once(&raw mut REGISTRY_ONCE, init_registry);
        &*REGISTRY_PTR
    }
}

// RAII registry lock guard.
struct RegistryGuard<'a> {
    cell: &'a RegistryCell,
}

impl<'a> RegistryGuard<'a> {
    fn lock(cell: &'a RegistryCell) -> Self {
        assert!(cell.mutex.lock(), "failed to lock POSIX thread registry");
        Self { cell }
    }
}

impl Deref for RegistryGuard<'_> {
    type Target = ThreadRegistry;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.cell.inner.get() }
    }
}

impl Drop for RegistryGuard<'_> {
    fn drop(&mut self) {
        assert!(
            self.cell.mutex.unlock(),
            "failed to unlock POSIX thread registry"
        );
    }
}

fn registry_lock() -> impl Deref<Target = ThreadRegistry> {
    RegistryGuard::lock(registry())
}

unsafe fn registry_inner_mut() -> &'static mut ThreadRegistry {
    // Caller must hold the registry mutex.
    &mut *registry().inner.get()
}

fn register_thread(id: usize, core: &Arc<ThreadCore>) {
    let _guard = registry_lock();
    unsafe {
        registry_inner_mut()
            .by_handle
            .insert(id, Arc::downgrade(core));
    }
}

fn unregister_thread(id: usize) {
    let _guard = registry_lock();
    unsafe {
        registry_inner_mut().by_handle.remove(&id);
    }
}

fn lookup_by_handle(handle: ThreadHandle) -> Option<Arc<ThreadCore>> {
    let _guard = registry_lock();
    unsafe {
        registry_inner_mut()
            .by_handle
            .get(&(handle as usize))
            .and_then(|w| w.upgrade())
    }
}

pub(crate) fn count_registered_threads() -> usize {
    ensure_main_thread_registered();
    let _guard = registry_lock();
    unsafe {
        let inner = registry_inner_mut();
        inner.by_handle.retain(|_, w| w.strong_count() > 0);
        inner.by_handle.len()
    }
}

pub(crate) fn snapshot_registered_threads() -> Vec<ThreadMetadata> {
    ensure_main_thread_registered();
    let _guard = registry_lock();
    let mut tasks = Vec::new();
    unsafe {
        for weak in registry_inner_mut().by_handle.values() {
            if let Some(core) = weak.upgrade() {
                let _g = core_lock(&core);
                let inner = core_inner_mut(&core);
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
    }
    tasks
}

pub(crate) fn current_thread_state() -> ThreadState {
    if let Some(core) = lookup_current() {
        let _g = core_lock(&core);
        unsafe { core_inner_mut(&core).state }
    } else {
        ThreadState::Running
    }
}

// ---------------------------------------------------------------------------
// pthread TLS for current-thread lookup
// ---------------------------------------------------------------------------

/// pthread_once-based global init for the current-thread TLS key.
///
/// # Safety
///
/// `pthread_once` guarantees exactly-once semantics.
static mut CURRENT_THREAD_KEY_ONCE: libc::pthread_once_t = libc::PTHREAD_ONCE_INIT;
static mut CURRENT_THREAD_KEY: libc::pthread_key_t = 0;

extern "C" fn init_current_thread_key() {
    unsafe {
        CURRENT_THREAD_KEY = sys_thread::key_create(None).expect("POSIX thread TLS key create");
    }
}

fn current_thread_key() -> libc::pthread_key_t {
    unsafe {
        libc::pthread_once(&raw mut CURRENT_THREAD_KEY_ONCE, init_current_thread_key);
        CURRENT_THREAD_KEY
    }
}

fn set_current_thread_id(id: usize) {
    let key = current_thread_key();
    let ptr = id as *mut c_void;
    unsafe {
        sys_thread::key_set(key, ptr);
    }
}

fn get_current_thread_id() -> Option<usize> {
    let key = current_thread_key();
    let ptr = unsafe { sys_thread::key_get(key) };
    if ptr.is_null() {
        None
    } else {
        Some(ptr as usize)
    }
}

fn lookup_current() -> Option<Arc<ThreadCore>> {
    let id = get_current_thread_id()?;
    lookup_by_handle(id as ThreadHandle)
}

/// pthread_once-based global init for the synthetic main-thread core.
///
/// Stores `*const Arc<ThreadCore>` (NOT `*const ThreadCore`) to preserve
/// `Arc::clone()` reference-counting semantics.  The Arc is leaked for
/// process lifetime — the main thread is never destroyed.
///
/// # Safety
///
/// `pthread_once` guarantees exactly-once semantics.  The raw pointer deref
/// is safe because the pointer is written exactly once before any read.
static mut MAIN_THREAD_CORE_ONCE: libc::pthread_once_t = libc::PTHREAD_ONCE_INIT;
static mut MAIN_THREAD_CORE_PTR: *const Arc<ThreadCore> = core::ptr::null();

extern "C" fn init_main_thread_core() {
    // The id is generated by the caller (ensure_main_thread_registered) before
    // calling pthread_once — we capture it indirectly through the outer
    // function's closure-like pattern.  Since pthread_once callbacks cannot
    // capture state, we set the id via a secondary static before calling
    // pthread_once, then read it inside the callback.
    let id = unsafe { PENDING_MAIN_THREAD_ID };
    let core = Arc::new(ThreadCore::new(id, "main", 0, 1));
    {
        let _g = core_lock(&core);
        let inner = unsafe { core_inner_mut(&core) };
        inner.state = ThreadState::Running;
    }
    let boxed = Box::new(core);
    unsafe {
        MAIN_THREAD_CORE_PTR = Box::into_raw(boxed);
    }
}

fn main_thread_core() -> Arc<ThreadCore> {
    unsafe {
        libc::pthread_once(&raw mut MAIN_THREAD_CORE_ONCE, init_main_thread_core);
        (*MAIN_THREAD_CORE_PTR).clone()
    }
}

/// Temporary storage for the main-thread id, set just before the first
/// `pthread_once` call to `init_main_thread_core`.
static mut PENDING_MAIN_THREAD_ID: usize = 0;

pub(crate) fn ensure_main_thread_registered() {
    if get_current_thread_id().is_some() {
        return;
    }
    let id = next_thread_id();
    set_current_thread_id(id);

    // Feed the id into the pthread_once callback via a temporary static.
    // This is safe because this is the only call site and the once guard
    // serialises all concurrent callers.
    unsafe {
        PENDING_MAIN_THREAD_ID = id;
    }
    let core = main_thread_core(); // Returns Arc<ThreadCore>

    register_thread(id, &core);
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
    pthread: Option<sys_thread::PosixThread>,

    spawn_started: bool,
    joined: bool,
    panic_payload: bool,
    callback_result: Option<Result<ThreadParam>>,

    /// Cooperative cancellation flag.
    delete_requested: bool,

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
            pthread: None,
            spawn_started: false,
            joined: false,
            panic_payload: false,
            callback_result: None,
            delete_requested: false,
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
    inner: PosixMutex,
    condvar: PosixCondvar,
    state: UnsafeCell<ThreadInner>,
}

unsafe impl Send for ThreadCore {}
unsafe impl Sync for ThreadCore {}

impl ThreadCore {
    fn new(id: usize, name: &str, stack_depth: StackType, priority: UBaseType) -> Self {
        Self {
            id,
            inner: PosixMutex::new(PTHREAD_MUTEX_ERRORCHECK).expect("POSIX thread core mutex"),
            condvar: PosixCondvar::new().expect("POSIX thread core condvar"),
            state: UnsafeCell::new(ThreadInner::new(id, name, stack_depth, priority)),
        }
    }
}

// RAII core lock guard.
struct CoreGuard<'a> {
    core: &'a ThreadCore,
}

impl<'a> CoreGuard<'a> {
    fn lock(core: &'a ThreadCore) -> Self {
        assert!(core.inner.lock(), "failed to lock POSIX thread core mutex");
        Self { core }
    }
}

impl Drop for CoreGuard<'_> {
    fn drop(&mut self) {
        assert!(
            self.core.inner.unlock(),
            "failed to unlock POSIX thread core mutex"
        );
    }
}

fn core_lock(core: &ThreadCore) -> CoreGuard<'_> {
    CoreGuard::lock(core)
}

/// Access ThreadInner — caller must hold the core mutex.
#[inline]
unsafe fn core_inner_mut(core: &ThreadCore) -> &mut ThreadInner {
    &mut *core.state.get()
}

// ---------------------------------------------------------------------------
// StartContext + trampoline
// ---------------------------------------------------------------------------

struct StartContext<F>
where
    F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam> + Send + Sync + 'static,
{
    id: usize,
    core: Arc<ThreadCore>,
    param: Option<ThreadParam>,
    callback: F,
}

extern "C" fn thread_trampoline<F>(arg: *mut c_void) -> *mut c_void
where
    F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam> + Send + Sync + 'static,
{
    let StartContext {
        id,
        core,
        param,
        callback,
    } = unsafe { *Box::from_raw(arg as *mut StartContext<F>) };

    set_current_thread_id(id);

    // Mark running.
    {
        let _g = core_lock(&core);
        let inner = unsafe { core_inner_mut(&core) };
        inner.state = ThreadState::Running;
    }

    let boxed_self: Box<dyn ThreadFn> = Box::new(Thread {
        core: Arc::clone(&core),
        handle: id as ThreadHandle,
    });

    // POSIX no_std backend does NOT catch panics across pthread boundaries.
    // Thread callbacks must not panic; use panic=abort or handle errors
    // inside callbacks.
    let result = callback(boxed_self, param);

    // Mark finished.
    {
        let _g = core_lock(&core);
        let inner = unsafe { core_inner_mut(&core) };

        match result {
            Ok(p) => inner.callback_result = Some(Ok(p)),
            Err(e) => inner.callback_result = Some(Err(e)),
        }

        inner.state = ThreadState::Deleted;
    }

    core.condvar.broadcast();
    core::ptr::null_mut()
}

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
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Clone for Thread {
    fn clone(&self) -> Self {
        Self {
            core: Arc::clone(&self.core),
            handle: self.handle,
        }
    }
}

impl Thread {
    // -- constructors ---------------------------------------------------

    pub fn new(name: &str, stack_depth: StackType, priority: UBaseType) -> Self {
        let id = next_thread_id();
        let handle = id as ThreadHandle;
        let core = Arc::new(ThreadCore::new(id, name, stack_depth, priority));
        register_thread(id, &core);
        Self { core, handle }
    }

    pub fn new_with_handle(
        _handle: ThreadHandle,
        name: &str,
        stack_depth: StackType,
        priority: UBaseType,
    ) -> Result<Self> {
        Ok(Self::new(name, stack_depth, priority))
    }

    pub fn new_with_to_priority(
        name: &str,
        stack_depth: StackType,
        priority: impl ToPriority,
    ) -> Self {
        Self::new(name, stack_depth, priority.to_priority())
    }

    pub fn new_with_handle_and_to_priority(
        handle: ThreadHandle,
        name: &str,
        stack_depth: StackType,
        priority: impl ToPriority,
    ) -> Result<Self> {
        Self::new_with_handle(handle, name, stack_depth, priority.to_priority())
    }

    // -- static helper --------------------------------------------------

    pub fn get_metadata_from_handle(handle: ThreadHandle) -> ThreadMetadata {
        if let Some(core) = lookup_by_handle(handle) {
            let _g = core_lock(&core);
            let inner = unsafe { core_inner_mut(&core) };
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

    pub fn wait_notification_with_to_tick(
        &self,
        bits_clear_entry: u32,
        bits_clear_exit: u32,
        timeout: impl ToTick,
    ) -> Result<u32> {
        self.wait_notification(bits_clear_entry, bits_clear_exit, timeout.to_ticks())
    }

    // -- cancellation query helpers -------------------------------------

    pub fn is_delete_requested(&self) -> bool {
        let _g = core_lock(&self.core);
        unsafe { core_inner_mut(&self.core).delete_requested }
    }

    pub fn is_cancellation_requested(&self) -> bool {
        self.is_delete_requested()
    }

    pub fn current_cancellation_requested() -> bool {
        Self::get_current().is_delete_requested()
    }

    // -- spawn helper ---------------------------------------------------

    fn spawn_inner<F>(&mut self, param: Option<ThreadParam>, callback: F) -> Result<Self>
    where
        F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam>
            + Send
            + Sync
            + 'static,
    {
        let stack_size = {
            let _g = core_lock(&self.core);
            let inner = unsafe { core_inner_mut(&self.core) };
            if inner.spawn_started || inner.pthread.is_some() {
                return Err(Error::ThreadAlreadyStarted);
            }
            inner.spawn_started = true;
            inner.state = ThreadState::Ready;
            inner.stack_depth as usize
        };

        let ctx = Box::new(StartContext {
            id: self.core.id,
            core: Arc::clone(&self.core),
            param,
            callback,
        });

        let arg = Box::into_raw(ctx) as *mut c_void;

        let pt = unsafe { sys_thread::create(Some(stack_size), thread_trampoline::<F>, arg) };

        match pt {
            Some(pt) => {
                {
                    let _g = core_lock(&self.core);
                    let inner = unsafe { core_inner_mut(&self.core) };
                    inner.pthread = Some(pt);
                    // Do NOT overwrite state — child owns Running/Deleted.
                }
                Ok(Self {
                    core: Arc::clone(&self.core),
                    handle: self.handle,
                })
            }
            None => {
                // Rollback.
                let _ = unsafe { Box::from_raw(arg as *mut StartContext<F>) };
                {
                    let _g = core_lock(&self.core);
                    let inner = unsafe { core_inner_mut(&self.core) };
                    inner.spawn_started = false;
                    inner.state = ThreadState::Deleted;
                }
                Err(Error::OutOfMemory)
            }
        }
    }

    fn spawn_simple_inner<F>(&mut self, callback: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let wrapper =
            move |_t: Box<dyn ThreadFn>, _p: Option<ThreadParam>| -> Result<ThreadParam> {
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
        F: Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam>
            + Send
            + Sync
            + 'static,
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
        let should_unregister = {
            let _g = core_lock(&self.core);
            let inner = unsafe { core_inner_mut(&self.core) };

            inner.delete_requested = true;

            if !inner.spawn_started || matches!(inner.state, ThreadState::Deleted) {
                inner.state = ThreadState::Deleted;
                true
            } else {
                false
            }
        };

        self.core.condvar.broadcast();

        if should_unregister {
            unregister_thread(self.core.id);
        }
    }

    fn suspend(&self) {}
    fn resume(&self) {}

    fn join(&self, _retval: DoublePtr) -> Result<i32> {
        let pt = {
            let _g = core_lock(&self.core);
            let inner = unsafe { core_inner_mut(&self.core) };
            if inner.joined {
                return Err(Error::ThreadAlreadyJoined);
            }
            if !inner.spawn_started {
                return Err(Error::ThreadNotStarted);
            }
            inner.joined = true;
            inner.pthread.take()
        };

        if let Some(pt) = pt {
            if !unsafe { sys_thread::join(pt) } {
                unregister_thread(self.core.id);

                let _g = core_lock(&self.core);
                let inner = unsafe { core_inner_mut(&self.core) };
                inner.state = ThreadState::Deleted;

                return Err(Error::ThreadJoinFailed);
            }

            let result = {
                let _g = core_lock(&self.core);
                let inner = unsafe { core_inner_mut(&self.core) };
                if inner.panic_payload {
                    Err(Error::ThreadJoinFailed)
                } else if matches!(&inner.callback_result, Some(Err(_))) {
                    Err(Error::ThreadJoinFailed)
                } else {
                    Ok(0)
                }
            };

            unregister_thread(self.core.id);

            {
                let _g = core_lock(&self.core);
                let inner = unsafe { core_inner_mut(&self.core) };
                inner.state = ThreadState::Deleted;
            }

            result
        } else {
            Err(Error::ThreadNotStarted)
        }
    }

    fn get_metadata(&self) -> ThreadMetadata {
        let _g = core_lock(&self.core);
        let inner = unsafe { core_inner_mut(&self.core) };
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
        if let Some(id) = get_current_thread_id() {
            if let Some(core) = lookup_by_handle(id as ThreadHandle) {
                return Self {
                    core: Arc::clone(&core),
                    handle: id as ThreadHandle,
                };
            }
        }

        // Lazy registration for main / non-OSAL threads.
        let id = next_thread_id();
        let handle = id as ThreadHandle;
        let core = Arc::new(ThreadCore::new(id, "main", 0, 1));
        // Override: main thread starts in Running state.
        {
            let _g = core_lock(&core);
            let inner = unsafe { core_inner_mut(&core) };
            inner.state = ThreadState::Running;
        }
        register_thread(id, &core);
        set_current_thread_id(id);
        Self { core, handle }
    }

    fn notify(&self, notification: ThreadNotification) -> Result<()> {
        let _g = core_lock(&self.core);
        let inner = unsafe { core_inner_mut(&self.core) };
        let (action, value) = notification.into();
        match action {
            0 => {}
            1 => inner.notification_value |= value,
            2 => inner.notification_value = inner.notification_value.wrapping_add(1),
            3 => inner.notification_value = value,
            4 => {
                if inner.notification_pending {
                    return Err(Error::QueueFull);
                }
                inner.notification_value = value;
            }
            _ => {}
        }
        inner.notification_pending = true;
        // Broadcast while holding the mutex — this is the standard
        // pattern for PosixCondvar (wake waiters before releasing).
        self.core.condvar.broadcast();
        Ok(())
    }

    fn notify_from_isr(&self, notification: ThreadNotification, hpw: &mut BaseType) -> Result<()> {
        // Non-blocking try-lock.
        if !self.core.inner.try_lock() {
            *hpw = 0;
            return Err(Error::QueueFull);
        }
        let inner = unsafe { core_inner_mut(&self.core) };
        let (action, value) = notification.into();
        match action {
            0 => {}
            1 => inner.notification_value |= value,
            2 => inner.notification_value = inner.notification_value.wrapping_add(1),
            3 => inner.notification_value = value,
            4 => {
                if inner.notification_pending {
                    assert!(
                        self.core.inner.unlock(),
                        "failed to unlock POSIX thread core mutex"
                    );
                    *hpw = 0;
                    return Err(Error::QueueFull);
                }
                inner.notification_value = value;
            }
            _ => {}
        }
        let was_waiting = inner.waiting_notification || inner.state == ThreadState::Blocked;
        inner.notification_pending = true;
        *hpw = if was_waiting { 1 } else { 0 };
        assert!(
            self.core.inner.unlock(),
            "failed to unlock POSIX thread core mutex"
        );
        self.core.condvar.broadcast();
        Ok(())
    }

    fn wait_notification(
        &self,
        bits_clear_entry: u32,
        bits_clear_exit: u32,
        timeout_ticks: TickType,
    ) -> Result<u32> {
        let _g = core_lock(&self.core);
        let inner = unsafe { core_inner_mut(&self.core) };

        inner.notification_value &= !bits_clear_entry;

        // Fast path.
        if inner.notification_pending {
            let v = inner.notification_value;
            inner.notification_value &= !bits_clear_exit;
            inner.notification_pending = false;
            return Ok(v);
        }

        // Cancellation / deletion takes priority.
        if inner.delete_requested || inner.state == ThreadState::Deleted {
            return Err(Error::Timeout);
        }

        if timeout_ticks == 0 {
            return Err(Error::Timeout);
        }

        // Infinite wait.
        if timeout_ticks == TickType::MAX {
            inner.waiting_notification = true;
            inner.state = ThreadState::Blocked;
            loop {
                self.core.condvar.wait(&self.core.inner);

                let inner2 = unsafe { core_inner_mut(&self.core) };

                if inner2.delete_requested || inner2.state == ThreadState::Deleted {
                    inner2.waiting_notification = false;
                    if !matches!(inner2.state, ThreadState::Deleted) {
                        inner2.state = ThreadState::Running;
                    }
                    return Err(Error::Timeout);
                }

                if inner2.notification_pending {
                    let v = inner2.notification_value;
                    inner2.notification_value &= !bits_clear_exit;
                    inner2.notification_pending = false;
                    inner2.waiting_notification = false;
                    inner2.state = ThreadState::Running;
                    return Ok(v);
                }
            }
        }

        // Finite wait with CLOCK_MONOTONIC deadline.
        let timeout_ms = (timeout_ticks as u64).saturating_mul(TICK_PERIOD_MS);
        let deadline = clock::deadline_from_ms(timeout_ms);

        inner.waiting_notification = true;
        inner.state = ThreadState::Blocked;

        loop {
            let signaled = self.core.condvar.timedwait(&self.core.inner, &deadline);

            let inner2 = unsafe { core_inner_mut(&self.core) };

            if inner2.delete_requested || inner2.state == ThreadState::Deleted {
                inner2.waiting_notification = false;
                if !matches!(inner2.state, ThreadState::Deleted) {
                    inner2.state = ThreadState::Running;
                }
                return Err(Error::Timeout);
            }

            if inner2.notification_pending {
                let v = inner2.notification_value;
                inner2.notification_value &= !bits_clear_exit;
                inner2.notification_pending = false;
                inner2.waiting_notification = false;
                inner2.state = ThreadState::Running;
                return Ok(v);
            }

            if !signaled {
                inner2.waiting_notification = false;
                inner2.state = ThreadState::Running;
                return Err(Error::Timeout);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Debug / Display
// ---------------------------------------------------------------------------

impl Debug for Thread {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        if self.core.inner.try_lock() {
            let inner = unsafe { core_inner_mut(&self.core) };
            let result = f
                .debug_struct("Thread")
                .field("id", &self.core.id)
                .field("name", &inner.name)
                .field("priority", &inner.priority)
                .field("state", &inner.state)
                .field("delete_requested", &inner.delete_requested)
                .field("spawn_started", &inner.spawn_started)
                .field("joined", &inner.joined)
                .finish();
            assert!(
                self.core.inner.unlock(),
                "failed to unlock POSIX thread core mutex"
            );
            result
        } else {
            f.debug_struct("Thread")
                .field("id", &self.core.id)
                .finish_non_exhaustive()
        }
    }
}

impl Display for Thread {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        if self.core.inner.try_lock() {
            let inner = unsafe { core_inner_mut(&self.core) };
            let result = write!(
                f,
                "Thread {{ id: {}, name: {}, priority: {}, state: {:?}, delete_requested: {} }}",
                self.core.id, inner.name, inner.priority, inner.state, inner.delete_requested
            );
            assert!(
                self.core.inner.unlock(),
                "failed to unlock POSIX thread core mutex"
            );
            result
        } else {
            write!(f, "Thread {{ id: {}, locked: true }}", self.core.id)
        }
    }
}
