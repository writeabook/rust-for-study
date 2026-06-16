//! Linux-specific timer tests.
//!
//! These tests verify the worker lifecycle, state machine, generation
//! mechanism, callback parameter write-back, panic/error recovery,
//! clone/drop lifecycle, and handle uniqueness of the Linux Timer
//! backend.  They are **not** part of the cross-backend common test suite.

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use alloc::sync::Arc;
use std::sync::{mpsc, Barrier};

use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};
use osal_rs::log_info;

const TAG: &str = "LinuxTimerTests";

fn ms(ms: u64) -> Duration { Duration::from_millis(ms) }

fn ret(p: Option<Arc<dyn core::any::Any + Send + Sync>>) -> Result<Arc<dyn core::any::Any + Send + Sync>> {
    Ok(p.unwrap_or_else(|| Arc::new(())))
}

// 1
pub fn test_timer_one_shot_exact() -> Result<()> {
    log_info!(TAG, "test_timer_one_shot_exact");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("os", 30, false, None, move |_t, p| { c.fetch_add(1, Ordering::SeqCst); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(150));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_one_shot_exact PASSED");
    Ok(())
}

// 2
pub fn test_timer_periodic_auto_reload() -> Result<()> {
    log_info!(TAG, "test_timer_periodic_auto_reload");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("per", 20, true, None, move |_t, p| { c.fetch_add(1, Ordering::SeqCst); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(80));
    timer.stop(0);
    let after_stop = counter.load(Ordering::SeqCst);
    assert!(after_stop >= 3, "expected >= 3, got {}", after_stop);
    std::thread::sleep(ms(80));
    assert_eq!(counter.load(Ordering::SeqCst), after_stop);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_periodic_auto_reload PASSED");
    Ok(())
}

// 3
pub fn test_timer_stop_before_expiry() -> Result<()> {
    log_info!(TAG, "test_timer_stop_before_expiry");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("sbe", 100, false, None, move |_t, p| { c.fetch_add(1, Ordering::SeqCst); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(30));
    timer.stop(0);
    std::thread::sleep(ms(150));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_stop_before_expiry PASSED");
    Ok(())
}

// 4
pub fn test_timer_restart_after_stop() -> Result<()> {
    log_info!(TAG, "test_timer_restart_after_stop");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("ras", 30, false, None, move |_t, p| { c.fetch_add(1, Ordering::SeqCst); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(10));
    timer.stop(0);
    timer.start(0);
    std::thread::sleep(ms(60));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_restart_after_stop PASSED");
    Ok(())
}

// 5
pub fn test_timer_repeated_start() -> Result<()> {
    log_info!(TAG, "test_timer_repeated_start");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("rs", 50, false, None, move |_t, p| { c.fetch_add(1, Ordering::SeqCst); ret(p) })?;
    for _ in 0..20 { timer.start(0); }
    std::thread::sleep(ms(120));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_repeated_start PASSED");
    Ok(())
}

// 6
pub fn test_timer_reset_deadline() -> Result<()> {
    log_info!(TAG, "test_timer_reset_deadline");
    let (tx, rx) = mpsc::channel();
    let timer = Timer::new("rd", 100, false, None, move |_t, p| { let _ = tx.send(Instant::now()); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(70));
    timer.reset(0);
    let t0 = Instant::now();
    rx.recv_timeout(ms(200)).expect("timer did not fire after reset");
    assert!(t0.elapsed() >= ms(80), "reset deadline too short: {:?}", t0.elapsed());
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_reset_deadline PASSED");
    Ok(())
}

// 7
pub fn test_timer_change_period_shorten() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_shorten");
    let (tx, rx) = mpsc::channel();
    let timer = Timer::new("cps", 200, false, None, move |_t, p| { let _ = tx.send(Instant::now()); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(30));
    let t0 = Instant::now();
    timer.change_period(50, 0);
    rx.recv_timeout(ms(150)).expect("timer did not fire after shorten");
    assert!(t0.elapsed() < ms(100));
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_change_period_shorten PASSED");
    Ok(())
}

// 8
pub fn test_timer_change_period_extend() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_extend");
    let (tx, rx) = mpsc::channel();
    let timer = Timer::new("cpe", 50, false, None, move |_t, p| { let _ = tx.send(Instant::now()); ret(p) })?;
    let t0 = Instant::now();
    timer.start(0);
    std::thread::sleep(ms(20));
    timer.change_period(150, 0);
    rx.recv_timeout(ms(300)).expect("timer did not fire after extend");
    assert!(t0.elapsed() >= ms(140));
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_change_period_extend PASSED");
    Ok(())
}

// 9 - FIXED: change_period on Stopped must NOT arm the timer
pub fn test_timer_change_period_from_stopped() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_from_stopped");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("cpfs", 200, false, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;

    // change_period on a stopped timer must NOT arm it
    assert_eq!(timer.change_period(30, 0), OsalRsBool::True);
    std::thread::sleep(ms(100));
    assert_eq!(counter.load(Ordering::SeqCst), 0,
        "change_period on stopped timer must NOT start it");

    // start() must fire using the stored period
    timer.start(0);
    std::thread::sleep(ms(80));
    assert_eq!(counter.load(Ordering::SeqCst), 1,
        "start after change_period must fire");

    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_change_period_from_stopped PASSED");
    Ok(())
}

// 10
pub fn test_timer_delete_before_expiry() -> Result<()> {
    log_info!(TAG, "test_timer_delete_before_expiry");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("dbe", 100, false, None, move |_t, p| { c.fetch_add(1, Ordering::SeqCst); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(20));
    let mut timer = timer; timer.delete(0);
    std::thread::sleep(ms(150));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    log_info!(TAG, "test_timer_delete_before_expiry PASSED");
    Ok(())
}

// 11
pub fn test_timer_commands_fail_after_delete() -> Result<()> {
    log_info!(TAG, "test_timer_commands_fail_after_delete");
    let mut timer = Timer::new("cfa", 30, false, None, |_t, p| ret(p))?;
    timer.delete(0);
    assert_eq!(timer.start(0), OsalRsBool::False);
    assert_eq!(timer.stop(0), OsalRsBool::False);
    assert_eq!(timer.reset(0), OsalRsBool::False);
    assert_eq!(timer.change_period(50, 0), OsalRsBool::False);
    assert_eq!(timer.delete(0), OsalRsBool::False);
    log_info!(TAG, "test_timer_commands_fail_after_delete PASSED");
    Ok(())
}

// 12
pub fn test_timer_drop_stops_worker() -> Result<()> {
    log_info!(TAG, "test_timer_drop_stops_worker");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("dsw", 30, true, None, move |_t, p| { c.fetch_add(1, Ordering::SeqCst); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(40));
    assert!(counter.load(Ordering::SeqCst) >= 1);
    drop(timer);
    let before = counter.load(Ordering::SeqCst);
    std::thread::sleep(ms(120));
    assert_eq!(counter.load(Ordering::SeqCst), before);
    log_info!(TAG, "test_timer_drop_stops_worker PASSED");
    Ok(())
}

// 13 - FIXED: now verifies param write-back via AtomicU32 tracking
pub fn test_timer_callback_param_update() -> Result<()> {
    log_info!(TAG, "test_timer_callback_param_update");
    let max_val = Arc::new(AtomicU32::new(0));
    let mv = Arc::clone(&max_val);

    let timer = Timer::new("cpu", 20, true, Some(Arc::new(0u32)), move |t: Box<dyn TimerFn>, p| {
        let val = p.and_then(|x| x.downcast_ref::<u32>().copied()).unwrap_or(0);
        mv.store(val, Ordering::SeqCst);
        if val >= 3 { t.stop(0); }
        Ok(Arc::new(val + 1))
    })?;

    timer.start(0);
    std::thread::sleep(ms(200));
    let observed = max_val.load(Ordering::SeqCst);
    assert!(observed >= 2, "param not propagated: max observed {}", observed);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_callback_param_update PASSED");
    Ok(())
}

// 14 - FIXED: must fire exactly once when stop is called in callback
pub fn test_timer_stop_inside_callback() -> Result<()> {
    log_info!(TAG, "test_timer_stop_inside_callback");
    let (tx, rx) = mpsc::channel();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    let timer = Timer::new("sic", 20, true, None, move |t: Box<dyn TimerFn>, p| {
        c.fetch_add(1, Ordering::SeqCst);
        t.stop(0);
        let _ = tx.send(());
        ret(p)
    })?;

    timer.start(0);
    // Wait for callback to fire and complete stop inside
    rx.recv_timeout(ms(200)).expect("callback did not fire");
    // Ample time — must NOT fire again
    std::thread::sleep(ms(120));
    assert_eq!(counter.load(Ordering::SeqCst), 1,
        "expected exactly 1 fire, got {}", counter.load(Ordering::SeqCst));
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_stop_inside_callback PASSED");
    Ok(())
}

// 15
pub fn test_timer_reset_inside_callback() -> Result<()> {
    log_info!(TAG, "test_timer_reset_inside_callback");
    let (tx, rx) = mpsc::channel();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("ric", 30, true, None, move |t: Box<dyn TimerFn>, p| {
        let n = c.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 1 { t.reset(0); let _ = tx.send(()); }
        if n == 2 { let _ = tx.send(()); }
        ret(p)
    })?;
    timer.start(0);
    rx.recv_timeout(ms(150)).expect("first fire");
    rx.recv_timeout(ms(150)).expect("second fire after reset");
    assert!(counter.load(Ordering::SeqCst) >= 2);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_reset_inside_callback PASSED");
    Ok(())
}

// 16
pub fn test_timer_change_period_inside_callback() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_inside_callback");
    let (tx, rx) = mpsc::channel();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("cpic", 30, true, None, move |t: Box<dyn TimerFn>, p| {
        let n = c.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 1 { t.change_period(50, 0); let _ = tx.send(()); }
        if n == 2 { let _ = tx.send(()); t.stop(0); }
        ret(p)
    })?;
    let t0 = Instant::now();
    timer.start(0);
    rx.recv_timeout(ms(80)).expect("first fire");
    rx.recv_timeout(ms(150)).expect("second fire after change");
    assert!(t0.elapsed() >= ms(60));
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_change_period_inside_callback PASSED");
    Ok(())
}

// 17
pub fn test_timer_callback_err_stops() -> Result<()> {
    log_info!(TAG, "test_timer_callback_err_stops");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("ces", 20, true, None, move |_t, _p| {
        c.fetch_add(1, Ordering::SeqCst);
        Err(osal_rs::utils::Error::Unhandled("test err"))
    })?;
    timer.start(0);
    std::thread::sleep(ms(80));
    assert!(counter.load(Ordering::SeqCst) <= 2, "should stop after error");
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_callback_err_stops PASSED");
    Ok(())
}

// 18
pub fn test_timer_callback_panic_caught() -> Result<()> {
    log_info!(TAG, "test_timer_callback_panic_caught");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("cpc", 30, false, None, move |_t, _p| {
        c.fetch_add(1, Ordering::SeqCst);
        panic!("intentional");
    })?;
    timer.start(0);
    std::thread::sleep(ms(100));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_callback_panic_caught PASSED");
    Ok(())
}

// 19
pub fn test_timer_period_zero_rejected() -> Result<()> {
    log_info!(TAG, "test_timer_period_zero_rejected");
    assert!(Timer::new("pz", 0, false, None, |_t, p| ret(p)).is_err());
    log_info!(TAG, "test_timer_period_zero_rejected PASSED");
    Ok(())
}

// 20
pub fn test_timer_unique_handles() -> Result<()> {
    log_info!(TAG, "test_timer_unique_handles");
    let mut t1 = Timer::new("h1", 100, false, None, |_t, p| ret(p))?;
    let mut t2 = Timer::new("h2", 100, false, None, |_t, p| ret(p))?;
    assert_ne!(*t1, *t2);
    t1.delete(0); t2.delete(0);
    log_info!(TAG, "test_timer_unique_handles PASSED");
    Ok(())
}

// 21
pub fn test_timer_clone_lifecycle() -> Result<()> {
    log_info!(TAG, "test_timer_clone_lifecycle");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("cl", 30, true, None, move |_t, p| { c.fetch_add(1, Ordering::SeqCst); ret(p) })?;
    let clone = timer.clone();
    clone.start(0);
    std::thread::sleep(ms(40));
    assert!(counter.load(Ordering::SeqCst) >= 1);
    drop(clone);
    std::thread::sleep(ms(50));
    assert!(counter.load(Ordering::SeqCst) >= 2);
    timer.stop(0);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_clone_lifecycle PASSED");
    Ok(())
}

// 22 - NEW: multi-thread concurrent command stress test
pub fn test_timer_concurrent_commands() -> Result<()> {
    log_info!(TAG, "test_timer_concurrent_commands");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Arc::new(Timer::new("cc", 50, true, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?);

    const THREADS: usize = 4;
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = vec![];

    for i in 0..THREADS {
        let t = Arc::clone(&timer);
        let b = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            b.wait();
            for _ in 0..50 {
                match i % 4 {
                    0 => { t.start(0); }
                    1 => { t.stop(0); }
                    2 => { t.reset(0); }
                    3 => { t.change_period(30 + (i as u32 % 5) * 10, 0); }
                    _ => {}
                }
                std::thread::yield_now();
            }
        }));
    }

    for h in handles { h.join().unwrap(); }

    let count = counter.load(Ordering::SeqCst);
    assert!(count < 500, "excessive callbacks: {}", count);

    let mut timer = Arc::try_unwrap(timer).unwrap_or_else(|a| (*a).clone());
    timer.delete(0);
    log_info!(TAG, "test_timer_concurrent_commands PASSED");
    Ok(())
}

// ============================================================================
// Run all
// ============================================================================

pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "========== Running Linux-Specific Timer Tests ==========");
    test_timer_one_shot_exact()?;
    test_timer_periodic_auto_reload()?;
    test_timer_stop_before_expiry()?;
    test_timer_restart_after_stop()?;
    test_timer_repeated_start()?;
    test_timer_reset_deadline()?;
    test_timer_change_period_shorten()?;
    test_timer_change_period_extend()?;
    test_timer_change_period_from_stopped()?;
    test_timer_delete_before_expiry()?;
    test_timer_commands_fail_after_delete()?;
    test_timer_drop_stops_worker()?;
    test_timer_callback_param_update()?;
    test_timer_stop_inside_callback()?;
    test_timer_reset_inside_callback()?;
    test_timer_change_period_inside_callback()?;
    test_timer_callback_err_stops()?;
    test_timer_callback_panic_caught()?;
    test_timer_period_zero_rejected()?;
    test_timer_unique_handles()?;
    test_timer_clone_lifecycle()?;
    test_timer_concurrent_commands()?;
    log_info!(TAG, "========== All Linux-Specific Timer Tests PASSED ==========");
    Ok(())
}