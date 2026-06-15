//! Software timer support for the Linux backend.
//!
//! # Design
//!
//! Each `Timer` has a dedicated worker `std::thread`.  The worker blocks
//! on a `Condvar` until `start()` is called, then sleeps for the period
//! (via `std::thread::sleep` with periodic cancellation checks), fires
//! the callback, and repeats if `auto_reload` is true.
//!
//! # Limitations
//!
//! - One OS thread per timer.
//! - `ticks_to_wait` parameter is ignored (no command queue).

use core::fmt::{Debug, Display, Formatter};
use core::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;

use std::sync::{Condvar, Mutex as StdMutex};
use std::thread::{Builder as ThreadBuilder, JoinHandle};
use std::time::{Duration, Instant};

use crate::traits::{TimerFn, TimerFnPtr, TimerParam, ToTick};
use crate::utils::{OsalRsBool, Result};
use super::types::TickType;

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

// ---------------------------------------------------------------------------
// TimerInner
// ---------------------------------------------------------------------------

struct TimerInner {
    name: String,
    period: TickType,
    auto_reload: bool,
    callback: Option<Arc<TimerFnPtr>>,
    param: Option<TimerParam>,
    running: bool,
    deleted: bool,
    /// `stop()` / `delete()` set this flag; the worker polls it every 5 ms.
    cancelled: bool,
    /// Handle to the worker thread (Some after first `start`).
    worker: Option<JoinHandle<()>>,
}

// ---------------------------------------------------------------------------
// Timer
// ---------------------------------------------------------------------------

pub struct Timer {
    id: usize,
    inner: Arc<StdMutex<TimerInner>>,
    /// Woken by `start()` / `stop()` / `delete()` to unblock the worker.
    condvar: Arc<Condvar>,
}

unsafe impl Send for Timer {}
unsafe impl Sync for Timer {}

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
        Ok(Self {
            id: NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed),
            inner: Arc::new(StdMutex::new(TimerInner {
                name: name.to_string(),
                period: period_in_ticks,
                auto_reload,
                callback: Some(Arc::new(callback)),
                param,
                running: false,
                deleted: false,
                cancelled: false,
                worker: None,
            })),
            condvar: Arc::new(Condvar::new()),
        })
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

    #[inline]
    pub fn start_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.start(t.to_ticks()) }
    #[inline]
    pub fn stop_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.stop(t.to_ticks()) }
    #[inline]
    pub fn reset_with_to_tick(&self, t: impl ToTick) -> OsalRsBool { self.reset(t.to_ticks()) }
    #[inline]
    pub fn change_period_with_to_tick(&self, p: impl ToTick, w: impl ToTick) -> OsalRsBool {
        self.change_period(p.to_ticks(), w.to_ticks())
    }
    #[inline]
    pub fn delete_with_to_tick(&mut self, t: impl ToTick) -> OsalRsBool { self.delete(t.to_ticks()) }

    // -- worker -----------------------------------------------------------

    fn spawn_worker(inner: Arc<StdMutex<TimerInner>>, cv: Arc<Condvar>, id: usize) {
        let _ = ThreadBuilder::new()
            .name(format!("osal-timer-{}", id))
            .spawn(move || worker_loop(inner, cv, id));
    }
}

/// Block until cancelled or deadline reached, polling every 5 ms.
fn sleep_or_cancel(inner: &StdMutex<TimerInner>, deadline: Instant) -> bool {
    loop {
        if std::time::Instant::now() >= deadline {
            return false; // deadline reached
        }
        if inner.lock().unwrap().cancelled {
            return true; // cancelled
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn worker_loop(inner: Arc<StdMutex<TimerInner>>, cv: Arc<Condvar>, timer_id: usize) {
    loop {
        // Wait for `start()`.
        {
            let mut g = inner.lock().unwrap();
            while !g.running && !g.deleted && !g.cancelled {
                g = cv.wait(g).unwrap();
            }
            if g.deleted || g.cancelled {
                return;
            }
        }

        loop {
            let (period, auto_reload, callback, param) = {
                let g = inner.lock().unwrap();
                (g.period, g.auto_reload, g.callback.clone(), g.param.clone())
            };

            let deadline = Instant::now() + Duration::from_millis(period as u64);
            let was_cancelled = sleep_or_cancel(&inner, deadline);
            if was_cancelled {
                // Worker has been cancelled while sleeping.
                // Check whether start() was called again since.
                if !inner.lock().unwrap().cancelled {
                    // cancelled was cleared by a new start() — restart sleep.
                    continue;
                }
                return;
            }

            // Fire callback.
            if let Some(ref cb) = callback {
                let boxed: Box<dyn TimerFn> = Box::new(Timer {
                    id: timer_id,
                    inner: Arc::clone(&inner),
                    condvar: Arc::clone(&cv),
                });
                let _ = cb(boxed, param);
            }

            if !auto_reload {
                inner.lock().unwrap().running = false;
                break; // return to outer "wait for start" loop
            }

            // Check if cancelled between callback and next iteration.
            if inner.lock().unwrap().cancelled {
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TimerFn
// ---------------------------------------------------------------------------

impl TimerFn for Timer {
    fn start(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        let mut g = self.inner.lock().unwrap();
        if g.deleted {
            return OsalRsBool::False;
        }
        g.running = true;
        g.cancelled = false;

        if g.worker.is_none() {
            let inner = Arc::clone(&self.inner);
            let cv = Arc::clone(&self.condvar);
            let id = self.id;
            drop(g);
            Self::spawn_worker(inner, cv, id);
        } else {
            self.condvar.notify_all();
        }
        OsalRsBool::True
    }

    fn stop(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        let mut g = self.inner.lock().unwrap();
        if g.deleted {
            return OsalRsBool::False;
        }
        g.running = false;
        g.cancelled = true;
        self.condvar.notify_all();
        OsalRsBool::True
    }

    fn reset(&self, _ticks_to_wait: TickType) -> OsalRsBool {
        self.stop(0);
        self.start(0)
    }

    fn change_period(&self, new_period_in_ticks: TickType, _ticks_to_wait: TickType) -> OsalRsBool {
        let mut g = self.inner.lock().unwrap();
        if g.deleted {
            return OsalRsBool::False;
        }
        g.period = new_period_in_ticks;
        self.condvar.notify_all();
        OsalRsBool::True
    }

    fn delete(&mut self, _ticks_to_wait: TickType) -> OsalRsBool {
        let mut g = self.inner.lock().unwrap();
        g.deleted = true;
        g.running = false;
        g.cancelled = true;
        self.condvar.notify_all();
        OsalRsBool::True
    }
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Clone for Timer {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            inner: Arc::clone(&self.inner),
            condvar: Arc::clone(&self.condvar),
        }
    }
}

impl Debug for Timer {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(g) => f
                .debug_struct("Timer")
                .field("id", &self.id)
                .field("name", &g.name)
                .field("period", &g.period)
                .field("running", &g.running)
                .finish(),
            Err(_) => f.debug_struct("Timer").finish_non_exhaustive(),
        }
    }
}

impl Display for Timer {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Ok(g) => write!(f, "Timer {{ id: {}, name: {} }}", self.id, g.name),
            Err(_) => write!(f, "Timer {{ <locked> }}"),
        }
    }
}