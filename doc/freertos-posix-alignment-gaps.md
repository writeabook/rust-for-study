# FreeRTOS ↔ POSIX Host Backend Alignment Gaps

> Documenting known behavioral differences between the FreeRTOS backend
> and the POSIX host backend.  These differences do not necessarily violate
> the OSAL contract; many stem from the fact that POSIX host user space
> does not provide RTOS scheduler, ISR, or deterministic memory semantics.
> Both backends pass the same public test suite.

Linux is not a standalone OSAL backend.  The current POSIX host backend
is validated on Linux through `posix/bsp/generic_linux`.

---

## Mutex

### 1. Mutex — Priority Inheritance

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | Recursive mutex via FreeRTOS `xSemaphoreTakeRecursive` | Recursive mutex via `pthread_mutex_t` (PTHREAD_MUTEX_RECURSIVE) or internal `posix/sys` wrappers | POSIX host priority depends on OS scheduler and pthread attributes |
| **Behavior** | FreeRTOS kernel temporarily elevates priority of the mutex holder to prevent priority inversion | No priority boosting — pthread mutex is fair but does not influence thread scheduling priorities | Tests should not depend on priority inheritance |
| **Mitigation** | Built into the kernel | Thread priorities are informational on host; deploy to FreeRTOS for real-time priority semantics | |

---

### 2. Mutex — ISR Context Switch

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `lock_from_isr` → `xSemaphoreTakeFromISR` + context-switch yield | `lock_from_isr` → non-blocking try-lock | POSIX host has no ISR context |
| **Behavior** | On success, signals the scheduler for a context switch | Pure try-lock — no context switch | Semantically correct as a non-blocking try-lock |
| **Mitigation** | Built into the kernel | `_from_isr` variants are non-blocking compatibility operations | |

---

### 3. Mutex — Dual-Layer Architecture

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `RawMutex` (recursive) + `Mutex<T>` wrapping the same recursive primitive | `RawMutex` (recursive, PTHREAD_MUTEX_RECURSIVE) + `Mutex<T>` (non-recursive, PTHREAD_MUTEX_ERRORCHECK + `UnsafeCell<Box<T>>`) | OSAL contract defines recursive behavior where required by the trait |
| **Behavior** | FreeRTOS mutexes are inherently recursive — `Mutex<T>` provides type-safe RAII on top | `RawMutex` follows the recursive contract. `Mutex<T>` is non-recursive: re-locking from the same thread returns `Error::MutexLockFailed` | Application code must not recursively lock `Mutex<T>` on POSIX host |
| **Mitigation** | Built into the kernel | Non-recursive behavior is intentional; documented and covered by tests | |

---

## System

### 4. System — Scheduler Start / Stop

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `start()` → `vTaskStartScheduler` (never returns) | `start()` / `stop()` — documented no-ops | POSIX user space has no application-level RTOS scheduler |
| **Behavior** | Launches the hardware scheduler | POSIX threads run immediately after `pthread_create` — no central scheduler | Portable code should not rely on `start()` side effects |
| **Mitigation** | Built into the kernel | Documented no-op | |

---

### 5. System — Scheduler Suspend / Resume

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `suspend_all` → `vTaskSuspendAll` / `resume_all` → `xTaskResumeAll` | Empty bodies | POSIX user space cannot atomically stop all other threads |
| **Behavior** | Globally pauses task switches | No-op | Must not be used for mutual exclusion — use `Mutex` |
| **Mitigation** | N/A | Documented no-op | |

---

### 6. System — Critical Sections

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | Disables interrupts up to a configurable priority level | Process-local recursive `pthread_mutex_t` (PTHREAD_MUTEX_RECURSIVE) | POSIX user space cannot disable interrupts |
| **Behavior** | True atomicity at the hardware level | Mutual exclusion among OSAL callers within the process, with per-thread nesting depth via `pthread_key_t` TLS. Does **not** disable OS scheduling or hardware interrupts | Must not be relied on for real atomicity on host |
| **Mitigation** | Built into the kernel | Use `Mutex` for data protection; the simulated critical section prevents races among OSAL callers | |

