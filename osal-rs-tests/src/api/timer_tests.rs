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

extern crate alloc;

use alloc::sync::Arc;
use core::any::Any;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};
use osal_rs::{log_debug, log_info};

const TAG: &str = "TimerTests";

pub fn test_timer_creation() -> Result<()> {
    log_info!(TAG, "Starting test_timer_creation");
    let timer = Timer::new(
        "test_timer",
        Duration::from_millis(100).to_ticks(),
        false,
        None,
        |_timer, param| Ok(param.unwrap_or_else(|| Arc::new(()))),
    );

    assert!(timer.is_ok());
    log_info!(TAG, "test_timer_creation PASSED");
    Ok(())
}

pub fn test_timer_one_shot() -> Result<()> {
    log_info!(TAG, "Starting test_timer_one_shot");

    let done = Arc::new(Semaphore::new(1, 0)?);
    let count = Arc::new(AtomicU32::new(0));

    let done_cb = done.clone();
    let count_cb = count.clone();

    let timer = Timer::new(
        "oneshot_timer",
        Duration::from_millis(50).to_ticks(),
        false,
        None,
        move |_timer, param| {
            count_cb.fetch_add(1, Ordering::SeqCst);
            done_cb.signal();
            Ok(param.unwrap_or_else(|| Arc::new(())))
        },
    )?;

    let result = timer.start(Duration::from_millis(10).to_ticks());
    assert_eq!(result, OsalRsBool::True);

    // Poll-wait with a generous total budget.  Timer tests share a global
    // timer-manager singleton whose worker may be busy processing stale
    // entries from earlier tests; retrying with short timeouts is robust
    // against transient delays.
    let mut fired = false;
    for _ in 0..60 {
        if done.wait(Duration::from_millis(100).to_ticks()) == OsalRsBool::True {
            fired = true;
            break;
        }
    }
    assert!(fired, "timer callback did not fire within 6000 ms");

    let n = count.load(Ordering::SeqCst);
    log_debug!(TAG, "Timer fired {} times", n);
    assert!(n >= 1, "expected at least 1 fire, got {}", n);

    // Explicit cleanup: let the worker finish before Drop races with it.
    timer.stop(Duration::from_millis(50).to_ticks());
    core::mem::drop(timer);

    log_info!(TAG, "test_timer_one_shot PASSED");
    Ok(())
}

pub fn test_timer_auto_reload() -> Result<()> {
    log_info!(TAG, "Starting test_timer_auto_reload");

    let done = Arc::new(Semaphore::new(1, 0)?);
    let count = Arc::new(AtomicU32::new(0));

    let done_cb = done.clone();
    let count_cb = count.clone();

    let timer = Timer::new(
        "autoreload_timer",
        Duration::from_millis(50).to_ticks(),
        true,
        None,
        move |_timer, param| {
            let n = count_cb.fetch_add(1, Ordering::SeqCst) + 1;
            if n >= 2 {
                done_cb.signal();
            }
            Ok(param.unwrap_or_else(|| Arc::new(())))
        },
    )?;

    let result = timer.start(Duration::from_millis(10).to_ticks());
    assert_eq!(result, OsalRsBool::True);

    // Poll-wait for the callback to fire twice.  A single long wait can be
    // defeated by cross-test interference through the global timer-manager
    // singleton; retrying with short timeouts is more robust.
    let mut fired = false;
    for _ in 0..60 {
        if done.wait(Duration::from_millis(100).to_ticks()) == OsalRsBool::True {
            fired = true;
            break;
        }
    }
    let n = count.load(Ordering::SeqCst);
    assert!(
        fired,
        "auto-reload timer: semaphore not signaled, fired {} times in 6000 ms",
        n
    );
    assert!(n >= 2, "expected at least 2 fires, got {}", n);

    timer.stop(Duration::from_millis(50).to_ticks());
    // Give the worker a tick to finish processing this timer before the
    // next test creates its own — avoids cross-test interference through
    // the global timer-manager singleton.
    core::mem::drop(timer);
    System::delay(1);

    log_info!(TAG, "test_timer_auto_reload PASSED");
    Ok(())
}

