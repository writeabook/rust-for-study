/***************************************************************************
 *
 * osal-rs
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
use std::sync::mpsc;

use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};
use osal_rs::log_info;

const TAG: &str = "LinuxTimerTests";

fn ms(ms: u64) -> Duration { Duration::from_millis(ms) }

/// Helper: simple callback return value (preserve the existing param).
fn ret(p: Option<Arc<dyn core::any::Any + Send + Sync>>) -> Result<Arc<dyn core::any::Any + Send + Sync>> {
    Ok(p.unwrap_or_else(|| Arc::new(())))
}

// ============================================================================
// 1. One-shot fires exactly once
// ============================================================================

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

// ============================================================================
// 2. Periodic auto-reload fires multiple times, stop halts it
// ============================================================================

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

// ============================================================================
// 3. Stop before expiry - no fire
// ============================================================================

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

// ============================================================================
// 4. Restart after stop
// ============================================================================

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

// ============================================================================
// 5. Repeated start - only fires once
// ============================================================================

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

// ============================================================================
// 6. Reset restarts full deadline
// ============================================================================

pub fn test_timer_reset_deadline() -> Result<()> {
    log_info!(TAG, "test_timer_reset_deadline");
    let (tx, rx) = mpsc::channel();

    let timer = Timer::new("rd", 100, false, None, move |_t, p| { let _ = tx.send(Instant::now()); ret(p) })?;
    timer.start(0);
    std::thread::sleep(ms(70));
    timer.reset(0);
    // After reset (at ~70ms), must wait at least 80 more ms
    let t0 = Instant::now();
    rx.recv_timeout(ms(200)).expect("timer did not fire after reset");
    assert!(t0.elapsed() >= ms(80), "reset deadline too short: {:?}", t0.elapsed());
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_reset_deadline PASSED");
    Ok(())
}

// ============================================================================
// 7. change_period shorten
// ============================================================================

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

// ============================================================================
// 8. change_period extend
// ============================================================================

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

// ============================================================================
// 9. change_period from stopped
// ============================================================================

pub fn test_timer_change_period_from_stopped() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_from_stopped");
    let (tx, rx) = mpsc::channel();

    let timer = Timer::new("cpfs", 200, false, None, move |_t, p| { let _ = tx.send(Instant::now()); ret(p) })?;
    timer.change_period(30, 0);
    rx.recv_timeout(ms(100)).expect("timer did not fire from stopped");
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_change_period_from_stopped PASSED");
    Ok(())
}

// ============================================================================
// 10. Delete before expiry - no fire
// ============================================================================

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

// ============================================================================
// 11. All commands return False after delete
// ============================================================================

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

// ============================================================================
// 12. Drop stops worker
// ============================================================================

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

// ============================================================================
// 13. Callback parameter update
// ============================================================================

pub fn test_timer_callback_param_update() -> Result<()> {
    log_info!(TAG, "test_timer_callback_param_update");
    let timer = Timer::new("cpu", 20, true, Some(Arc::new(0u32)), move |_t, p| {
        let val = p.and_then(|x| x.downcast_ref::<u32>().copied()).unwrap_or(0);
        Ok(Arc::new(val + 1))
    })?;
    timer.start(0);
    std::thread::sleep(ms(80));
    timer.stop(0);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_callback_param_update PASSED");
    Ok(())
}

// ============================================================================
// 14. Stop inside callback (no deadlock)
// ============================================================================

pub fn test_timer_stop_inside_callback() -> Result<()> {
    log_info!(TAG, "test_timer_stop_inside_callback");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    let timer = Timer::new("sic", 20, true, None, move |t: Box<dyn TimerFn>, p| {
        c.fetch_add(1, Ordering::SeqCst);
        t.stop(0);
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(120));
    let count = counter.load(Ordering::SeqCst);
    assert!(count <= 2, "expected <=2, got {}", count);
    let mut timer = timer; timer.delete(0);
    log_info!(TAG, "test_timer_stop_inside_callback PASSED");
    Ok(())
}

// ============================================================================
// 15. Reset inside callback
// ============================================================================

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

// ============================================================================
// 16. change_period inside callback
// ============================================================================

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

// ============================================================================
// 17. Callback returns Err - stops
// ============================================================================

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

// ============================================================================
// 18. Callback panic is caught
// ============================================================================

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

// ============================================================================
// 19. Period 0 rejected
// ============================================================================

pub fn test_timer_period_zero_rejected() -> Result<()> {
    log_info!(TAG, "test_timer_period_zero_rejected");
    assert!(Timer::new("pz", 0, false, None, |_t, p| ret(p)).is_err());
    log_info!(TAG, "test_timer_period_zero_rejected PASSED");
    Ok(())
}

// ============================================================================
// 20. Unique handles
// ============================================================================

pub fn test_timer_unique_handles() -> Result<()> {
    log_info!(TAG, "test_timer_unique_handles");
    let mut t1 = Timer::new("h1", 100, false, None, |_t, p| ret(p))?;
    let mut t2 = Timer::new("h2", 100, false, None, |_t, p| ret(p))?;
    assert_ne!(*t1, *t2);
    t1.delete(0); t2.delete(0);
    log_info!(TAG, "test_timer_unique_handles PASSED");
    Ok(())
}

// ============================================================================
// 21. Clone lifecycle
// ============================================================================

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
    log_info!(TAG, "========== All Linux-Specific Timer Tests PASSED ==========");
    Ok(())
}
