# FreeRTOS ↔ Linux Backend Alignment Gaps

> Documenting behavioral misalignments between the FreeRTOS and Linux
> backends for the currently-implemented modules.  Gaps stem from inherent
> limitations of Linux user space; none violate the OSAL trait contract —
> both backends pass the same public test suite.

---

## Mutex

### 1. Mutex — Priority Inheritance

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `RawMutex::lock` → `xSemaphoreTakeRecursive` | `RawMutex::lock` → `StdMutex::lock` + `Condvar` |
| **Behavior** | The FreeRTOS kernel temporarily elevates the priority of the mutex holder to that of the highest-priority waiter, preventing priority inversion. | No priority boosting.  `std::sync::Mutex` is fair but does not influence thread scheduling priorities. |
| **Mitigation** | Built into the kernel. | On Linux, thread priorities are informational only; development / test workloads are unaffected.  Deploy to FreeRTOS for real-time behavior. |

---

### 2. Mutex — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `RawMutex::lock_from_isr` → `xSemaphoreTakeFromISR` + `System::yield_from_isr` | `RawMutex::lock_from_isr` → `StdMutex::try_lock` |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task runs immediately after the ISR. | Pure try-lock with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `lock_from_isr` is semantically correct as a non-blocking try-lock. |

---

### 3. Mutex — Dual-Layer Architecture

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | Only `RawMutex` (recursive mutex via `xSemaphoreTakeRecursive` / `xSemaphoreGiveRecursive`). `Mutex<T>` wraps a FreeRTOS recursive mutex plus RAII guard. | `RawMutex` (recursive: `StdMutex<State>` + `Condvar` + `owner: ThreadId` + `recursion: u32`) **and** `Mutex<T>` (non-recursive: `Box<StdMutex<T>>` for data + `StdMutex<Option<ThreadId>>` for ownership tracking). |
| **Behavior** | FreeRTOS mutexes are inherently recursive. `Mutex<T>` provides type-safe RAII on top of the same recursive primitive. | `RawMutex` follows the trait contract (recursive). `Mutex<T>` is **non-recursive** — if the same thread attempts to lock a `Mutex<T>` it already holds, it returns `Error::MutexLockFailed`. `lock_from_isr` on `Mutex<T>` is implemented as `try_lock` (non-blocking). |
| **Mitigation** | Built into the kernel. | `Mutex<T>`'s non-recursive behavior is intentional and differs from the FreeRTOS backend's recursive behavior. Application code must not recursively lock the same `Mutex<T>` from the same thread. |

---

## System

### 4. System — Scheduler Start / Stop

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::start()` → `vTaskStartScheduler` / `System::stop()` → `vTaskEndScheduler` | `System::start()` / `System::stop()` — empty bodies |
| **Behavior** | `start()` launches the hardware scheduler and never returns. | No-op.  Linux threads run immediately via `std::thread::spawn` — there is no central scheduler to start. |
| **Mitigation** | Built into the kernel. | Documented no-op.  Application code should not rely on side-effects of `start()`. |

---

### 5. System — Scheduler Suspend / Resume

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::suspend_all` → `vTaskSuspendAll` / `System::resume_all` → `xTaskResumeAll` | `System::suspend_all` / `System::resume_all` — empty bodies |
| **Behavior** | Globally pauses task switches. | Linux user space cannot atomically stop all other threads. |
| **Mitigation** | N/A. | Must not be used for mutual exclusion (use `Mutex` instead).  Documented as no-op. |

---

### 6. System — Critical Sections

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::enter_critical` / `System::critical_section_enter` → disables interrupts | `System::enter_critical` / `System::critical_section_enter` → `enter_global_critical()` — acquires a process-local recursive lock |
| **Behavior** | Disables interrupts up to a configurable priority level, providing true atomicity. | A global `StdMutex<()>` (`OnceLock`-initialized) provides mutual exclusion among all OSAL callers within the process. Per-thread nesting depth is tracked via `thread_local!` (`CriticalThreadState`), so the same thread may nest calls. `enter_critical_from_isr()` returns the previous nesting depth as a saved interrupt status. **This does NOT disable Linux interrupts or OS scheduling** — it only provides mutual exclusion within the process. |
| **Mitigation** | Built into the kernel. | Must not be relied on for real atomicity on Linux (use `Mutex` instead).  The simulated critical section prevents data races among OSAL callers but offers no hard-real-time guarantees. |

---

### 7. System — ISR Support

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::yield_from_isr` / `System::end_switching_isr` | `System::yield_from_isr` / `System::end_switching_isr` — empty bodies |
| **Behavior** | Signals the scheduler for a context switch. | Linux user space neither implements nor runs ISRs. |
| **Mitigation** | N/A. | APIs retained for compatibility.  `_from_isr` variants are themselves implemented as non-blocking. |

