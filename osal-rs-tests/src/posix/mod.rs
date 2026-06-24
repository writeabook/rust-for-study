//! POSIX backend test runner.
//!
//! Each `#[test]` function executes a single OSAL contract test from
//! `crate::api`.  Fine-grained wrappers make test-failure attribution
//! precise.
//!
//! Backend-specific POSIX smoke tests live in `crate::port::posix_smoke_tests`.

// ---------------------------------------------------------------------------
// Duration
// ---------------------------------------------------------------------------
#[test]
fn duration_all() { crate::api::duration_tests::run_all_tests().unwrap(); }

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------
#[test]
fn system_all() { crate::api::system_tests::run_all_tests().unwrap(); }

// ---------------------------------------------------------------------------
// Mutex
// ---------------------------------------------------------------------------
#[test]
fn mutex_creation() { crate::api::mutex_tests::test_mutex_creation().unwrap(); }
#[test]
fn mutex_lock_unlock() { crate::api::mutex_tests::test_mutex_lock_unlock().unwrap(); }
#[test]
fn mutex_modify_data() { crate::api::mutex_tests::test_mutex_modify_data().unwrap(); }
#[test]
fn mutex_multiple_locks() { crate::api::mutex_tests::test_mutex_multiple_locks().unwrap(); }
#[test]
fn mutex_guard_drop() { crate::api::mutex_tests::test_mutex_guard_drop().unwrap(); }
#[test]
fn mutex_with_struct() { crate::api::mutex_tests::test_mutex_with_struct().unwrap(); }
#[test]
fn mutex_non_recursive() { crate::api::mutex_tests::test_mutex_non_recursive().unwrap(); }
#[test]
fn raw_mutex_recursive() { crate::api::mutex_tests::test_raw_mutex_recursive().unwrap(); }
#[test]
fn mutex_drop() { crate::api::mutex_tests::test_mutex_drop().unwrap(); }
#[test]
fn mutex_cross_thread_exclusion() { crate::api::mutex_tests::test_mutex_provides_mutual_exclusion_across_threads().unwrap(); }

// ---------------------------------------------------------------------------
// Semaphore
// ---------------------------------------------------------------------------
#[test]
fn semaphore_creation() { crate::api::semaphore_tests::test_semaphore_creation().unwrap(); }
#[test]
fn semaphore_creation_with_count() { crate::api::semaphore_tests::test_semaphore_creation_with_count().unwrap(); }
#[test]
fn semaphore_signal_wait() { crate::api::semaphore_tests::test_semaphore_signal_wait().unwrap(); }
#[test]
fn semaphore_wait_timeout() { crate::api::semaphore_tests::test_semaphore_wait_timeout().unwrap(); }
#[test]
fn semaphore_multiple_signals() { crate::api::semaphore_tests::test_semaphore_multiple_signals().unwrap(); }
#[test]
fn semaphore_max_count() { crate::api::semaphore_tests::test_semaphore_max_count().unwrap(); }
#[test]
fn semaphore_initial_count() { crate::api::semaphore_tests::test_semaphore_initial_count().unwrap(); }
#[test]
fn semaphore_binary() { crate::api::semaphore_tests::test_semaphore_binary().unwrap(); }
#[test]
fn semaphore_drop() { crate::api::semaphore_tests::test_semaphore_drop().unwrap(); }
#[test]
fn semaphore_wait_blocks_until_signal() { crate::api::semaphore_tests::test_semaphore_wait_blocks_until_signal().unwrap(); }
#[test]
fn semaphore_no_lost_wakeup_under_race() { crate::api::semaphore_tests::test_semaphore_no_lost_wakeup_under_race().unwrap(); }

