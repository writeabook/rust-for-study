/***************************************************************************
 *
 * osal-rs — Linux-specific thread tests
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

//! Linux-specific thread tests (registry, handles, state machine,
//! notifications, spawn lifecycle).

extern crate alloc;

use core::time::Duration;
use alloc::sync::Arc;

use osal_rs::os::*;
use osal_rs::os::types::TickType;
use osal_rs::utils::Result;
use osal_rs::log_info;

const TAG: &str = "LinuxThreadTests";

// ---------------------------------------------------------------------------
// Handles & Registry
// ---------------------------------------------------------------------------

pub fn test_thread_handles_unique() -> Result<()> {
    log_info!(TAG, "test_thread_handles_unique");
    let t1 = Thread::new("h1", 1024, 1);
    let t2 = Thread::new("h2", 1024, 1);
    assert_ne!(*t1, *t2, "handles must differ");
    t1.delete(); t2.delete();
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_handle_matches_metadata() -> Result<()> {
    log_info!(TAG, "test_thread_handle_matches_metadata");
    let mut t = Thread::new("meta", 2048, 3);
    let m = t.get_metadata();
    assert_eq!(*t, m.thread);
    let spawned = t.spawn(None, |_, p| Ok(p.unwrap_or_else(|| Arc::new(()))))?;
    let m2 = spawned.get_metadata();
    assert_eq!(*spawned, m2.thread);
    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_get_metadata_from_handle_real() -> Result<()> {
    log_info!(TAG, "test_thread_get_metadata_from_handle_real");
    let t = Thread::new("real", 1024, 5);
    let m = Thread::get_metadata_from_handle(*t);
    assert_eq!(m.name.as_str(), "real");
    assert_eq!(m.priority, 5);
    t.delete();
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_get_metadata_invalid_handle() -> Result<()> {
    log_info!(TAG, "test_thread_get_metadata_invalid_handle");
    let m = Thread::get_metadata_from_handle(0xDEAD as osal_rs::os::types::ThreadHandle);
    assert_eq!(m.state, ThreadState::Invalid);
    log_info!(TAG, "PASSED");
    Ok(())
}

// ---------------------------------------------------------------------------
// Spawn lifecycle
// ---------------------------------------------------------------------------

pub fn test_thread_spawn_twice_rejected() -> Result<()> {
    log_info!(TAG, "test_thread_spawn_twice_rejected");
    let mut t = Thread::new("twice", 1024, 1);
    t.spawn(None, |_, p| Ok(p.unwrap_or_else(|| Arc::new(()))))?;
    let r2 = t.spawn(None, |_, p| Ok(p.unwrap_or_else(|| Arc::new(()))));
    assert!(r2.is_err());
    t.delete();
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_fast_exit_not_ready() -> Result<()> {
    log_info!(TAG, "test_thread_fast_exit_not_ready");
    let mut t = Thread::new("fast", 1024, 1);
    let s = t.spawn(None, |_, p| Ok(p.unwrap_or_else(|| Arc::new(()))))?;
    s.join(core::ptr::null_mut())?;
    let m = s.get_metadata();
    assert_eq!(m.state, ThreadState::Deleted);
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_join_after_panic_sets_deleted() -> Result<()> {
    log_info!(TAG, "test_thread_join_after_panic_sets_deleted");
    let mut t = Thread::new("panic", 1024, 1);
    let s = t.spawn(None, |_, _p| { panic!("intentional"); })?;
    let r = s.join(core::ptr::null_mut());
    assert!(r.is_err());
    // After join, state should be Deleted
    let m = s.get_metadata();
    assert_eq!(m.state, ThreadState::Deleted);
    log_info!(TAG, "PASSED");
    Ok(())
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

pub fn test_thread_notify_max_delay() -> Result<()> {
    log_info!(TAG, "test_thread_notify_max_delay");
    let mut t = Thread::new("max", 1024, 1);
    let spawned = t.spawn(None, |thread, _p| {
        let v = thread.wait_notification(0, 0xFFFF_FFFF, TickType::MAX)?;
        assert_eq!(v, 0xABCD);
        Ok(Arc::new(()))
    })?;
    std::thread::sleep(Duration::from_millis(20));
    spawned.notify(ThreadNotification::SetValueWithOverwrite(0xABCD))?;
    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_notify_timeout() -> Result<()> {
    log_info!(TAG, "test_thread_notify_timeout");
    let mut t = Thread::new("to", 1024, 1);
    let spawned = t.spawn(None, |thread, _p| {
        let r = thread.wait_notification(0, 0xFFFF_FFFF, 30);
        assert!(r.is_err());
        Ok(Arc::new(()))
    })?;
    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_notify_from_isr_hpw() -> Result<()> {
    log_info!(TAG, "test_thread_notify_from_isr_hpw");
    let t = Thread::new("hpw", 1024, 1);
    let mut hpw: i32 = 0;
    t.notify_from_isr(ThreadNotification::SetBits(1), &mut hpw)?;
    assert_eq!(hpw, 0, "hpw should be 0 when no waiter is blocked");
    t.delete();
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_get_current_waits_for_notification() -> Result<()> {
    log_info!(TAG, "test_thread_get_current_waits_for_notification");
    let mut t = Thread::new("gc", 1024, 1);
    let spawned = t.spawn(None, |_thread, _p| {
        // Use get_current(), not the callback parameter
        let current = Thread::get_current();
        let v = current.wait_notification(0, 0xFFFF_FFFF, TickType::MAX)?;
        assert_eq!(v, 0xDEAD);
        Ok(Arc::new(()))
    })?;
    std::thread::sleep(Duration::from_millis(20));
    spawned.notify(ThreadNotification::SetValueWithOverwrite(0xDEAD))?;
    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

// ---------------------------------------------------------------------------
// Run all
// ---------------------------------------------------------------------------

pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "========== Running Linux-Specific Thread Tests ==========");
    test_thread_handles_unique()?;
    test_thread_handle_matches_metadata()?;
    test_thread_get_metadata_from_handle_real()?;
    test_thread_get_metadata_invalid_handle()?;
    test_thread_spawn_twice_rejected()?;
    test_thread_fast_exit_not_ready()?;
    test_thread_join_after_panic_sets_deleted()?;
    test_thread_notify_max_delay()?;
    test_thread_notify_timeout()?;
    test_thread_notify_from_isr_hpw()?;
    test_thread_get_current_waits_for_notification()?;
    log_info!(TAG, "========== All Linux-Specific Thread Tests PASSED ==========");
    Ok(())
}