---

### 8. System — Tick Overflow Behavior

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::get_tick_count` → `xTaskGetTickCount` (32-bit) / `System::check_timer` | `System::get_tick_count` → `Instant::elapsed` (64-bit) / `System::check_timer` |
| **Behavior** | `TickType(u32)` wraps after ~49 days.  `check_timer` has an explicit overflow-safe branch (`CpuRegisterSize::Bit32`). | `std::time::Instant` is a 64-bit monotonic clock.  `check_timer` uses `Duration` arithmetic — no wrap-around handling needed. |
| **Mitigation** | `wrapping_sub` is the cross-backend-safe idiom. | Processes do not run for 49 days in tests; outputs are equivalent in practice. |

---

### 9. System — Thread Introspection

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `System::count_threads` → `uxTaskGetNumberOfTasks` / `System::get_all_thread` → `uxTaskGetSystemState` | `System::count_threads` → `thread::count_registered_threads()` / `System::get_all_thread` → `snapshot_registered_threads()` returning `SystemState` |
| **Behavior** | FreeRTOS maintains a complete task list (name, priority, state, stack high-water mark). | Linux maintains a dynamic `ThreadRegistry` (`HashMap<usize, Weak<ThreadCore>>` + `HashMap<ThreadId, usize>`) backed by a global `OnceLock<StdMutex<ThreadRegistry>>`. `ensure_main_thread_registered()` lazily registers the main thread. `count_threads()` returns the number of registered threads. `get_all_thread()` returns a complete `SystemState` snapshot. `get_state()` returns the current thread's `ThreadState`. |
| **Mitigation** | Built into the kernel. | The registry is now fully functional. Both backends pass the same introspection tests. |

---

## Semaphore

### 10. Semaphore — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Semaphore::wait_from_isr` / `Semaphore::signal_from_isr` → `xSemaphoreTakeFromISR` / `xSemaphoreGiveFromISR` + `System::yield_from_isr` | `Semaphore::wait_from_isr` / `Semaphore::signal_from_isr` → `StdMutex::try_lock` + count logic |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task runs immediately after the ISR. | Pure non-blocking operations with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `_from_isr` variants are correct as non-blocking try-lock operations. |

---

### 11. Semaphore — Highest-Priority Waiter Unblocking

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Semaphore::signal` → `xSemaphoreGive` | `Semaphore::signal` → `Condvar::notify_one` |
| **Behavior** | FreeRTOS unblocks the **highest-priority** task waiting on the semaphore. | `Condvar::notify_one` wakes one waiter in FIFO order (or arbitrary ordering depending on the OS scheduler). |
| **Mitigation** | Built into the kernel. | On Linux thread priorities are informational only; the order of wake-up does not impact correctness for development/test workloads.  Deploy to FreeRTOS for priority-ordered wake-up. |

---

## EventGroup

### 12. EventGroup — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `EventGroup::set_from_isr` → `xEventGroupSetBitsFromISR` + `System::yield_from_isr` | `EventGroup::set_from_isr` → `StdMutex::try_lock` + `Condvar::notify_all` |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task unblocked by the bit-set runs immediately. | Pure non-blocking bit-set with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `set_from_isr` is semantically correct as a non-blocking operation. |

---

### 13. EventGroup — ISR Busy-Lock Behavior

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `EventGroup::get_from_isr` → `xEventGroupGetBitsFromISR` | `EventGroup::get_from_isr` → `StdMutex::try_lock` |
| **Behavior** | FreeRTOS provides a direct ISR-safe read that always returns the current bits, regardless of whether the event group is locked. | Linux uses `StdMutex::try_lock` — if another thread holds the lock, `get_from_isr` returns `0` (silent fallback). |
| **Mitigation** | N/A. | Linux has no ISR context; the `get_from_isr` method is informational only.  Application code should use `get()` for critical reads. |

---

### 14. EventGroup — Wake Strategy

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `EventGroup::set` → `xEventGroupSetBits` | `EventGroup::set` → `StdMutex::lock` + `Condvar::notify_all` |
| **Behavior** | FreeRTOS wakes only the waiters whose conditions are **satisfied** by the newly-set bits (precise wake-up). | Linux wakes **all** waiting threads via `notify_all()` — threads whose condition is not yet satisfied will check and re-enter `Condvar::wait_timeout`. |
| **Mitigation** | Built into the kernel. | Spurious wake-ups are handled by a loop checking the wait condition.  The extra wake-ups add minor overhead but are functionally correct. |

---

### 15. EventGroup — Resource Destruction

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `EventGroup::delete` / `Drop` → `vEventGroupDelete` | `EventGroup::delete` / `Drop` — empty body |
| **Behavior** | FreeRTOS deallocates the kernel event group object and sets the handle to null. | Linux has no kernel resources to free; Rust reclaims the `StdMutex` + `Condvar` memory automatically. |
| **Mitigation** | N/A. | Documented no-op.  Application code should not rely on `delete()` for synchronization. |

---

## Queue

### 16. Queue — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::fetch_from_isr` → `xQueueReceiveFromISR` + `System::yield_from_isr` / `Queue::post_from_isr` → `xQueueSendToBackFromISR` + `System::yield_from_isr` | `Queue::fetch_from_isr` / `Queue::post_from_isr` → `StdMutex::try_lock` |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task runs immediately after the ISR. | Pure try-lock with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `_from_isr` variants are semantically correct as non-blocking try-lock operations. |

