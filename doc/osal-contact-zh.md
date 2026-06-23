# `osal-rs` v0.1 OSAL 契约

## 1. 目的

本文档定义了 `osal-rs` Linux 后端的预期行为。

Linux 后端不应定义单独的行为模型。它应实现与当前 FreeRTOS 后端相同的公共 OSAL 语义，除非某个 FreeRTOS 特定功能在 Linux 用户空间中没有有意义的等价物。

目标如下：

* 应用代码应仅依赖 `osal-rs` 公共 OSAL API。
* 相同的应用级代码在 FreeRTOS 和 Linux 上应表现一致。
* Linux 应作为 OSAL 层的开发、测试、CI 和模拟后端。
* 后端差异必须是显式的、有文档记录的且可测试的。

本契约描述的是行为，而非实现。

---

## 2. 通用后端原则

### 2.1 API 兼容性

Linux 后端必须暴露与通过 `osal_rs::os::*` 选择的现有 OSAL 后端相同的公共 API 接口。

Linux 后端不得将仅限 Linux 的行为引入公共 OSAL API，除非它隐藏在 Linux 特定的扩展功能之后。

### 2.2 行为兼容性

Linux 后端应在 OSAL 行为层面与 FreeRTOS 后端保持一致。

它不需要照搬 FreeRTOS 的内部实现细节。

例如：

* FreeRTOS 可能使用 `xTaskGetTickCount()`。
* Linux 可能使用 `std::time::Instant`。
* 两者都必须暴露一个单调递增的 OSAL tick 计数器。

### 2.3 不支持的行为

如果某个 FreeRTOS 功能在 Linux 用户空间中没有直接等价物，Linux 后端必须执行以下操作之一：

* 提供安全的近似行为；
* 如果 API 允许，返回明确的不支持/错误结果；
* 仅在不执行操作是安全的且不会误导用户时，实现一个有文档记录的 no-op。

Linux 后端不得对未实际实现的行为静默地声称成功。

### 2.4 阻塞和超时约定

除非另有说明：

* `timeout = 0` 表示非阻塞 / 立即返回。
* 有限超时表示最多阻塞所请求的 OSAL tick 时长。
* 最大延迟值表示在 API 支持的情况下永远等待。
* 超时过期应返回与 FreeRTOS 后端相同的错误或 false-like 结果。

Linux 用户空间调度默认不是实时的。因此，基于时间的 API 保证不会早于请求的延迟返回，但不保证精确的唤醒时刻。

---

## 3. Tick 和 Duration 契约

### 3.1 Tick 的含义

`TickType` 表示一个 OSAL 逻辑 tick。

Linux 后端必须定义一个 OSAL tick 与墙上时钟时长之间的稳定映射。首个版本应使用与现有 OSAL 配置相同的 tick 周期模型。

### 3.2 单调时间

`System::get_tick_count()` 必须返回一个单调递增的 tick 计数器。

它表示从后端初始化或进程启动以来经过的时间。

它不得基于墙上时钟时间，因为墙上时钟时间可能回退或向前跳变。

推荐的 Linux 实现：

* 使用 `std::time::Instant`；
* 存储一个进程启动时的 `Instant`；
* 计算从该时刻到当前的经过时长；
* 将经过时长转换为 OSAL tick。

### 3.3 Duration 到 tick 的转换

`Duration::to_ticks()` 必须将标准 duration 转换为 OSAL tick。

首个 Linux 版本应保留现有的 FreeRTOS 风格的整数转换行为，除非整个项目后续决定更改共享契约。

当前的基准行为：

```text
ticks = duration_millis * tick_rate_hz / 1000
```

这是整数除法。因此，小于一个 tick 的 duration 可能变为零 tick。

如果后续将此行为改进为将非零 duration 向上取整，则该更改必须在所有后端中一致应用。

### 3.4 Tick 到 duration 的转换

将 tick 转换为 `Duration` 必须使用配置的 OSAL tick 频率或 tick 周期。

转换应在溢出时安全地饱和或失败。

### 3.5 延迟

`System::delay(ticks)` 必须阻塞当前执行上下文至少请求的 OSAL tick 数。

Linux 实现可以使用 `std::thread::sleep`。

