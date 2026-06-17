# FreeRTOS ↔ Linux Backend Alignment Gaps

> 记录已实现模块中 FreeRTOS 与 Linux 后端行为未完全对齐的特性。
> 差异来源于 Linux 用户空间的固有限制，不违反 OSAL trait 契约——
> 两后端通过相同的公共测试套件。

---

## 1. Mutex — 优先级继承

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `RawMutex::lock` → `xSemaphoreTakeRecursive` | `RawMutex::lock` → `StdMutex::lock` + `Condvar` |
| **行为** | FreeRTOS 内核自动将持有 mutex 的低优先级线程提升到等待线程的最高优先级，防止优先级反转。 | 无优先级提升。`std::sync::Mutex` 公平但不影响线程调度优先级。 |
| **缓解措施** | 内置于内核。 | Linux 上优先级仅作信息用途，开发/测试无需处理。需真实时间行为时部署至 FreeRTOS。 |

---

## 2. Mutex — ISR 上下文切换

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `RawMutex::lock_from_isr` → `xSemaphoreTakeFromISR` + `System::yield_from_isr` | `RawMutex::lock_from_isr` → `StdMutex::try_lock` |
| **行为** | ISR 成功后通知调度器进行上下文切换，让更高优先级任务立即运行。 | 纯 try-lock，无上下文切换。 |
| **缓解措施** | 内置于内核。 | Linux 无 ISR 上下文，`lock_from_isr` 作为非阻塞 try-lock 语义正确。 |

---

## 3. System — 调度器启动/停止

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `System::start()` → `vTaskStartScheduler` / `System::stop()` → `vTaskEndScheduler` | `System::start()` / `System::stop()` — 空函数体 |
| **行为** | `start()` 启动硬件调度器且永不返回。 | 无操作。Linux 线程通过 `std::thread::spawn` 直接运行，无中央调度器。 |
| **缓解措施** | 内置于内核。 | 已文档化为无操作。应用代码不应依赖 `start()` 的副作用。 |

---

## 4. System — 调度器挂起/恢复

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `System::suspend_all` → `vTaskSuspendAll` / `System::resume_all` → `xTaskResumeAll` | `System::suspend_all` / `System::resume_all` — 空函数体 |
| **行为** | 全局暂停任务切换。 | Linux 用户空间无法原子地停止所有其他线程。 |
| **缓解措施** | 不适用。 | 不得用于互斥（使用 `Mutex` 替代）。已文档化为无操作。 |

---

## 5. System — 临界区

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `System::enter_critical` / `System::critical_section_enter` → 禁用中断 | `System::enter_critical` / `System::critical_section_enter` → `enter_global_critical()` —— 获取进程级递归锁 |
| **行为** | 禁用中断到可配置的优先级级别，提供真正的原子性。 | 使用全局 `StdMutex<()>`（`OnceLock` 初始化）提供进程内所有 OSAL 调用者之间的互斥。通过 `thread_local!`（`CriticalThreadState`）跟踪每线程嵌套深度，同一线程可嵌套调用。`enter_critical_from_isr()` 返回嵌套前的深度作为保存的中断状态。**此锁不禁用 Linux 中断或 OS 调度**——仅提供进程内的互斥。 |
| **缓解措施** | 内置于内核。 | 不得依赖 Linux 上的真正原子性（使用 `Mutex` 替代）。模拟临界区防止 OSAL 调用者之间的数据竞争，但不提供硬实时保证。 |

---

## 6. System — ISR 支持

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `System::yield_from_isr` / `System::end_switching_isr` | `System::yield_from_isr` / `System::end_switching_isr` — 空函数体 |
| **行为** | 向调度器发出信号进行上下文切换。 | Linux 用户空间既无也无运行 ISR。 |
| **缓解措施** | 不适用。 | API 保留用于兼容性。`_from_isr` 变体自身已实现为非阻塞。 |

---

