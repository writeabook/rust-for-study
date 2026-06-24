//! System-level operations for the POSIX backend.
//!
//! Timing is backed by `CLOCK_MONOTONIC` through the POSIX sys clock layer.
//! Scheduler and ISR operations are host-simulation no-ops because POSIX
//! user space does not provide an RTOS scheduler or real ISR context.
//!
//! # Design
//!
//! - **Startup anchor**: A `pthread_once_t` + `static mut u64` captures the
//!   monotonic clock on first access so all tick / duration values are
//!   relative to process start.
//! - **Delays**: [`libc::nanosleep`] with signal-interruption restart.
//! - **Critical sections**: `PosixMutex` (`PTHREAD_MUTEX_RECURSIVE`) with
//!   per-thread `pthread_key_t` TLS nesting depth — simulates mutual
//!   exclusion among OSAL callers but does NOT disable OS scheduling or
//!   hardware interrupts.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::time::Duration;

use libc::{PTHREAD_MUTEX_RECURSIVE, nanosleep};

use super::config::TICK_PERIOD_MS;
use super::sys::clock;
use super::sys::mutex::PosixMutex;
use super::thread::{ThreadMetadata, ThreadState};
use super::types::{BaseType, TickType, UBaseType};

use crate::traits::{SystemFn, ToTick};
use crate::utils::OsalRsBool;

// ---------------------------------------------------------------------------
// Startup-time anchor
// ---------------------------------------------------------------------------

/// pthread_once-based global init for the startup monotonic timestamp.
///
/// # Safety
///
/// `pthread_once` guarantees exactly-once semantics.  The raw pointer /
/// `static mut` access is protected by this guard.
static mut START_NS_ONCE: libc::pthread_once_t = libc::PTHREAD_ONCE_INIT;
static mut START_NS: u64 = 0;

extern "C" fn init_start_ns() {
    unsafe {
        START_NS = clock::now_ns();
    }
}

/// Returns the monotonic nanosecond timestamp captured the first time any
/// OSAL timing function is called.
fn startup_ns() -> u64 {
    unsafe {
        libc::pthread_once(&raw mut START_NS_ONCE, init_start_ns);
        START_NS
    }
}

/// Nanoseconds elapsed since the startup anchor.
fn elapsed_ns() -> u64 {
    clock::now_ns().saturating_sub(startup_ns())
}

/// Elapsed `Duration` since the startup anchor.
fn elapsed_duration() -> Duration {
    Duration::from_nanos(elapsed_ns())
}

// ---------------------------------------------------------------------------
// Sleep helper
// ---------------------------------------------------------------------------

/// Sleep for `ns` nanoseconds via `nanosleep`, restarting if interrupted by
/// a signal.
///
// TODO(posix): retry only on EINTR once a portable errno helper is added.
// Currently any non-zero return triggers a retry, which is conservative but
// may loop on non-EINTR errors.  A proper fix should check `errno` through a
// platform-correct mechanism (e.g. `libc::__errno_location()` on Linux/glibc,
// `extern fn __error()` on macOS/BSD).
fn sleep_ns(mut ns: u64) {
    while ns > 0 {
        let req = clock::ns_to_timespec(ns);
        let mut rem = clock::ns_to_timespec(0);

        let ret = unsafe { nanosleep(&req, &mut rem) };

        if ret == 0 {
            break;
        }

        // Interrupted — sleep the remaining time.
        ns = (rem.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(rem.tv_nsec as u64);
    }
}

/// Sleep for `ticks` OSAL logical ticks.
fn sleep_ticks(ticks: TickType) {
    if ticks == 0 {
        return;
    }

    let ms = (ticks as u64).saturating_mul(TICK_PERIOD_MS);
    let ns = clock::ms_to_ns(ms);
    sleep_ns(ns);
}

// ---------------------------------------------------------------------------
// Global critical-section lock
// ---------------------------------------------------------------------------

/// pthread_once-based global init for the critical-section mutex.
///
/// Initialisation failure is fatal (`expect` aborts).  The mutex is leaked
/// (process-lifetime), which matches OSAL global-singleton semantics.
///
/// # Safety
///
/// `pthread_once` guarantees exactly-once semantics.  The raw pointer deref
/// is safe because the pointer is only written once before any read.
static mut CRITICAL_LOCK_ONCE: libc::pthread_once_t = libc::PTHREAD_ONCE_INIT;
static mut CRITICAL_LOCK_PTR: *const PosixMutex = core::ptr::null();

extern "C" fn init_critical_lock() {
    let lock = Box::new(
        PosixMutex::new(PTHREAD_MUTEX_RECURSIVE)
            .expect("failed to initialize POSIX critical-section mutex"),
    );
    unsafe {
        CRITICAL_LOCK_PTR = Box::into_raw(lock);
    }
}

fn global_critical_lock() -> &'static PosixMutex {
    unsafe {
        libc::pthread_once(&raw mut CRITICAL_LOCK_ONCE, init_critical_lock);
        &*CRITICAL_LOCK_PTR
    }
}

