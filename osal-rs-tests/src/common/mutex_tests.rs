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

use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};
use osal_rs::{log_debug, log_info};

const TAG: &str = "MutexTests";

pub fn test_mutex_creation() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_creation");
    let _mutex = Mutex::new(0u32);
    log_info!(TAG, "test_mutex_creation PASSED");
    Ok(())
}

pub fn test_mutex_lock_unlock() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_lock_unlock");
    let mutex = Mutex::new(42u32);
    
    {
        let guard = mutex.lock();
        assert!(guard.is_ok());
        
        if let Ok(g) = guard {
            log_debug!(TAG, "Mutex locked, value: {}", *g);
            assert_eq!(*g, 42);
        }
    }
    
    {
        let guard = mutex.lock();
        assert!(guard.is_ok());
        log_debug!(TAG, "Mutex re-locked successfully");
    }
    log_info!(TAG, "test_mutex_lock_unlock PASSED");
    Ok(())
}

pub fn test_mutex_modify_data() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_modify_data");
    let mutex = Mutex::new(0u32);
    
    {
        let mut guard = mutex.lock()?;
        *guard = 100;
        log_debug!(TAG, "Modified value to: {}", *guard);
    }
    
    {
        let guard = mutex.lock()?;
        log_debug!(TAG, "Read value: {}", *guard);
        assert_eq!(*guard, 100);
    }
    log_info!(TAG, "test_mutex_modify_data PASSED");
    Ok(())
}

pub fn test_mutex_multiple_locks() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_multiple_locks");
    let mutex = Mutex::new(0u32);
    
    for i in 0..10 {
        let mut guard = mutex.lock()?;
        *guard += 1;
        assert_eq!(*guard, i + 1);
    }
    
    let guard = mutex.lock()?;
    log_debug!(TAG, "Final counter value: {}", *guard);
    assert_eq!(*guard, 10);
    log_info!(TAG, "test_mutex_multiple_locks PASSED");
    Ok(())
}

pub fn test_mutex_guard_drop() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_guard_drop");
    let mutex = Mutex::new(42u32);
    
    {
        let _guard = mutex.lock()?;
        log_debug!(TAG, "Guard acquired, will drop on scope exit");
    }
    
    let guard = mutex.lock();
    assert!(guard.is_ok());
    log_info!(TAG, "test_mutex_guard_drop PASSED");
    Ok(())
}

pub fn test_mutex_with_struct() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_with_struct");
    #[derive(Debug, PartialEq)]
    struct TestData {
        value: u32,
        flag: bool,
    }
    
    let mutex = Mutex::new(TestData { value: 0, flag: false });
    
    {
        let mut guard = mutex.lock()?;
        guard.value = 123;
        guard.flag = true;
        log_debug!(TAG, "Modified struct - value: {}, flag: {}", guard.value, guard.flag);
    }
    
    {
        let guard = mutex.lock()?;
        assert_eq!(guard.value, 123);
        assert_eq!(guard.flag, true);
    }
    log_info!(TAG, "test_mutex_with_struct PASSED");
    Ok(())
}

pub fn test_mutex_non_recursive() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_non_recursive");
    let mutex = Mutex::new(0u32);

    let _guard = mutex.lock()?;
    log_debug!(TAG, "First lock acquired");

    let second = mutex.lock();
    assert!(
        second.is_err(),
        "typed Mutex<T> must not return a second mutable guard"
    );

    log_info!(TAG, "test_mutex_non_recursive PASSED");
    Ok(())
}

pub fn test_raw_mutex_recursive() -> Result<()> {
    log_info!(TAG, "Starting test_raw_mutex_recursive");
    use std::thread;

    let raw = Arc::new(RawMutex::new()?);

    // Recursive acquisition: 3 locks
    assert_eq!(raw.lock(), OsalRsBool::True);
    assert_eq!(raw.lock(), OsalRsBool::True);
    assert_eq!(raw.lock(), OsalRsBool::True);

    // Partial unlock: 2 unlocks — mutex still held (recursion > 0)
    assert_eq!(raw.unlock(), OsalRsBool::True);
    assert_eq!(raw.unlock(), OsalRsBool::True);

    // Cross-thread check: another thread must NOT be able to acquire it
    {
        let raw_clone = Arc::clone(&raw);
        let handle = thread::spawn(move || {
            // should fail because main thread still holds the mutex
            raw_clone.lock_from_isr()
        });
        assert_eq!(handle.join().unwrap(), OsalRsBool::False);
    }

    // Final unlock — recursion reaches zero, mutex fully released
    assert_eq!(raw.unlock(), OsalRsBool::True);

    // Extra unlock on a free mutex should fail
    assert_eq!(raw.unlock(), OsalRsBool::False);

    // Cross-thread check: another thread should now succeed
    {
        let raw_clone2 = Arc::clone(&raw);
        let handle2 = thread::spawn(move || {
            let result = raw_clone2.lock_from_isr();
            if result == OsalRsBool::True {
                raw_clone2.unlock_from_isr();
            }
            result
        });
        assert_eq!(handle2.join().unwrap(), OsalRsBool::True);
    }

    log_info!(TAG, "test_raw_mutex_recursive PASSED");
    Ok(())
}

pub fn test_mutex_drop() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_drop");
    let mutex = Mutex::new(42u32);
    drop(mutex);
    log_info!(TAG, "test_mutex_drop PASSED");
    Ok(())
}

pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "========== Running Mutex Tests ==========");
    test_mutex_creation()?;
    test_mutex_lock_unlock()?;
    test_mutex_modify_data()?;
    test_mutex_multiple_locks()?;
    test_mutex_guard_drop()?;
    test_mutex_with_struct()?;
    test_mutex_non_recursive()?;
    test_raw_mutex_recursive()?;
    test_mutex_drop()?;
    log_info!(TAG, "========== All Mutex Tests PASSED ==========");
    Ok(())
}