## 7. System — Tick 溢出行为

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `System::get_tick_count` → `xTaskGetTickCount` (32-bit) / `System::check_timer` | `System::get_tick_count` → `Instant::elapsed` (64-bit) / `System::check_timer` |
| **行为** | `TickType(u32)` 约 49 天后回绕。`check_timer` 有明确的溢出安全分支 (`CpuRegisterSize::Bit32`)。 | `std::time::Instant` 为 64 位单调时钟。`check_timer` 使用 `Duration` 运算，无需回绕处理。 |
| **缓解措施** | `wrapping_sub` 是跨后端的安全做法。 | 进程在测试中不会运行 49 天。实际输出等效。 |

---

## 8. System — 线程内省

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `System::count_threads` → `uxTaskGetNumberOfTasks` / `System::get_all_thread` → `uxTaskGetSystemState` | `System::count_threads` → `thread::count_registered_threads()` / `System::get_all_thread` → `snapshot_registered_threads()` 返回 `SystemState` |
| **行为** | FreeRTOS 维护完整的任务列表（名称、优先级、状态、栈高水位）。 | Linux 维护动态 `ThreadRegistry`（`HashMap<usize, Weak<ThreadCore>>` + `HashMap<ThreadId, usize>`），由全局 `OnceLock<StdMutex<ThreadRegistry>>` 支持。`ensure_main_thread_registered()` 懒注册主线程。`count_threads()` 返回注册线程数。`get_all_thread()` 返回完整 `SystemState` 快照。`get_state()` 返回当前线程的 `ThreadState`。 |
| **缓解措施** | 内置于内核。 | 注册表现已完全功能化。两后端通过相同的内省测试。 |

---

## 9. Semaphore — ISR 上下文切换

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Semaphore::wait_from_isr` / `Semaphore::signal_from_isr` → `xSemaphoreTakeFromISR` / `xSemaphoreGiveFromISR` + `System::yield_from_isr` | `Semaphore::wait_from_isr` / `Semaphore::signal_from_isr` → `StdMutex::try_lock` + 计数逻辑 |
| **行为** | ISR 成功后通知调度器进行上下文切换，让更高优先级任务立即运行。 | 纯非阻塞操作，无上下文切换。 |
| **缓解措施** | 内置于内核。 | Linux 无 ISR 上下文，`_from_isr` 变体作为非阻塞 try-lock 操作语义正确。 |

---

## 10. Semaphore — 最高优先级等待者唤醒

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Semaphore::signal` → `xSemaphoreGive` | `Semaphore::signal` → `Condvar::notify_one` |
| **行为** | FreeRTOS 唤醒等待信号量的**最高优先级**任务。 | `Condvar::notify_one` 按 FIFO 顺序唤醒一个等待者（或根据操作系统调度器的任意顺序）。 |
| **缓解措施** | 内置于内核。 | Linux 上线程优先级仅作信息用途；唤醒顺序不影响开发/测试的正确性。需要优先级顺序唤醒时部署至 FreeRTOS。 |

---

## 11. Memory — 堆分配

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `get_free_heap_size` → `xPortGetFreeHeapSize` | `System::get_free_heap_size` 返回 `usize::MAX` |
| **行为** | FreeRTOS 预分配固定大小的堆，`get_free_heap_size` 返回可用字节数——对象创建可能因 `OutOfMemory` 失败。 | Linux 提供虚拟内存；Rust 分配几乎永不失败。返回 `usize::MAX`——没有 RTOS 堆限制，进程可分配至操作系统允许的上限。 |
| **缓解措施** | 不适用。 | `RawMutex::new` 使用 `unwrap()`。测试分配失败需额外 `#[cfg]` 端点。`usize::MAX` 满足所有可移植测试中的 `> 0` 断言。 |

---

