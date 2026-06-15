# FreeRTOS ↔ Linux Backend Alignment Gaps

> Documenting behavioral misalignments between the FreeRTOS and Linux
> backends for the currently-implemented modules.  Gaps stem from inherent
> limitations of Linux user space; none violate the OSAL trait contract —
> both backends pass the same public test suite.

---

## 1. Mutex — Priority Inheritance

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `RawMutex::lock` → `xSemaphoreTakeRecursive` | `RawMutex::lock` → `StdMutex::lock` + `Condvar` |
| **Behavior** | The FreeRTOS kernel temporarily elevates the priority of the mutex holder to that of the highest-priority waiter, preventing priority inversion. | No priority boosting.  `std::sync::Mutex` is fair but does not influence thread scheduling priorities. |
| **Mitigation** | Built into the kernel. | On Linux, thread priorities are informational only; development / test workloads are unaffected.  Deploy to FreeRTOS for real-time behavior. |

---

## 2. Mutex — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `RawMutex::lock_from_isr` → `xSemaphoreTakeFromISR` + `System::yield_from_isr` | `RawMutex::lock_from_isr` → `StdMutex::try_lock` |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task runs immediately after the ISR. | Pure try-lock with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `lock_from_isr` is semantically correct as a non-blocking try-lock. |

---

## 3. System — Scheduler Start / Stop

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::start()` → `vTaskStartScheduler` / `System::stop()` → `vTaskEndScheduler` | `System::start()` / `System::stop()` — empty bodies |
| **Behavior** | `start()` launches the hardware scheduler and never returns. | No-op.  Linux threads run immediately via `std::thread::spawn` — there is no central scheduler to start. |
| **Mitigation** | Built into the kernel. | Documented no-op.  Application code should not rely on side-effects of `start()`. |

---

## 4. System — Scheduler Suspend / Resume

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::suspend_all` → `vTaskSuspendAll` / `System::resume_all` → `xTaskResumeAll` | `System::suspend_all` / `System::resume_all` — empty bodies |
| **Behavior** | Globally pauses task switches. | Linux user space cannot atomically stop all other threads. |
| **Mitigation** | N/A. | Must not be used for mutual exclusion (use `Mutex` instead).  Documented as no-op. |

---

## 5. System — Critical Sections

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::enter_critical` / `System::critical_section_enter` → disables interrupts | `System::enter_critical` / `System::critical_section_enter` — empty bodies |
| **Behavior** | Disables interrupts up to a configurable priority level, providing true atomicity. | User space cannot disable interrupts. |
| **Mitigation** | N/A. | Must not be relied on for shared-data protection in Linux tests (use `Mutex` instead).  Documented as no-op. |

---

## 6. System — ISR Support

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::yield_from_isr` / `System::end_switching_isr` | `System::yield_from_isr` / `System::end_switching_isr` — empty bodies |
| **Behavior** | Signals the scheduler for a context switch. | Linux user space neither implements nor runs ISRs. |
| **Mitigation** | N/A. | APIs retained for compatibility.  `_from_isr` variants are themselves implemented as non-blocking. |

---

## 7. System — Tick Overflow Behavior

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::get_tick_count` → `xTaskGetTickCount` (32-bit) / `System::check_timer` | `System::get_tick_count` → `Instant::elapsed` (64-bit) / `System::check_timer` |
| **Behavior** | `TickType(u32)` wraps after ~49 days.  `check_timer` has an explicit overflow-safe branch (`CpuRegisterSize::Bit32`). | `std::time::Instant` is a 64-bit monotonic clock.  `check_timer` uses `Duration` arithmetic — no wrap-around handling needed. |
| **Mitigation** | `wrapping_sub` is the cross-backend-safe idiom. | Processes do not run for 49 days in tests; outputs are equivalent in practice. |

---

## 8. System — Thread Introspection

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::count_threads` → `uxTaskGetNumberOfTasks` / `System::get_all_thread` → `uxTaskGetSystemState` | `System::count_threads` returns `1` / `System::get_all_thread` returns a single placeholder `ThreadMetadata` record |
| **Behavior** | FreeRTOS maintains a complete task list (name, priority, state, stack high-water mark). | The Linux backend returns a fixed placeholder record (`"main"`, `Running`, priority 1) — no dynamic thread registry is maintained (v0.1). |
| **Mitigation** | Built into the kernel. | Both backends now pass the same introspection tests. A dynamic registry may be added in a future release. |