`delay(0)` 应文档化为无延迟。如果实现选择改为让出 CPU（yield），则该行为必须有文档记录并经过测试。

### 3.6 延迟直到

`System::delay_until(previous_wake_time, time_increment)` 必须实现周期性延迟行为。

预期行为：

* 基于 `previous_wake_time + time_increment` 计算下一次唤醒时间；
* 如果该逻辑 tick 时间仍在未来，则休眠到该时刻；
* 将 `previous_wake_time` 更新为下一次唤醒时间；
* 如果下一次唤醒时间已经过去，则不进行额外阻塞直接返回，但仍然推进 `previous_wake_time`。

这应与周期性任务循环兼容。

### 3.7 当前时间 duration

`System::get_current_time_us()` 应返回单调递增的运行时间，类型为 `Duration`。

尽管当前方法名如此，其语义含义应为"当前单调运行时间"，而非墙上时钟日期/时间。

### 3.8 Timer 检查

`System::check_timer(timestamp, time)` 如果自 `timestamp` 以来的经过单调时间大于或等于 `time`，则必须返回 true。

否则必须返回 false。

溢出行为必须是安全的。

---

## 4. System 契约

### 4.1 调度器启动

`System::start()` 在 FreeRTOS 上启动 FreeRTOS 调度器。

在 Linux 用户空间中，没有等价的应用级调度器可以启动。

Linux 后端 v0.1 可以将 `System::start()` 实现为：

* no-op；或者
* 如果项目后续引入一个入口点，则作为一个有文档记录的阻塞运行时入口点。

对于首个 Linux 后端，no-op 是可以接受的，但必须有文档记录。

### 4.2 调度器停止

`System::stop()` 在 FreeRTOS 上停止 FreeRTOS 调度器。

在 Linux 用户空间中，没有等价的全局调度器停止操作。

Linux 后端 v0.1 可以将 `System::stop()` 实现为一个有文档记录的 no-op。

### 4.3 全部挂起和恢复

`System::suspend_all()` 和 `System::resume_all()` 挂起和恢复 FreeRTOS 调度。

Linux 用户空间没有安全的全进程等价物。

Linux 后端 v0.1 不应假装全局停止所有线程。

v0.1 可接受的行为：

* 带有文档说明的 no-op；或者
* 如果 Linux 后端后续拥有所有 OSAL 线程，则使用内部 OSAL 运行时锁。

如果实现为 no-op，测试不得依赖它来实现互斥。

### 4.4 临界区

FreeRTOS 临界区禁用中断或进入内核临界区域。

Linux 用户空间无法禁用中断。

Linux 后端 v0.1 应仅在拥有全局锁的情况下将临界区定义为进程本地的 OSAL 临界区。

否则，`critical_section_enter()` 和 `critical_section_exit()` 可以作为有文档记录的 no-op。

除非使用真正的同步机制实现，否则它们不得在 Linux 后端测试中用于保护共享数据。

### 4.5 ISR API

Linux 用户空间没有真正的 ISR 上下文。

任何以 `_from_isr` 结尾的 API 都必须谨慎映射。

推荐的 Linux v0.1 规则：

* `_from_isr` 函数必须是非阻塞的。
* 它们可以调用与普通 API 相同的非阻塞逻辑。
* 它们不得阻塞。
* 它们不得虚假地模拟硬件中断语义。
* 如果操作无法安全支持，应在可能的情况下返回失败或不支持错误。

### 4.6 ISR 中的 Yield

`System::yield_from_isr()` 和 `System::end_switching_isr()` 是 FreeRTOS 调度钩子。

Linux 后端 v0.1 可以将它们实现为有文档记录的 no-op。

---

## 5. Mutex 契约

### 5.1 Mutex 类型

OSAL mutex 契约应遵循当前 FreeRTOS 后端的行为：

Mutex 是递归的。

这意味着同一线程可以多次获取同一个 mutex。

每次成功的加锁必须对应一次解锁。

仅当递归计数归零时，mutex 才完全释放。

### 5.2 加锁行为

`Mutex::lock()` 必须阻塞直到获取到 mutex，或直到后端报告不可恢复的错误。

对于 v0.1，`lock()` 应表现为无限等待。