## 12. EventGroup — ISR 上下文切换

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `EventGroup::set_from_isr` → `xEventGroupSetBitsFromISR` + `System::yield_from_isr` | `EventGroup::set_from_isr` → `StdMutex::try_lock` + `Condvar::notify_all` |
| **行为** | 成功后通知调度器进行上下文切换，让被位设置解除阻塞的更高优先级任务立即运行。 | 纯非阻塞位设置，无上下文切换。 |
| **缓解措施** | 内置于内核。 | Linux 无 ISR 上下文；`set_from_isr` 作为非阻塞操作语义正确。 |

---

## 13. EventGroup — ISR 锁忙行为

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `EventGroup::get_from_isr` → `xEventGroupGetBitsFromISR` | `EventGroup::get_from_isr` → `StdMutex::try_lock` |
| **行为** | FreeRTOS 提供直接 ISR 安全读取，始终返回当前位，无论事件组是否被锁定。 | Linux 使用 `StdMutex::try_lock`——如果另一个线程持有锁，`get_from_isr` 返回 `0`（静默回退）。 |
| **缓解措施** | 不适用。 | Linux 无 ISR 上下文；`get_from_isr` 方法仅供信息用途。应用代码应使用 `get()` 进行关键读取。 |

---

## 14. EventGroup — 唤醒策略

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `EventGroup::set` → `xEventGroupSetBits` | `EventGroup::set` → `StdMutex::lock` + `Condvar::notify_all` |
| **行为** | FreeRTOS 仅唤醒其条件被新设置的位**满足**的等待者（精确唤醒）。 | Linux 通过 `notify_all()` 唤醒**所有**等待线程——条件尚未满足的线程将检查并重新进入 `Condvar::wait_timeout`。 |
| **缓解措施** | 内置于内核。 | 虚假唤醒由检查等待条件的循环处理。额外的唤醒会增加微小开销，但功能上正确。 |

---

## 15. EventGroup — 资源销毁

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `EventGroup::delete` / `Drop` → `vEventGroupDelete` | `EventGroup::delete` / `Drop` — 空函数体 |
| **行为** | FreeRTOS 释放内核事件组对象并将句柄设为 null。 | Linux 无内核资源需释放；Rust 自动回收 `StdMutex` + `Condvar` 内存。 |
| **缓解措施** | 不适用。 | 已文档化为无操作。应用代码不应依赖 `delete()` 进行同步。 |

---

## 16. Queue — ISR 上下文切换

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Queue::fetch_from_isr` → `xQueueReceiveFromISR` + `System::yield_from_isr` / `Queue::post_from_isr` → `xQueueSendToBackFromISR` + `System::yield_from_isr` | `Queue::fetch_from_isr` / `Queue::post_from_isr` → `StdMutex::try_lock` |
| **行为** | ISR 成功后通知调度器进行上下文切换，让更高优先级任务立即运行。 | 纯 try-lock，无上下文切换。 |
| **缓解措施** | 内置于内核。 | Linux 无 ISR 上下文；`_from_isr` 变体作为非阻塞 try-lock 操作语义正确。 |

---

## 17. Queue — 消息存储策略

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Queue::new` → `xQueueGenericCreate` / `Queue::post` → `xQueueSendToBack` | `Queue::new` → `StdMutex<VecDeque<Vec<u8>>>` / `Queue::post` → `item.to_vec()` + `push_back` |
| **行为** | FreeRTOS 在创建时从内核预分配固定大小的缓冲区。消息通过 memcpy 拷贝到预分配的槽位——无按消息的堆分配。 | 每次 `post()` 都将消息克隆到新的 `Vec<u8>` 堆分配中。`VecDeque` 在容量限制内动态增减。 |
| **缓解措施** | 不适用。 | 功能契约相同——两者都保证有序传递和有界容量。对延迟敏感的工作负载，一次性分配队列并复用；堆开销在开发/测试中可忽略。需要确定性内存行为时部署至 FreeRTOS。 |

---

