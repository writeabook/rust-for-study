# `osal-rs` OSAL 行为契约

## 1. 目的

本文档定义 `osal-rs` 公共 OSAL API 的可移植行为契约。

该契约适用于所有通过 `osal_rs::os::*` 暴露的后端实现，包括：

- **FreeRTOS 后端**
- **POSIX 后端**

Linux 不再作为独立 OSAL 后端存在。当前 Linux 主机支持由 POSIX 后端
结合 `posix/bsp/generic_linux` BSP 提供。因此，本文档不再定义"Linux
后端"行为，而是定义公共 OSAL API 的可移植行为语义。

目标：

- 应用代码应仅依赖 `osal-rs` 公共 OSAL API。
- 相同的应用层代码在 FreeRTOS 与 POSIX 上应有行为一致的表现。
- POSIX（通过 `generic_linux` BSP）作为 OSAL 层的开发、测试、CI 和
  仿真平台。
- 后端差异必须明确、文档化且可测试。

本契约描述**行为**，而非实现。

---

## 2. 范围与后端模型

`freertos` 与 `posix` 是 OSAL 后端 feature。

`posix` 选择 POSIX 操作系统适配层，而非选择某一个具体操作系统。

当前 POSIX 主机 BSP 为 `generic_linux`，用于提供 Linux 主机验证所需
的平台常量和类型别名。

POSIX 测试通过表示当前主机平台上的 POSIX 后端通过验证，不代表所有
POSIX-like 平台或未来 BSP 都自动受到支持。

---

## 3. 通用后端原则

### 3.1 API 兼容性

每个后端必须暴露相同的公共 API 接口（通过 `osal_rs::os::*` 选择）。

后端不得将仅适用于特定后端的行为引入通用 OSAL API，除非该行为隐藏在
显式扩展 feature 之后。

### 3.2 行为兼容性

各后端应在 OSAL 行为层面保持一致。不需要共享内部实现细节。

例如：

- FreeRTOS 可用 `xTaskGetTickCount()`。
- POSIX 可用 `clock_gettime(CLOCK_MONOTONIC)`。
- 二者均须暴露单调递增的 OSAL tick 计数值。

### 3.3 不支持的行为

如果某个功能在特定后端上没有有意义的等价实现，该后端必须：

- 提供安全的近似行为；
- API 允许时返回明确的"不支持/错误"结果；
- 仅在 no-op 安全且不会误导用户时，实现为已文档化的 no-op。

后端不得对未实际实现的行为静默声明成功。

### 3.4 阻塞与超时约定

除非另有说明：

- `timeout = 0` 表示非阻塞 / 立即返回。
- 有限超时表示最多阻塞请求的 OSAL tick 时长。
- 最大延迟值表示在 API 支持时永久等待。
- 超时到期应返回与其他后端相同的错误或 false 类结果。

POSIX 主机调度默认不是实时的。因此基于时间的 API 保证不会早于请求的
延迟返回，但不保证精确的唤醒时刻。

---

## 4. Tick 与 Duration 契约

### 4.1 Tick 含义

`TickType` 表示 OSAL 逻辑 tick。

每个后端必须定义 OSAL tick 与实际经过时间之间的稳定映射。POSIX 后端
从活动的 POSIX BSP 获取 tick 周期（`posix/bsp/generic_linux` —
`TICK_PERIOD_MS = 1`，每毫秒一个 tick）。

### 4.2 单调时间

`System::get_tick_count()` 必须返回单调递增的 tick 计数值。

它表示自后端初始化或进程启动以来经过的时间。

不得基于墙上时钟，因为墙上时钟可能回退或跳变。

推荐 POSIX 实现方式：

- 使用单调时钟，如 `clock_gettime(CLOCK_MONOTONIC)`；
- 通过 `pthread_once_t` 初始化保存进程启动时刻的时间戳；
- 从该时刻开始计算经过时长；
- 将经过时长转换为 OSAL tick。

### 4.3 Duration 转 Tick

`Duration::to_ticks()` 必须将标准 Duration 转换为 OSAL tick。

当前基线行为为整数除法：

```text
ticks = duration_millis * tick_rate_hz / 1000
```

不足一个 tick 的 Duration 可能变为零 tick。

如果后续改进为非零 Duration 向上取整，该变更必须在所有后端一致应用。

### 4.4 Tick 转 Duration

