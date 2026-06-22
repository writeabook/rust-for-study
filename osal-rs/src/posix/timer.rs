//! POSIX timer — single Timer Service Thread + deadline heap + lazy invalidation.
//!
//! # Architecture (FreeRTOS Timer Service Task pattern)
//!
//! - One global daemon thread for ALL timers.
//! - `PosixMutex` + `PosixCondvar` (CLOCK_MONOTONIC) + `BTreeMap` + `BinaryHeap`.
//! - Generation counters for lazy invalidation of stale heap entries.
//! - Callbacks execute outside the lock; commands during callback go through
//!   `CallbackCommand` and are applied after the callback returns.
//! - Fixed-period auto-reload with catch-up prevention.
//!
//! # Migration status (std → no_std)
//!
//! TODO(posix-no-std): The Timer Service Thread currently uses `std::thread`,
//! `OnceLock`, and `catch_unwind` as transitional helpers.  The final POSIX
//! core should migrate to `libc::pthread_create`, `pthread_once_t`, and
//! `panic=abort`, with collections from `alloc`.

use core::cell::UnsafeCell;
use core::cmp::Ordering;
use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BinaryHeap};
use alloc::sync::Arc;

use std::sync::OnceLock;
use std::thread::{Builder as ThreadBuilder, JoinHandle};

use libc::PTHREAD_MUTEX_ERRORCHECK;

use crate::traits::{TimerFn, TimerFnPtr, TimerParam, ToTick};
use super::sys::clock;
use super::sys::condvar::PosixCondvar;
use super::sys::mutex::PosixMutex;
use super::types::{TickType, TimerHandle};
use crate::utils::{Error, OsalRsBool, Result};

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerState {
    Stopped,
    Running { deadline_ns: u64 },
    CallbackRunning { deadline_ns: u64, command: CallbackCommand },
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallbackCommand { None, Stop, Reset, Delete }

// ---------------------------------------------------------------------------
// Inner types
// ---------------------------------------------------------------------------

struct TimerRecord {
    id: u64,
    period_ms: u64,
    auto_reload: bool,
    state: TimerState,
    generation: u64,
    callback: Arc<TimerFnPtr>,
    param: Option<TimerParam>,
}

struct TimerHeapEntry {
    deadline_ns: u64,
    timer_id: u64,
    generation: u64,
}

impl Eq for TimerHeapEntry {}
impl PartialEq for TimerHeapEntry {
    fn eq(&self, other: &Self) -> bool { self.deadline_ns == other.deadline_ns }
}
impl PartialOrd for TimerHeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for TimerHeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.deadline_ns.cmp(&self.deadline_ns) // min-heap
    }
}

// ---------------------------------------------------------------------------
// Global TimerManager
// ---------------------------------------------------------------------------

struct TimerManager {
    mtx: PosixMutex,
    cond: PosixCondvar,
    timers: UnsafeCell<BTreeMap<u64, TimerRecord>>,
    heap: UnsafeCell<BinaryHeap<TimerHeapEntry>>,
    next_id: UnsafeCell<u64>,
}

unsafe impl Send for TimerManager {}
unsafe impl Sync for TimerManager {}

static MGR: OnceLock<TimerManager> = OnceLock::new();
static WORKER: OnceLock<JoinHandle<()>> = OnceLock::new();

fn mgr() -> *const TimerManager {
    MGR.get_or_init(|| {
        let mgr = TimerManager {
            mtx: PosixMutex::new(PTHREAD_MUTEX_ERRORCHECK).expect("TimerManager: mutex"),
            cond: PosixCondvar::new().expect("TimerManager: cond"),
            timers: UnsafeCell::new(BTreeMap::new()),
            heap: UnsafeCell::new(BinaryHeap::new()),
            next_id: UnsafeCell::new(1),
        };
        WORKER.get_or_init(|| {
            ThreadBuilder::new().name("osal-timer-svc".into())
                .spawn(|| worker_loop())
                .expect("TimerManager: spawn")
        });
        mgr
    }) as *const TimerManager
}

// Raw pointer accessors — pthread mutex provides safety
macro_rules! field {
    ($p:expr, $f:ident) => { unsafe { &mut *(*$p).$f.get() } }
}

// ---------------------------------------------------------------------------
// Worker loop — simplified: collect ALL expired, fire OUTSIDE lock, repeat
// ---------------------------------------------------------------------------

