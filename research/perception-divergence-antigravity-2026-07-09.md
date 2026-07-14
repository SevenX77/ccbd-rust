# 感知机制两难题的发散设计与架构方案（独立思考轮）

本设计由 `a3-antigravity` 基于第一性原理产出。主要解决 Agent Hypervisor (ah) 在编排交互式 CLI coding agent 时面临淡核心感知难题：多信号源的状态收敛，以及交互式 CLI agent 的“任务真完成”判定。

---

## 难题一：多信号源如何收敛为单一可信状态？

### 1. 现状审视与失效机制批判
根据代码审查，当前的代理状态更新由多个监视器在不同事件触发时“各自写状态”或“先到先得”：
- **Pty/FIFO 读取与日志监控器**（如 `src/completion/monitor.rs#L37-L45`）在检测到 `TurnComplete` 时，调用 `mark_agent_idle_log_event`。
- **健康检查监控器**（如 `src/provider/health_check.rs#L134-L138`）在超时无进度时，调用 `mark_agent_stuck` 将状态强制置为 `STUCK`。
- **TMUX 屏幕捕获与 Pane Diff 监视器**（如 `src/pane_diff/mod.rs#L294-L311`）在匹配到 UI 提示符时，异步调用 `mark_ui_completion_recaptured_agent` 收回状态至 `IDLE`。

**失效场景分析**（`grep-before-claim` 验证）：
1. **回合结束与任务完成混淆（先到先得竞争）**：在 `src/db/state_machine.rs#L427-L487` 的 `mark_agent_idle_recaptured_outcome_sync_with_pane_inner` 中，如果 PTY 缓冲区里前一次任务的残留屏幕文本与当前新 job 的提示符混淆，Pane Diff 可能会抢在 Log Monitor 之前将正处于 `BUSY` 甚至刚被 Dispatch 的 agent 误判定为 `IDLE`（即 `STATE_IDLE`），导致新 Job 的输出流丢失或被后续 job 覆盖。
2. **幽灵提示符与假死锁定**：在 `src/pane_diff/mod.rs#L461-L481` 里的 `sanitize_for_diff` 对 spinner 等进行了过滤，但在某些情况下，如果 agent CLI 的输出包含类似 `esc to cancel` 的特征码（如 `src/provider/manifest.rs#L418` 针对 `antigravity` 定义的 `idle_anti_pattern`），正则匹配的抖动会导致状态在 `STUCK` 和 `IDLE` 之间来回跳变。

---

### 2. 三个候选收敛架构方案

#### 候选架构一：集中式异步事件流与单写状态机仲裁（Centralized Arbitrated FSM / Single Writer）
- **哲学**：剥夺所有独立监测器（Monitor Tasks）直接写入数据库状态的权力。将其降级为只读的“信号生成器”（Signal Producers），它们将带时间戳和序列号的 `PerceptionEvent` 推入统一的异步事件通道（Event Channel）。由一个独立的、单线程逻辑的“状态仲裁者”（Arbitrator）作为唯一的状态写入者（Single Writer）来顺序处理事件并维护状态机。
- **适用前提**：系统支持统一的内部事件总线，且状态转换图及转移判定规则能够完全集中维护。
- **失效模式**：仲裁者线程/协程一旦死锁或卡顿，整个感知系统将瘫痪；如果输入信号由于进程调度产生极端乱序，需要有时间戳重排（Reordering Window）机制，否则会误判。
- **复杂度代价**：中等。需要实现标准的事件总线和集中的仲裁状态机。
- **业界先例**：
  - *确定的先例*：Kubernetes Reconciler (Controller Manager)。各个组件将状态报告给 API Server，但最终的状态调节和决策由单独的 Controller 逻辑串行化处理。（确定存在）
  - *确定的先例*：Erlang/OTP 中的 `gen_statem`。通过隔离状态和并发消息传递，确保状态转移是确定且无竞争的。（确定存在）
  - *确定的先例*：自动驾驶或航空航天中的卡尔曼滤波与集中式传感器状态估计。（确定存在）