pub fn test_timer_start_stop() -> Result<()> {
    log_info!(TAG, "Starting test_timer_start_stop");
    let timer = Timer::new(
        "startstop_timer",
        Duration::from_millis(100).to_ticks(),
        false,
        None,
        |_timer, param| Ok(param.unwrap_or_else(|| Arc::new(()))),
    )?;

    let start_result = timer.start(Duration::from_millis(10).to_ticks());
    log_debug!(TAG, "Timer started");
    assert_eq!(start_result, OsalRsBool::True);

    let stop_result = timer.stop(Duration::from_millis(10).to_ticks());
    log_debug!(TAG, "Timer stopped");
    assert_eq!(stop_result, OsalRsBool::True);
    log_info!(TAG, "test_timer_start_stop PASSED");
    Ok(())
}

pub fn test_timer_reset() -> Result<()> {
    log_info!(TAG, "Starting test_timer_reset");
    let timer = Timer::new(
        "reset_timer",
        Duration::from_millis(100).to_ticks(),
        false,
        None,
        |_timer, param| Ok(param.unwrap_or_else(|| Arc::new(()))),
    )?;

    timer.start(Duration::from_millis(10).to_ticks());

    let reset_result = timer.reset(Duration::from_millis(10).to_ticks());
    log_debug!(TAG, "Timer reset");
    assert_eq!(reset_result, OsalRsBool::True);

    timer.stop(Duration::from_millis(10).to_ticks());
    log_info!(TAG, "test_timer_reset PASSED");
    Ok(())
}

pub fn test_timer_change_period() -> Result<()> {
    log_info!(TAG, "Starting test_timer_change_period");
    let timer = Timer::new(
        "period_timer",
        Duration::from_millis(100).to_ticks(),
        false,
        None,
        |_timer, param| Ok(param.unwrap_or_else(|| Arc::new(()))),
    )?;

    timer.start(Duration::from_millis(10).to_ticks());

    log_debug!(TAG, "Changing period from 100ms to 200ms");
    let change_result = timer.change_period(
        Duration::from_millis(200).to_ticks(),
        Duration::from_millis(10).to_ticks(),
    );
    assert_eq!(change_result, OsalRsBool::True);

    timer.stop(Duration::from_millis(10).to_ticks());
    log_info!(TAG, "test_timer_change_period PASSED");
    Ok(())
}

pub fn test_timer_with_param() -> Result<()> {
    log_info!(TAG, "Starting test_timer_with_param");
    let test_value: u32 = 42;

    let done = Arc::new(Semaphore::new(1, 0)?);
    let received = Arc::new(AtomicU32::new(0));

    let done_cb = done.clone();
    let received_cb = received.clone();

    let param: Arc<dyn Any + Send + Sync> = Arc::new(test_value);

    let timer = Timer::new(
        "param_timer",
        Duration::from_millis(50).to_ticks(),
        false,
        Some(param),
        move |_timer, param| {
            if let Some(ref p) = param {
                if let Some(val) = p.downcast_ref::<u32>() {
                    received_cb.store(*val, Ordering::SeqCst);
                }
            }
            done_cb.signal();
            Ok(param.unwrap_or_else(|| Arc::new(())))
        },
    )?;

    assert_eq!(
        timer.start(Duration::from_millis(10).to_ticks()),
        OsalRsBool::True
    );

    let mut fired = false;
    for _ in 0..60 {
        if done.wait(Duration::from_millis(100).to_ticks()) == OsalRsBool::True {
            fired = true;
            break;
        }
    }
    assert!(fired, "param timer should fire within 6000 ms");

    assert_eq!(received.load(Ordering::SeqCst), 42);

    timer.stop(Duration::from_millis(50).to_ticks());
    core::mem::drop(timer);
    System::delay(1);

    log_info!(TAG, "test_timer_with_param PASSED");
    Ok(())
}

pub fn test_timer_delete() -> Result<()> {
    log_info!(TAG, "Starting test_timer_delete");
    let mut timer = Timer::new(
        "delete_timer",
        Duration::from_millis(100).to_ticks(),
        false,
        None,
        |_timer, param| Ok(param.unwrap_or_else(|| Arc::new(()))),
    )?;

    let delete_result = timer.delete(Duration::from_millis(10).to_ticks());
    assert_eq!(delete_result, OsalRsBool::True);
    log_info!(TAG, "test_timer_delete PASSED");
    Ok(())
}

pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "========== Running Timer Tests ==========");
    test_timer_creation()?;
    test_timer_one_shot()?;
    test_timer_auto_reload()?;
    test_timer_start_stop()?;
    test_timer_reset()?;
    test_timer_change_period()?;
    test_timer_with_param()?;
    test_timer_delete()?;
    log_info!(TAG, "========== All Timer Tests PASSED ==========");
    Ok(())
}