fn worker_loop() {
    let p = mgr();
    loop {
        let _ = unsafe { (*p).mtx.lock() };

        // ---- Step 1: collect all valid expired entries ----
        let mut expired: Vec<(u64, u64, Arc<TimerFnPtr>, Option<TimerParam>, bool, u64, u64)> = Vec::new();
        let now = clock::now_ns();

        loop {
            // Validate top
            let top = match field!(p, heap).peek() {
                Some(t) => t,
                None => break,
            };
            let record = match field!(p, timers).get(&top.timer_id) {
                Some(r) => r,
                None => { field!(p, heap).pop(); continue; }
            };
            let ok = record.generation == top.generation
                && matches!(record.state, TimerState::Running { deadline_ns } if deadline_ns == top.deadline_ns);
            if !ok { field!(p, heap).pop(); continue; }

            if top.deadline_ns > now { break; } // not yet expired

            // Expired — pop and transition
            field!(p, heap).pop();
            let r = field!(p, timers).get_mut(&top.timer_id).unwrap();
            let dl = match r.state { TimerState::Running { deadline_ns } => deadline_ns, _ => unreachable!() };
            r.state = TimerState::CallbackRunning { deadline_ns: dl, command: CallbackCommand::None };
            expired.push((top.timer_id, dl, Arc::clone(&r.callback), r.param.clone(), r.auto_reload, r.period_ms, r.id));
        }

        // ---- Step 2: if nothing expired, wait ----
        if expired.is_empty() {
            match field!(p, heap).peek() {
                Some(top) => {
                    let abs = clock::ns_to_timespec(top.deadline_ns);
                    unsafe { (*p).cond.timedwait(&(*p).mtx, &abs) };
                }
                None => {
                    unsafe { (*p).cond.wait(&(*p).mtx) };
                }
            }
            unsafe { (*p).mtx.unlock() };
            continue;
        }

        unsafe { (*p).mtx.unlock() };

        // ---- Step 3: execute callbacks OUTSIDE the lock ----
        for &(_, _, ref cb, ref param, _, _, id) in &expired {
            // id=0: callback handle — Drop won't call stop() on this
            let t: Box<dyn TimerFn> = Box::new(Timer { id: 0, handle: id as TimerHandle });
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { let _ = (cb)(t, param.clone()); }));
        }

        // ---- Step 4: re-lock, apply post-callback state ----
        let _ = unsafe { (*p).mtx.lock() };
        for (timer_id, deadline_ns, _cb, _param, auto_reload, period_ms, id) in &expired {
            let Some(r) = field!(p, timers).get_mut(id) else { continue };
            if let TimerState::CallbackRunning { command, .. } = r.state {
                match command {
                    CallbackCommand::Delete => { r.state = TimerState::Deleted; field!(p, timers).remove(id); }
                    CallbackCommand::Stop  => { r.state = TimerState::Stopped; }
                    CallbackCommand::Reset => {
                        let dl = clock::now_ns() + clock::ms_to_ns(*period_ms);
                        r.generation = r.generation.wrapping_add(1);
                        r.state = TimerState::Running { deadline_ns: dl };
                        field!(p, heap).push(TimerHeapEntry { deadline_ns: dl, timer_id: *timer_id, generation: r.generation });
                    }
                    CallbackCommand::None => {
                        if *auto_reload && *period_ms > 0 {
                            let mut next = *deadline_ns + clock::ms_to_ns(*period_ms);
                            let now2 = clock::now_ns();
                            while next <= now2 { next += clock::ms_to_ns(*period_ms); }
                            r.generation = r.generation.wrapping_add(1);
                            r.state = TimerState::Running { deadline_ns: next };
                            field!(p, heap).push(TimerHeapEntry { deadline_ns: next, timer_id: *timer_id, generation: r.generation });
                        } else {
                            r.state = TimerState::Stopped;
                        }
                    }
                }
            }
        }
        unsafe { (*p).mtx.unlock() };
    }
}

// ---------------------------------------------------------------------------
// Timer — public handle
// ---------------------------------------------------------------------------

pub struct Timer {
    id: u64,
    handle: TimerHandle,
}

unsafe impl Send for Timer {}
unsafe impl Sync for Timer {}

impl Timer {
    pub fn new<F>(
        _name: &str, period_ticks: TickType, auto_reload: bool,
        param: Option<TimerParam>, callback: F,
    ) -> Result<Self>
    where F: Fn(Box<dyn TimerFn>, Option<TimerParam>) -> Result<TimerParam> + Send + Sync + Clone + 'static
    {
        if period_ticks == 0 { return Err(Error::InvalidTimerPeriod); }
        let p = mgr();
        let _ = unsafe { (*p).mtx.lock() };
        let id = *field!(p, next_id);
        *field!(p, next_id) = id.checked_add(1).expect("Timer id overflow");
        field!(p, timers).insert(id, TimerRecord {
            id, period_ms: period_ticks as u64,
            auto_reload, state: TimerState::Stopped, generation: 0,
            callback: Arc::new(callback), param,
        });
        unsafe { (*p).mtx.unlock() };
        Ok(Self { id, handle: id as TimerHandle })
    }