#### 候选架构二：逻辑纪元与悲观屏障锁共识（Epoch-based Pessimistic Barrier / Versioned Consensus）
- **哲学**：在数据库/共享状态层引入单调递增的逻辑纪元（Epoch / State Version）。状态从 `BUSY` 转移到 `IDLE` 不能通过单一信号触发，而是必须在当前 Epoch 内集齐“证据链屏障”（Evidence Barrier）。例如，必须集齐 `[Hook: TurnComplete]`、`[Log: EOF]`、`[UI: Stable Prompt]` 三个证据槽。任何过期的信号（Epoch 小于当前）直接作废。
- **适用前提**：底层存储支持原子的 CAS（Compare-And-Swap）操作（如当前 `src/db/state_machine.rs` 中利用的 `state_version`）。
- **失效模式**：当某个低可靠信号源（如 Hook 推送接口）因网络故障永久丢失时，若无悲观的超时降级保护，状态机会发生“活锁”（Livelock），永久卡在 `BUSY`。
- **复杂度代价**：高。需要维护分布式的证据屏障与复杂的降级路径。
- **业界先例**：
  - *确定的先例*：Raft/Paxos 协议中的 Term / Epoch。用于解决分区环境下的状态覆盖问题。（确定存在）
  - *确定的先例*：工业控制双机热备（如铁路连锁系统）的“二取二”（2-out-of-2）投票逻辑，必须两个独立通道一致才放行。（确定存在）
  - *不确定的印象*：某些工控表决系统会使用“动态优先级屏障锁”，我们印象里其在硬件层面实现，称为“双轨符合检测”。

#### 候选架构三：反应式时间窗口加权投票（Reactive Sliding Window Weighted Voting）
- **哲学**：引入滑动时间窗口（如 500ms），周期性收集在此窗口内所有监视器发出的状态判定。每个监视器拥有不同的静态权重（静态优先级分配：`pidfd` 权重 100，`Hook` 权重 80，`Log` 权重 60，`UI Capture` 权重 30）。在窗口结束时计算加权得分，超过得分阈值则执行状态迁移。
- **适用前提**：系统允许一定的状态判定滞后（即窗口大小），信号生成频率较高。
- **失效模式**：由于网络或 IO 调度导致高权重信号（如 Hook）被延迟到下一个窗口，而低权重信号在当前窗口高频堆叠，导致误判；滑动窗口的边界切分问题可能导致信号割裂。
- **复杂度代价**：低。利用 Rx (Reactive Extensions) 或 Rust `tokio-stream` 的滑动窗口函数容易实现。
- **业界先例**：
  - *确定的先例*：自动驾驶中激光雷达、摄像头与超声波的多传感器数据融合（Sensor Fusion）。（确定存在）
  - *不确定的印象*：网络安全入侵检测系统（IDS）通过滑动窗口关联多个低可靠性日志来判定是否发生真实攻击。（不确定）

---

### 3. 高可靠信号缺失/失效时的退避与探测机制

当高可靠信号（如 `pidfd` 退出事件、`Hook` 推送）缺失或失效时，系统**绝对不能**盲目信任低可靠信号（如 `UI Capture`）来推进状态，必须采取**悲观降级与主动探测相结合**的策略：

1. **状态自动挂起与悲观降级**：
   - 如果高可靠信号丢失（例如 `pidfd` 失效但进程表仍能查到该 pid），状态机应立即转移至 `SPAWNING_INTERVENTION` 或 `STUCK`。
   - 封锁任务派发队列，挂起该代理的调度，防止破坏现场。
2. **主动心跳探测（Active Probing / Injection）**：
   - 向 TMUX 交互式终端隐式注入安全的“空指令”或“控制序列”（如 `echo $?` 或 ANSI 查询指令 `\x1b[6n`）。
   - 监听 PTY 是否有即时、标准的回显。如果能成功回显，说明终端 Shell 处于活跃可响应状态；若超时无回显，判定为真实卡死，执行强制销毁。
3. **带超时的安全熔断（Failsafe Cutoff）**：
   - 设定基于物理时间（Wall-clock time）的绝对最大空闲阈值。一旦触发，无论低可靠信号如何宣称“忙碌”或“思考”，均执行 cgroup 级的 `SIGKILL` 终止，保证资源释放。

---

### 4. 难题一的推荐方案与第一性理由

**推荐：候选架构一（集中式异步事件流与单写状态机仲裁 Arbitrated FSM）**

