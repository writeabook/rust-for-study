//! System-level operations for the Linux backend.
//!
//! # Overview
//!
//! Provides the `System` struct and `SystemState` snapshot type with real
//! monotonic time via `std::time::Instant`. All timing methods (tick count,
//! delay, delay-until, check-timer) are backed by the OS monotonic clock,
//! matching the behavioural contract defined in `doc/osal-contact-zh.md`.
//!
//! # Design
//!
//! - **Startup anchor**: A [`OnceLock<Instant>`] captures the process start
//!   time on first access. All tick / duration calculations are relative to
//!   this anchor, ensuring monotonicity.
//! - **Tick period**: 1 tick = 1 ms (see [`config::TICK_PERIOD_MS`]).
//! - **Scheduler methods** (`start`, `stop`, …): documented no-ops because
//!   Linux user space has no application-level RTOS scheduler.
//! - **ISR methods** (`_from_isr`, `yield_from_isr`, …): documented no-ops.
//! - **Critical sections**: simulated with a process-local global recursive
//!   lock. This does **not** disable Linux interrupts or OS scheduling; it
//!   only provides mutual exclusion among OSAL callers inside the current
//!   process. Nesting is supported via per-thread depth counting.
//!
//! # Mock/Stub Limitations
//!
//! | Method                  | Behaviour                                       |
//! |-------------------------|-------------------------------------------------|
//! | `get_state()`           | Delegates to the thread registry                |
//! | `count_threads()`       | Returns the number of registered OSAL threads   |
//! | `get_all_thread()`      | Returns a snapshot of the thread registry       |
//! | `get_free_heap_size()`  | Returns `usize::MAX` (no heap limit)            |
//! | `critical_section_*()`  | Process-local recursive lock                    |
//! | `suspend_all / resume`  | No-op                                           |
//! | `start()` / `stop()`    | No-op                                           |
//! | `yield_from_isr()` …    | No-op                                           |

use alloc::vec::Vec;
use core::cell::RefCell;
use core::time::Duration;

use std::sync::{LockResult, Mutex as StdMutex, MutexGuard as StdMutexGuard, OnceLock};
use std::thread;
use std::time::Instant;

use super::config::TICK_PERIOD_MS;
use super::thread::{ThreadMetadata, ThreadState};
use super::types::{BaseType, TickType, UBaseType};
use crate::traits::{SystemFn, ToTick};
use crate::utils::OsalRsBool;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Recovers from a poisoned mutex lock.  Keeps system-level operations
/// usable even after a panic inside a critical section.
fn recover_lock<T>(result: LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ---------------------------------------------------------------------------
// Startup-time anchor
// ---------------------------------------------------------------------------

/// Returns the `Instant` captured when the process first accessed any
/// OSAL timing function.
///
/// Uses [`OnceLock`] for thread-safe lazy initialisation. All tick and
/// duration calculations are relative to this instant.
fn startup_instant() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

// ---------------------------------------------------------------------------
// Global critical-section lock
// ---------------------------------------------------------------------------

/// Returns the global critical-section mutex, lazily initialised.
fn global_critical_lock() -> &'static StdMutex<()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
}

/// Per-thread state for reentrant critical sections.
struct CriticalThreadState {
    depth: usize,
    guard: Option<StdMutexGuard<'static, ()>>,
}

thread_local! {
    static CRITICAL_THREAD_STATE: RefCell<CriticalThreadState> =
        RefCell::new(CriticalThreadState {
            depth: 0,
            guard: None,
        });
}

/// Acquires the global critical lock for the current thread.
///
/// On first entry (`depth` 0 → 1) the thread actually locks
/// `global_critical_lock()`.  Subsequent entries only increment `depth`.
///
/// Returns the nesting depth **before** this entry (usable as saved
/// interrupt status in `_from_isr` variants).
///
/// In the Linux backend this may block like the task-level simulated
/// critical section; it is not intended to model a real hardware ISR.
fn enter_global_critical() -> UBaseType {
    CRITICAL_THREAD_STATE.with(|state_cell| {
        let mut state = state_cell.borrow_mut();
        let previous_depth = state.depth;

        if state.depth == 0 {
            let guard = recover_lock(global_critical_lock().lock());
            state.guard = Some(guard);
        }

        state.depth = state
            .depth
            .checked_add(1)
            .expect("Linux critical section nesting depth overflow");

        previous_depth as UBaseType
    })
}