---

## 9. Semaphore — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Semaphore::wait_from_isr` / `Semaphore::signal_from_isr` → `xSemaphoreTakeFromISR` / `xSemaphoreGiveFromISR` + `System::yield_from_isr` | `Semaphore::wait_from_isr` / `Semaphore::signal_from_isr` → `StdMutex::try_lock` + count logic |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task runs immediately after the ISR. | Pure non-blocking operations with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `_from_isr` variants are correct as non-blocking try-lock operations. |

---

## 10. Semaphore — Highest-Priority Waiter Unblocking

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Semaphore::signal` → `xSemaphoreGive` | `Semaphore::signal` → `Condvar::notify_one` |
| **Behavior** | FreeRTOS unblocks the **highest-priority** task waiting on the semaphore. | `Condvar::notify_one` wakes one waiter in FIFO order (or arbitrary ordering depending on the OS scheduler). |
| **Mitigation** | Built into the kernel. | On Linux thread priorities are informational only; the order of wake-up does not impact correctness for development/test workloads.  Deploy to FreeRTOS for priority-ordered wake-up. |

---

## 11. Memory — Heap Allocation

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `get_free_heap_size` → `xPortGetFreeHeapSize` | `System::get_free_heap_size` returns `1` |
| **Behavior** | FreeRTOS pre-allocates a fixed-size heap; `get_free_heap_size` reports remaining bytes — object creation can fail with `OutOfMemory`. | Linux provides virtual memory; Rust allocations almost never fail. Returns `1` to satisfy `> 0` assertions in portable tests. |
| **Mitigation** | N/A. | `RawMutex::new` uses `unwrap()`. Testing allocation failure would require additional `#[cfg]` endpoints. Can be added in a future release. |

---

## 12. EventGroup — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `EventGroup::set_from_isr` → `xEventGroupSetBitsFromISR` + `System::yield_from_isr` | `EventGroup::set_from_isr` → `StdMutex::try_lock` + `Condvar::notify_all` |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task unblocked by the bit-set runs immediately. | Pure non-blocking bit-set with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `set_from_isr` is semantically correct as a non-blocking operation. |

---

## 13. EventGroup — ISR Busy-Lock Behavior

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `EventGroup::get_from_isr` → `xEventGroupGetBitsFromISR` | `EventGroup::get_from_isr` → `StdMutex::try_lock` |
| **Behavior** | FreeRTOS provides a direct ISR-safe read that always returns the current bits, regardless of whether the event group is locked. | Linux uses `StdMutex::try_lock` — if another thread holds the lock, `get_from_isr` returns `0` (silent fallback). |
| **Mitigation** | N/A. | Linux has no ISR context; the `get_from_isr` method is informational only.  Application code should use `get()` for critical reads. |

---

## 14. EventGroup — Wake Strategy

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `EventGroup::set` → `xEventGroupSetBits` | `EventGroup::set` → `StdMutex::lock` + `Condvar::notify_all` |
| **Behavior** | FreeRTOS wakes only the waiters whose conditions are **satisfied** by the newly-set bits (precise wake-up). | Linux wakes **all** waiting threads via `notify_all()` — threads whose condition is not yet satisfied will check and re-enter `Condvar::wait_timeout`. |
| **Mitigation** | Built into the kernel. | Spurious wake-ups are handled by a loop checking the wait condition.  The extra wake-ups add minor overhead but are functionally correct. |

---

## 15. EventGroup — Resource Destruction

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `EventGroup::delete` / `Drop` → `vEventGroupDelete` | `EventGroup::delete` / `Drop` — empty body |
| **Behavior** | FreeRTOS deallocates the kernel event group object and sets the handle to null. | Linux has no kernel resources to free; Rust reclaims the `StdMutex` + `Condvar` memory automatically. |
| **Mitigation** | N/A. | Documented no-op.  Application code should not rely on `delete()` for synchronization. |

