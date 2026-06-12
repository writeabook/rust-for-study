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
| **缓解措施** | 不适用。 | 不得用于互斥（使用 `Mutex` 替代）。已文档化。 |

---

## 5. System — 临界区

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `System::enter_critical` / `System::critical_section_enter` → 禁用中断 | `System::enter_critical` / `System::critical_section_enter` — 空函数体 |
| **行为** | 禁用中断到可配置的优先级级别，提供真正的原子性。 | 用户空间无法禁用中断。 |
| **缓解措施** | 不适用。 | 不得在 Linux 测试中用于保护共享数据（使用 `Mutex` 替代）。已文档化为无操作。 |

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
| **函数** | `System::count_threads` → `uxTaskGetNumberOfTasks` / `System::get_all_thread` → `uxTaskGetSystemState` | `System::count_threads` 返回 `1` / `System::get_all_thread` 返回空 `SystemState` |
| **行为** | FreeRTOS 维护完整的任务列表（名称、优先级、状态、栈高水位）。 | Linux 后端尚无内部线程注册表（v0.1）。 |
| **缓解措施** | 内置于内核。 | 相关测试在 Linux 下标记 `#[cfg(feature = "freertos")]`。未来可能维护注册表。 |

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
| **行为** | FreeRTOS 预分配固定大小的堆，`get_free_heap_size` 返回可用字节数——对象创建可能因 `OutOfMemory` 失败。 | Linux 提供虚拟内存；Rust 分配几乎永不失败。 |
| **缓解措施** | 不适用。 | `RawMutex::new` 使用 `unwrap()`。测试分配失败需额外 `#[cfg]` 端点。可在未来的版本中添加。 |

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
| **函数** | `Queue::delete` / `Drop` → `vQueueDelete` + 句柄置 null | `Queue::delete` / `Drop` → 设置 `closed` 标志 + `Condvar::notify_all` |
| **行为** | FreeRTOS 释放内核队列对象并将句柄指针置 null。阻塞在队列上的任何任务被解除阻塞。 | Linux 设置 `closed` 标志并通知所有等待线程，使其以 `Error::Timeout` 解除阻塞。Rust 在 `self` 释放时回收 `StdMutex` + `Condvar` + `VecDeque` 内存。 |
| **缓解措施** | 不适用。 | 两后端均解除阻塞等待任务并回收资源。应用代码不应依赖契约之外的删除后行为。 |

---

## 20. Queue — 拷贝进出 vs 原地反序列化

| | FreeRTOS | Linux |
|---|---|---|
| **函数** | `QueueStreamed<T>::fetch` → 使用 `T::from_bytes` 从 `Vec<u8>` 反序列化 | `QueueStreamed<T>::fetch` → 使用 `T::from_bytes`（或 serde）从 `Vec<u8>` 反序列化 |
| **行为** | 两后端均为原始消息分配临时 `Vec<u8>`，然后反序列化到调用者的 `&mut T` 缓冲区。OSAL 契约要求消息大小一致性——`Vec` 容量等于 `T::len()`。 | 逻辑相同。Linux 后端显式地从 `VecDeque<Vec<u8>>`（其中已包含 `Vec<u8>`）拷贝到临时 `Vec`，然后反序列化——相比 FreeRTOS 内核从其内部缓冲区直接 memcpy，多了一次额外拷贝。 |
| **缓解措施** | 不适用。 | 额外拷贝在开发/测试中可忽略，不影响公共 API 契约。两后端通过相同的测试套件。 |