**第一性理由**：
1. **单一可信源写入（Single Source of Truth Write）**：消除竞态的唯一方案是消除并发写。让所有的 Monitor 弱化为“爆料人”（只发 Event），让 Arbitrator 成为唯一的“法官”（写 DB 状态），从根本上杜绝了因“先到先得”导致的状态倒流与重写。
2. **延迟防抖与时序恢复（Temporal Debounce & Ordering）**：仲裁者可以容易地在内存中实现“事件防抖”。例如，UI Capture 匹配到 IDLE 时，仲裁者不立刻转移状态，而是等待 `300ms`，若在这期间收到了 Log Monitor 判定为 `BUSY` 的高优先级事件，则直接丢弃 UI 匹配事件。这在分布式或多监视器架构中是最健壮的收敛手段。
3. **极其优异的调试审计性（Auditability）**：由于所有的输入都变成了明确的结构化 Event，仲裁者可以持久化“Perception Log”，开发者可以通过回放事件流（Event Stream）轻松复现和定位任何状态误判的 Bug，这对于复杂的 daemon 守护进程是至关重要的。

---
---

## 难题二：交互式 CLI agent 的"任务真完成"如何判定？

### 1. 对"判完成前检查 pane 下有无存活的非 shell 子进程"提议的攻击

该提议的核心漏洞在于：它假设**进程树的父子层级结构能够完美等价于逻辑上的工作从属关系**。然而在现代 OS 及复杂多进程链下，这种等价性在两个方向上都会发生严重误判。

#### 误判方向一：不该判完（任务还在进行，但判定为完成 - False Positive）
1. **后台双 fork 守护化（Double-fork / Daemonization 脱离）**：
   - 如果 Agent 发起的子进程执行了经典的后台守护化（如编译守护进程 `rust-analyzer` 自启、持久化测试服务器拉起、Webpack 等前端热更新服务双 fork 退出），该进程的父进程 PID 将变为 1（或在 Linux cgroup sub-scope 中被回收至宿主，不再隶属于 Tmux Pane Shell 的父子树）。
   - 此时，普通的进程树遍历（如从 Shell PID 往下寻找子进程）将无法匹配到这些脱离父子链的真实工作进程。系统误判为“无非 shell子进程存活”，提前标记完成并终止环境或启动下一步，导致任务在后台被无预警掐断或状态踩踏。
2. **进程拉起时间间隙（Inter-stage Spawning Gap）**：
   - 在复杂的多阶段脚本执行中（例如 `cargo build && cargo test`），前一个阶段进程（`cargo build`）已经退出，而前台 Shell 在准备拉起后一个阶段进程（`cargo test`）时存在微秒级的操作系统调度间隙。
   - 如果监控器的轮询 tick 恰好落在这个极小的间隙中，检测到的非 shell 子进程数为 0，系统会宣告任务提早结束，导致随后的 `cargo test` 被异常终止。
3. **任务异步挂起（Backgrounding via job control）**：
   - Agent 如果执行了带 `&` 的后台命令（如 `pytest tests/ &`），前台 Shell 随即打印了提示符，但在微秒级时间内子进程尚未完全被 `fork/exec` 出来并纳入监测视图。若此时扫描进程树，会由于时间窗口偏差产生严重的过早完成误判。

#### 误判方向二：该判完（任务已经结束，但被判定为仍在工作而挂起/卡死 - False Negative）
1. **开发常驻辅助进程（Ambient / Ambient Daemons）**：
   - 在开发工作流中，很多工具在首次运行后会启动常驻开发辅助后台进程（如 `gpg-agent`、`direnv` 自动加载、`nix-daemon`、甚至某些编译器缓存守护进程）。它们将长期存在于当前 Shell 的子进程树中且不主动退出。
   - 如果强制实施“必须无任何存活的非 shell 子进程”才算任务完成，系统将判定 Agent 一直处于 `BUSY` 状态（最终被健康检查踢入 `STUCK`，参见 `src/pane_diff/mod.rs#L217-L233`），导致调度器死锁，无法接收新 Job。
2. **僵尸子进程残留（Defunct / Zombie Processes）**：
   - Agent 执行的某个测试用例异常崩溃，产生了一个僵尸进程（Zombie），它占用了进程 PID 但已经停止了任何逻辑活动，等待 Shell 退出或父进程回收。
   - 系统如果盲目探测其存活，会认为非 shell 子进程仍在运行，导致无法收尾。

---

### 2. 三个替代或增强判定机制