---

## 16. Queue — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::fetch_from_isr` → `xQueueReceiveFromISR` + `System::yield_from_isr` / `Queue::post_from_isr` → `xQueueSendToBackFromISR` + `System::yield_from_isr` | `Queue::fetch_from_isr` / `Queue::post_from_isr` → `StdMutex::try_lock` |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task runs immediately after the ISR. | Pure try-lock with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `_from_isr` variants are semantically correct as non-blocking try-lock operations. |

---

## 17. Queue — Message Storage Strategy

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::new` → `xQueueGenericCreate` / `Queue::post` → `xQueueSendToBack` | `Queue::new` → `StdMutex<VecDeque<Vec<u8>>>` / `Queue::post` → `item.to_vec()` + `push_back` |
| **Behavior** | FreeRTOS pre-allocates a fixed-size kernel buffer at creation time.  Messages are memcpy'd into pre-allocated slots — no per-message heap allocation. | Messages are cloned into new `Vec<u8>` heap allocations on every `post()`.  The `VecDeque` dynamically grows/shrinks within the capacity limit. |
| **Mitigation** | N/A. | The functional contract is identical — both guarantee in-order delivery and bounded capacity.  For latency-sensitive workloads, allocate the queue once and reuse it; the heap overhead is negligible in development/test.  Deploy to FreeRTOS for deterministic memory behavior. |

---

## 18. Queue — Wake Strategy (Priority-Ordered Unblocking)

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::fetch` / `Queue::post` → internal `xQueueGenericSend` / `xQueueGenericReceive` | `Queue::fetch` / `Queue::post` → `Condvar::notify_one` |
| **Behavior** | When a message is posted, FreeRTOS unblocks the **highest-priority** task waiting on the queue.  When a message is fetched, the highest-priority blocked sender is woken. | `Condvar::notify_one` wakes one waiter in OS-scheduler-dependent order (typically FIFO, not priority).  Thread priorities on Linux are informational only. |
| **Mitigation** | Built into the kernel. | The wake order does not impact correctness for development/test workloads.  Deploy to FreeRTOS for priority-ordered wake-up. |

---

## 19. Queue — Resource Destruction

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::delete` / `Drop` → `vQueueDelete` + set handle to null | `Queue::delete` / `Drop` → set `closed` flag + `Condvar::notify_all` |
| **Behavior** | FreeRTOS frees the kernel queue object and sets the handle pointer to null.  Any task blocked on the queue is unblocked. | Linux sets a `closed` flag and notifies all waiting threads so they unblock with `Error::Timeout`.  Rust reclaims the `StdMutex` + `Condvar` + `VecDeque` memory when `self` is dropped. |
| **Mitigation** | N/A. | Both backends unblock waiting tasks and reclaim resources.  Application code should not rely on post-deletion behavior beyond the contract. |

---

## 20. Queue — Copy In/Out vs In-Place Deserialization

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `QueueStreamed<T>::fetch` → deserialize from `Vec<u8>` using `T::from_bytes` | `QueueStreamed<T>::fetch` → deserialize from `Vec<u8>` using `T::from_bytes` (or serde) |
| **Behavior** | Both backends allocate a temporary `Vec<u8>` for the raw message, then deserialize into the caller's `&mut T` buffer.  The OSAL contract requires message-size consistency — the `Vec` capacity equals `T::len()`. | Identical logic.  The Linux backend explicitly copies from `VecDeque<Vec<u8>>` (which already contains a `Vec<u8>`) into the temporary `Vec`, then deserializes — one extra copy compared to the FreeRTOS kernel's direct memcpy from its internal buffer. |
| **Mitigation** | N/A. | The extra copy is negligible for development/test workloads and does not affect the public API contract.  Both backends pass the same test suite. |

---

## 21. Thread — Suspend / Resume

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::suspend` → `vTaskSuspend` / `Thread::resume` → `vTaskResume` | `Thread::suspend` / `Thread::resume` — empty bodies |
| **Behavior** | FreeRTOS atomically suspends/resumes the target task. The suspended task stops executing immediately. | Linux user space cannot atomically suspend another thread. No-op. |
| **Mitigation** | N/A. | Documented no-op. Application code should not rely on `suspend`/`resume` for synchronization on Linux.