---

### 17. Queue — Message Storage Strategy

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::new` → `xQueueGenericCreate` / `Queue::post` → `xQueueSendToBack` | `Queue::new` → `StdMutex<VecDeque<Vec<u8>>>` / `Queue::post` → `item.to_vec()` + `push_back` |
| **Behavior** | FreeRTOS pre-allocates a fixed-size kernel buffer at creation time.  Messages are memcpy'd into pre-allocated slots — no per-message heap allocation. | Messages are cloned into new `Vec<u8>` heap allocations on every `post()`.  The `VecDeque` dynamically grows/shrinks within the capacity limit. |
| **Mitigation** | N/A. | The functional contract is identical — both guarantee in-order delivery and bounded capacity.  For latency-sensitive workloads, allocate the queue once and reuse it; the heap overhead is negligible in development/test.  Deploy to FreeRTOS for deterministic memory behavior. |

---

### 18. Queue — Wake Strategy (Priority-Ordered Unblocking)

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::fetch` / `Queue::post` → internal `xQueueGenericSend` / `xQueueGenericReceive` | `Queue::fetch` / `Queue::post` → `Condvar::notify_one` |
| **Behavior** | When a message is posted, FreeRTOS unblocks the **highest-priority** task waiting on the queue.  When a message is fetched, the highest-priority blocked sender is woken. | `Condvar::notify_one` wakes one waiter in OS-scheduler-dependent order (typically FIFO, not priority).  Thread priorities on Linux are informational only. |
| **Mitigation** | Built into the kernel. | The wake order does not impact correctness for development/test workloads.  Deploy to FreeRTOS for priority-ordered wake-up. |

---