## 18. Queue — 唤醒策略（优先级有序解除阻塞）

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Queue::fetch` / `Queue::post` → 内部 `xQueueGenericSend` / `xQueueGenericReceive` | `Queue::fetch` / `Queue::post` → `Condvar::notify_one` |
| **行为** | 当投递消息时，FreeRTOS 唤醒等待队列中**最高优先级**的任务。当获取消息时，唤醒最高优先级的阻塞发送者。 | `Condvar::notify_one` 按操作系统调度器依赖的顺序唤醒一个等待者（通常为 FIFO，非优先级）。Linux 上线程优先级仅作信息用途。 |
| **缓解措施** | 内置于内核。 | 唤醒顺序不影响开发/测试的正确性。需要优先级有序唤醒时部署至 FreeRTOS。 |

---

## 19. Queue — 资源销毁

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Queue::delete` / `Drop` → `vQueueDelete` + 句柄置 null | `Queue::delete` / `Drop` → `Queue::close()` 设置 `closed` 标志 + 两个 Condvar 上 `Condvar::notify_all` |
| **行为** | FreeRTOS 释放内核队列对象并将句柄指针置 null。阻塞在队列上的任何任务被解除阻塞。 | Linux 设置 `closed` 标志并通知所有等待线程（通过两个 Condvar），使其以 `Error::QueueClosed` 解除阻塞。`close()` 是幂等的。Rust 在 `self` 释放时回收 `StdMutex` + `Condvar` + `VecDeque` 内存。 |
| **缓解措施** | 不适用。 | 两后端均解除阻塞等待任务并回收资源。在 Linux 上，阻塞操作返回 `Error::QueueClosed` 而非 `Error::Timeout`，允许调用者区分队列关闭与超时（见 §35）。 |

---

## 20. Queue — 拷贝进出 vs 原地反序列化

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `QueueStreamed<T>::fetch` → 使用 `T::from_bytes` 从 `Vec<u8>` 反序列化 | `QueueStreamed<T>::fetch` → 使用 `T::from_bytes`（或 serde）从 `Vec<u8>` 反序列化 |
| **行为** | 两后端均为原始消息分配临时 `Vec<u8>`，然后反序列化到调用者的 `&mut T` 缓冲区。OSAL 契约要求消息大小一致性——`Vec` 容量等于 `T::len()`。 | 逻辑相同。Linux 后端显式地从 `VecDeque<Vec<u8>>`（其中已包含 `Vec<u8>`）拷贝到临时 `Vec`，然后反序列化——相比 FreeRTOS 内核从其内部缓冲区直接 memcpy，多了一次额外拷贝。 |
| **缓解措施** | 不适用。 | 额外拷贝在开发/测试中可忽略，不影响公共 API 契约。两后端通过相同的测试套件。 |

---

## 21. Thread — 挂起/恢复

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Thread::suspend` → `vTaskSuspend` / `Thread::resume` → `vTaskResume` | `Thread::suspend` / `Thread::resume` — 空函数体 |
| **行为** | FreeRTOS 原子地挂起/恢复目标任务。挂起的任务立即停止执行。 | Linux 用户空间无法原子地挂起另一个线程。无操作。 |
| **缓解措施** | 不适用。 | 已文档化为无操作。应用代码不应在 Linux 上依赖 `suspend`/`resume` 进行同步。 |

---

## 22. Thread — 栈高水位标记

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Thread::get_metadata` → `uxTaskGetStackHighWaterMark` | `Thread::get_metadata` → 直接填入 `stack_depth` |
| **行为** | FreeRTOS 记录历史最小剩余栈空间。 | Linux 用初始 `stack_depth` 填充 `stack_high_water_mark`——无运行时跟踪。 |
| **缓解措施** | 不适用。 | Linux 上栈溢出检测需要单独的工具（如 valgrind、ASan）。 |

---