#### 替代机制一：基于 Systemd Scope / Cgroup v2 的资源活跃度静默期审计（cgroup Quietness Detection）
- **确定性来源**：既然本仓库已将 Agent 进程隔离在独立的 systemd transient scope (cgroup) 中（如 `src/systemd_unit.rs` 定义），我们可以直接读取 `/sys/fs/cgroup/systemd/ahd-session-xxxx.service/cpu.stat` 以及 `io.stat`。计算整个 scope 下所有子孙进程的 CPU 累计消耗和磁盘 IO 变动。如果在滑动的 $T$ 秒内，cgroup 的累计 CPU 时间增量 $\Delta CPU < \epsilon$ 且磁盘 IO 增量 $\Delta IO = 0$，即使有百个子进程存活，也可确信全进程组处于休眠/空闲态。
- **坑**：某些命令如果处于长久的纯等待或网络等待态（例如执行 `sleep 60` 或 `curl --retry`），其 CPU/IO 确实在一定时间内为 0。因此静默期时间窗口必须设置合理，并结合 Shell 提示符特征表决。

#### 替代机制二：物理证据阻断屏障（Physical Evidence Barrier Assertions）
- **确定性来源**：在 Orchestrator 状态收敛前，根据 Job 类型进行“物理证据”校验。如代码修改任务（Mutating Job）在宣告结束前，必须有 Git 工作区变更（`git diff`）或特定的测试覆盖率日志文件产生，否则判定为“无效退出”，拒绝状态进入 `IDLE`。
  - 注：当前 `src/db/state_machine.rs#L30` 中的 `EVIDENCE_DENY_MESSAGE` 已经有了此思路雏形。
- **坑**：只读查询类任务（Read-only Job）天然不修改代码，物理断言会产生死锁。必须在任务分发阶段由 Orchestrator 进行静态标记（如 `is_mutating: bool`），针对只读任务豁免此断言。

#### 替代机制三：基于 PTY 终端的主动回显心跳插桩（Active PTY Sync Probing）
- **确定性来源**：在监测器匹配到 UI 层的 IDLE 提示符后，如果检测到仍有后台子进程，Monitor 向 PTY 伪终端隐式注入一条安全的控制探测序列（例如 ANSI Device Status Report `\x1b[6n` 或写入特定的无害控制键）。若能立即收到伪终端的标准回显，说明前台 Shell 正处于交互态且前台无命令阻塞，那些活跃的子进程皆被压入了后台，前台任务在逻辑上已经“真正收尾”。
- **坑**：如果 Agent CLI 此时处于全屏交互编辑器（如 `vim`、`less`）或正在等待交互式输入的 prompt 界面，盲目注入控制键会直接破坏终端布局，且对于 `antigravity` 这类使用 `Escape` 做 cancellation 的 CLI，会有意外终止推理的风险。

---

### 3. 这些机制在 ccbd-rust 技术栈下的移植性说明

在 ccbd-rust 目前的技术架构（Rust + tmux + systemd scope + 三种异构 provider CLI：Claude, Codex, Antigravity）下，这三种替代机制的可移植性表现如下：

1. **Systemd Cgroup 资源静默检测**：
   - **移植性限制**：强依赖 Linux 宿主机环境。对于本地 macOS 或 Windows 调试环境，没有 Systemd Scope 也没有 cgroup，此功能无法工作，必须提供退避（Fallback）到纯进程树判定（如使用 `sysinfo` 库在用户态递归查询进程状态）的机制。
2. **物理证据阻断屏障**：
   - **移植性限制**：极好。它完全跑在 Rust 和 SQLite DB 的上层业务逻辑层，跨平台（Linux / macOS / Windows）及异构 CLI Provider 100% 兼容，是极佳的通用安全垫。
3. **PTY 终端心跳插桩**：
   - **移植性限制**：中等。虽然 `tmux send-keys` 跨平台，但对于不同的异构 Provider，匹配规则存在极大差异。Claude Code 和 Antigravity 对控制字符的响应不同（如 `antigravity` 接管 `Escape` 键），因此对于不同的 provider，必须在 manifest 中声明独立的 Probing 密钥映射表。

---

### 4. 难题二的复合判定推荐组合

**推荐采用：“前台交互提示符 + cgroup 静默审计 + 物理证据屏障”的复合表决机制**

**工作流设计（逐级收网逻辑）**：
1. **第一级：UI 稳定提示符判定（Frontend Guardian）**：
   - 必须通过 VT100 解析器捕获当前 Tmux Pane 的最后一屏，且提示符在连续的 2 个 Tick 内哈希未改变（即 `src/pane_diff/mod.rs#L161` 里的稳定 tick 校验已通过）。
