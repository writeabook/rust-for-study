# FreeRTOS ↔ POSIX 主机后端行为差异说明

> 记录 FreeRTOS 后端与 POSIX 主机后端之间已知的行为差异。这些差异不一定违反
> OSAL 行为契约，其中许多差异来自 POSIX 主机用户态不具备 RTOS 调度器、中断
> 上下文和确定性内存语义。两个后端通过同一套公共测试套件。

Linux 不再作为独立 OSAL 后端存在。当前 POSIX 主机后端通过
`posix/bsp/generic_linux` BSP 在 Linux 上进行验证。本文档中的主机侧差异
统一称为 POSIX 主机后端差异。

---

## Mutex（互斥锁）

### 1. Mutex — 优先级继承

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | 递归 mutex，通过 `xSemaphoreTakeRecursive` 实现 | 递归 mutex，通过 `pthread_mutex_t`（PTHREAD_MUTEX_RECURSIVE）或 `posix/sys` 封装实现 | POSIX 主机优先级受操作系统调度器和 pthread 属性影响 |
| **行为** | FreeRTOS 内核暂时提升 mutex 持有者优先级以防止优先级反转 | 无优先级提升——pthread mutex 公平但不会影响线程调度优先级 | 测试不应依赖优先级继承 |
| **处理** | 内核内置 | 主机上线程优先级是信息性的；部署到 FreeRTOS 获取实时优先级语义 | |

---

### 2. Mutex — ISR 上下文切换

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `lock_from_isr` → `xSemaphoreTakeFromISR` + 上下文切换 | `lock_from_isr` → 非阻塞 try-lock | POSIX 主机无 ISR 上下文 |
| **行为** | 成功时通知调度器进行上下文切换 | 纯 try-lock，无上下文切换 | 语义上正确的非阻塞操作 |
| **处理** | 内核内置 | `_from_isr` 变体是非阻塞兼容操作 | |

---

### 3. Mutex — 双层架构

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `RawMutex`（递归）+ `Mutex<T>` 封装同一递归原语 | `RawMutex`（递归，PTHREAD_MUTEX_RECURSIVE）+ `Mutex<T>`（非递归，PTHREAD_MUTEX_ERRORCHECK + `UnsafeCell<Box<T>>`） | OSAL 契约在 trait 要求处定义递归行为 |
| **行为** | FreeRTOS mutex 本质递归——`Mutex<T>` 在此基础上提供类型安全 RAII | `RawMutex` 遵循递归契约。`Mutex<T>` 非递归：同一线程重复加锁返回 `Error::MutexLockFailed` | 应用代码在 POSIX 主机上不得递归锁定 `Mutex<T>` |
| **处理** | 内核内置 | 非递归行为是有意设计；有文档和测试覆盖 | |

---

## System（系统）

### 4. System — 调度器启动/停止

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `start()` → `vTaskStartScheduler`（永不返回） | `start()` / `stop()` — 文档化 no-op | POSIX 用户态没有应用级 RTOS 调度器 |
| **行为** | 启动硬件调度器 | POSIX 线程在 `pthread_create` 后立即运行——无中央调度器 | 可移植代码不应依赖 `start()` 的副作用 |
| **处理** | 内核内置 | 文档化 no-op | |

---

### 5. System — 调度器挂起/恢复

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `suspend_all` → `vTaskSuspendAll` / `resume_all` → `xTaskResumeAll` | 空函数体 | POSIX 用户态无法原子地停止所有其他线程 |
| **行为** | 全局暂停任务切换 | No-op | 不得用于互斥——使用 `Mutex` |
| **处理** | N/A | 文档化 no-op | |

---

### 6. System — 临界区

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | 关闭指定优先级的中断 | 进程内递归 `pthread_mutex_t`（PTHREAD_MUTEX_RECURSIVE） | POSIX 用户态无法关闭中断 |
| **行为** | 硬件级真正原子性 | 进程内 OSAL 调用者间互斥，per-thread 嵌套深度通过 `pthread_key_t` TLS 管理。**不**关闭操作系统调度或硬件中断 | 不得在主机上依赖真原子性 |
| **处理** | 内核内置 | 用 `Mutex` 保护数据；模拟临界区防止 OSAL 调用者间竞争 | |

---

