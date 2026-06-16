//! Software timer support for the Linux backend.
//!
//! # Design
//!
//! Each `Timer` owns one dedicated worker `std::thread` created at
//! construction time.  The worker blocks on a `Condvar` waiting for
//! commands (`start`, `stop`, `reset`, `change_period`) or deadline
//! expiry, then fires the callback *outside* the internal lock.
//!
//! # State machine
//!
//! ```text
//! Stopped → Armed     (start/reset/change_period)
//! Armed   → Executing (deadline reached)
//! Armed   → Stopped   (stop/delete)
//! Executing → Armed   (periodic auto-reload, or command in callback)
//! Executing → Stopped (one-shot finished, or stop/delete in callback)
//! Any     → Deleted   (delete / last handle dropped)
//! ```
//!
//! # Generation
//!
//! A `generation` counter is bumped on every command that changes the
//! timing schedule.  The worker remembers which generation it observed
//! before sleeping; on wake-up it checks whether a newer command
//! arrived and discards its own state if so.
//!
//! # Limitations
//!
//! - One OS thread per timer (lifetime = lifetime of the timer).
//! - `ticks_to_wait` is accepted only for API compatibility with the
//!   timer trait; commands are applied synchronously.
//! - No FreeRTOS-style command queue.

use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use core::time::Duration;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;

use std::sync::{Condvar, Mutex as StdMutex};
use std::thread::{Builder as ThreadBuilder, JoinHandle};
use std::time::Instant;

use crate::traits::{TimerFn, TimerFnPtr, TimerParam, ToTick};
use crate::utils::{Error, OsalRsBool, Result};
use super::types::{TickType, TimerHandle};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

static NEXT_TIMER_HANDLE: AtomicUsize = AtomicUsize::new(1);

fn next_timer_handle() -> TimerHandle {
    NEXT_TIMER_HANDLE
        .fetch_update(AtomicOrdering::Relaxed, AtomicOrdering::Relaxed, |c| c.checked_add(1))
        .expect("Linux timer handle space exhausted") as TimerHandle
}

fn deadline_from_period(period: TickType) -> Option<Instant> {
    if period == 0 { return None; }
    Instant::now().checked_add(Duration::from_millis(period as u64))
}

fn bump_generation(g: &mut u64) {
    *g = g.checked_add(1).expect("Linux timer generation exhausted");
}

// ---------------------------------------------------------------------------
// TimerState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerState {
    Stopped,
    Armed,
    Executing,
    Deleted,
}

// ---------------------------------------------------------------------------
// Common-command logic
// ---------------------------------------------------------------------------

struct TimerInner {
    name: String,
    period: TickType,
    auto_reload: bool,

    callback: Arc<TimerFnPtr>,
    param: Option<TimerParam>,

    state: TimerState,
    deadline: Option<Instant>,
    generation: u64,
}

struct TimerCore {
    id: usize,
    handle: TimerHandle,
    inner: StdMutex<TimerInner>,
    condvar: Condvar,
    public_handles: AtomicUsize,
    worker: StdMutex<Option<JoinHandle<()>>>,
}

// Safety: StdMutex + Condvar + AtomicUsize are all Send + Sync.
unsafe impl Send for TimerCore {}
unsafe impl Sync for TimerCore {}

/// Shared command body: set state to Armed with a fresh deadline.
fn command_arm(inner: &mut TimerInner, period: TickType) {
    if let Some(dl) = deadline_from_period(period) {
        inner.state = TimerState::Armed;
        inner.deadline = Some(dl);
        bump_generation(&mut inner.generation);
    }
}

/// Shared command body: set state to Stopped, clear deadline.
fn command_stop(inner: &mut TimerInner) {
    inner.state = TimerState::Stopped;
    inner.deadline = None;
    bump_generation(&mut inner.generation);
}

/// Shut down the worker and join it (unless called from within the worker).
fn worker_shutdown(core: &TimerCore) {
    // Wake the worker
    core.condvar.notify_all();

    let worker = {
        let mut slot = recover_lock(core.worker.lock());
        slot.take()
    };

    if let Some(w) = worker {
        if w.thread().id() != std::thread::current().id() {
            let _ = w.join();
        }
        // If we are the worker, just drop the handle (no self-join).
    }
}