---

### 7. System — ISR Support

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `yield_from_isr` / `end_switching_isr` → scheduler hooks | Empty bodies | POSIX host user space has no ISR context |
| **Behavior** | Signals the scheduler for a context switch | No-op | APIs retained for compatibility; `_from_isr` variants are non-blocking |
| **Mitigation** | N/A | Documented no-ops | |

---

### 8. System — Tick Overflow Behavior

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `get_tick_count` → `xTaskGetTickCount` (32-bit) | `get_tick_count` → `clock_gettime(CLOCK_MONOTONIC)` monotonic clock | POSIX clock provides stable monotonic time |
| **Behavior** | `TickType(u32)` wraps after ~49 days; `check_timer` has overflow-safe branch | Monotonic nanosecond clock → tick conversion; `check_timer` uses `Duration` arithmetic | `wrapping_sub` is the cross-backend-safe idiom |
| **Mitigation** | `wrapping_sub` corrects for wrap | Processes do not run for 49 days in tests; outputs equivalent | |

---

### 9. System — Thread Introspection

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `count_threads` → `uxTaskGetNumberOfTasks` / `get_all_thread` → `uxTaskGetSystemState` | `count_threads` → registry query / `get_all_thread` → `snapshot_registered_threads()` | POSIX registry backed by `pthread_once_t` + `PosixMutex` + `BTreeMap` |
| **Behavior** | FreeRTOS maintains a complete kernel task list | Dynamic `ThreadRegistry` with lazy main-thread registration; returns full `SystemState` snapshot | Both backends pass the same introspection tests |
| **Mitigation** | Built into the kernel | Registry is fully functional | |

---

## Semaphore

### 10. Semaphore — ISR Context Switch

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `wait_from_isr` / `signal_from_isr` → `xSemaphoreTakeFromISR` / `xSemaphoreGiveFromISR` + context-switch yield | `wait_from_isr` / `signal_from_isr` → non-blocking try-lock + count logic | POSIX host has no ISR context |
| **Behavior** | On success, signals scheduler for context switch | Pure non-blocking operations, no context switch | `_from_isr` variants are non-blocking compatibility APIs |
| **Mitigation** | Built into the kernel | Semantically correct as non-blocking operations | |

---

### 11. Semaphore — Highest-Priority Waiter Unblocking

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `signal` → `xSemaphoreGive` | `signal` → POSIX mutex + condition-variable notification | POSIX host wakes one waiter per OS scheduler behavior |
| **Behavior** | FreeRTOS unblocks the highest-priority waiting task | POSIX condition-variable wakes one waiter; no FreeRTOS-style priority ordering | Thread priorities are informational on host |
| **Mitigation** | Built into the kernel | Wake order does not impact correctness; deploy to FreeRTOS for priority-ordered wake-up | |

---

## EventGroup

### 12. EventGroup — ISR Context Switch

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `set_from_isr` → `xEventGroupSetBitsFromISR` + context-switch yield | `set_from_isr` → non-blocking try-lock + broadcast | POSIX host has no ISR context |
| **Behavior** | On success, signals scheduler for context switch | Pure non-blocking bit-set, no context switch | Semantically correct as a non-blocking operation |
| **Mitigation** | Built into the kernel | `_from_isr` variants are non-blocking compatibility APIs | |

---

### 13. EventGroup — ISR Busy-Lock Behavior

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `get_from_isr` → `xEventGroupGetBitsFromISR` (direct ISR-safe read) | `get_from_isr` → non-blocking try-lock | POSIX host has no ISR context |
| **Behavior** | Always returns current bits regardless of lock state | If another thread holds the lock, returns `0` (silent fallback) | Application code should use `get()` for critical reads |
| **Mitigation** | N/A | `get_from_isr` is informational only on host | |

---

### 14. EventGroup — Wake Strategy

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `set` → `xEventGroupSetBits` | `set` → POSIX mutex lock + condition-variable broadcast | POSIX condition-variable may wake more broadly |
| **Behavior** | FreeRTOS wakes only waiters whose conditions are satisfied (precise) | Broadcasts to all waiters; unsatisfied waiters re-check and re-wait | Spurious wake-ups handled by condition loop; functionally correct |
| **Mitigation** | Built into the kernel | Minor overhead but functionally equivalent | |

