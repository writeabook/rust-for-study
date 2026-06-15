//! Linux-specific mutex tests.
//!
//! These tests exercise behaviours that are specific to the Linux backend
//! (std::sync::Mutex poisoning, std::thread-based contention, ISR host
//! simulation) and are therefore **not** part of the cross-backend common
//! test suite.

extern crate alloc;

use alloc::sync::Arc;
use std::thread;

use osal_rs::os::*;
use osal_rs::utils::Result;
use osal_rs::{log_debug, log_info};

const TAG: &str = "LinuxMutexTests";

pub fn test_mutex_multi_thread_contention() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_multi_thread_contention");

    let mutex = Arc::new(Mutex::new(0u32));
    const THREADS: usize = 8;
    const ITERS: u32 = 10_000;

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let m = Arc::clone(&mutex);
            thread::spawn(move || {
                for _ in 0..ITERS {
                    let mut guard = m.lock().unwrap();
                    *guard += 1;
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let final_val = *mutex.lock().unwrap();
    assert_eq!(
        final_val,
        THREADS as u32 * ITERS,
        "multi-thread contention: expected {}, got {}",
        THREADS as u32 * ITERS,
        final_val
    );

    log_info!(TAG, "test_mutex_multi_thread_contention PASSED");
    Ok(())
}

pub fn test_mutex_poison_recovery() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_poison_recovery");

    let mutex = Arc::new(Mutex::new(0u32));

    // Panic while holding the lock — poisons the inner StdMutex
    let m = Arc::clone(&mutex);
    let handle = thread::spawn(move || {
        let _guard = m.lock().unwrap();
        panic!("intentional panic to poison the mutex");
    });
    let _ = handle.join(); // ignore poison panic

    // After recovery the mutex must still be usable
    let guard = mutex.lock();
    assert!(guard.is_ok(), "mutex must be lockable after poison recovery");
    assert_eq!(*guard.unwrap(), 0, "guarded data must be intact");

    log_info!(TAG, "test_mutex_poison_recovery PASSED");
    Ok(())
}

pub fn test_mutex_isr_path() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_isr_path");

    let mutex = Mutex::new(99u32);

    // 1. Immediate success when free
    {
        let guard = mutex.lock_from_isr();
        assert!(guard.is_ok(), "ISR lock must succeed when mutex is free");
        assert_eq!(*guard.unwrap(), 99);
        // MutexGuardFromIsr Drop is tested implicitly here
    }

    // 2. Immediate failure when occupied
    {
        let _guard = mutex.lock()?;
        let result = mutex.lock_from_isr();
        assert!(result.is_err(), "ISR lock must fail when mutex is held");
    }

    // 3. Normal lock() succeeds after ISR guard drop
    {
        let guard = mutex.lock();
        assert!(guard.is_ok(), "normal lock must succeed after guard drop");
    }

    // 4. lock_from_isr_explicit is callable
    {
        let guard = mutex.lock_from_isr_explicit();
        assert!(guard.is_ok(), "ISR explicit lock must succeed");
    }

    // 5. Poisoned data lock recovery (ISR path)
    let poisoned = Arc::new(Mutex::new(0u32));
    let p = Arc::clone(&poisoned);
    let panic_handle = thread::spawn(move || {
        let _g = p.lock().unwrap();
        panic!("poison for ISR test");
    });
    let _ = panic_handle.join();

    {
        let guard = poisoned.lock_from_isr();
        assert!(guard.is_ok(), "ISR lock must recover from poison");
        assert_eq!(*guard.unwrap(), 0);
    }

    log_info!(TAG, "test_mutex_isr_path PASSED");
    Ok(())
}

pub fn run_all_tests() -> Result<()> {
    log_info!(TAG, "========== Running Linux-Specific Mutex Tests ==========");
    test_mutex_multi_thread_contention()?;
    test_mutex_poison_recovery()?;
    test_mutex_isr_path()?;
    log_info!(TAG, "========== All Linux-Specific Mutex Tests PASSED ==========");
    Ok(())
}