### 7. System — ISR 支持

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `yield_from_isr` / `end_switching_isr` → 调度器钩子 | 空函数体 | POSIX 主机用户态无 ISR 上下文 |
| **行为** | 通知调度器进行上下文切换 | No-op | API 保留以保持兼容性；`_from_isr` 变体是非阻塞的 |
| **处理** | N/A | 文档化 no-op | |

---

### 8. System — Tick 溢出行为

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `get_tick_count` → `xTaskGetTickCount`（32 位） | `get_tick_count` → `clock_gettime(CLOCK_MONOTONIC)` 单调时钟 | POSIX 时钟提供稳定单调时间 |
| **行为** | `TickType(u32)` 约 49 天回绕；`check_timer` 有溢出安全分支 | 单调纳秒时钟→tick 转换；`check_timer` 使用 `Duration` 运算 | `wrapping_sub` 是跨后端安全的惯用法 |
| **处理** | `wrapping_sub` 修正回绕 | 测试中进程不会运行 49 天；输出等价 | |

---

### 9. System — 线程内省

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `count_threads` → `uxTaskGetNumberOfTasks` / `get_all_thread` → `uxTaskGetSystemState` | `count_threads` → 注册表查询 / `get_all_thread` → `snapshot_registered_threads()` | POSIX 注册表由 `pthread_once_t` + `PosixMutex` + `BTreeMap` 支撑 |
| **行为** | FreeRTOS 维护完整内核任务列表 | 动态 `ThreadRegistry`，主线程惰性注册；返回完整 `SystemState` 快照 | 两个后端通过相同的内省测试 |
| **处理** | 内核内置 | 注册表功能完整 | |

---

## Semaphore（信号量）

### 10. Semaphore — ISR 上下文切换

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `wait_from_isr` / `signal_from_isr` → `xSemaphoreTakeFromISR` / `xSemaphoreGiveFromISR` + 上下文切换 | `wait_from_isr` / `signal_from_isr` → 非阻塞 try-lock + 计数逻辑 | POSIX 主机无 ISR 上下文 |
| **行为** | 成功时通知调度器进行上下文切换 | 纯非阻塞操作，无上下文切换 | `_from_isr` 变体是非阻塞兼容 API |
| **处理** | 内核内置 | 语义上正确的非阻塞操作 | |

---

### 11. Semaphore — 最高优先级等待者唤醒

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `signal` → `xSemaphoreGive` | `signal` → POSIX mutex + 条件变量通知 | POSIX 主机按操作系统调度器行为唤醒一个等待者 |
| **行为** | FreeRTOS 唤醒等待信号量的最高优先级任务 | POSIX 条件变量唤醒一个等待者；无 FreeRTOS 风格优先级排序 | 主机上线程优先级是信息性的 |
| **处理** | 内核内置 | 唤醒顺序不影响正确性；部署到 FreeRTOS 获取优先级排序唤醒 | |

---

## EventGroup（事件组）

### 12. EventGroup — ISR 上下文切换

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `set_from_isr` → `xEventGroupSetBitsFromISR` + 上下文切换 | `set_from_isr` → 非阻塞 try-lock + broadcast | POSIX 主机无 ISR 上下文 |
| **行为** | 成功时通知调度器进行上下文切换 | 纯非阻塞位设置，无上下文切换 | 语义上正确的非阻塞操作 |
| **处理** | 内核内置 | `_from_isr` 变体是非阻塞兼容 API | |

---

### 13. EventGroup — ISR 忙锁行为

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `get_from_isr` → `xEventGroupGetBitsFromISR`（直接 ISR 安全读取） | `get_from_isr` → 非阻塞 try-lock | POSIX 主机无 ISR 上下文 |
| **行为** | 无论锁定状态，始终返回当前位值 | 如果其他线程持有锁，返回 `0`（静默降级） | 应用代码应在关键读取处使用 `get()` |
| **处理** | N/A | 主机上 `get_from_isr` 仅用于信息性用途 | |

---

### 14. EventGroup — 唤醒策略

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `set` → `xEventGroupSetBits` | `set` → POSIX mutex lock + 条件变量 broadcast | POSIX 条件变量可能更广泛唤醒 |
| **行为** | FreeRTOS 仅唤醒条件满足的等待者（精确唤醒） | 广播至所有等待者；条件不满足者重新检查并重新等待 | 虚假唤醒由条件循环处理；功能正确 |
| **处理** | 内核内置 | 轻微开销，功能等价 | |