---

### 15. EventGroup — Resource Destruction

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `delete` / `Drop` → `vEventGroupDelete` | `delete` / `Drop` — empty body | POSIX host has no kernel resources to free |
| **Behavior** | Deallocates kernel event group object | Rust reclaims POSIX mutex + condition variable memory automatically | Documented no-op; do not rely on `delete()` for synchronization |
| **Mitigation** | N/A | Resource cleanup is automatic | |

---

## Queue

### 16. Queue — ISR Context Switch

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `fetch_from_isr` / `post_from_isr` → `xQueueReceiveFromISR` / `xQueueSendToBackFromISR` + context-switch yield | `fetch_from_isr` / `post_from_isr` → non-blocking try-lock | POSIX host has no ISR context |
| **Behavior** | On success, signals scheduler for context switch | Pure try-lock, no context switch | Semantically correct as non-blocking operations |
| **Mitigation** | Built into the kernel | `_from_isr` variants are non-blocking compatibility APIs | |

---

### 17. Queue — Message Storage Strategy

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | Pre-allocated fixed-size kernel buffer at creation time | Heap-backed host data structures internally | OSAL contract guarantees bounded capacity, FIFO, message-size checking, timeout, error reporting |
| **Behavior** | Messages memcpy'd into pre-allocated slots — no per-message heap allocation | Messages may use host heap allocation internally. Functional contract identical | For deterministic memory, deploy to FreeRTOS |
| **Mitigation** | N/A | The functional contract is identical; heap overhead negligible in dev/test | |

---

### 18. Queue — Wake Strategy (Priority-Ordered Unblocking)

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `fetch` / `post` → internal `xQueueGenericSend` / `xQueueGenericReceive` | `fetch` / `post` → POSIX condition-variable notification | POSIX host wakes one waiter per OS scheduler behavior |
| **Behavior** | FreeRTOS unblocks the highest-priority waiting task | OS-scheduler-dependent order; no FreeRTOS-style priority ordering | Wake order does not impact correctness |
| **Mitigation** | Built into the kernel | Deploy to FreeRTOS for priority-ordered wake-up | |

---

### 19. Queue — Resource Destruction

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `delete` / `Drop` → `vQueueDelete` + null handle | `delete` / `Drop` → `close()` sets closed flag + condvar broadcast | Both backends unblock waiting tasks and reclaim resources |
| **Behavior** | Frees kernel queue object, unblocks waiting tasks | Sets closed flag, notifies all waiters via both condvars. Blocking ops return `Error::QueueClosed` | `close()` is idempotent; portable code should handle both timeout and closed errors |
| **Mitigation** | N/A | `Error::QueueClosed` allows callers to distinguish closure from timeout | |

---

### 20. Queue — Typed Queue Serialization (QueueStreamed\<T\>)

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `QueueStreamed<T>` serializes typed messages before queue transfer | Same OSAL-level behavior | Requires `serde` feature and `T: Serialize + Deserialize + BytesHasLen` |
| **Behavior** | FIFO and timeout must match raw queue contract | Same FIFO and timeout behavior as underlying queue | Serialization/deserialization failures must be reported as OSAL errors; no partial messages |
| **Mitigation** | N/A | Both backends share the same `QueueStreamed` abstraction | |

---

### 21. Queue — Close Lifecycle

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `delete` → `vQueueDelete` | `delete` / `close` → set `closed = true` + broadcast on both condvars | POSIX close is explicit and idempotent |
| **Behavior** | Frees queue, unblocks waiting tasks; return value undefined | All pending and future operations return `Error::QueueClosed`; `Drop` also calls `close()` | Portable code should handle both `Error::Timeout` and `Error::QueueClosed` |
| **Mitigation** | N/A | `Error::QueueClosed` gives portability advantage for host-side tests | |

---

## Thread