/// Shutdown (close) the timer.  Marks Deleted, notifies the worker,
/// and joins it.
fn shutdown(core: &TimerCore) {
    {
        let mut inner = recover_lock(core.inner.lock());
        if inner.state == TimerState::Deleted { return; }
        inner.state = TimerState::Deleted;
        inner.deadline = None;
        bump_generation(&mut inner.generation);
    }
    worker_shutdown(core);
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

fn worker_loop(core: Arc<TimerCore>) {
    loop {
        // --- Wait while Stopped or (Armed with future deadline) ---
        let (cb, param, auto_reload, fired_generation) = {
            let mut inner = recover_lock(core.inner.lock());

            // --- Stopped ---
            while inner.state == TimerState::Stopped {
                inner = recover_lock(core.condvar.wait(inner));
            }

            // --- Exit on Deleted ---
            if inner.state == TimerState::Deleted { return; }

            // --- Armed: wait for deadline or command ---
            if inner.state == TimerState::Armed {
                let deadline = inner.deadline.expect("Armed without deadline");
                let generation = inner.generation;
                let now = Instant::now();

                if now < deadline {
                    let remaining = deadline - now;
                    let (next, wait_result) =
                        recover_lock(core.condvar.wait_timeout(inner, remaining));
                    inner = next;

                    // After wake-up
                    if inner.state == TimerState::Deleted { return; }
                    if inner.generation != generation {
                        // A command arrived — re-evaluate state
                        continue;
                    }
                    if !wait_result.timed_out() {
                        // Spurious wake-up — re-evaluate
                        continue;
                    }
                    // Timed out → deadline reached
                }

                // Deadline reached: transition to Executing
                inner.state = TimerState::Executing;
                inner.deadline = None;

                let cb = Arc::clone(&inner.callback);
                let param = inner.param.clone();
                let auto_reload = inner.auto_reload;
                let fired_generation = inner.generation;
                drop(inner); // release lock before callback

                (cb, param, auto_reload, fired_generation)
            } else {
                // Shouldn't happen, but loop again
                continue;
            }
        };

        // --- Execute callback OUTSIDE the lock ---
        // Increment public_handles so the callback-timer's Drop does not
        // trigger shutdown() when the real public handle is still alive.
        core.public_handles.fetch_add(1, AtomicOrdering::Relaxed);
        let callback_timer: Box<dyn TimerFn> = Box::new(Timer {
            core: Arc::clone(&core),
        });

        let callback_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (cb)(callback_timer, param)
        }));

        // --- Post-callback: re-lock and process result ---
        {
            let mut inner = recover_lock(core.inner.lock());

            // 1) Deleted after callback started → exit
            if inner.state == TimerState::Deleted { return; }

            // 2) A command changed state during callback → save new param, don't auto-reload
            let command_changed_state = inner.generation != fired_generation;

            match callback_result {
                Ok(Ok(new_param)) => {
                    inner.param = Some(new_param);
                }
                _ => {
                    // Err or panic → stop, no auto-reload
                    inner.state = TimerState::Stopped;
                    inner.deadline = None;
                    bump_generation(&mut inner.generation);
                    drop(inner);
                    core.condvar.notify_all();
                    continue;
                }
            }

            if command_changed_state {
                // A command already set the new state — don't overwrite
                drop(inner);
                core.condvar.notify_all();
                continue;
            }

            // 3) No command during callback: auto-reload if periodic.
            //    Internal state transitions do NOT bump generation —
            //    the worker checks generation only for external commands.
            if auto_reload {
                inner.state = TimerState::Armed;
                if let Some(dl) = deadline_from_period(inner.period) {
                    inner.deadline = Some(dl);
                } else {
                    inner.state = TimerState::Stopped;
                }
                drop(inner);
                core.condvar.notify_all();
                continue;
            } else {
                inner.state = TimerState::Stopped;
                inner.deadline = None;
                drop(inner);
                core.condvar.notify_all();
                continue;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Timer — public handle
// ---------------------------------------------------------------------------

pub struct Timer {
    core: Arc<TimerCore>,
}

unsafe impl Send for Timer {}
unsafe impl Sync for Timer {}

impl Deref for Timer {
    type Target = TimerHandle;
    fn deref(&self) -> &Self::Target { &self.core.handle }
}

impl Timer {
    pub fn new<F>(
        name: &str,
        period_in_ticks: TickType,
        auto_reload: bool,
        param: Option<TimerParam>,
        callback: F,
    ) -> Result<Self>
    where
        F: Fn(Box<dyn TimerFn>, Option<TimerParam>) -> Result<TimerParam>,
        F: Send + Sync + Clone + 'static,
    {
        if period_in_ticks == 0 {
            return Err(Error::InvalidTimerPeriod);
        }

        let id = NEXT_TIMER_HANDLE.load(AtomicOrdering::Relaxed);
        let handle = next_timer_handle();

        let core = Arc::new(TimerCore {
            id,
            handle,
            inner: StdMutex::new(TimerInner {
                name: name.to_string(),
                period: period_in_ticks,
                auto_reload,
                callback: Arc::new(callback),
                param,
                state: TimerState::Stopped,
                deadline: None,
                generation: 0,
            }),
            condvar: Condvar::new(),
            public_handles: AtomicUsize::new(1),
            worker: StdMutex::new(None),
        });

        let worker_core = Arc::clone(&core);
        let worker = ThreadBuilder::new()
            .name(format!("osal-timer-{}", id))
            .spawn(move || worker_loop(worker_core))
            .map_err(|_| Error::TimerWorkerCreationFailed)?;

        *recover_lock(core.worker.lock()) = Some(worker);

        Ok(Self { core })
    }

    #[inline]
    pub fn new_with_to_tick<F>(
        name: &str,
        period: impl ToTick,
        auto_reload: bool,
        param: Option<TimerParam>,
        callback: F,
    ) -> Result<Self>
    where
        F: Fn(Box<dyn TimerFn>, Option<TimerParam>) -> Result<TimerParam>,
        F: Send + Sync + Clone + 'static,
    {
        Self::new(name, period.to_ticks(), auto_reload, param, callback)
    }

    #[inline] pub fn start_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.start(t.to_ticks()) }
    #[inline] pub fn stop_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.stop(t.to_ticks()) }
    #[inline] pub fn reset_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.reset(t.to_ticks()) }
    #[inline]
    pub fn change_period_with_to_tick(&self, p: impl ToTick, w: impl ToTick) -> OsalRsBool {
        self.change_period(p.to_ticks(), w.to_ticks())
    }
    #[inline] pub fn delete_with_to_tick(&mut self, t: impl ToTick) -> OsalRsBool { self.delete(t.to_ticks()) }

    /// Close the timer (wake worker, mark deleted, join).
    /// Used internally by delete and drop.
    fn close(&self) {
        shutdown(&self.core);
    }
}

// ---------------------------------------------------------------------------
// TimerFn
// ---------------------------------------------------------------------------

impl TimerFn for Timer {
    fn start(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        let mut inner = recover_lock(self.core.inner.lock());
        if inner.state == TimerState::Deleted { return OsalRsBool::False; }
        let period = inner.period;
        command_arm(&mut inner, period);
        drop(inner);
        self.core.condvar.notify_all();
        OsalRsBool::True
    }

    fn stop(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        let mut inner = recover_lock(self.core.inner.lock());
        if inner.state == TimerState::Deleted { return OsalRsBool::False; }
        command_stop(&mut inner);
        drop(inner);
        self.core.condvar.notify_all();
        OsalRsBool::True
    }

    fn reset(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        // Atomic: arm with fresh deadline in a single lock acquisition.
        let mut inner = recover_lock(self.core.inner.lock());
        if inner.state == TimerState::Deleted { return OsalRsBool::False; }
        let period = inner.period;
        command_arm(&mut inner, period);
        drop(inner);
        self.core.condvar.notify_all();
        OsalRsBool::True
    }

    fn change_period(&self, new_period: TickType, _ticks_to_wait: TickType) -> OsalRsBool {
        if new_period == 0 { return OsalRsBool::False; }
        let mut inner = recover_lock(self.core.inner.lock());
        if inner.state == TimerState::Deleted { return OsalRsBool::False; }
        inner.period = new_period;
        // If the timer is already Armed (or Executing), restart with the new
        // period.  If Stopped, only store the period — the timer stays
        // Stopped (matching the trait contract: "if stopped, new period takes
        // effect when started").
        let was_stopped = inner.state == TimerState::Stopped;
        if !was_stopped {
            command_arm(&mut inner, new_period);
        }
        drop(inner);
        if !was_stopped {
            self.core.condvar.notify_all();
        }
        OsalRsBool::True
    }

    fn delete(&mut self, _ticks_to_wait: TickType) -> OsalRsBool {
        let was_deleted = {
            let inner = recover_lock(self.core.inner.lock());
            inner.state == TimerState::Deleted
        };
        if was_deleted { return OsalRsBool::False; }
        self.close();
        OsalRsBool::True
    }
}

// ---------------------------------------------------------------------------
// Clone / Drop
// ---------------------------------------------------------------------------

impl Clone for Timer {
    fn clone(&self) -> Self {
        self.core.public_handles.fetch_add(1, AtomicOrdering::Relaxed);
        Self { core: Arc::clone(&self.core) }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let prev = self.core.public_handles.fetch_sub(1, AtomicOrdering::AcqRel);
        if prev == 1 {
            // Last public handle — shut down
            shutdown(&self.core);
        }
    }
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Debug for Timer {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.core.inner.try_lock() {
            Ok(inner) => f
                .debug_struct("Timer")
                .field("id", &self.core.id)
                .field("name", &inner.name)
                .field("period", &inner.period)
                .field("state", &inner.state)
                .finish(),
            Err(_) => f.debug_struct("Timer").finish_non_exhaustive(),
        }
    }
}

impl Display for Timer {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.core.inner.try_lock() {
            Ok(inner) => write!(f, "Timer {{ id: {}, name: {} }}", self.core.id, inner.name),
            Err(_) => write!(f, "Timer {{ <locked> }}"),
        }
    }
}