## 23. Thread — 优先级有序的通知唤醒

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Thread::notify` / `Thread::wait_notification` → `xTaskNotify` / `xTaskNotifyWait` | `Thread::notify` / `Thread::wait_notification` → `StdMutex::lock` + `Condvar` |
| **行为** | FreeRTOS 任务通知使用按优先级排序的唤醒。如果多个任务正在等待通知，最高优先级任务首先解除阻塞。 | Linux 使用 `Condvar::notify_all`——所有等待者唤醒并竞争锁。 |
| **缓解措施** | 不适用。 | Linux 上线程优先级仅作信息用途，唤醒顺序不影响开发/测试的正确性。 |

---

## 24. Thread — ISR 上下文切换

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Thread::notify_from_isr` → `xTaskNotifyFromISR` + `System::yield_from_isr` | `Thread::notify_from_isr` → `StdMutex::try_lock` + `Condvar::notify_all` |
| **行为** | 成功后通知调度器进行上下文切换，让更高优先级任务在 ISR 之后立即运行。 | 纯非阻塞通知，无上下文切换。 |
| **缓解措施** | 内置于内核。 | Linux 无 ISR 上下文；`notify_from_isr` 作为非阻塞操作语义正确。 |

---

## 25. Timer — 调度器架构

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Timer::new` → `xTimerCreate` 在定时器守护任务中注册 | `Timer::new` 为每个定时器创建专用的 `std::thread` 工作线程 |
| **行为** | FreeRTOS 使用一个定时器服务任务处理所有定时器。回调在守护任务上下文中顺序执行。 | 每个定时器在构造时生成自己的操作系统线程。工作线程在 `Condvar` 上等待命令或 deadline 到期，然后在内部锁之外触发回调。 |
| **缓解措施** | 不适用。 | 每定时器一线程模型功能上等效——回调仍按定时器顺序执行。对于深度嵌入场景，部署至 FreeRTOS 以避免每定时器的线程开销。 |

---

## 26. Timer — 调度精度

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | 定时器到期由内核 tick 中断触发 | 定时器到期通过 `Condvar::wait_timeout` 加精确 deadline 计算 |
| **行为** | FreeRTOS 定时器在周期结束后下一个 tick 边界到期（通常 ±1 tick 抖动）。 | 每个定时器的工作线程使用 `Condvar::wait_timeout(deadline - now)` 等待精确的剩余时间。在 `Stopped` 状态下，工作线程在 `Condvar::wait` 上无限等待。在 `Armed` 状态下，计算到 deadline 的剩余时间。精度取决于操作系统调度粒度（通常 ±1 ms 或更好），而非固定轮询间隔。 |
| **缓解措施** | 不适用。 | 对于开发/测试工作负载可接受。需要硬实时定时器保证时部署至 FreeRTOS。 |

---

## 27. Timer — 命令队列 vs 同步操作

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `start` / `stop` / `reset` / `change_period` → 向定时器守护任务队列发送命令 | `start` / `stop` / `reset` / `change_period` → 直接修改共享状态 + 通过 `Condvar` 通知工作线程 |
| **行为** | FreeRTOS 为定时器操作使用内部命令队列。队列满时调用者阻塞至多 `ticks_to_wait`。 | Linux 忽略 `ticks_to_wait`——所有操作均为同步，不可阻塞（无有界队列）。 |
| **缓解措施** | `ticks_to_wait` 实现为 `_ticks_to_wait: TickType`（未使用）。 | 应用代码不应在 Linux 上依赖 `ticks_to_wait` 参数。 |

---

## 28. Timer — 资源销毁

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Timer::delete` / `Drop` → `xTimerDelete` | `Timer::delete` / `Drop` → `shutdown()` + `worker_shutdown()` |
| **行为** | FreeRTOS 异步删除定时器对象并释放内核资源。 | `shutdown()` 将状态设为 `Deleted`、清除 deadline、并递增 generation 计数器。`worker_shutdown()` 通过 `Condvar::notify_all` 唤醒工作线程，取走 `JoinHandle`，并在调用线程不是工作线程本身时调用 `JoinHandle::join()` 等待操作系统线程退出。`Timer` 使用 `Arc<TimerCore>` 及 `public_handles: AtomicUsize` 引用计数；`Clone` 递增计数，`Drop` 递减计数，最后一个句柄触发 `shutdown()`。 |
| **缓解措施** | 不适用。 | 非自 join 的删除会阻塞直到工作线程退出，确保干净的资源回收。自 join（在回调内删除定时器）会释放 `JoinHandle` 而不 join，工作线程在下一次迭代时退出。 |