---

### 15. EventGroup — 资源销毁

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `delete` / `Drop` → `vEventGroupDelete` | `delete` / `Drop` — 空函数体 | POSIX 主机没有内核资源需要释放 |
| **行为** | 释放内核事件组对象 | Rust 自动回收 POSIX mutex + 条件变量内存 | 文档化 no-op；不得依赖 `delete()` 做同步 |
| **处理** | N/A | 资源清理是自动的 | |

---

## Queue（队列）

### 16. Queue — ISR 上下文切换

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `fetch_from_isr` / `post_from_isr` → `xQueueReceiveFromISR` / `xQueueSendToBackFromISR` + 上下文切换 | `fetch_from_isr` / `post_from_isr` → 非阻塞 try-lock | POSIX 主机无 ISR 上下文 |
| **行为** | 成功时通知调度器进行上下文切换 | 纯 try-lock，无上下文切换 | 语义上正确的非阻塞操作 |
| **处理** | 内核内置 | `_from_isr` 变体是非阻塞兼容 API | |

---

### 17. Queue — 消息存储策略

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | 创建时预分配固定大小内核缓冲区 | 内部使用堆分配的主机数据结构 | OSAL 契约保证有界容量、FIFO、消息大小检查、超时、错误报告 |
| **行为** | 消息 memcpy 至预分配槽位——无每次消息的堆分配 | 消息可能内部使用主机堆分配。功能契约完全一致 | 确定性内存需求部署到 FreeRTOS |
| **处理** | N/A | 功能契约一致；堆开销在开发/测试中可忽略 | |

---

### 18. Queue — 唤醒策略（优先级排序）

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `fetch` / `post` → 内部 `xQueueGenericSend` / `xQueueGenericReceive` | `fetch` / `post` → POSIX 条件变量通知 | POSIX 主机按操作系统调度器行为唤醒 |
| **行为** | FreeRTOS 唤醒最高优先级等待任务 | 操作系统调度器相关顺序；无 FreeRTOS 风格优先级排序 | 唤醒顺序不影响正确性 |
| **处理** | 内核内置 | 部署到 FreeRTOS 获取优先级排序唤醒 | |

---

### 19. Queue — 资源销毁

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `delete` / `Drop` → `vQueueDelete` + 空句柄 | `delete` / `Drop` → `close()` 设置关闭标志 + condvar broadcast | 两个后端都唤醒等待任务并回收资源 |
| **行为** | 释放内核队列对象，唤醒等待任务 | 设置关闭标志，通过两个 condvar 通知所有等待者。阻塞操作返回 `Error::QueueClosed` | `close()` 是幂等的；可移植代码应同时处理超时和关闭错误 |
| **处理** | N/A | `Error::QueueClosed` 使调用者能区分关闭与超时 | |

---

### 20. Queue — 类型化队列序列化（QueueStreamed\<T\>）

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `QueueStreamed<T>` 在队列传输前序列化类型化消息 | 相同 OSAL 级行为 | 需 `serde` feature 和 `T: Serialize + Deserialize + BytesHasLen` |
| **行为** | FIFO 和超时行为必须匹配原始队列契约 | 与底层队列相同的 FIFO 和超时行为 | 序列化/反序列化失败必须报告为 OSAL 错误；不允许部分消息 |
| **处理** | N/A | 两个后端共享相同 `QueueStreamed` 抽象 | |

---

### 21. Queue — 关闭生命周期

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `delete` → `vQueueDelete` | `delete` / `close` → 设置 `closed = true` + 两个 condvar broadcast | POSIX 关闭是显式且幂等的 |
| **行为** | 释放队列，唤醒等待任务；返回值未定义 | 所有待处理和未来的操作返回 `Error::QueueClosed`；`Drop` 也会调用 `close()` | 可移植代码应同时处理 `Error::Timeout` 和 `Error::QueueClosed` |
| **处理** | N/A | `Error::QueueClosed` 在主机侧测试中提供可移植性优势 | |

---

## Thread（线程）