// ---------------------------------------------------------------------------
// Per-thread critical-section depth (pthread TLS)
// ---------------------------------------------------------------------------

/// pthread TLS key with destructor — avoids per-thread `Box<usize>` leak.
///
/// # Safety
///
/// Key creation (`pthread_key_create`) is guarded by `pthread_once`.
/// The destructor frees the heap-allocated `usize` when a thread exits.
static mut CRITICAL_DEPTH_KEY: libc::pthread_key_t = 0;
static mut CRITICAL_DEPTH_KEY_ONCE: libc::pthread_once_t = libc::PTHREAD_ONCE_INIT;

extern "C" fn drop_critical_depth(ptr: *mut c_void) {
    if !ptr.is_null() {
        unsafe {
            drop(Box::from_raw(ptr as *mut usize));
        }
    }
}

extern "C" fn init_critical_depth_key() {
    unsafe {
        let rc = libc::pthread_key_create(&raw mut CRITICAL_DEPTH_KEY, Some(drop_critical_depth));
        assert_eq!(rc, 0, "failed to create POSIX critical depth TLS key");
    }
}

/// Returns a pointer to the calling thread's critical-section nesting depth.
///
/// On first access for a given thread the value is heap-allocated and
/// initialised to 0.
fn critical_depth_ptr() -> *mut usize {
    unsafe {
        libc::pthread_once(&raw mut CRITICAL_DEPTH_KEY_ONCE, init_critical_depth_key);

        let ptr = libc::pthread_getspecific(CRITICAL_DEPTH_KEY) as *mut usize;
        if !ptr.is_null() {
            return ptr;
        }

        let boxed = Box::new(0usize);
        let raw = Box::into_raw(boxed);
        let rc = libc::pthread_setspecific(CRITICAL_DEPTH_KEY, raw as *mut c_void);
        if rc != 0 {
            drop(Box::from_raw(raw));
            panic!("failed to set POSIX critical depth TLS");
        }
        raw
    }
}

fn enter_global_critical() -> UBaseType {
    let depth = critical_depth_ptr();
    unsafe {
        let previous_depth = *depth;

        if previous_depth == 0 {
            let locked = global_critical_lock().lock();
            assert!(locked, "failed to lock POSIX critical-section mutex");
        }

        *depth = previous_depth
            .checked_add(1)
            .expect("POSIX critical-section nesting depth overflow");

        previous_depth as UBaseType
    }
}

fn exit_global_critical() {
    let depth = critical_depth_ptr();
    unsafe {
        if *depth == 0 {
            debug_assert!(
                false,
                "POSIX critical-section exit called without matching enter"
            );
            return;
        }

        *depth -= 1;

        if *depth == 0 {
            let unlocked = global_critical_lock().unlock();
            assert!(unlocked, "failed to unlock POSIX critical-section mutex");
        }
    }
}

// ---------------------------------------------------------------------------
// SystemState
// ---------------------------------------------------------------------------

/// Snapshot of system-wide thread state.
#[derive(Debug, Clone)]
pub struct SystemState {
    /// Metadata for each tracked thread.
    pub tasks: Vec<ThreadMetadata>,
    /// Accumulated run-time across all threads (milliseconds).
    pub total_run_time: u32,
}