Tick 转 `Duration` 必须使用配置的 OSAL tick rate 或 tick period。溢出
时应饱和或安全失败。

### 4.5 Delay

`System::delay(ticks)` 必须将当前执行上下文阻塞至少请求的 OSAL tick 数。

POSIX 后端可使用 POSIX 休眠原语（如 `nanosleep`）或条件变量等待。
`delay(0)` 必须立即返回，不阻塞。

### 4.6 Delay Until

`System::delay_until(previous_wake_time, time_increment)` 必须实现等周期
延迟：

- 基于 `previous_wake_time + time_increment` 计算下次唤醒时刻；
- 如果该时刻仍在未来，休眠至该逻辑 tick；
- 将 `previous_wake_time` 更新为下次唤醒时刻；
- 如果下次唤醒时刻已经过去，不再额外阻塞，但仍前进 `previous_wake_time`。

### 4.7 当前时间 Duration

`System::get_current_time_us()` 应返回单调递增的运行时间 Duration。尽管
方法名包含 "us"，其语义应为"当前单调运行时长"，而非墙上时钟/日期。

### 4.8 Timer Check

`System::check_timer(timestamp, time)` 如果自 `timestamp` 起的单调时间
大于等于 `time`，必须返回 true；否则返回 false。溢出行为必须安全。

---

## 5. System 契约

### 5.1 Scheduler 启动/停止

POSIX 主机环境中没有与 FreeRTOS 调度器等价的用户态调度器。

POSIX 后端可将 `System::start()` 和 `System::stop()` 实现为已文档化的
no-op，除非未来引入 POSIX 运行时管理器。

### 5.2 全部挂起/恢复

`System::suspend_all()` 和 `System::resume_all()` 在 POSIX 上没有安全
的进程级等价实现。POSIX 后端可将其实现为已文档化的 no-op。测试不得
依赖它们来实现互斥。

### 5.3 临界区

FreeRTOS 临界区通过关闭中断或进入内核临界区域实现。POSIX 用户态无法
关闭中断。

POSIX 后端使用递归 `pthread_mutex_t` 实现进程内 OSAL 临界区，
per-thread 嵌套深度通过 `pthread_key_t` TLS 管理。这为 OSAL 调用者
提供互斥，但**不**会关闭操作系统调度或硬件中断。

### 5.4 ISR API

POSIX 主机用户态没有真正的 ISR 上下文。

任何以 `_from_isr` 结尾的 API 都必须谨慎映射：

- `_from_isr` 函数应为非阻塞。
- 可复用正常 API 的同逻辑非阻塞版本。
- 不得阻塞。
- 不得虚假模拟硬件中断语义。
- 如果操作无法安全支持，应返回失败或不支持错误（如有）。

### 5.5 Yield from ISR

`System::yield_from_isr()` 和 `System::end_switching_isr()` 是 FreeRTOS
调度挂钩。POSIX 后端可将其实现为已文档化的 no-op。

---

## 6. Mutex 契约

### 6.1 Mutex 类型

OSAL mutex 是递归的。同一线程可多次获取同一个 mutex。

每次成功 lock 必须匹配一次 unlock。仅在递归计数归零时 mutex 才完全释放。

### 6.2 Lock 行为

`Mutex::lock()` 必须阻塞直到获取 mutex 或后端报告不可恢复错误。

### 6.3 Guard 行为

锁 guard 必须提供 RAII 语义。guard drop 时恰好释放一层锁。如果同一
线程锁了三次，drop 一个 guard 只释放一层递归。

### 6.4 互斥

Mutex 必须保护内部值。某线程持有 mutex 时，其他线程在 mutex 释放前
不得进入被保护的临界区。

### 6.5 递归所有权

后端必须追踪所有权。只有持有线程可以递归获取 mutex 而不阻塞。其他
线程必须阻塞或根据所用 API 返回失败。

### 6.6 ISR Lock 行为

如果公共 API 中存在 `lock_from_isr()`，POSIX 后端应将其视为非阻塞
try-lock：

- mutex 立即可用时返回成功；
- 其他线程持有时返回失败；
- 永不阻塞。

### 6.7 实现说明

契约不要求特定实现方式。FreeRTOS 后端可使用 FreeRTOS 递归 mutex 原语。
POSIX 后端可使用 `pthread_mutex_t`（PTHREAD_MUTEX_RECURSIVE /
ERRORCHECK）或其内部的 `posix/sys` 封装。