2. **第二级：进程树与 cgroup 协同过滤（OS Auditing）**：
   - 检查 Pane Shell 下的子进程。如果没有任何子进程，或者虽然有非 shell 子进程（如 `rust-analyzer`），但对应的 systemd scope cgroup 的 CPU/IO 在最近 3 秒内增量低于阈值，则认为该子进程不占有前台控制权，通过这一级。
3. **第三级：任务级物理证据核验（Evidence Assertion）**：
   - 由 Orchestrator 判断当前 Job 是否有修改意图（如 `mutating: true`）。
   - 若有，调用 `git diff` 检查是否有任何代码修改，或检查工作区是否新增了测试/构建产物。如果“两手空空”，则状态拒绝退回 `IDLE`，同时向 PTY 发送系统 Nudge 命令（如 `src/db/state_machine.rs#L30`），迫使 Agent 继续干活。

---
---

## 落地折衷方案与自我诊断

### 1. 落地折衷（Pragmatic Implementation Compromises for CCBD-Rust）

鉴于 ccbd-rust 目前的架构实现，为了避免对异步并发框架和数据库模型进行破坏性重构，在具体实施推荐方案时，我们建议采取以下折衷设计：

1. **利用 SQLite 事件日志作为“虚拟事件总线”与仲裁序列化器**：
   - **折衷设计**：与其引入类似 `actix` 或复杂的多生产者单消费者（mpsc）物理 Actor 线程来做仲裁，不如直接利用已有的 SQLite DB 作为事件排序器。
   - 所有 Monitor 任务仅通过执行简单的原子 SQL 插入，将观测到的感知事件写进新表 `perception_events`（含时间戳、AgentID、源头、内容）。
   - 在已有的 `orchestrator::wake_up()` 调度周期（通常由 `crate::orchestrator::pubsub` 触发）中，串行提取该 Agent 未处理的 perception 事件流，完成判定后，用单次 CAS 事务更新 `agents` 状态。这能在保留并发 Monitor 的同时，消除状态修改的并发竞态。
2. **基于 `sysinfo` 用户态库在非 Linux 平台退避降级**：
   - **折衷设计**：在非 Linux 平台（如 macOS/Windows 本地测试）直接关闭 cgroup 静默期检测功能（使用 `#[cfg(target_os = "linux")]` 条件编译隔离）。
   - 降级为使用 Rust 的 `sysinfo` 库获取 shell 树下的子进程列表，并通过遍历进程状态（检查进程是否处于 `Sleep` 状态且累积 CPU 占用变化极低）来实现近似的“静默度检测”。

---

### 2. 实际覆盖范围与最弱区域自我诊断（Coverage & Weakest Areas Audit）

- **实际覆盖范围（Coverage Audit）**：
  - 本设计已经彻底完成了对 `src/monitor/agent_watch.rs`（物理死亡监视）、`src/completion/monitor.rs`（日志解析监视）、`src/pane_diff/mod.rs`（屏幕 UI 变化监视）及 `src/provider/health_check.rs`（健康超时与恢复判定）四大核心状态感知层的代码级批判。
  - 针对多信号源先到先得的并发竞争问题、开发辅助常驻子进程干扰判定、双 fork 进程逃逸误判等边界场景，给出了第一性原理级别的机制架构方案。

- **最弱区域与潜在风险（Weakest Areas / Residual Risks）**：
  - **进程创建空档期的微秒级误判风险**：尽管有 cgroup 活跃度检测，但在多阶段任务（如 `cargo build && cargo test`）的极短暂进程交接期内，依然存在 cgroup 的 CPU/IO 增量在微秒级时间内为 0 且 Shell 处于前台处理期的极端盲区。针对此项，单纯依赖被动观测是不够的，必须在 Monitor 端引入一小段“静止确认防抖延迟”（如 300~500ms），待静默状态持续半秒以上才允许触发状态变更。
  - **只读任务与修改任务的识别精度**：如果 Orchestrator 错误地将一个原本只读的任务标记为了 `mutating: true`，或者 Agent 正在运行的任务因为权限不足导致无法做出任何实际的代码修改而只好放弃，证据阻断机制会陷入无限拒绝状态转换的死循环。必须为证据屏障设计“最大拦截限制次数”（例如：连续发出 2 次 nudge 警告后，第三次强行释放状态或交由人工干预）。