### 5.3 Guard 行为

锁 guard 必须提供 RAII 语义。

当 guard 被丢弃时，恰好释放一层锁。

如果同一线程对同一个 mutex 加锁了三次，丢弃一个 guard 仅释放一层递归。

### 5.4 互斥

Mutex 必须保护对包含值的访问。

如果一个线程持有 mutex，另一个线程不得进入受保护的临界区，直到 mutex 被释放。

### 5.5 递归所有权

后端必须追踪所有权。

只有持有线程可以不阻塞地递归获取 mutex。

不同的线程必须根据所使用的 API 阻塞或失败。

### 5.6 ISR 加锁行为

如果公共 API 中存在 `lock_from_isr()`，Linux 后端 v0.1 应将其视为非阻塞的 try-lock 操作。

预期行为：

* 如果 mutex 立即可用，返回成功；
* 如果另一个线程持有 mutex，返回失败；
* 永不阻塞。

如果支持持有者的递归 try-lock，它必须增加递归计数。

### 5.7 实现说明

Linux v0.1 可以使用 Rust 标准同步原语实现递归 mutex：

* 内部使用 `std::sync::Mutex<State>`；
* `std::sync::Condvar`；
* 持有者线程 ID；
* 递归计数。

v0.1 中不需要使用 pthread FFI。

---

## 6. Semaphore 契约

### 6.1 信号量类型

信号量是计数信号量。

它具有：

* 最大计数；
* 当前计数；
* 等待/获取操作；
* 发送/释放操作。

### 6.2 创建

`Semaphore::new(max_count, initial_count)` 必须创建一个具有指定最大计数和初始计数的信号量。

如果 `initial_count > max_count`，Linux 后端应返回错误，而不是静默地创建一个无效的信号量。

如果分配失败，应返回与 FreeRTOS 后端用于分配失败的错误类别相同的错误。

### 6.3 等待

`wait(timeout)` 尝试递减信号量计数。

预期行为：

* 如果 count > 0，递减计数并返回 true；
* 如果 count == 0 且 timeout == 0，立即返回 false；
* 如果 count == 0 且 timeout 有限，阻塞直到收到信号或超时过期；
* 如果超时过期，返回 false；
* 如果请求永远等待，阻塞直到收到信号。

### 6.4 发送信号

`signal()` 递增信号量计数。

预期行为：

* 如果 count < max_count，递增计数并唤醒一个等待者；
* 如果 count == max_count，根据现有 OSAL API 返回 false 或失败。

### 6.5 ISR 变体

Linux `_from_isr` 信号量函数必须是非阻塞的。

`wait_from_isr()` 应尝试立即获取。

`signal_from_isr()` 应在不阻塞的情况下发送信号。

它们不应模拟硬件中断优先级或上下文切换。

---

## 7. Queue 契约

### 7.1 队列类型

队列是有界 FIFO 消息队列。

它具有：

* 固定容量；
* 根据现有 API 的固定消息类型或消息大小；
* 发送/投递操作；
* 接收/获取操作；
* 超时行为。

### 7.2 创建

使用无效容量或无效消息大小创建队列必须失败。

Linux 后端不得创建实际上无法存储消息的队列。

### 7.3 投递/发送

向队列投递消息必须遵循 FIFO 行为。

预期行为：

* 如果队列未满，压入元素并返回成功；
* 如果队列已满且 timeout == 0，立即返回队列满或超时风格失败；
* 如果队列已满且 timeout 有限，等待直到有空间可用或超时过期；
* 如果超时过期，返回与 FreeRTOS 后端用于队列发送失败的错误类别相同的错误；
* 如果请求永远等待，阻塞直到有空间可用。

### 7.4 获取/接收

从队列获取消息必须移除最旧的元素。

预期行为：

* 如果队列非空，弹出最旧元素并返回成功；
* 如果队列为空且 timeout == 0，立即返回超时风格失败；
* 如果队列为空且 timeout 有限，等待直到有元素可用或超时过期；
* 如果请求永远等待，阻塞直到有元素可用。

### 7.5 唤醒规则

当有元素被投递时，应唤醒一个等待中的接收者。

当有元素被获取时，应唤醒一个等待中的发送者。