---

## 7. Semaphore 契约

### 7.1 信号量类型

计数信号量，具有最大计数、当前计数、wait/take 操作和 signal/give 操作。

### 7.2 创建

`Semaphore::new(max_count, initial_count)` 必须创建具有指定最大和初始
计数的信号量。

如果 `initial_count > max_count`，创建必须失败，不得静默产生无效信号量。

### 7.3 Wait

`wait(timeout)` 尝试递减信号量计数：

- 如果 count > 0，递减计数并返回 true；
- 如果 count == 0 且 timeout == 0，立即返回 false；
- 如果 count == 0 且 timeout 有限，阻塞直到收到 signal 或超时；
- 超时到期返回 false；
- 请求永久等待时，阻塞直到收到 signal。

### 7.4 Signal

`signal()` 递增信号量计数：

- 如果 count < max_count，递增计数并唤醒一个等待者；
- 如果 count == max_count，返回 false。

### 7.5 ISR 变体

`_from_isr` 信号量函数必须是非阻塞的。在 POSIX 主机环境中，应表现为
非阻塞变体，不得模拟硬件中断优先级。

---

## 8. Queue 契约

### 8.1 队列类型

OSAL 队列契约同时涵盖原始消息队列和类型化流式队列：

- `Queue` 传输固定大小的原始消息。
- `QueueStreamed<T>` 启用 `serde` feature 时，通过 `osal-rs-serde`
  自动序列化/反序列化 `T` 来传输类型化消息。

两者共享相同的行为契约：

- 有界 FIFO；
- 固定容量；
- 满/空时超时；
- post 唤醒接收者；
- fetch 唤醒发送者。

`QueueStreamed<T>` 必须保持与底层队列相同的 FIFO 和超时行为。序列化
失败必须报告为 OSAL 错误，且不得将部分消息入队。反序列化失败必须
报告为 OSAL 错误，且不得暴露部分初始化的用户数据。

### 8.2 创建

创建无效容量或无效消息大小的队列必须失败。

### 8.3 Post / 发送

- 队列不满时，压入消息并返回成功；
- 队列满且 timeout == 0 时，立即返回队列满或超时类失败；
- 队列满且 timeout 有限时，等待直到出现空闲位置或超时；
- 请求永久等待时，阻塞直到有空闲位置。

### 8.4 Fetch / 接收

- 队列不空时，弹出最旧消息（FIFO）并返回成功；
- 队列空且 timeout == 0 时，立即返回超时类失败；
- 队列空且 timeout 有限时，等待直到有消息可用或超时；
- 请求永久等待时，阻塞直到有消息可用。

### 8.5 唤醒规则

消息入队时唤醒一个等待的接收者。消息出队时唤醒一个等待的发送者。

### 8.6 ISR 变体

`_from_isr` 队列 API 如有，必须是非阻塞的。可映射为立即 try-send 或
try-receive。

---

## 9. EventGroup 契约

### 9.1 事件位

事件组存储事件位。通用可用掩码应遵循 FreeRTOS 兼容模型，即高位可能
保留。

### 9.2 Set

`set(bits)` 设置指定位并返回操作后的事件位值。任何条件变为满足的
等待者应被唤醒。

### 9.3 Get / Clear

`get()` 返回当前事件位（非阻塞）。`clear(bits)` 清除指定位。

### 9.4 Wait

`wait(mask, timeout_ticks)` 必须等待 `mask` 中的任意一个位被设置：

- 等待任意位，而非全部位；
- 退出时不自动清除位；
- 超时以 OSAL tick 表示；
- 函数返回时返回当前事件位值；
- 调用者通过 `returned_bits & mask != 0` 判断成功；
- 超时到期时，返回的位值可能不包含请求的掩码。

### 9.5 ISR 变体

`_from_isr` 事件组 API 必须是非阻塞的。可短暂使用同一内部锁，但不得
在条件变量上等待。

---

## 10. Timer 契约

### 10.1 定时器类型

软件定时器，具有名称、周期、单次/周期模式、回调、start / stop / reset
/ change-period 操作。

### 10.2 回调执行

回调必须在定时器到期后执行。POSIX 主机后端可使用后台定时器管理线程。

回调不得在配置的周期之前运行。由于主机调度延迟，可能延后。实时精度
取决于后端；契约保证顺序和最短延迟，但不保证精确唤醒延迟。