// ---------------------------------------------------------------------------
// EventGroup
// ---------------------------------------------------------------------------
#[test]
fn event_group_creation() { crate::api::event_group_tests::test_event_group_creation().unwrap(); }
#[test]
fn event_group_set_get() { crate::api::event_group_tests::test_event_group_set_get().unwrap(); }
#[test]
fn event_group_multiple_bits() { crate::api::event_group_tests::test_event_group_multiple_bits().unwrap(); }
#[test]
fn event_group_clear() { crate::api::event_group_tests::test_event_group_clear().unwrap(); }
#[test]
fn event_group_clear_all() { crate::api::event_group_tests::test_event_group_clear_all().unwrap(); }
#[test]
fn event_group_wait() { crate::api::event_group_tests::test_event_group_wait().unwrap(); }
#[test]
fn event_group_wait_timeout() { crate::api::event_group_tests::test_event_group_wait_timeout().unwrap(); }
#[test]
fn event_group_wait_partial() { crate::api::event_group_tests::test_event_group_wait_partial().unwrap(); }
#[test]
fn event_group_sequential_ops() { crate::api::event_group_tests::test_event_group_sequential_operations().unwrap(); }
#[test]
fn event_group_all_bits() { crate::api::event_group_tests::test_event_group_all_bits().unwrap(); }
#[test]
fn event_group_drop() { crate::api::event_group_tests::test_event_group_drop().unwrap(); }
#[test]
fn event_group_wait_any_unblocks() { crate::api::event_group_tests::test_event_group_wait_any_unblocks_after_set().unwrap(); }
#[test]
fn event_group_clear_affects_waits() { crate::api::event_group_tests::test_event_group_clear_bits_affects_future_waits().unwrap(); }
#[test]
fn event_group_no_lost_set_race() { crate::api::event_group_tests::test_event_group_no_lost_set_under_race().unwrap(); }

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------
#[test]
fn queue_creation() { crate::api::queue_tests::test_queue_creation().unwrap(); }
#[test]
fn queue_post_fetch() { crate::api::queue_tests::test_queue_post_fetch().unwrap(); }
#[test]
fn queue_timeout() { crate::api::queue_tests::test_queue_timeout().unwrap(); }
#[test]
fn queue_multiple_items() { crate::api::queue_tests::test_queue_multiple_items().unwrap(); }
#[test]
fn queue_drop() { crate::api::queue_tests::test_queue_drop().unwrap(); }
#[test]
fn queue_receive_blocks_until_send() { crate::api::queue_tests::test_queue_receive_blocks_until_send().unwrap(); }
#[test]
fn queue_fifo_order() { crate::api::queue_tests::test_queue_preserves_fifo_order().unwrap(); }

// ---------------------------------------------------------------------------
// Thread
// ---------------------------------------------------------------------------
#[test]
fn thread_creation() { crate::api::thread_tests::test_thread_creation().unwrap(); }
#[test]
fn thread_spawn() { crate::api::thread_tests::test_thread_spawn().unwrap(); }
#[test]
fn thread_with_param() { crate::api::thread_tests::test_thread_with_param().unwrap(); }
#[test]
fn thread_suspend_resume() { crate::api::thread_tests::test_thread_suspend_resume().unwrap(); }
#[test]
fn thread_get_metadata() { crate::api::thread_tests::test_thread_get_metadata().unwrap(); }
#[test]
fn thread_notification() { crate::api::thread_tests::test_thread_notification().unwrap(); }
#[test]
fn thread_get_current() { crate::api::thread_tests::test_thread_get_current().unwrap(); }
#[test]
fn thread_spawn_simple() { crate::api::thread_tests::test_thread_spawn_simple().unwrap(); }
#[test]
fn thread_spawn_shared_data() { crate::api::thread_tests::test_thread_spawn_simple_with_shared_data().unwrap(); }
#[test]
fn thread_multiple_concurrent() { crate::api::thread_tests::test_multiple_threads_can_run_concurrently().unwrap(); }

// ---------------------------------------------------------------------------
// Timer
// ---------------------------------------------------------------------------
#[test]
fn timer_creation() { crate::api::timer_tests::test_timer_creation().unwrap(); }
#[test]
fn timer_one_shot() { crate::api::timer_tests::test_timer_one_shot().unwrap(); }
#[test]
fn timer_auto_reload() { crate::api::timer_tests::test_timer_auto_reload().unwrap(); }
#[test]
fn timer_start_stop() { crate::api::timer_tests::test_timer_start_stop().unwrap(); }
#[test]
fn timer_reset() { crate::api::timer_tests::test_timer_reset().unwrap(); }
#[test]
fn timer_change_period() { crate::api::timer_tests::test_timer_change_period().unwrap(); }
#[test]
fn timer_with_param() { crate::api::timer_tests::test_timer_with_param().unwrap(); }
#[test]
fn timer_delete() { crate::api::timer_tests::test_timer_delete().unwrap(); }

// ---------------------------------------------------------------------------
// API surface (compile-time check)
// ---------------------------------------------------------------------------
#[test]
fn api_surface() { crate::api::api_surface::run_all_tests().unwrap(); }