### 7.6 ISR 变体

Linux 队列 `_from_isr` API（如果存在）必须是非阻塞的。

它们可以映射为立即 try-send 或 try-receive 行为。

它们不得阻塞。

---

## 8. EventGroup 契约

### 8.1 事件位

事件组存储事件位。

公共可用掩码应遵循 FreeRTOS 兼容模型，其中高位可能被保留。

Linux 后端应保留与公共 API 相同的 `MAX_MASK` 行为。

### 8.2 设置

`set(bits)` 设置指定的位。

它返回操作后的事件位。

任何条件变为真的等待者应被唤醒。

### 8.3 获取

`get()` 返回当前的事件位。

它是非阻塞的。

### 8.4 清除

`clear(bits)` 清除指定的位。

根据当前公共 API 行为，返回清除之前或之后的事件位。Linux 后端必须与 OSAL API 测试所使用的 FreeRTOS 后端行为匹配。

### 8.5 等待

`wait(mask, timeout_ticks)` 必须等待，直到 `mask` 中指定的任意位被设置。

默认契约遵循当前 FreeRTOS 后端的调用风格：

* 等待任意位，而非所有位；
* 退出时不自动清除位；
* 超时以 OSAL tick 表示；
* 函数返回时返回当前的事件位。

调用者通过检查以下条件来判断成功：

```text
returned_bits & mask != 0
```

如果超时过期，返回的位可能不包含请求的掩码。

### 8.6 ISR 变体

Linux 事件组 `_from_isr` API 必须是非阻塞的。

它们可以短暂地使用相同的内部锁，但不得在条件变量上等待。

---

## 9. Timer 契约

### 9.1 Timer 类型

定时器是软件定时器。

它具有：

* 名称；
* 周期；
* 如果现有 API 支持，单次触发或周期性模式；
* 回调；
* 启动操作；
* 停止操作；
* 重置操作；
* 更改周期操作。

### 9.2 回调执行

回调必须在定时器到期后执行。

Linux 后端 v0.1 可以使用后台定时器管理线程。

回调不得在配置的周期经过之前运行。

由于 Linux 调度延迟，回调可能稍晚运行。

### 9.3 周期性行为

对于周期性定时器，定时器应在每次回调后重新调度。

在可行的情况下，周期应从计划的到期时间开始度量，而不仅仅是从回调完成时间开始，除非实现另有文档说明。

### 9.4 停止

停止定时器应阻止未来的回调执行。

如果回调已在运行中，`stop()` 不需要强制中断它。

### 9.5 重置

重置定时器应从重置时刻重新开始倒计时。

### 9.6 更改周期

更改定时器周期应更新后续到期的定时器周期。

如果定时器处于活动状态，实现必须文档说明新周期是立即生效还是在下一次重启时生效。

### 9.7 实现说明

定时器不应是第一个实现的 Linux 后端模块。

它依赖于正确的时间、线程、mutex 和条件变量行为。

---

## 10. Thread 契约

### 10.1 线程创建

一个线程具有：

* 名称；
* 栈大小；
* 优先级；
* 入口闭包/函数。

Linux 后端 v0.1 可以使用 `std::thread::Builder`。

在可能的情况下，线程名称和栈大小应传递给 Linux/Rust 线程构建器。

### 10.2 线程启动

如果公共 API 将构造和启动分开，Linux 必须保留该生命周期。

已创建的线程在调用 `start()` 之前不得执行用户代码。

如果使用 `std::thread` 难以实现，Linux 后端应在内部存储闭包，仅在 `start()` 时 spawn。

### 10.3 加入（Join）

如果 join 是公共 API 的一部分，它应等待直到目标线程退出，并根据现有 API 返回其结果/状态。

### 10.4 优先级

FreeRTOS 优先级直接映射到 RTOS 调度优先级。

Linux 用户空间优先级并不等价。

Linux 后端 v0.1 应保留优先级字段，但不需要强制执行真正的调度优先级。

它必须文档说明当前优先级仅是信息性的，除非未来的 Linux 实时功能被启用。

它不得静默地暗示确定性的优先级调度。

### 10.5 线程状态

Linux 线程状态信息可能是近似的。

