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
| **Function** | `System::count_threads` → `uxTaskGetNumberOfTasks` / `System::get_all_thread` → `uxTaskGetSystemState` | `System::count_threads` returns `1` / `System::get_all_thread` returns empty `SystemState` |
| **Behavior** | FreeRTOS maintains a complete task list (name, priority, state, stack high-water mark). | The Linux backend does not yet maintain an internal thread registry (v0.1). |
| **Mitigation** | Built into the kernel. | Introspection tests are gated with `#[cfg(feature = "freertos")]` on Linux.  A registry may be added in a future release. |

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
| **Function** | `get_free_heap_size` → `xPortGetFreeHeapSize` | `System::get_free_heap_size` returns `usize::MAX` |
| **Behavior** | FreeRTOS pre-allocates a fixed-size heap; `get_free_heap_size` reports remaining bytes — object creation can fail with `OutOfMemory`. | Linux provides virtual memory; Rust allocations almost never fail. |
| **Mitigation** | N/A. | `RawMutex::new` uses `unwrap()`.  Testing allocation failure would require additional `#[cfg]` endpoints.  Can be added in a future release. |