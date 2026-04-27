# 议题 A-5：状态调谐策略与双保险模型

## 第 1 轮 (Round 1) - Master Claude 立场

### 1. 核心架构：混合模型
- **主路径 (Push-driven)**：利用 Linux `pidfd_open` 监控进程死掉，`inotify` 监控沙盒目录完整性，通过 `epoll` 实现亚毫秒级响应。
- **兜底路径 (Polling-based)**：每 30 秒执行一次全量幂等对账，捕获 Push 路径遗漏的竞态或极端异常。

### 2. 辩论：为什么不能只有 Polling？
- **违反 R-DISPATCH-2**：如果 Agent 崩溃，Caller (L3) 必须立即收到通知。30s 的轮询间隔意味着长达半分钟的「状态盲区」，这在自动化流水线中是不可接受的。
- **议题 1b 闭环失效**：`Unknown` 状态判定依赖精确的计时器。只有实时捕获 PTY 流量，才能在提示符消失后的 N 秒内立即触发 LLM 兜底，而非等待轮询周期。

---

## 第 2 轮 (Round 2) - Gemini 架构评估

### 1. 核心判断
我完全认同该混合模型。**Push 决定了系统的「灵敏度上限」，Polling 决定了系统的「可靠性下限」**。两者的结合使 L2 调度层真正具备了工业级的鲁棒性。

### 2. 工程实现细化

#### A-5-1：pidfd 与进程监控
- **判定**：**全面应用 pidfd_open (Linux 5.3+)。**
- **理由**：相比传统的 `waitpid` 或轮询 `/proc`，`pidfd` 不受 PID 回绕影响，且能通过 `epoll` 在进程退出的瞬间被唤醒。
- **实例**：当 Agent 遭遇 OOM 或 SIGSEGV 崩溃时，L2 守护进程能在亚毫秒内捕获 `EPOLLIN`，将状态转为 `Crashed` 并向所有 UDS 订阅者推送 Notification。

#### A-5-2：inotify 的监控范畴
- **判定**：**仅监控 Sandbox 根目录。**
- **理由**：避免文件系统事件泛滥。Sandbox 根目录的删除 (DELETE_SELF) 或重命名 (MOVE_SELF) 通常意味着沙盒环境遭到物理破坏。
- **策略**：一旦触发，立即切断 PTY stdin 并强制标记 Agent 死亡。

#### A-5-3：跨平台降级 (macOS)
- **判定**：**macOS 退化为纯 Polling (1Hz)。**
- **理由**：遵守 A-6 决议中「Linux 优先」的原则。在 macOS 上使用 1 秒一轮的高频 Polling 替代 Push 逻辑。
- **意义**：保持 Rust 核心逻辑简洁，通过抽象 `HealthProvider` Trait 在编译期自动切换 Linux(Push) 或 Darwin(Poll)。

#### A-5-4：Unknown 状态计时器
- **判定**：**Push 驱动的实时 Timer (MarkerTimeout)。**
- **机制**：
    1. 每次 `agent.send` 成功后，启动 Timer。
    2. 每次 PTY 有输出且 vt100 解析到内容，重置 Timer。
    3. 如果 Timer 触发（5s-10s 可配），且状态仍为 `Busy` 或匹配失败，立即流转至 `Unknown` 并触发 Evidence Dump。
- **意义**：这是议题 1b 能够实现「快速响应」的物理前提。

#### A-5-5：并发竞态与幂等性
- **判定**：**基于 SQLite CAS 的状态转移。**
- **理由**：Push 事件与 Polling 对账可能在毫秒级并发。
- **策略**：所有状态流转必须使用带 `WHERE current_state` 的更新语句。
- **实例**：`UPDATE agents SET state = 'Crashed' WHERE id = ? AND state NOT IN ('Crashed', 'Killed')`。这种原子操作确保了清理逻辑（如关闭 PTY、回收目录）只会被触发一次。

### 3. 调谐循环的终极守则
无论 Push 触发了多少次，30s 一次的对账循环必须对比「SQLite 记录的 Running」与「OS 物理层（PID + /proc）」的真实情况。如果发现数据库记为 Running 但 PID 已消失，**以 OS 物理层为准**，强制修正数据库。