### 22. Thread — Suspend / Resume

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `suspend` → `vTaskSuspend` / `resume` → `vTaskResume` | Empty bodies | POSIX user space cannot atomically suspend another thread |
| **Behavior** | Atomically suspends/resumes the target task | No-op | Do not rely on `suspend`/`resume` for synchronization on host |
| **Mitigation** | N/A | Documented no-op | |

---

### 23. Thread — Stack High Water Mark

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `get_metadata` → `uxTaskGetStackHighWaterMark` | `get_metadata` → fills `stack_depth` as-is | POSIX host has no runtime stack-watermark tracking |
| **Behavior** | Tracks minimum remaining stack space | Reports initial `stack_depth` — no runtime tracking | Stack overflow detection requires external tooling (valgrind, ASan) |
| **Mitigation** | N/A | Use external tools for stack analysis on host | |

---

### 24. Thread — Priority-Ordered Notification Wake

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `notify` / `wait_notification` → `xTaskNotify` / `xTaskNotifyWait` | `notify` / `wait_notification` → POSIX mutex lock + condition-variable broadcast | POSIX host thread priorities are informational |
| **Behavior** | FreeRTOS task notifications use priority-ordered wake-up | Broadcast to all waiters; they compete for the lock | Wake order does not impact correctness |
| **Mitigation** | N/A | Thread priorities are informational on host | |

---

### 25. Thread — ISR Context Switch

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `notify_from_isr` → `xTaskNotifyFromISR` + context-switch yield | `notify_from_isr` → non-blocking try-lock + broadcast | POSIX host has no ISR context |
| **Behavior** | On success, signals scheduler for context switch | Pure non-blocking notification, no context switch | Semantically correct as a non-blocking operation |
| **Mitigation** | Built into the kernel | `_from_isr` variants are non-blocking compatibility APIs | |

---

### 26. Thread — Handle-Based Constructors & Introspection

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `new_with_handle` wraps existing FreeRTOS task handle; `get_metadata_from_handle` queries kernel | Same signatures — fully implemented via `ThreadRegistry` | POSIX registry backed by `pthread_once_t` + `PosixMutex` + `BTreeMap` |
| **Behavior** | Kernel queries via `vTaskGetInfo` | Registry provides `register_thread`, `lookup_by_handle`, `lookup_current`, `unregister_thread`. Lazy main-thread registration | Both backends pass the same introspection tests |
| **Mitigation** | N/A | Registry is fully functional | |

---

### 27. Thread — Cooperative Cancellation

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `delete` → `vTaskDelete` (immediate termination) | `delete` → sets `delete_requested = true` + condvar broadcast | POSIX cannot force-terminate a pthread |
| **Behavior** | Immediately terminates the target task, frees stack and TCB | Cooperative cancellation: the callback should poll `is_delete_requested()` and return naturally | Long-running callbacks should periodically check the cancellation flag |
| **Mitigation** | Built into the kernel | Documented cooperative cancellation model | |

---

### 28. Thread — Join Support

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | No equivalent — `vTaskDelete` makes the task vanish | `join` → `pthread_join` via `posix/sys/thread` | POSIX threads are joinable when created |
| **Behavior** | FreeRTOS has no thread reclamation after deletion | `join()` blocks until target thread exits, unregisters from registry, reclaims pthread resources | Returns `Error::ThreadNotStarted` / `Error::ThreadAlreadyJoined` |
| **Mitigation** | N/A | Join to reclaim host thread resources | |

---

### 29. Thread — Notification System

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `notify` → `xTaskNotify` / `wait_notification` → `xTaskNotifyWait` | `notify` → POSIX mutex lock + condvar broadcast / `wait_notification` → condvar timed-wait with `CLOCK_MONOTONIC` deadline | 32-bit notification value supports all `ThreadNotification` enum variants |
| **Behavior** | FreeRTOS task notifications wake the highest-priority waiting task | Broadcast to all waiters; they re-check and re-wait or return | `SetValueWithoutOverwrite` returns `Error::QueueFull` when a pending notification exists |
| **Mitigation** | Built into the kernel | Wake order does not affect correctness | |