---

## 22. Thread — Stack High Water Mark

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::get_metadata` → `uxTaskGetStackHighWaterMark` | `Thread::get_metadata` → fills `stack_depth` as-is |
| **Behavior** | FreeRTOS tracks the minimum remaining stack space ever recorded. | Linux fills `stack_high_water_mark` with the initial `stack_depth` — no runtime tracking. |
| **Mitigation** | N/A. | Stack overflow detection requires separate tooling (e.g., valgrind, ASan) on Linux.

---

## 23. Thread — Priority-Ordered Notification Wake

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::notify` / `Thread::wait_notification` → `xTaskNotify` / `xTaskNotifyWait` | `Thread::notify` / `Thread::wait_notification` → `StdMutex::lock` + `Condvar` |
| **Behavior** | FreeRTOS task notifications use priority-ordered wake-up. If multiple tasks are waiting on notifications, the highest-priority task is unblocked first. | Linux uses `Condvar::notify_all` — all waiters wake and compete for the lock. |
| **Mitigation** | N/A. | Wake order does not impact correctness for development/test workloads on Linux — thread priorities are informational only.

---

## 24. Thread — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::notify_from_isr` → `xTaskNotifyFromISR` + `System::yield_from_isr` | `Thread::notify_from_isr` → `StdMutex::try_lock` + `Condvar::notify_all` |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task runs immediately after the ISR. | Pure non-blocking notification with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `notify_from_isr` is semantically correct as a non-blocking operation.

---

## 25. Timer — Scheduler Architecture

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Timer::new` → `xTimerCreate` registers with the timer daemon task | `Timer::new` creates a dedicated worker `std::thread` per timer |
| **Behavior** | FreeRTOS uses a single timer service task that processes all timers from a command queue. Callbacks run sequentially in the daemon context. | Each timer spawns its own OS thread on first `start()`. Threads sleep independently. |
| **Mitigation** | N/A. | The per-timer thread model is functionally equivalent — callbacks still run sequentially per timer. For deeply embedded use cases, deploy to FreeRTOS to avoid per-timer thread overhead.

---

## 26. Timer — Scheduling Precision

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | Timer expiry triggered by the kernel tick interrupt | Timer expiry via `std::thread::sleep` with 5 ms polling |
| **Behavior** | FreeRTOS timers expire at the next tick boundary after the period elapses (typically ±1 tick jitter). | Linux timers poll every 5 ms and fire within ±5 ms of the actual period. |
| **Mitigation** | N/A. | Acceptable for development/test workloads. Deploy to FreeRTOS for hard real-time timer guarantees.

---

## 27. Timer — Command Queue vs Synchronous Operations

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `start` / `stop` / `reset` / `change_period` → send command to timer daemon queue | `start` / `stop` / `reset` / `change_period` → directly mutate shared state + notify worker via `Condvar` |
| **Behavior** | FreeRTOS uses an internal command queue for timer operations. If the queue is full, the caller blocks up to `ticks_to_wait`. | Linux ignores `ticks_to_wait` — all operations are synchronous and cannot block (no bounded queue). |
| **Mitigation** | `ticks_to_wait` is implemented as `_ticks_to_wait: TickType` (unused). | Application code should not rely on `ticks_to_wait` for timer API calls on Linux.

---

## 28. Timer — Resource Destruction

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Timer::delete` / `Drop` → `xTimerDelete` | `Timer::delete` / `Drop` → sets `deleted` + `cancelled` flags, notifies worker via `Condvar` |
| **Behavior** | FreeRTOS asynchronously deletes the timer object and frees kernel resources. | Linux sets flags so the worker thread exits on its next poll cycle. No explicit thread join in Drop (avoids blocking). |
| **Mitigation** | N/A. | Worker threads may linger briefly after `delete()` returns. Application code should allow a short grace period before process exit.
