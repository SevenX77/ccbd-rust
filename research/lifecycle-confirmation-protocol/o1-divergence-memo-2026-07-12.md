# o1 发散备忘录 · Agent 任务生命周期确认协议设计辩论

## 一、 不同 Provider "任务开始信号" 可行载体评估

为解决任务开始端信号缺失的问题，我们必须在三家异构的 Provider 运行时中寻找能够承载“任务已开始/已收到提示词”的物理载体。根据源码与二进制实证，评估如下：

### 1. Claude (Claude Code)
- **原生载体**：`UserPromptSubmit` (UPS) 事件。
- **可行性评估**：
  - **Master 模式**：[agent.rs:988](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/agent.rs#L988) 已经实现了对 master 的 `userpromptsubmit` 信号的消费，并成功将状态转移至 `BUSY`。
  - **Worker 模式**：理论可行但未经实证。在 [home_layout.rs:955-980](file:///home/sevenx/coding/ccbd-rust/src/provider/home_layout.rs#L955-L980)，我们仅向 `.claude/settings.json` 写入了 `Stop` 事件钩子，并未写入 `UserPromptSubmit`。因为 Worker 是通过 CLI 非交互式执行的（例如 `claude "prompt"`），我们需要实证 Claude Code 在接受命令行直接传参启动时，是否依然会触发 `UserPromptSubmit` 钩子。
  - **备选载体**：`SessionStart` 钩子。由于 Worker 生命期内仅执行一次任务，会话的启动即等价于任务的开始。
  - **无原生信号时的降级**：当 PTY 输入清空后，以发送键值后 200ms 的 `capture-pane` 回显中出现首行 CLI 输出，作为弱确认降级信号。

### 2. Antigravity (Gemini CLI / agy)
- **原生载体**：`PreInvocationHook` / `PreInvocationHookArgs` 符号。
- **可行性评估**：
  - **理论可行**：根据 [.kiro/specs/ah-hook-push-completion/antigravity-hooks-preverify.md](file:///home/sevenx/coding/ccbd-rust/.kiro/specs/ah-hook-push-completion/antigravity-hooks-preverify.md)，`agy` 二进制中确凿存在 `PreInvocationHook` 等名为 Invocation 级别的钩子符号。由于 Gemini 每次请求均被称为一次 Invocation，该钩子会在大模型真正构思输出前被拉起。
  - **实证状态**：未验证。当前 `home_layout.rs:358` 仅对 `Stop` 钩子进行了 merge（`merge_antigravity_hooks`），并未注入 `PreInvocation`。需要确认 `.gemini/config/hooks.json` 能否正确路由并阻塞该事件。
  - **无原生信号时的降级**：进程级探针。通过包装脚本（Wrapper）在进程拉起后向 `CCB_SOCKET` 报告 `worker_process_launched` 事件，作为弱确认。

### 3. Codex (Codex CLI)
- **原生载体**：**不存在任何原生的任务开始钩子**。
- **可行性评估**：
  - **事实证明**：根据 cmux 的 prior art 调研与 [.kiro/specs/ah-hook-push-completion/research.md:85](file:///home/sevenx/coding/ccbd-rust/.kiro/specs/ah-hook-push-completion/research.md#L85) 记录，Codex 的 `hooks.json` 仅暴露 `Stop` 钩子用于任务结束，二进制提取中完全缺失任何诸如 `UserPromptSubmit` 或 `PreToolUse` 的前置钩子逻辑。
  - **降级与重建方案（必须造信号）**：
    - **包装器插桩（Wrapper Instrumentation）**：必须弃用直连 binary 的执行方式，改由包装脚本（如 `ah-agent-wrapper`）代为启动 `codex`。包装脚本在 fork 并 exec 前，先通过 `CCB_SOCKET` 向 `ahd` 发送 `agent.notify` (事件为自定义的 `started`)，人工补齐任务开始信号。

---

## 二、 用户原则的红队与批判性检验

用户提出：**“生命周期推进 = while(loop) { 重试/纠正动作 }，直到 100% 可靠跳出信号达成”**。我们对此原则提出三点严厉的红队质询，揭示其在极端物理世界下的失效模式：

### 1. "100%可靠信号" 的乌托邦幻觉与系统死锁
- **失效场景（Silent Hook Failure）**：在真实沙箱中，极其容易发生“物理隔离击穿 Hook”的情况。例如：由于文件权限漂移导致 `.claude/settings.json` 被写保护，或者 Sandbox 容器挂载丢失导致 Hook 执行时找不到 `ah` 命令或无法连接 `CCB_SOCKET`。
- **致命后果**：此时，Agent 实际上已经接收到 Tty 投键并在后台默默运行（消耗 Token 且进行文件写），但由于 Hook 通道损坏，`userpromptsubmit` 信号**永远不会到达**。如果我们的状态机严格基于“等待 100% 信号”的 Loop，且不设超时退化，那么控制循环将陷入**无限阻塞或无限向输入框敲回车重发**的灾难中，从而彻底锁死整个会话。
- **纠偏建议**：不存在 100% 可靠的逻辑信号。Loop 必须包含 **“最大重试次数”与“超时衰退降级”** 防线。当强信号（UPS Hook）超时丢失时，系统必须允许通过弱信号（PTY 字符变化 / cgroup CPU 活跃）进行“降级确认”，或者触发 Escalation 抛给用户拍板，而不是死等。

### 2. 异步 PTY 视图延迟与自愈自激震荡
- **失效场景（Visual Delay / Command Queue Buffering）**：用户以“清空输入框”子 Loop 为例：`按 ctrl+u -> 观察 pane 是否空 -> 不空则补空格 -> 再观察`。
- **致命后果**：PTY 的 `capture-pane` 读取是**非阻塞的异步渲染流**。当宿主机 CPU 瞬时高载或 tmux 渲染延迟时，即使 `ctrl+u` 已经清空了 PTY 的输入缓冲，`capture-pane` 在前几次 Loop 里读出来的字符依然可能带有残留。如果子 Loop 响应速度太快（例如无间隔 Busy-loop），它会误判为“清空失败”，疯狂发送空格和 `ctrl+u`。这些指令在 tmux 管道中积压，待系统卡顿恢复后，会在 PTY 中瞬间爆发，**反而把原本已经干净的输入框塞满垃圾字符**，甚至触发 Agent 执行非法命令。
- **纠偏建议**：所有的 PTY “读-判-写” 循环必须是 **非阻塞异步且带时间间隔的（Staggered with Backoff）**。判定“是否成功”前必须给予 PTY 足够的缓冲时间让渲染落盘，不能采用强同步的事务性思维来写 PTY 脚本。

### 3. 子 Loop 递归膨胀的工程深渊
- **失效场景**：如果清空输入框需要一个子 Loop，那么“发送 ctrl+u”这个动作本身是否成功也需要被确认；进而，确认“是否观察到字符”的 tmux 指令本身如果卡死（如 tmux socket 挂起），是否也需要再起一层子 Loop 监控？
- **致命后果**：系统陷入无限的子 Loop 递归，状态机复杂度将呈指数级爆炸，退化为不可维护的意大利面条代码。
- **纠偏建议**：限定最大闭环层级。只有“任务级（Task-level）”和“原子动作级（Action-action）”两层 Loop，不允许出现无限制的子 Loop 嵌套。

---

## 三、 候选 Loop 状态机切分方案

我们将一个完整的 Agent 任务周期分解为以下五个阶段的 Loop，并给出理想跳出条件及构造方式：

```
+------------------+     +--------------------+     +-------------------+     +-------------------------+     +----------------------+
| 1. Ready-Gate    | --> | 2. Dispatch-Gate   | --> | 3. Start-Gate     | --> | 4. Execution-Gate       | --> | 5. Completion-Gate   |
| (环境与输入框就绪) |     | (提示词投递与敲回车)|     | (模型接受开工确认) |     | (运行期心跳与存活检测)   |     | (物理与逻辑双锁完成)  |
+------------------+     +--------------------+     +-------------------+     +-------------------------+     +----------------------+
```

### 1. Loop 1: Ready-Gate (环境就绪循环)
- **动作**：向 PTY 发送 `ctrl+u` 并清空残留。
- **跳出条件**：`capture-pane` 连续两次采样（间隔 150ms）确认输入行除 Prompt 引导符外无任何英文字符，且无悬空的 `stdout` 输出。
- **构造方式**：当前代码库无此逻辑。需在 `realign.rs` 派单前置动作中，封装 PTY 读写环实现。

### 2. Loop 2: Dispatch-Gate (投递确认循环)
- **动作**：向 PTY 投递 Brief 文本并写入回车键 `\n`。
- **跳出条件**：`capture-pane` 检测到刚才输入的文本已经向上滚动至“历史输出区”，且底部的 Composer 区域重新进入不可编辑或等待状态。
- **构造方式**：利用正则表达式对 PTY 屏幕字符做精确的位置比对。

### 3. Loop 3: Start-Gate (任务开始确认循环)
- **动作**：静默等待。
- **跳出条件**：`UserPromptSubmit` (Claude) / `PreInvocation` (Agy) / `worker.started` wrapper rpc (Codex) 到达 `CCB_SOCKET` 并由事件日志录入。
- **构造方式**：对齐 [agent.rs](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/agent.rs) 的事件响应逻辑，将 `userpromptsubmit` 信号扩展至 Worker，并在数据库中将状态标记为 `BUSY`。

### 4. Loop 4: Execution-Gate (运行存活循环 - Liveness)
- **动作**：周期性（如每 10 秒）安全观测。
- **跳出条件**：当收到 `PreToolUse` / `PostToolUse` 事件，或观测到 cgroup 物理资源有 CPU 占用增加，或 PTY 的 `mtime` 有文件变更增量。
- **构造方式**：通过 cgroup 委托（LF1）与 Reconciler 定时心跳（GC pass）联动，对处于 `BUSY` 状态的 Agent 做无损检测。

### 5. Loop 5: Completion-Gate (完成收敛循环)
- **动作**：静默等待。
- **跳出条件**：同时满足物理与逻辑的**双重联锁（Double-Lock）**：
  1. 逻辑信号：`Stop` 钩子推送到达。
  2. 物理信号：cgroup `populated` 状态翻转为 0，确认该 slot 下没有遗留的孤儿后台进程。
- **构造方式**：与 GF4（显式完成协议）和 LF1（cgroup 委托）直接对接。

---

## 四、 与编排底座 GF1/GF4 重构的关系

任务生命周期确认协议绝不是一个孤立的 feature，它与正在进行的底座重构有强烈的双向绑定：

```
       +---------------------------------------------+
       |           GF1 (事件日志与 Reconciler 脊柱)    |
       |  - 唯一持有 DB 写连接                         |
       |  - 驱动状态转移 (DISPATCH -> BUSY -> IDLE)   |
       +---------------------------------------------+
                              ^
                              | 投递 PerceptionEvent
                              |
       +---------------------------------------------+
       |          任务生命周期协议 (LCP)              |
       |  - 采集 UPS/PreInvocation/started 信号     |
       |  - 充当 GF4 (完成协议) 的开工断言             |
       +---------------------------------------------+
```

1. **它是 GF4 (显式完成协议) 的绝对对称件**：
   - GF4 核心负责“如何优雅、显式地收尾”，但它假设任务状态已经进入了 `BUSY`。如果没有生命周期确认协议在开始端进行“UPS / started 信号”的拦截与确认，GF4 就无法断言“当前任务是真的开始并需要等待完成，还是根本卡在了派发通道”。两者必须合起来构成完整的生命周期协议。
2. **它是 GF1 (事件脊柱) 的高置信度数据源**：
   - 根据 GF1 的低耦合断言（[design-substrate-redesign-draft-2026-07-12.md:177](file:///home/sevenx/coding/ccbd-rust/research/orchestration-substrate-redesign/design-substrate-redesign-draft-2026-07-12.md#L177)），外部模块不允许直接调用 `mark_agent_*` 写 DB。
   - 因此，生命周期确认协议中采集到的所有信号（UPS, started, PTY回显字符）都必须作为 **`PerceptionEvent`** 投递给 GF1 事件脊柱，由唯一的 `StateReconciler` 统一处理状态演进。这强化了 GF1 作为单一真相源的设计。

---

## 五、 同族病历的第一性归因

以下四个典型历史故障在第一性原理下的病理归因如下，并评估“Loop + 可靠信号”是否为对症之药：

### 1. dispatch-ACK 竞态
- **病理归因**：**输入阻抗不匹配（Input Impedance Mismatch）**。系统直接执行 `tmux send-keys`（写操作）后，没有等待 PTY 的物理缓冲与 TUI 的 AST 解析完毕，就默认投递已经“生效”并标记为了 ACK。
- **LCP 对症性**：**完全有效**。引入 Loop 2 和 Loop 3，只有在 `capture-pane` 中确认 Brief 回显，且接收到 Provider 端被动抛出的 `UserPromptSubmit`/`started` 信号后，才将状态移交 `BUSY`。若信号超时，自动发起重置与重试。

### 2. claude banner/幽灵提示词误判
- **病理归因**：**无状态事件流的世代污染（Generation Pollution）**。由于没有“任务开始信号”做隔离，旧的完成检测器（Pull 模式）在非活动期间错误读取了 PTY 上的历史文本。由于缺乏 Epoch（世代）标识，它无法分清该 `stop_reason` 是上一轮任务的遗留还是本轮任务的产物。
- **LCP 对症性**：**有效**。结合 Loop 1（物理清空）与 LCP 开始信号，我们强制为每一次任务绑定唯一的 `state_version`（即 Epoch）。所有在此 Epoch 开始信号之前发生的 PTY 字符都将被彻底过滤。

### 3. R2 就绪探针被自己的注入击穿
- **病理归因**：**侵入式主动探测（Active Intrusion）破坏了状态机内部的一致性**。在单线程、带缓冲的 AI TUI 交互中，外部主动注入探测字符会直接污染 Agent 正在编辑的输入框，将其搞乱。
- **LCP 对症性**：**部分失效。此病根源在于“主动探测”这一反模式，而非缺乏 Loop**。该病历的治本方案是摒弃一切侵入式 poll 动作，彻底转向**被动接收（Passive Reception）**。Loop 的跳出条件必须是 Wrapper 进程或 Agent 自发抛出的事件，而不是由 hypervisor 注入试探键值。

### 4. agy 假 COMPLETED
- **病理归因**：**判定主体的权力错位（Authority Illusion）**。Agent 在对话层面宣称“已完成”，但操作系统底层的 tmux / 编译器进程实际上还在消耗 CPU 写入。
- **LCP 对症性**：**单靠逻辑信号无效，必须引入物理资源联锁**。大模型的话是不可信的，所以 `Stop` 逻辑 Hook 不足以作为 100% 可靠的跳出条件。必须在 Loop 5（完成确认）中，将“逻辑 Hook”与“物理 cgroup 委托的 `populated=0` 特征”做**与逻辑联锁（Double-Lock）**，才能根治此病。