---

## Timer

### 30. Timer — Scheduler Architecture

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `Timer::new` registers with the FreeRTOS timer daemon task | Single global detached pthread worker for ALL timers (process-lifetime) | POSIX worker started once via `pthread_once_t` |
| **Behavior** | FreeRTOS uses a single timer service task; callbacks run sequentially in daemon context | One background worker thread; callbacks execute outside the lock; post-callback state applied under lock | Functionally equivalent for dev/test workloads |
| **Mitigation** | N/A | Deploy to FreeRTOS for hard real-time timer guarantees | |

---

### 31. Timer — Scheduling Precision

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | Timer expiry triggered by kernel tick interrupt | Timer expiry via `pthread_cond_timedwait` with `CLOCK_MONOTONIC` absolute deadlines | POSIX precision depends on OS scheduling granularity |
| **Behavior** | Expires at next tick boundary (±1 tick jitter) | Waits until deadline; precision typically ±1 ms or better, not tick-based | Real-time precision is backend-dependent |
| **Mitigation** | N/A | Acceptable for dev/test; deploy to FreeRTOS for hard real-time | |

---

### 32. Timer — Command Queue vs Synchronous Operations

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `start` / `stop` / `reset` / `change_period` → send command to timer daemon queue | Directly mutates shared state + signals worker via condvar | POSIX operations are synchronous; caller does not block on a bounded queue |
| **Behavior** | FreeRTOS uses internal command queue; caller blocks if queue is full | `ticks_to_wait` is accepted for API compatibility but has no bounded-queue meaning on host | Portable code should not rely on `ticks_to_wait` side effects |
| **Mitigation** | `ticks_to_wait` may block on FreeRTOS | Documented as compatibility parameter | |

---

### 33. Timer — Resource Destruction

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `delete` / `Drop` → `xTimerDelete` | `delete` / `Drop` → sets state `Deleted` + generation bump + condvar signal | POSIX uses generation-based lazy invalidation |
| **Behavior** | Asynchronously deletes timer object and frees kernel resources | Id=0 handles (callback temporaries) skip delete to avoid deadlock. Stale heap entries filtered by generation check | Self-deletion from within callback is safe |
| **Mitigation** | N/A | Clean resource reclamation via generation counters | |

---

## Memory

### 34. Memory — Heap Allocation

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `get_free_heap_size` → `xPortGetFreeHeapSize` | `get_free_heap_size` returns `usize::MAX` | POSIX host provides virtual memory |
| **Behavior** | Fixed-size pre-allocated heap; object creation can fail with `OutOfMemory` | Libc malloc almost never fails; returns `usize::MAX` — no RTOS heap | `usize::MAX` satisfies all `> 0` assertions in portable tests |
| **Mitigation** | N/A | Allocation failure testing requires `#[cfg]` endpoints | |

---

## Cross-Cutting

### 35. Handle Deref Compatibility

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | `Deref<Target=XxxHandle>` returns the real FreeRTOS kernel handle | `Deref<Target=XxxHandle>` returns a monotonically increasing atomic ID — not a dereferenceable pointer | POSIX handles are opaque identifiers |
| **Behavior** | Handle can be passed to C FFI or low-level FreeRTOS API calls | Each module uses `AtomicUsize` counter. `fetch_add` generates unique ID cast to `*const c_void` | Application code must not dereference handle values on POSIX host |
| **Mitigation** | N/A | Compile-time API compatibility shim; handles are unique opaque identifiers | |

---

### 36. Poison Recovery

| Aspect | FreeRTOS backend | POSIX host backend | Notes / Mitigation |
|---|---|---|---|
| **Function** | N/A — FreeRTOS has no mutex poisoning | POSIX no_std backend uses `panic=abort` — no unwind, no poisoning | Thread and timer callbacks must not panic |
| **Behavior** | N/A | If a callback panics, the process aborts. No poisoned-mutex recovery path | Panic handling is the caller's responsibility; use `panic=abort` or handle errors inside callbacks |
| **Mitigation** | N/A | POSIX no_std backend does not catch panics across pthread boundaries | |