    #[inline] pub fn new_with_to_tick<F>(n: &str, pe: impl ToTick, a: bool, pa: Option<TimerParam>, cb: F) -> Result<Self>
    where F: Fn(Box<dyn TimerFn>, Option<TimerParam>) -> Result<TimerParam> + Send + Sync + Clone + 'static
    { Self::new(n, pe.to_ticks(), a, pa, cb) }

    #[inline] pub fn start_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.start(t.to_ticks()) }
    #[inline] pub fn stop_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.stop(t.to_ticks()) }
    #[inline] pub fn reset_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.reset(t.to_ticks()) }
    #[inline] pub fn change_period_with_to_tick(&self, pe: impl ToTick, w: impl ToTick) -> OsalRsBool { self.change_period(pe.to_ticks(), w.to_ticks()) }
    #[inline] pub fn delete_with_to_tick(&mut self, t: impl ToTick) -> OsalRsBool { self.delete(t.to_ticks()) }
}

// Helper: bump generation, arm timer, push heap entry
unsafe fn arm(p: *const TimerManager, id: u64, period_ms: u64) -> u64 {
    let dl = clock::now_ns() + clock::ms_to_ns(period_ms.max(1));
    let r = field!(p, timers).get_mut(&id).unwrap();
    r.generation = r.generation.wrapping_add(1);
    r.state = TimerState::Running { deadline_ns: dl };
    field!(p, heap).push(TimerHeapEntry { deadline_ns: dl, timer_id: id, generation: r.generation });
    dl
}

impl TimerFn for Timer {
    fn start(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        let p = mgr(); let _ = unsafe { (*p).mtx.lock() };
        let Some(r) = field!(p, timers).get_mut(&self.id) else { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; };
        if r.state == TimerState::Deleted { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; }
        let _ = unsafe { arm(p, self.id, r.period_ms) };
        unsafe { (*p).cond.signal() }; unsafe { (*p).mtx.unlock() };
        OsalRsBool::True
    }

    fn stop(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        let p = mgr(); let _ = unsafe { (*p).mtx.lock() };
        let Some(r) = field!(p, timers).get_mut(&self.id) else { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; };
        if r.state == TimerState::Deleted { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; }
        match r.state {
            TimerState::CallbackRunning { ref mut command, .. } => *command = CallbackCommand::Stop,
            _ => { r.state = TimerState::Stopped; r.generation = r.generation.wrapping_add(1); }
        }
        unsafe { (*p).cond.signal() }; unsafe { (*p).mtx.unlock() };
        OsalRsBool::True
    }

    fn reset(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        let p = mgr(); let _ = unsafe { (*p).mtx.lock() };
        let Some(r) = field!(p, timers).get_mut(&self.id) else { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; };
        if r.state == TimerState::Deleted { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; }
        let period = r.period_ms;
        match r.state {
            TimerState::CallbackRunning { ref mut command, .. } => *command = CallbackCommand::Reset,
            _ => { let _ = unsafe { arm(p, self.id, period) }; }
        }
        unsafe { (*p).cond.signal() }; unsafe { (*p).mtx.unlock() };
        OsalRsBool::True
    }

    fn change_period(&self, new_period: TickType, _ticks_to_wait: TickType) -> OsalRsBool {
        if new_period == 0 { return OsalRsBool::False; }
        let p = mgr(); let _ = unsafe { (*p).mtx.lock() };
        let Some(r) = field!(p, timers).get_mut(&self.id) else { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; };
        if r.state == TimerState::Deleted { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; }
        r.period_ms = new_period as u64;
        if let TimerState::Running { .. } = r.state {
            let _ = unsafe { arm(p, self.id, new_period as u64) };
        }
        unsafe { (*p).cond.signal() }; unsafe { (*p).mtx.unlock() };
        OsalRsBool::True
    }

    fn delete(&mut self, _ticks_to_wait: TickType) -> OsalRsBool {
        let p = mgr(); let _ = unsafe { (*p).mtx.lock() };
        let Some(r) = field!(p, timers).get_mut(&self.id) else { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; };
        if r.state == TimerState::Deleted { unsafe { (*p).mtx.unlock() }; return OsalRsBool::False; }
        r.generation = r.generation.wrapping_add(1);
        if let TimerState::CallbackRunning { ref mut command, .. } = r.state {
            *command = CallbackCommand::Delete;
        } else {
            r.state = TimerState::Deleted;
            field!(p, timers).remove(&self.id);
        }
        unsafe { (*p).cond.signal() }; unsafe { (*p).mtx.unlock() };
        OsalRsBool::True
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        // id == 0 indicates a callback-box handle — do NOT call delete()
        if self.id > 0 {
            self.delete(0);
        }
    }
}
impl Deref for Timer { type Target = TimerHandle; fn deref(&self) -> &Self::Target { &self.handle } }
impl Debug for Timer {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Timer").field("id", &self.id).finish()
    }
}
impl Display for Timer {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Timer {{ id: {} }}", self.id)
    }
}