### 19. Queue — Resource Destruction

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::delete` / `Drop` → `vQueueDelete` + set handle to null | `Queue::delete` / `Drop` → `Queue::close()` which sets `closed` flag + `Condvar::notify_all` on both Condvars |
| **Behavior** | FreeRTOS frees the kernel queue object and sets the handle pointer to null.  Any task blocked on the queue is unblocked. | Linux sets a `closed` flag and notifies all waiting threads via both Condvars so they unblock with `Error::QueueClosed`.  `close()` is idempotent.  Rust reclaims the `StdMutex` + `Condvar` + `VecDeque` memory when `self` is dropped. |
| **Mitigation** | N/A. | Both backends unblock waiting tasks and reclaim resources.  On Linux, blocking operations return `Error::QueueClosed` instead of `Error::Timeout`, allowing callers to distinguish queue closure from time-outs (see §22). |

---

### 20. Queue — Copy In/Out vs In-Place Deserialization

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `QueueStreamed<T>::fetch` → deserialize from `Vec<u8>` using `T::from_bytes` | `QueueStreamed<T>::fetch` → deserialize from `Vec<u8>` using `T::from_bytes` (or serde) |
| **Behavior** | Both backends allocate a temporary `Vec<u8>` for the raw message, then deserialize into the caller's `&mut T` buffer.  The OSAL contract requires message-size consistency — the `Vec` capacity equals `T::len()`. | Identical logic.  The Linux backend explicitly copies from `VecDeque<Vec<u8>>` (which already contains a `Vec<u8>`) into the temporary `Vec`, then deserializes — one extra copy compared to the FreeRTOS kernel's direct memcpy from its internal buffer. |
| **Mitigation** | N/A. | The extra copy is negligible for development/test workloads and does not affect the public API contract.  Both backends pass the same test suite. |

---

### 21. Queue — Close Lifecycle

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Queue::delete` → `vQueueDelete` | `Queue::delete` / `Queue::close` → set `closed = true` + `notify_all` on both Condvars |
| **Behavior** | FreeRTOS deletes the queue and unblocks all waiting tasks, but the return value for unblocked tasks is undefined. | Linux explicitly makes all `post` / `fetch` operations return `Error::QueueClosed` (not `Error::Timeout`) once the queue is closed. `close()` is idempotent. `Drop` also calls `close()`. |
| **Mitigation** | N/A. | `Error::QueueClosed` is a Linux-backend-specific error variant. Portable code should handle both `Error::Timeout` and `Error::QueueClosed`. |

---

## Thread

### 22. Thread — Suspend / Resume

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::suspend` → `vTaskSuspend` / `Thread::resume` → `vTaskResume` | `Thread::suspend` / `Thread::resume` — empty bodies |
| **Behavior** | FreeRTOS atomically suspends/resumes the target task. The suspended task stops executing immediately. | Linux user space cannot atomically suspend another thread. No-op. |
| **Mitigation** | N/A. | Documented no-op. Application code should not rely on `suspend`/`resume` for synchronization on Linux. |

---

### 23. Thread — Stack High Water Mark

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::get_metadata` → `uxTaskGetStackHighWaterMark` | `Thread::get_metadata` → fills `stack_depth` as-is |
| **Behavior** | FreeRTOS tracks the minimum remaining stack space ever recorded. | Linux fills `stack_high_water_mark` with the initial `stack_depth` — no runtime tracking. |
| **Mitigation** | N/A. | Stack overflow detection requires separate tooling (e.g., valgrind, ASan) on Linux. |

---

### 24. Thread — Priority-Ordered Notification Wake

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::notify` / `Thread::wait_notification` → `xTaskNotify` / `xTaskNotifyWait` | `Thread::notify` / `Thread::wait_notification` → `StdMutex::lock` + `Condvar` |
| **Behavior** | FreeRTOS task notifications use priority-ordered wake-up. If multiple tasks are waiting on notifications, the highest-priority task is unblocked first. | Linux uses `Condvar::notify_all` — all waiters wake and compete for the lock. |
| **Mitigation** | N/A. | Wake order does not impact correctness for development/test workloads on Linux — thread priorities are informational only. |

---

### 25. Thread — ISR Context Switch

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::notify_from_isr` → `xTaskNotifyFromISR` + `System::yield_from_isr` | `Thread::notify_from_isr` → `StdMutex::try_lock` + `Condvar::notify_all` |
| **Behavior** | On success, signals the scheduler to perform a context switch so a higher-priority task runs immediately after the ISR. | Pure non-blocking notification with no context switch. |
| **Mitigation** | Built into the kernel. | Linux has no ISR context; `notify_from_isr` is semantically correct as a non-blocking operation. |

---