impl core::ops::Deref for SystemState {
    type Target = [ThreadMetadata];

    fn deref(&self) -> &Self::Target {
        &self.tasks
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// System-level operations for the POSIX backend.
pub struct System;

impl System {
    /// Convenience wrapper: delays using a `ToTick`-implementing type.
    #[inline]
    pub fn delay_with_to_tick(ticks: impl ToTick) {
        Self::delay(ticks.to_ticks());
    }

    /// Convenience wrapper: delays until using a `ToTick`-implementing type.
    #[inline]
    pub fn delay_until_with_to_tick(
        previous_wake_time: &mut TickType,
        time_increment: impl ToTick,
    ) {
        Self::delay_until(previous_wake_time, time_increment.to_ticks());
    }

    // ------------------------------------------------------------------
    // Scheduler control (no-ops on POSIX)
    // ------------------------------------------------------------------

    /// Starts the scheduler — **no-op on POSIX**.
    pub fn start() {}

    /// Stops the scheduler — **no-op on POSIX**.
    pub fn stop() {}

    /// Suspends all threads — **no-op on POSIX**.
    pub fn suspend_all() {}

    /// Resumes all threads — **no-op on POSIX**.
    pub fn resume_all() -> BaseType {
        0
    }

    // ------------------------------------------------------------------
    // Timing
    // ------------------------------------------------------------------

    /// Returns the current OSAL tick count based on `CLOCK_MONOTONIC`.
    ///
    /// Wraps naturally at `TickType::MAX`, matching RTOS tick-count semantics.
    /// Callers should use `wrapping_sub` to compute elapsed ticks.
    pub fn get_tick_count() -> TickType {
        let elapsed_ms = elapsed_ns() / 1_000_000;
        let ticks = elapsed_ms / TICK_PERIOD_MS;

        ticks as TickType
    }

    /// Returns the elapsed monotonic `Duration` since the first OSAL
    /// timing call.
    pub fn get_current_time_us() -> Duration {
        elapsed_duration()
    }

    /// Converts a `Duration` to an OSAL tick count.
    ///
    /// Delegates to [`ToTick::to_ticks`].
    pub fn get_us_from_tick(duration: &Duration) -> TickType {
        duration.to_ticks()
    }

    // ------------------------------------------------------------------
    // Delays
    // ------------------------------------------------------------------

    /// Blocks the calling thread for at least `ticks` OSAL ticks using
    /// `nanosleep`.  `ticks = 0` returns immediately.
    pub fn delay(ticks: TickType) {
        sleep_ticks(ticks);
    }

    /// Delays until an absolute tick time (FreeRTOS `xTaskDelayUntil`
    /// semantics).
    pub fn delay_until(previous_wake_time: &mut TickType, time_increment: TickType) {
        let next = previous_wake_time.wrapping_add(time_increment);
        let now = Self::get_tick_count();

        if next > now {
            let diff = next.wrapping_sub(now);
            sleep_ticks(diff);
        }

        *previous_wake_time = next;
    }

    // ------------------------------------------------------------------
    // Timer check
    // ------------------------------------------------------------------

    /// Checks whether `time` has elapsed since `timestamp`.
    pub fn check_timer(timestamp: &Duration, time: &Duration) -> OsalRsBool {
        let now = elapsed_duration();

        let elapsed = if now >= *timestamp {
            now - *timestamp
        } else {
            // Clock adjustment edge-case: treat as already expired.
            *time
        };

        if elapsed >= *time {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    // ------------------------------------------------------------------
    // Critical sections (process-local recursive PosixMutex)
    // ------------------------------------------------------------------

    /// Enters a critical section using the global recursive POSIX mutex.
    ///
    /// This does **not** disable OS scheduling or hardware interrupts.
    /// It only provides mutual exclusion among OSAL callers.
    pub fn critical_section_enter() {
        enter_global_critical();
    }

    /// Exits a critical section.
    pub fn critical_section_exit() {
        exit_global_critical();
    }

    /// Enters a task-level critical section — aliases `critical_section_enter`.
    pub fn enter_critical() {
        enter_global_critical();
    }

    /// Exits a task-level critical section — aliases `critical_section_exit`.
    pub fn exit_critical() {
        exit_global_critical();
    }

    /// Enters a critical section from ISR context (host-simulation).
    ///
    /// Reuses the same recursive lock as task-level calls.  Returns the
    /// previous nesting depth as a saved interrupt status for API
    /// compatibility.
    pub fn enter_critical_from_isr() -> UBaseType {
        enter_global_critical()
    }

    /// Exits a critical section from ISR context (host-simulation).
    ///
    /// `_saved_interrupt_status` is accepted for API compatibility but
    /// not used to restore state (nesting depth already tracks this).
    pub fn exit_critical_from_isr(_saved_interrupt_status: UBaseType) {
        exit_global_critical();
    }

    // ------------------------------------------------------------------
    // ISR support (no-ops on POSIX)
    // ------------------------------------------------------------------

    /// Yields from ISR — **no-op on POSIX** (no real ISR context).
    pub fn yield_from_isr(_higher_priority_task_woken: BaseType) {}

    /// Ends an ISR — **no-op on POSIX** (no real ISR context).
    pub fn end_switching_isr(_switch_required: BaseType) {}

    // ------------------------------------------------------------------
    // System introspection
    // ------------------------------------------------------------------

    /// Returns the current thread state via the thread registry.
    pub fn get_state() -> ThreadState {
        super::thread::current_thread_state()
    }

    /// Returns the number of live threads in the OSAL thread registry.
    pub fn count_threads() -> usize {
        super::thread::count_registered_threads()
    }

    /// Returns a snapshot of all threads in the OSAL thread registry.
    pub fn get_all_thread() -> SystemState {
        let tasks = super::thread::snapshot_registered_threads();

        SystemState {
            tasks,
            total_run_time: 1,
        }
    }

    /// Returns the amount of free heap memory.
    ///
    /// Returns [`usize::MAX`] on POSIX — there is no RTOS heap.
    pub fn get_free_heap_size() -> usize {
        usize::MAX
    }
}

// ---------------------------------------------------------------------------
// SystemFn trait implementation
// ---------------------------------------------------------------------------

impl SystemFn for System {
    fn start() {
        Self::start();
    }

    fn get_state() -> ThreadState {
        Self::get_state()
    }

    fn suspend_all() {
        Self::suspend_all();
    }

    fn resume_all() -> BaseType {
        Self::resume_all()
    }

    fn stop() {
        Self::stop();
    }

    fn get_tick_count() -> TickType {
        Self::get_tick_count()
    }

    fn get_current_time_us() -> Duration {
        Self::get_current_time_us()
    }

    fn get_us_from_tick(duration: &Duration) -> TickType {
        Self::get_us_from_tick(duration)
    }

    fn count_threads() -> usize {
        Self::count_threads()
    }

    fn get_all_thread() -> SystemState {
        Self::get_all_thread()
    }

    fn delay(ticks: TickType) {
        Self::delay(ticks);
    }

    fn delay_until(previous_wake_time: &mut TickType, time_increment: TickType) {
        Self::delay_until(previous_wake_time, time_increment);
    }

    fn critical_section_enter() {
        Self::critical_section_enter();
    }

    fn critical_section_exit() {
        Self::critical_section_exit();
    }

    fn check_timer(timestamp: &Duration, time: &Duration) -> OsalRsBool {
        Self::check_timer(timestamp, time)
    }

    fn yield_from_isr(higher_priority_task_woken: BaseType) {
        Self::yield_from_isr(higher_priority_task_woken);
    }

    fn end_switching_isr(switch_required: BaseType) {
        Self::end_switching_isr(switch_required);
    }

    fn enter_critical() {
        Self::enter_critical();
    }

    fn exit_critical() {
        Self::exit_critical();
    }

    fn enter_critical_from_isr() -> UBaseType {
        Self::enter_critical_from_isr()
    }

    fn exit_critical_from_isr(saved_interrupt_status: UBaseType) {
        Self::exit_critical_from_isr(saved_interrupt_status);
    }

    fn get_free_heap_size() -> usize {
        Self::get_free_heap_size()
    }
}