### 22. Thread — 挂起/恢复

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `suspend` → `vTaskSuspend` / `resume` → `vTaskResume` | 空函数体 | POSIX 用户态无法原子地挂起另一个线程 |
| **行为** | 原子地挂起/恢复目标任务 | No-op | 不得在主机上依赖 `suspend`/`resume` 做同步 |
| **处理** | N/A | 文档化 no-op | |

---

### 23. Thread — 栈高水位标记

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `get_metadata` → `uxTaskGetStackHighWaterMark` | `get_metadata` → 原样填充 `stack_depth` | POSIX 主机无运行时栈水印跟踪 |
| **行为** | 跟踪有记录以来的最小剩余栈空间 | 报告初始 `stack_depth`——无运行时跟踪 | 栈溢出检测需外部工具（valgrind、ASan） |
| **处理** | N/A | 主机上用外部工具做栈分析 | |

---

### 24. Thread — 优先级排序通知唤醒

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `notify` / `wait_notification` → `xTaskNotify` / `xTaskNotifyWait` | `notify` / `wait_notification` → POSIX mutex lock + condvar broadcast | POSIX 主机线程优先级是信息性的 |
| **行为** | FreeRTOS 任务通知使用优先级排序唤醒 | 广播至所有等待者；它们竞争锁 | 唤醒顺序不影响正确性 |
| **处理** | N/A | 主机上线程优先级是信息性的 | |

---

### 25. Thread — ISR 上下文切换

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `notify_from_isr` → `xTaskNotifyFromISR` + 上下文切换 | `notify_from_isr` → 非阻塞 try-lock + broadcast | POSIX 主机无 ISR 上下文 |
| **行为** | 成功时通知调度器进行上下文切换 | 纯非阻塞通知，无上下文切换 | 语义上正确的非阻塞操作 |
| **处理** | 内核内置 | `_from_isr` 变体是非阻塞兼容 API | |

---

### 26. Thread — 基于句柄的构造和内省

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `new_with_handle` 封装已有 FreeRTOS 任务句柄；`get_metadata_from_handle` 查询内核 | 相同签名——通过 `ThreadRegistry` 完整实现 | POSIX 注册表由 `pthread_once_t` + `PosixMutex` + `BTreeMap` 支撑 |
| **行为** | 通过 `vTaskGetInfo` 内核查询 | 注册表提供 `register_thread`、`lookup_by_handle`、`lookup_current`、`unregister_thread`。主线程惰性注册 | 两个后端通过相同的内省测试 |
| **处理** | N/A | 注册表功能完整 | |

---

### 27. Thread — 协作取消

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `delete` → `vTaskDelete`（立即终止） | `delete` → 设置 `delete_requested = true` + condvar broadcast | POSIX 无法强制终止 pthread |
| **行为** | 立即终止目标任务，释放栈和 TCB | 协作取消：回调应轮询 `is_delete_requested()` 并自然返回 | 长时间运行的回调应定期检查取消标志 |
| **处理** | 内核内置 | 文档化协作取消模型 | |

---

### 28. Thread — Join 支持

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | 无等价功能——`vTaskDelete` 使任务立即消失 | `join` → 通过 `posix/sys/thread` 调用 `pthread_join` | POSIX 线程创建时是可 join 的 |
| **行为** | FreeRTOS 删除后无线程回收机制 | `join()` 阻塞直到目标线程退出，从注册表注销，回收 pthread 资源 | 返回 `Error::ThreadNotStarted` / `Error::ThreadAlreadyJoined` |
| **处理** | N/A | Join 以回收主机线程资源 | |

---

### 29. Thread — 通知系统

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `notify` → `xTaskNotify` / `wait_notification` → `xTaskNotifyWait` | `notify` → POSIX mutex lock + condvar broadcast / `wait_notification` → condvar timed-wait（`CLOCK_MONOTONIC` 截止时间） | 32 位通知值支持全部 `ThreadNotification` 枚举变体 |
| **行为** | FreeRTOS 任务通知唤醒最高优先级等待任务 | 广播至所有等待者；它们重新检查并重新等待或返回 | `SetValueWithoutOverwrite` 在已有待处理通知时返回 `Error::QueueFull` |
| **处理** | 内核内置 | 唤醒顺序不影响正确性 | |

---

## Timer（定时器）