---

## 29. 句柄 Deref 兼容性

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Deref<Target=XxxHandle>` 为每个 OS 对象（`Thread`、`Queue`、`Semaphore`、`Mutex`、`EventGroup`、`Timer`）返回真实的 FreeRTOS 内核句柄。 | `Deref<Target=XxxHandle>` 返回单调递增的原子 ID——不是可解引用的指针 |
| **行为** | 句柄可传递给 C FFI 函数或用于底层 FreeRTOS API 调用。 | 每个模块维护自己的 `AtomicUsize` 计数器（`NEXT_QUEUE_HANDLE`、`NEXT_SEMAPHORE_HANDLE`、`NEXT_MUTEX_HANDLE`、`NEXT_EVENT_GROUP_HANDLE`、`NEXT_TIMER_HANDLE`、`NEXT_THREAD_ID`）。每次 `new()` 时，`fetch_add` 生成唯一 ID 并转换为 `XxxHandle = *const c_void`。该值**不是**有效指针——仅作为不透明唯一标识符用于比较和诊断。 |
| **缓解措施** | 不适用。 | 这纯粹是编译期 API 兼容层。应用代码绝对不能在 Linux 上解引用句柄值。 |

---

## 30. Thread — 基于句柄的构造函数与内省

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Thread::new_with_handle`、`new_with_to_priority`、`new_with_handle_and_to_priority`、`get_metadata_from_handle`、`get_metadata`、`wait_notification_with_to_tick` | 相同签名——通过 `ThreadRegistry` 完整实现 |
| **行为** | `new_with_handle` 包装已有的 FreeRTOS 任务句柄。`get_metadata_from_handle` 通过 `vTaskGetInfo` 查询内核。 | `ThreadRegistry` 由全局 `OnceLock<StdMutex<ThreadRegistry>>` 支持，提供 `register_thread`、`lookup_by_handle`、`lookup_current`、`unregister_thread` 函数。`get_metadata_from_handle()` 查询注册表返回真实元数据。`get_current()` 优先查找注册表，若当前线程未注册则懒注册。`new_with_handle()` 创建新 `Thread` 并注册（忽略传入句柄，改用自增 ID）。`new_with_handle_and_to_priority()` 同理。 |
| **缓解措施** | 不适用。 | 注册表现已完全功能化。在 Linux 上使用 `Thread::new()` + `spawn()` 创建线程。 |

---

## 31. Mutex — 双层架构

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | 仅 `RawMutex`（递归互斥锁，通过 `xSemaphoreTakeRecursive`/`xSemaphoreGiveRecursive` 实现）。`Mutex<T>` 在 FreeRTOS 递归互斥锁上加 RAII Guard。 | `RawMutex`（递归：`StdMutex<State>` + `Condvar` + `owner: ThreadId` + `recursion: u32`）以及 `Mutex<T>`（非递归：`Box<StdMutex<T>>` 数据锁 + `StdMutex<Option<ThreadId>>` 所有权追踪）。 |
| **行为** | FreeRTOS 的互斥锁天然是递归的。`Mutex<T>` 在同一递归原语之上提供类型安全的 RAII。 | `RawMutex` 遵循契约（递归）。`Mutex<T>` 是**非递归**的——如果同一线程尝试锁定已持有的 `Mutex<T>`，返回 `Error::MutexLockFailed`。`Mutex<T>` 上的 `lock_from_isr` 实现为 `try_lock`（非阻塞）。 |
| **缓解措施** | 内置于内核。 | `Mutex<T>` 的非递归行为是刻意设计，与 FreeRTOS 后端的递归行为不同。应用代码绝对不能从同一线程递归锁定同一 `Mutex<T>`。 |

---