### 26. Thread — Handle-Based Constructors & Introspection

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::new_with_handle`, `new_with_to_priority`, `new_with_handle_and_to_priority`, `get_metadata_from_handle`, `get_metadata`, `wait_notification_with_to_tick` | Same signatures — fully implemented via `ThreadRegistry` |
| **Behavior** | `new_with_handle` wraps an existing FreeRTOS task handle. `get_metadata_from_handle` queries the kernel via `vTaskGetInfo`. | `ThreadRegistry` is backed by a global `OnceLock<StdMutex<ThreadRegistry>>` providing `register_thread`, `lookup_by_handle`, `lookup_current`, `unregister_thread`. `get_metadata_from_handle()` queries the registry and returns real metadata. `get_current()` prefers the registry; if the current thread is not registered, it lazily registers it. `new_with_handle()` creates a new `Thread` and registers it (ignoring the passed-in handle, substituting an auto-incrementing ID). `new_with_handle_and_to_priority()` follows the same pattern. |
| **Mitigation** | N/A. | The registry is now fully functional. Use `Thread::new()` + `spawn()` for Linux thread creation. |

---

### 27. Thread — Cooperative Cancellation

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::delete` → `vTaskDelete` | `Thread::delete` → sets `delete_requested = true` + `Condvar::notify_all()` |
| **Behavior** | FreeRTOS `vTaskDelete` immediately terminates the target task, freeing its stack and TCB. | Linux cannot force-terminate a `std::thread`. `delete()` sets a cooperative cancellation flag and wakes blocked waiters. The callback should poll `is_delete_requested()` or `is_cancellation_requested()` and return naturally. |
| **Mitigation** | Built into the kernel. | Documented as a cooperative cancellation model. Long-running callbacks should periodically check the cancellation flag. |

---

### 28. Thread — Join Support

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | No equivalent `join` API — `vTaskDelete` makes the task vanish immediately. | `Thread::join(&mut retval) -> Result<i32>` — uses `JoinHandle::join()` to wait for the OS thread to complete. |
| **Behavior** | FreeRTOS has no thread reclamation mechanism after deletion. | Linux `join()` blocks until the target thread exits, reclaims OS resources, and unregisters the thread from the registry. Returns `Error::ThreadNotStarted` if the thread was never started, `Error::ThreadAlreadyJoined` if already joined. |
| **Mitigation** | N/A. | `join()` is a Linux-backend extension capability not present in the FreeRTOS backend trait. |

---

### 29. Thread — Notification System

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Thread::notify` → `xTaskNotify` / `Thread::wait_notification` → `xTaskNotifyWait` | `Thread::notify` → `StdMutex<ThreadInner>` + `Condvar::notify_all()` / `Thread::wait_notification` → `Condvar::wait` / `Condvar::wait_timeout` |
| **Behavior** | FreeRTOS task notifications wake the **highest-priority** waiting task. | Linux uses `Condvar::notify_all` — all waiters wake and compete for the lock. The notification value (32-bit) supports `ThreadNotification` enum variants: `NoAction`, `SetBits`, `Increment`, `SetValueWithOverwrite`, `SetValueWithoutOverwrite`. |
| **Mitigation** | Built into the kernel. | Wake order does not affect correctness — waiters check their condition and re-wait or return. `SetValueWithoutOverwrite` returns `Error::QueueFull` when a pending notification already exists. |

---

## Timer

### 30. Timer — Scheduler Architecture

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Timer::new` → `xTimerCreate` registers with the timer daemon task | `Timer::new` creates a dedicated worker `std::thread` per timer |
| **Behavior** | FreeRTOS uses a single timer service task that processes all timers from a command queue. Callbacks run sequentially in the daemon context. | Each timer spawns its own OS thread at construction time. The worker blocks on a `Condvar` waiting for commands or deadline expiry, then fires the callback outside the internal lock. |
| **Mitigation** | N/A. | The per-timer thread model is functionally equivalent — callbacks still run sequentially per timer. For deeply embedded use cases, deploy to FreeRTOS to avoid per-timer thread overhead. |

---

### 31. Timer — Scheduling Precision

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | Timer expiry triggered by the kernel tick interrupt | Timer expiry via `Condvar::wait_timeout` with precise deadline calculation |
| **Behavior** | FreeRTOS timers expire at the next tick boundary after the period elapses (typically ±1 tick jitter). | Each timer's worker thread uses `Condvar::wait_timeout(deadline - now)` to wait for the exact remaining time. In the `Stopped` state, the worker blocks indefinitely on `Condvar::wait`. In the `Armed` state, it computes the remaining time to the deadline. Precision depends on OS scheduling granularity (typically ±1 ms or better), not a fixed polling interval. |
| **Mitigation** | N/A. | Acceptable for development/test workloads. Deploy to FreeRTOS for hard real-time timer guarantees. |

---