/// Releases the global critical lock for the current thread.
///
/// Decrements `depth`.  When `depth` reaches 0 the guard is dropped,
/// releasing `global_critical_lock()` for other threads.
///
/// If called without a matching `enter`, a `debug_assert!` fires in
/// debug builds; in release builds the call is silently ignored.
fn exit_global_critical() {
    CRITICAL_THREAD_STATE.with(|state_cell| {
        let mut state = state_cell.borrow_mut();

        if state.depth == 0 {
            debug_assert!(
                false,
                "Linux critical section exit called without matching enter"
            );
            return;
        }

        state.depth -= 1;

        if state.depth == 0 {
            state.guard.take(); // drop the guard, releasing the lock
        }
    })
}

// ---------------------------------------------------------------------------
// SystemState
// ---------------------------------------------------------------------------

/// Snapshot of system-wide thread state.
///
/// Captures metadata for threads currently known to the Linux OSAL
/// runtime.  The snapshot is best-effort and reflects the registry
/// state at collection time.
///
/// # Examples
///
/// ```ignore
/// use osal_rs::os::{System, SystemFn};
///
/// let state = System::get_all_thread();
/// println!("Tasks: {}, runtime: {}", state.tasks.len(), state.total_run_time);
/// ```
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

/// System-level operations.
///
/// Static methods mirroring the FreeRTOS `System` API. Time-related
/// methods use `std::time::Instant`; scheduler and ISR methods are
/// documented no-ops on Linux.
pub struct System;

impl System {
    // ------------------------------------------------------------------
    // Convenience helpers (mirror FreeRTOS System)
    // ------------------------------------------------------------------

    /// Delays execution using a type that implements [`ToTick`].
    ///
    /// Convenience method that accepts `Duration` or other
    /// tick-convertible types and calls [`delay`](Self::delay).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use osal_rs::os::System;
    /// use core::time::Duration;
    ///
    /// System::delay_with_to_tick(Duration::from_millis(100));
    /// ```
    #[inline]
    pub fn delay_with_to_tick(ticks: impl ToTick) {
        Self::delay(ticks.to_ticks());
    }

    /// Delays until an absolute time point with tick conversion.
    ///
    /// Convenience method that converts `time_increment` via
    /// [`ToTick`] and delegates to [`delay_until`](Self::delay_until).
    ///
    /// # Parameters
    ///
    /// * `previous_wake_time` — Previous wake time (will be updated).
    /// * `time_increment` — Period between wake times.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use osal_rs::os::System;
    /// use core::time::Duration;
    ///
    /// let mut last_wake = System::get_tick_count();
    /// loop {
    ///     System::delay_until_with_to_tick(&mut last_wake, Duration::from_millis(100));
    /// }
    /// ```
    #[inline]
    pub fn delay_until_with_to_tick(
        previous_wake_time: &mut TickType,
        time_increment: impl ToTick,
    ) {
        Self::delay_until(previous_wake_time, time_increment.to_ticks());
    }

    // ------------------------------------------------------------------
    // Scheduler control (no-ops on Linux)
    // ------------------------------------------------------------------

    /// Starts the scheduler.
    ///
    /// **No-op on Linux** — there is no application-level RTOS scheduler
    /// to start. Threads created via the OSAL thread API execute
    /// immediately upon spawn.
    pub fn start() {}

    /// Stops the scheduler.
    ///
    /// **No-op on Linux** — see [`start`](Self::start).
    pub fn stop() {}

    /// Suspends all OSAL-managed threads.
    ///
    /// **No-op** — thread suspension is not implemented in the Linux backend.
    pub fn suspend_all() {}