### 30. Timer — 调度器架构

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `Timer::new` 向 FreeRTOS 定时器守护任务注册 | 所有定时器共享一个全局 detached pthread worker（进程生命周期） | POSIX worker 通过 `pthread_once_t` 启动一次 |
| **行为** | FreeRTOS 使用单个定时器服务任务；回调在守护上下文顺序执行 | 一个后台 worker 线程；回调在锁外执行；回调后状态在锁下更新 | 开发/测试中功能等价 |
| **处理** | N/A | 部署到 FreeRTOS 获取硬实时定时器保证 | |

---

### 31. Timer — 调度精度

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | 定时器到期由内核 tick 中断触发 | 通过 `pthread_cond_timedwait` + `CLOCK_MONOTONIC` 绝对截止时间 | POSIX 精度取决于操作系统调度粒度 |
| **行为** | 下一 tick 边界到期（±1 tick 抖动） | 等待至截止时间；精度通常 ±1 ms 或更好，非 tick 基准 | 实时精度取决于后端 |
| **处理** | N/A | 开发/测试中可接受；部署到 FreeRTOS 获取硬实时 | |

---

### 32. Timer — 命令队列 vs 同步操作

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `start` / `stop` / `reset` / `change_period` → 发送命令至定时器守护队列 | 直接修改共享状态 + 通过 condvar 通知 worker | POSIX 操作是同步的；调用者不会阻塞在有界队列上 |
| **行为** | FreeRTOS 使用内部命令队列；队列满时调用者阻塞 | `ticks_to_wait` 接受以保持 API 兼容，但主机上没有有界队列语义 | 可移植代码不应依赖 `ticks_to_wait` 副作用 |
| **处理** | `ticks_to_wait` 在 FreeRTOS 可能阻塞 | 文档化为兼容参数 | |

---

### 33. Timer — 资源销毁

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `delete` / `Drop` → `xTimerDelete` | `delete` / `Drop` → 设状态 `Deleted` + 递增代际 + condvar 信号 | POSIX 使用基于代际的惰性淘汰 |
| **行为** | 异步删除定时器对象并释放内核资源 | Id=0 的句柄（回调临时对象）跳过删除以避免死锁。过期堆条目由代际检查过滤 | 回调内自删除安全 |
| **处理** | N/A | 通过代际计数器干净回收资源 | |

---

## Memory（内存）

### 34. Memory — 堆分配

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `get_free_heap_size` → `xPortGetFreeHeapSize` | `get_free_heap_size` 返回 `usize::MAX` | POSIX 主机提供虚拟内存 |
| **行为** | 固定大小预分配堆；对象创建可能因 `OutOfMemory` 失败 | Libc malloc 几乎从不失败；返回 `usize::MAX`——无 RTOS 堆 | `usize::MAX` 满足可移植测试中所有 `> 0` 断言 |
| **处理** | N/A | 分配失败测试需 `#[cfg]` 端点 | |

---

## Cross-Cutting（跨模块）

### 35. Handle Deref 兼容性

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | `Deref<Target=XxxHandle>` 返回真实 FreeRTOS 内核句柄 | `Deref<Target=XxxHandle>` 返回单调递增原子 ID——不是可解引用指针 | POSIX 句柄是不透明标识符 |
| **行为** | 句柄可传递给 C FFI 或低级 FreeRTOS API | 各模块使用 `AtomicUsize` 计数器。`fetch_add` 生成唯一 ID 并转换为 `*const c_void` | 应用代码不得在 POSIX 主机上解引用句柄值 |
| **处理** | N/A | 编译时 API 兼容 shim；句柄是唯一不透明标识符 | |

---

### 36. Poison Recovery（中毒恢复）

| 方面 | FreeRTOS 后端 | POSIX 主机后端 | 处理方式 |
|---|---|---|---|
| **功能** | N/A——FreeRTOS 无 mutex 中毒概念 | POSIX no_std 后端使用 `panic=abort`——无 unwind，无中毒 | 线程回调和定时器回调不得 panic |
| **行为** | N/A | 回调 panic 则进程 abort。无中毒 mutex 恢复路径 | panic 处理由调用者负责；使用 `panic=abort` 或在回调内处理错误 |
| **处理** | N/A | POSIX no_std 后端不在 pthread 边界捕获 panic | |