### 32. Timer — Command Queue vs Synchronous Operations

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `start` / `stop` / `reset` / `change_period` → send command to timer daemon queue | `start` / `stop` / `reset` / `change_period` → directly mutate shared state + notify worker via `Condvar` |
| **Behavior** | FreeRTOS uses an internal command queue for timer operations. If the queue is full, the caller blocks up to `ticks_to_wait`. | Linux ignores `ticks_to_wait` — all operations are synchronous and cannot block (no bounded queue). |
| **Mitigation** | `ticks_to_wait` is implemented as `_ticks_to_wait: TickType` (unused). | Application code should not rely on `ticks_to_wait` for timer API calls on Linux. |

---

### 33. Timer — Resource Destruction

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Timer::delete` / `Drop` → `xTimerDelete` | `Timer::delete` / `Drop` → `shutdown()` + `worker_shutdown()` |
| **Behavior** | FreeRTOS asynchronously deletes the timer object and frees kernel resources. | `shutdown()` sets the state to `Deleted`, clears the deadline, and bumps the generation counter. `worker_shutdown()` wakes the worker via `Condvar::notify_all`, takes the `JoinHandle`, and — if the calling thread is not the worker itself — calls `JoinHandle::join()` to wait for the OS thread to exit. `Timer` uses `Arc<TimerCore>` with `public_handles: AtomicUsize` reference counting; `Clone` increments the count, `Drop` decrements it, and the last handle triggers `shutdown()`. |
| **Mitigation** | N/A. | Non-self-join deletions block until the worker thread exits, ensuring clean resource reclamation. Self-join (deleting a timer from within its own callback) drops the `JoinHandle` without joining, and the worker exits on its next iteration. |

---

## Memory

### 34. Memory — Heap Allocation

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `get_free_heap_size` → `xPortGetFreeHeapSize` | `System::get_free_heap_size` returns `usize::MAX` |
| **Behavior** | FreeRTOS pre-allocates a fixed-size heap; `get_free_heap_size` reports remaining bytes — object creation can fail with `OutOfMemory`. | Linux provides virtual memory; Rust allocations almost never fail. Returns `usize::MAX` — there is no RTOS heap, and the process can allocate as much as the OS permits. |
| **Mitigation** | N/A. | `RawMutex::new` uses `unwrap()`. Testing allocation failure would require additional `#[cfg]` endpoints. `usize::MAX` satisfies all `> 0` assertions in portable tests. |

---

## Cross-Cutting

### 35. Handle Deref Compatibility

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | `Deref<Target=XxxHandle>` returns the real FreeRTOS kernel handle for every OS object (`Thread`, `Queue`, `Semaphore`, `Mutex`, `EventGroup`, `Timer`). | `Deref<Target=XxxHandle>` returns a monotonically increasing atomic ID — not a dereferenceable pointer |
| **Behavior** | The handle can be passed to C FFI functions or used for low-level FreeRTOS API calls. | Each module maintains its own `AtomicUsize` counter (`NEXT_QUEUE_HANDLE`, `NEXT_SEMAPHORE_HANDLE`, `NEXT_MUTEX_HANDLE`, `NEXT_EVENT_GROUP_HANDLE`, `NEXT_TIMER_HANDLE`, `NEXT_THREAD_ID`). On each `new()`, `fetch_add` generates a unique ID cast to `XxxHandle = *const c_void`. The value is **not** a valid pointer — it serves as an opaque unique identifier for comparison and diagnostics only. |
| **Mitigation** | N/A. | This is purely a compile-time API compatibility shim. Application code must not dereference handle values on Linux. |

---

### 36. Poison Recovery

| | FreeRTOS | Linux |
|---|---|---|
| **Function** | N/A — FreeRTOS has no mutex poisoning concept. | All Linux primitives (`RawMutex`, `Mutex<T>`, `Queue`, `Semaphore`, `EventGroup`, `Thread`, `System` critical section, `Timer`) use `recover_lock()` to recover from poisoned `StdMutex`. |
| **Behavior** | N/A. | If a thread panics while holding a Rust `StdMutex`, the mutex becomes "poisoned." `recover_lock()` unpacks the `PoisonError` and continues using the inner data, ensuring that one thread's panic does not permanently disable a synchronization primitive. Each module includes `#[cfg(test)]` tests verifying that primitives remain usable after a panic. |
| **Mitigation** | N/A. | Recovered data may be inconsistent — callers should perform their own validation. FreeRTOS has no mutex poisoning, so this behavior is a Linux-specific safety net. |