### 10.3 周期性行为

对于周期定时器，应在每次回调后重新调度。周期应尽可能以调度到期时刻
为基准，而非仅在回调完成时刻之后计算，除非实现另有文档说明。

### 10.4 Stop / Reset / Change Period

- `stop()` 应阻止后续回调执行。如果回调正在运行，无需强制中断。
- `reset()` 应从复位时刻重新开始倒计时。
- `change_period()` 应更新后续到期的定时器周期。

---

## 11. Thread 契约

### 11.1 线程创建

线程具有名称、栈大小、优先级和入口闭包/函数。

POSIX 后端可通过 `posix/sys/thread` 封装使用 `pthread_create` /
`pthread_join`。POSIX 主机调度优先级与 FreeRTOS 任务优先级不等价，
除非显式实现实时调度支持。

### 11.2 线程生命周期

如果公共 API 分离构造和启动步骤，后端必须保持该生命周期：创建的线程
在 `start()` 调用前不得执行用户代码。

### 11.3 Join

如果 Join 是公共 API 的一部分，应等待目标线程退出并按照现有 API 返回
结果/状态。

### 11.4 优先级

FreeRTOS 优先级直接映射到 RTOS 调度优先级。除非显式配置实时调度，
POSIX 主机优先级是建议性的。

后端必须文档说明优先级当前仅作信息性用途，除非启用实时调度功能。

### 11.5 线程状态

POSIX 主机线程状态信息可近似表示。后端可暴露有限状态（Created、
Running、Finished、Invalid/Unknown），并必须文档说明限制。

---

## 12. Error 契约

### 12.1 错误一致性

各后端应使用已有的 `osal-rs` 错误类型。后端不得通过公共 API 暴露
原始后端特定错误码（如 `errno` 或 FreeRTOS 状态码），除非显式封装。

### 12.2 分配失败

如果对象创建因内存或系统资源不足而失败，应返回与其他后端相同的分配
相关错误。

### 12.3 超时

超时失败应使用跨后端一致的超时类错误或 false 返回行为。

### 12.4 不支持的行为

如果已有错误类型没有 `Unsupported` 变体，后端可返回最接近的安全已有
错误。后续 API 改进可增加显式的不支持错误变体。

---

## 13. POSIX 主机说明

当前的 POSIX 后端通过 `posix/sys` 封装使用 POSIX API 实现所有 OSAL
原语（pthread mutex、pthread condvar、`clock_gettime(CLOCK_MONOTONIC)`、
`pthread_create`/`pthread_join`）。

全局初始化使用 `pthread_once_t`；per-thread 状态使用 `pthread_key_t`
TLS；默认分配器委托给 `libc::malloc` / `libc::free`。

平台常量和类型别名由 `posix/bsp/generic_linux`（当前唯一的 BSP 目标）
提供。

---

## 14. 一致性测试

同一套契约测试应在所有可行的后端上运行。

当前主机侧验证命令：

```bash
cargo test -p osal-rs-tests --no-default-features --features posix
cargo test -p osal-rs-tests --no-default-features --features "posix serde"
```

FreeRTOS 一致性测试可能需要目标或模拟器专用 runner。

### 最小契约测试

**Time / System:** tick 计数单调；delay 等待不少于请求时间；delay_until
更新上次唤醒时间；Duration 转 tick 符合契约；check_timer 结果正确。

**Mutex:** 基本 create/lock/unlock；guard drop 解锁；递归 lock 成功；
其他线程被正确阻塞；多线程计数器测试达到准确期望值；from_isr/try-lock
是非阻塞的。

**Semaphore:** 初始计数正确；wait 递减计数；signal 递增计数；计数为零
时 wait 超时；signal 唤醒等待线程；满计数时 signal 返回 false。

**Queue:** FIFO 有序；不满时 send 成功；不空时 receive 成功；满时 send
失败/超时；空时 receive 失败/超时；接收者消费后阻塞的发送者被唤醒；
发送者入队后阻塞的接收者被唤醒。

**EventGroup:** set/get 正确；clear 正确；wait 在任意请求位设置时返回；
wait 不自动清除位；无请求位时 wait 超时。

**Timer:** 单次定时器触发一次；周期定时器重复触发；stop 阻止后续回调；
reset 重新开始倒计时；change period 更新定时。