    /// Resumes all OSAL-managed threads.
    ///
    /// **No-op**. Always returns `0`.
    pub fn resume_all() -> BaseType {
        0
    }

    // ------------------------------------------------------------------
    // Timing
    // ------------------------------------------------------------------

    /// Returns the current OSAL tick count.
    ///
    /// Each tick equals [`TICK_PERIOD_MS`] milliseconds (1 ms on Linux).
    /// The value is derived from `std::time::Instant` and is therefore
    /// **monotonic** — it will never decrease and is not affected by
    /// wall-clock adjustments.
    ///
    /// # Returns
    ///
    /// Milliseconds since first OSAL timing call, truncated to
    /// [`TickType`] range.
    pub fn get_tick_count() -> TickType {
        let elapsed = startup_instant().elapsed();
        let millis = elapsed.as_millis() as u64;
        (millis / TICK_PERIOD_MS) as TickType
    }

    /// Returns the current monotonic time as a `Duration`.
    ///
    /// The returned value is relative to the first OSAL timing call
    /// and will never decrease.
    ///
    /// # Returns
    ///
    /// Elapsed `Duration` since the first OSAL timing call.
    pub fn get_current_time_us() -> Duration {
        startup_instant().elapsed()
    }

    /// Converts a `Duration` to an OSAL tick count.
    ///
    /// Delegates to [`ToTick::to_ticks`] on the provided duration.
    ///
    /// # Parameters
    ///
    /// * `duration` — The duration to convert.
    pub fn get_us_from_tick(duration: &Duration) -> TickType {
        duration.to_ticks()
    }

    // ------------------------------------------------------------------
    // Delays
    // ------------------------------------------------------------------

    /// Blocks the calling thread for at least `ticks` OSAL ticks.
    ///
    /// Uses `std::thread::sleep`. The actual sleep duration may be
    /// slightly longer due to OS scheduling granularity.
    ///
    /// # Parameters
    ///
    /// * `ticks` — Minimum number of ticks to delay.
    ///
    /// # Panics
    ///
    /// Does not panic. `ticks = 0` returns immediately.
    pub fn delay(ticks: TickType) {
        if ticks == 0 {
            return;
        }
        let ms = ticks.saturating_mul(TICK_PERIOD_MS as TickType) as u64;
        thread::sleep(Duration::from_millis(ms));
    }

    /// Delays until an absolute tick time.
    ///
    /// Calculates the next wake time as `*previous_wake_time + time_increment`.
    /// If that time is still in the future, sleeps until then.
    /// Always updates `*previous_wake_time` to the calculated next wake time,
    /// even if the time has already passed (matching FreeRTOS `xTaskDelayUntil`).
    ///
    /// # Parameters
    ///
    /// * `previous_wake_time` — Previous wake time; will be set to
    ///   `*previous_wake_time + time_increment`.
    /// * `time_increment` — Period between wake times in ticks.
    pub fn delay_until(previous_wake_time: &mut TickType, time_increment: TickType) {
        let next = previous_wake_time.wrapping_add(time_increment);
        let now = Self::get_tick_count();

        if next > now {
            let diff = next.wrapping_sub(now);
            let ms = diff.saturating_mul(TICK_PERIOD_MS as TickType) as u64;
            thread::sleep(Duration::from_millis(ms));
        }

        *previous_wake_time = next;
    }

    // ------------------------------------------------------------------
    // Timer check
    // ------------------------------------------------------------------

