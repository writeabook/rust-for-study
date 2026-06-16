//! Linux-specific system tests.
//!
//! These tests supplement the common system test suite with tests
//! that exercise Linux-backend-specific behaviours: critical-section
//! mutual exclusion, reentrancy, API alias sharing, and ISR-path
//! lock reuse.

use osal_rs::os::{System, SystemFn};
use osal_rs::utils::Result;

/// Entry-point called from `mod.rs` to run all Linux-specific
/// system tests.
pub fn run_all_tests() -> Result<()> {
    critical_section_is_mutually_exclusive()?;
    critical_section_is_reentrant_on_same_thread()?;
    critical_section_aliases_share_same_lock()?;
    critical_section_from_isr_uses_same_lock()?;
    critical_section_blocks_other_threads_until_exit()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Mutual exclusion across threads
// ---------------------------------------------------------------------------

/// Multiple threads entering the critical section must never overlap.
fn critical_section_is_mutually_exclusive() -> Result<()> {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    let inside = Arc::new(AtomicBool::new(false));
    let overlap_detected = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();

    for _ in 0..4 {
        let inside = Arc::clone(&inside);
        let overlap_detected = Arc::clone(&overlap_detected);

        handles.push(std::thread::spawn(move || {
            for _ in 0..50 {
                System::enter_critical();

                if inside.swap(true, Ordering::SeqCst) {
                    overlap_detected.store(true, Ordering::SeqCst);
                }

                std::thread::sleep(std::time::Duration::from_millis(1));

                inside.store(false, Ordering::SeqCst);
                System::exit_critical();
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    assert!(!overlap_detected.load(Ordering::SeqCst));
    Ok(())
}

// ---------------------------------------------------------------------------
// Same-thread nesting does not deadlock
// ---------------------------------------------------------------------------

/// The same thread can nest `enter_critical()` calls without deadlocking.
fn critical_section_is_reentrant_on_same_thread() -> Result<()> {
    System::enter_critical();
    System::enter_critical();
    System::enter_critical();

    System::exit_critical();
    System::exit_critical();
    System::exit_critical();
    Ok(())
}

// ---------------------------------------------------------------------------
// API aliases share the same lock
// ---------------------------------------------------------------------------

/// `critical_section_enter/exit` and `enter_critical/exit_critical`
/// share the same nesting counter.
fn critical_section_aliases_share_same_lock() -> Result<()> {
    System::critical_section_enter();
    System::enter_critical();

    System::exit_critical();
    System::critical_section_exit();
    Ok(())
}

// ---------------------------------------------------------------------------
// ISR simulation path shares the same lock
// ---------------------------------------------------------------------------

/// `enter_critical_from_isr` / `exit_critical_from_isr` reuse the same
/// recursive lock as task-level calls.
fn critical_section_from_isr_uses_same_lock() -> Result<()> {
    let saved = System::enter_critical_from_isr();

    System::enter_critical();
    System::exit_critical();

    System::exit_critical_from_isr(saved);
    Ok(())
}

// ---------------------------------------------------------------------------
// Blocking: held critical section prevents other threads from entering
// ---------------------------------------------------------------------------

/// When one thread holds the critical lock, another thread blocks on
/// `enter_critical()` until the first thread releases it.
fn critical_section_blocks_other_threads_until_exit() -> Result<()> {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    let entered = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));

    System::enter_critical();

    let entered_clone = Arc::clone(&entered);
    let completed_clone = Arc::clone(&completed);

    let handle = std::thread::spawn(move || {
        entered_clone.store(true, Ordering::SeqCst);

        System::enter_critical();
        completed_clone.store(true, Ordering::SeqCst);
        System::exit_critical();
    });

    // Wait until the spawned thread has attempted to enter.
    while !entered.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }

    // Give it time to block (it cannot enter while we hold the lock).
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(!completed.load(Ordering::SeqCst));

    System::exit_critical();

    handle.join().unwrap();
    assert!(completed.load(Ordering::SeqCst));
    Ok(())
}