## 32. Thread — 协作式取消

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Thread::delete` → `vTaskDelete` | `Thread::delete` → 设置 `delete_requested = true` + `Condvar::notify_all()` |
| **行为** | FreeRTOS `vTaskDelete` 立即终止目标任务，释放其栈和 TCB。 | Linux 无法强制终止 `std::thread`。`delete()` 设置协作取消标志并唤醒阻塞的等待者。回调应轮询 `is_delete_requested()` 或 `is_cancellation_requested()` 并自然返回。 |
| **缓解措施** | 内置于内核。 | 文档化为协作取消模型。长期运行的回调应定期检查取消标志。 |

---

## 33. Thread — join 支持

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | 无等价的 `join` API——`vTaskDelete` 后任务立即消失。 | `Thread::join(&mut retval) -> Result<i32>` —— 使用 `JoinHandle::join()` 等待操作系统线程完成。 |
| **行为** | FreeRTOS 线程删除后无回收机制。 | Linux `join()` 阻塞直到目标线程退出，回收操作系统资源，并从注册表中注销线程。如果线程未启动返回 `Error::ThreadNotStarted`，如果已 join 返回 `Error::ThreadAlreadyJoined`。 |
| **缓解措施** | 不适用。 | `join()` 是 Linux 后端的扩展能力，不在 FreeRTOS 后端 trait 中。 |

---

## 34. Thread — 通知系统

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Thread::notify` → `xTaskNotify` / `Thread::wait_notification` → `xTaskNotifyWait` | `Thread::notify` → `StdMutex<ThreadInner>` + `Condvar::notify_all()` / `Thread::wait_notification` → `Condvar::wait` / `Condvar::wait_timeout` |
| **行为** | FreeRTOS 任务通知唤醒**最高优先级**的等待任务。 | Linux 使用 `Condvar::notify_all`——所有等待者唤醒并竞争锁。通知值（32 位）支持 `ThreadNotification` 枚举变体：`NoAction`、`SetBits`、`Increment`、`SetValueWithOverwrite`、`SetValueWithoutOverwrite`。 |
| **缓解措施** | 内置于内核。 | 唤醒顺序不影响正确性——等待者检查条件后重新等待或返回。`SetValueWithoutOverwrite` 在已有待处理通知时返回 `Error::QueueFull`。 |

---

## 35. Queue — close 生命周期

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `Queue::delete` → `vQueueDelete` | `Queue::delete` / `Queue::close` → 设置 `closed = true` + 两个 Condvar 上 `notify_all` |
| **行为** | FreeRTOS 删除队列时解除阻塞所有等待任务，但等待任务的返回值未定义。 | Linux 在 `closed` 状态下明确使所有 `post`/`fetch` 操作返回 `Error::QueueClosed`（而非 `Error::Timeout`）。`close()` 是幂等的。`Drop` 也会调用 `close()`。 |
| **缓解措施** | 不适用。 | `Error::QueueClosed` 是 Linux 后端特有的错误变体，FreeRTOS 后端不使用。可移植代码应同时处理 `Error::Timeout` 和 `Error::QueueClosed`。 |

---

## 36. Poison 恢复

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | 不适用——FreeRTOS 没有 mutex poison 概念。 | 所有 Linux 原语（`RawMutex`、`Mutex<T>`、`Queue`、`Semaphore`、`EventGroup`、`Thread`、`System` 临界区、`Timer`）使用 `recover_lock()` 从 poisoned `StdMutex` 恢复。 |
| **行为** | 不适用。 | 如果某个线程在持有 Rust `StdMutex` 时 panic，该 mutex 变为 "poisoned"。`recover_lock()` 解包 `PoisonError` 并继续使用内部数据，保证一个线程的 panic 不会永久禁用同步原语。各模块包含 `#[cfg(test)]` 测试验证 panic 后原语仍可用。 |
| **缓解措施** | 不适用。 | 恢复后的数据可能不一致——调用者需自行验证。FreeRTOS 没有 mutex poison，因此此行为是 Linux 特有的安全保障。 |