如果无法获得精确的 FreeRTOS 风格状态，Linux 后端可以暴露有限的状态，例如：

* 已创建（Created）；
* 运行中（Running）；
* 已完成（Finished）；
* 无效/未知（Invalid/Unknown）。

此限制必须有文档记录。

---

## 11. Error 契约

### 11.1 错误一致性

Linux 后端应使用现有的 `osal-rs` 错误类型。

它不应将 Linux 特定的原始 errno 值引入公共的通用 API。

### 11.2 分配失败

如果由于内存或系统资源不可用导致对象创建失败，返回与 FreeRTOS 后端使用的分配相关错误相同的错误。

### 11.3 超时

超时失败应使用与当前 FreeRTOS 后端相同的超时风格错误或 false 返回行为。

### 11.4 不支持的行为

如果现有错误类型没有 `Unsupported` 变体，Linux 后端 v0.1 可以返回最接近的安全现有错误。

后续 API 改进可以添加显式的不支持错误。

---

## 12. Linux 后端实现策略 v0.1

首个 Linux 后端应优先选择安全的 Rust 标准库原语。

推荐映射：

```text
系统时间          -> std::time::Instant
延迟              -> std::thread::sleep
线程              -> std::thread::Builder
递归 Mutex        -> std::sync::Mutex + std::sync::Condvar + 持有者 ThreadId + 递归计数
信号量            -> std::sync::Mutex + std::sync::Condvar + 计数器
队列              -> std::sync::Mutex + std::sync::Condvar + VecDeque
事件组            -> std::sync::Mutex + std::sync::Condvar + 位掩码
定时器            -> 后台 TimerManager 线程，后续阶段实现
```

Linux 后端 v0.1 不需要直接的 FFI。

后续可以添加 FFI 或 Linux 原生 API 用于：

* pthread 递归 mutex；
* 实时调度；
* CPU 亲和性；
* timerfd；
* eventfd；
* epoll；
* futex；
* `/proc` 运行时统计。

---

## 13. 一致性测试

每个契约规则都应有后端无关的测试。

相同的测试用例应尽可能在 FreeRTOS 和 Linux 上运行。

Linux v0.1 最低测试要求：

### 时间/系统

* tick 计数是单调的；
* delay 至少等待请求的时间；
* delay_until 更新上一次唤醒时间；
* duration 到 tick 转换符合契约；
* check_timer 在超时前返回 false，超时后返回 true。

### Mutex

* 基本创建/加锁/解锁；
* guard 丢弃时解锁；
* 保护的值变异（mutation）有效；
* 同一线程递归加锁有效；
* 当 mutex 被持有时，其他线程阻塞；
* 多线程计数器测试达到精确的期望值；
* from_isr/try-lock 行为是非阻塞的。

### 信号量

* 初始计数有效；
* wait 递减计数；
* signal 递增计数；
* 当计数为零时 wait 超时；
* signal 唤醒等待中的线程；
* 当计数达到最大值时 signal 失败或返回 false。

### 队列

* FIFO 顺序；
* 队列未满时 send 成功；
* 队列非空时 receive 成功；
* 队列满时 send 失败或超时；
* 队列空时 receive 失败或超时；
* 阻塞的发送者在接收者消费后被唤醒；
* 阻塞的接收者在发送者投递后��唤醒。

### 事件组

* set/get 有效；
* clear 有效；
* 当任意请求位被设置时 wait 返回；
* wait 不会自动清除位；
* 当没有请求位被设置时 wait 超时。

### 定时器

* 单次触发定时器触发一次；
* 周期性定时器重复触发；
* stop 阻止后续回调；
* reset 重启倒计时；
* 更改周期更新定时。

定时器测试可以推迟到 Linux 后端具有稳定的线程和同步原语之后。

---

## 14. 开发顺序

推荐的 Linux 后端开发顺序：

```text
1. 时间 / Duration / 系统 tick
2. 递归 Mutex
3. 信号量
4. 队列
5. 事件组
6. 线程生命周期
7. 定时器
8. Linux 特定扩展
```

第一个里程碑不应包含 Linux 特定的 API。

第一个里程碑应证明公共 OSAL 行为可以在 Linux 用户空间中安全运行。