    /// Checks whether a timeout has expired.
    ///
    /// Compares the elapsed time since `timestamp` against `time`.
    /// On Linux (64-bit `Duration`) overflow is practically impossible,
    /// so we simply compare `elapsed >= *time`.
    ///
    /// # Parameters
    ///
    /// * `timestamp` — Start time reference.
    /// * `time` — Timeout duration to check against.
    ///
    /// # Returns
    ///
    /// * `True` — The timeout has expired.
    /// * `False` — The timeout has not yet expired.
    pub fn check_timer(timestamp: &Duration, time: &Duration) -> OsalRsBool {
        let elapsed = startup_instant().elapsed();
        let time_passing = if elapsed >= *timestamp {
            elapsed - *timestamp
        } else {
            // Clock adjustment edge-case: treat as already expired.
            *time
        };

        if time_passing >= *time {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }

    // ------------------------------------------------------------------
    // Critical sections (process-local recursive lock on Linux)
    // ------------------------------------------------------------------

    /// Enters a critical section.
    ///
    /// Uses a process-local recursive lock.  The same thread may nest
    /// calls; the lock is only released when `exit` has been called the
    /// same number of times.
    ///
    /// **Linux note**: This does NOT disable interrupts or OS scheduling.
    /// It only provides mutual exclusion among OSAL callers within the
    /// current process.
    pub fn critical_section_enter() {
        enter_global_critical();
    }

    /// Exits a critical section.
    ///
    /// Decrements the nesting counter.  When the counter reaches zero
    /// the global critical lock is released, allowing other threads to
    /// enter.
    pub fn critical_section_exit() {
        exit_global_critical();
    }

    /// Enters a critical section at task level.
    ///
    /// Shares the same recursive lock as [`critical_section_enter`].
    pub fn enter_critical() {
        enter_global_critical();
    }

    /// Exits a critical section at task level.
    ///
    /// Shares the same release logic as [`critical_section_exit`].
    pub fn exit_critical() {
        exit_global_critical();
    }

    /// Enters a critical section from ISR context.
    ///
    /// Linux has no real ISR context in user space — this reuses the same
    /// simulated critical-section lock as task-level calls.  Returns the
    /// previous nesting depth as a saved interrupt status.
    pub fn enter_critical_from_isr() -> UBaseType {
        enter_global_critical()
    }

    /// Exits a critical section from ISR context.
    ///
    /// `_saved_interrupt_status` is accepted for API compatibility but
    /// not used to restore state (the recursive lock depth already
    /// tracks this).
    pub fn exit_critical_from_isr(_saved_interrupt_status: UBaseType) {
        exit_global_critical();
    }

    // ------------------------------------------------------------------
    // ISR support (no-ops on Linux)
    // ------------------------------------------------------------------

    /// Yields from ISR if a higher-priority task was woken.
    ///
    /// **No-op on Linux** — there is no ISR context in user space.
    pub fn yield_from_isr(_higher_priority_task_woken: BaseType) {}

    /// Ends an ISR and performs a context switch if required.
    ///
    /// **No-op on Linux** — see [`yield_from_isr`](Self::yield_from_isr).
    pub fn end_switching_isr(_switch_required: BaseType) {}

    // ------------------------------------------------------------------
    // System introspection
    // ------------------------------------------------------------------

    /// Returns the current thread state.
    ///
    /// Delegates to the OSAL thread registry.
    pub fn get_state() -> ThreadState {
        super::thread::current_thread_state()
    }

    /// Returns the number of live threads in the OSAL thread registry.
    pub fn count_threads() -> usize {
        super::thread::count_registered_threads()
    }

    /// Returns a snapshot of all threads registered in the OSAL thread
    /// registry.
    pub fn get_all_thread() -> SystemState {
        let tasks = super::thread::snapshot_registered_threads();
        SystemState { tasks, total_run_time: 1 }
    }

    /// Returns the amount of free heap memory.
    ///
    /// Returns [`usize::MAX`] on Linux — there is no RTOS heap, and
    /// the process can allocate as much as the OS permits.
    pub fn get_free_heap_size() -> usize {
        usize::MAX
    }
}

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

// ---------------------------------------------------------------------------
// Internal poison-recovery test
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// After the global critical lock is poisoned, subsequent
    /// enter/exit calls should still work without panicking.
    #[test]
    fn critical_lock_recovers_from_poison() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = global_critical_lock().lock().unwrap();
            panic!("poison critical lock");
        }));

        // These must not panic after poison recovery.
        System::enter_critical();
        System::exit_critical();
    }
}