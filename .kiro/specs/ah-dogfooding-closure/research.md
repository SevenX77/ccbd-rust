# Research: ah dogfooding closure (自驱闭环研究)

## §1 user 立项目标 precise statement

> 主控用 ah 派 a1/a2/a3 自驱跑一个完整 SOP-08 任务 (PR-6 体量), 整个过程不撞 ccb 痛点 (无 cancel stuck job / 无 loop poll 干等 / 无 capture pane 人工 verify). e2e 测试本身 dogfood: e2e 测试用 ah master client 替代 ccb ask 来 dispatch agents.

> 用户参与只有需求挖掘和目标敲定. 实施全是主控+agents 按 SOP-08 13 步自驱闭环 (research→design→impl→e2e→不闭合再回 research→闭合才请 user squash merge).

## §2 现有 ah 覆盖图谱

### 2.1 PR 覆盖
- **PR-1 (Mainline)**: `tests/ah_full_e2e_main.rs` 锁住了基础生命周期 (Start -> Ask -> Up -> Ask -> Stop)，但 `Ask` 步骤使用了 `dispatch_and_complete_job` 手动修改 DB 状态 (test seam)，尚未实现真实的 completion detection。
- **PR-2 (Drift)**: `tests/ah_full_e2e_drift.rs` 锁住了 ENV/HOOKS/PLUGINS 漂移与 NEW agent 拓扑演进，解决了配置不一致导致的各类诡异行为。
- **PR-3+6 (Extra)**: `tests/ah_full_e2e_realign_extra.rs` 锁住了 ORPHAN、BUSY (skip/force) 与 CRASHED recovery，解决了 "pane alive ≠ provider alive" 的核心痛点。

### 2.2 CLI & RPC 工程事实
- **CLI Subcommands (16个)**: `Ping`, `Version`, `Ps`, `Start`, `Up`, `Ask`, `Pend`, `Cancel`, `Kill`, `Watch`, `Logs`, `Attach`, `Stop`, `Doctor`, `Config`, `Prompt`。
- **RPC Methods (22个)**: 
  - Session: `create`, `kill`, `spawn_master_pane`, `realign`, `list`
  - Agent: `spawn`, `realign`, `send`, `read`, `watch`, `kill`, `resolve_prompt`, `assert_state`, `discard_evidence`
  - Evidence: `insert`
  - Job: `has_evidence`, `mark_requires_evidence`, `submit`, `wait`, `cancel`
  - System: `dump`, `shutdown`
- **内部状态机**: `STATE_SPAWNING`, `STATE_IDLE`, `STATE_WAITING_FOR_ACK`, `STATE_BUSY`, `STATE_PROMPT_PENDING`, `STATE_STUCK`, `STATE_CRASHED`, `STATE_KILLED`, `STATE_UNKNOWN`。

## §3 群 A 现象 (14 项) vs ah 现状对照表

| 群 A 现象 (Synthesis 4-26) | ah 状态 | 实证 / 备注 |
|---|---|---|
| tmux 投递成功但无 Enter | ⚠️ | `agent.send` 逻辑含但无 e2e 物理 verify；且 paste 路径未映射 slash cmd |
| shell quoting 损坏 prompt | ✅ | Rust 使用强类型 JSON-RPC 传参，避免了 bash 拼接 |
| killed task 无 GC，永久排队 | ❌ | **未闭合**。PR-3 case_07 仅是 ORPHAN 清理，非队列自动 GC (见 §5 痛点) |
| mailbox_state=delivering 但丢失 | ⚠️ | 仍需 `agent.read` 实现 ACK 闭环，目前仅靠 DB 状态 |
| reply 重影 | ✅ | Rust `vt100` parser 与 `agent_io` 隔离了 raw output 与 event |
| Codex 虚报 commit hash | ✅ | ah 提供真 provider 监听，但不干预 LLM 幻想 (归属 PR-7+ 验证器) |
| TD-008: Gemini Announce 被当 reply | ⚠️ | **未闭合**。需要 provider-aware detector (目前 §6.1 仍为 test seam) |
| settle_window 全局参数过短 | ✅ | ah 不使用固定时间等待，基于 marker 事件流驱动 |
| Gemini 静态状态栏干扰检测 | ✅ | `vt100` parser 增量扫描，忽略静态区域 |
| watch_status 误捕 shell 准备日志 | ✅ | 基于 `InitProbe` 状态机，只有 IDLE 后的输出才视为有效 output |
| pane alive ≠ provider alive | ⚠️ | PR-6 锁了 CRASHED 路径，但 completion 真路径尚未 e2e 验证 |
| Bash tool 10min 超时切断 --wait | ⚠️ | 架构支持但无 e2e 验证 "ah ask --wait timeout 不丢 job" |
| completion detector 捕获旧 READY | ✅ | SQLite SoT 与 event_id 保证只取 job 提交后的新事件 |
| codex stale session ID 死循环 | ⚠️ | PR-6 仅针对 claude worker，codex resume 逻辑仍存风险 |

## §4 群 D 缺陷 (15 项) vs ah 现状对照表

| 群 D 设计缺陷 (Synthesis 4-26) | ah 状态 | 实证 / 备注 |
|---|---|---|
| #1 缺 SoT (散在 FS) | ✅ | SQLite 统一持久化 (`src/db/mod.rs`) |
| #2 env allowlist 静默剥离 | ✅ | `manifest.env_passthrough` 显式白名单与强类型配置 |
| #3 ccbd singleton 没隔离 | ⚠️ | PR-1 多 session 验证不等价单 cwd 多 master 共享隔离 |
| #4 完成态没主动通知 | ⚠️ | 已有 in-process 广播给 `job.wait`，但缺持久/外部可靠 push |
| #5 pane alive ≠ provider alive | ⚠️ | 已识别 CRASHED，但 `STUCK` 状态的主动判定尚浅 |
| #6 session 文件写死 stale ID | ⚠️ | PR-6 仅对 claude 实施，未覆盖所有 provider 机制 |
| #7 默认无 stuck 检测 | ⚠️ | 核心逻辑在 `pane_diff` 与 `orchestrator` 300s 阈值，需 e2e 锁死 |
| #8 状态机抖动无 backoff | ✅ | `realign` 采用调谐逻辑，非简单 loop restart |
| #9 janitor 始终时钟偏移 | ✅ | 内部 Reconciliation Loop 不依赖外部 systemd LastTrigger |
| #10 CLI 退出码与健康脱节 | ✅ | `ah` 客户端统一通过 RPC 错误码映射退出码 |
| #11 .ccb-requests 互锁 Git | ✅ | ah 不使用 `.ccb-requests` 投递，直接 RPC 内存 → DB |
| #12 provider ready 单次驱动 | ✅ | `InitProbe` 支持 per-provider 的动态稳定性窗口 |
| #13 CCB v6 symlink 不全 | ✅ | `home_layout` 物化逻辑统一管理所有 link 依赖 |
| #14 CCB v5→v6 不兼容 | ✅ | ah.toml v1 格式锁定，不再依赖旧 JSON layout |
| #15 claude-sandbox MemoryMax | ✅ | `wrap_command` 默认不加硬限，除非显式配置 |

## §5 PR-6 实测介入点根因映射

| 介入行为 | 频率 | 映射到 A/D 缺陷 | ah 闭合方案 |
|---|---|---|---|
| `ccb ask cancel` (stuck) | 6 次 | A (Announce 误判), D (无 stuck 检测) | Provider-aware detector + STUCK timer |
| `tmux capture-pane` | 15+ 次 | D (信号不可信), A (pane alive 假象) | `ah attach` + 强类型状态机事件订阅 |
| `ScheduleWakeup` (poll) | 12 次 | D (无 push 通知) | `ah watch` 长连接事件推送 |
| Thinking hang | 1 次 | D (无 stuck 检测) | `stuck_timeout` 自动记录并上报 |
| Stuck queue 堵塞 | 3 次 | A (队列 GC 失效) | SQLite 事务性状态变更，KILLED 自动置换 Head |

## §6 dogfooding 前提: ah master client 缺失项

若要实现 "主控用 ah ask 替代 ccb ask"：
1. **Completion Detection (核心)**: 必须移除 `dispatch_and_complete_job` 这一 test seam，实现真正的 `agent_io` 监听 tmux pane 并根据 `InitProbe` 和 `MarkerMatcher` 自动驱动 Job 状态 (BUSY -> IDLE)。
2. **Push Notification Layer**: `ah daemon` 需要一个实时的广播层，当 Job 状态在 SQLite 中变更时，能立即通知阻塞在 `ah ask --wait` 上的客户端。
3. **Slash Command Mapping**: 必须支持 `/clear`、`/new` 等在各 provider (Claude/Gemini/Codex) 间的透明映射，解决 Bug X。
4. **Context Integrity**: 确保 `ah ask` 提交的文本在大体量 (100KB+) 时不会被 tmux buffer 或 shell limit 截断 (Bug: TD-008)。

### 6.5 端到端 SOP-08 跑完 verify 设计

- **e2e 谁主笔**: a1 (step 3 tests-first 主笔), tests/ah_dogfooding.rs
- **模拟一个完整 SOP-08 任务**: 通过 ah master client 派 fake_a1/fake_a2/fake_a3, 走完 13 步 (research → 1b audit → 1c idea → 1d audit → 1e design → 1f doc audit → 1g recheck → step 2 tasks → step 3 tests → step 4 src → step 5 audit → step 6 docs → step 7 PR report)
- **fake provider**: 使用 fake_claude/codex/gemini bash scripts 真发 idle marker (echo "<<ah-idle>>"), 不依赖真 LLM. ah daemon 真识别 marker → 真 push notify master
- **CI 跑**: 整个 e2e 在 CI 中可重复, 不需要 OAuth 等外部依赖

## §7 dogfooding e2e 测试设计原则

- **测试主体**: 使用编译后的 `ah` 二进制作为 client，而非直接调 `handlers.rs`。
- **模拟任务**: 模拟一个 SOP-08 流程，例如：`ah start` -> `ah ask "research..."` -> `ah up` -> `ah ask "impl..."` -> `ah stop`。
- **可衡量断言 (5 指标)**:
  - **0 cancel stuck job**: 整个 e2e 跑完, 主控调 `ah cancel` 次数 = 0
  - **0 manual verify**: 整个 e2e 跑完, 主控调 `tmux capture-pane` 次数 = 0; 全凭 `ah ps` / `ah logs` 状态断言
  - **0 ScheduleWakeup poll**: 主控调 ScheduleWakeup 次数 = 0; 全程阻塞于 `ah ask --wait` 直到 push 到位
  - **push 通知延迟 ≤500ms (p95)**: job state SQLite 变更 → master client 收到 push, p95 延迟 ≤500ms
  - **stuck escalate 时延 ≤310s**: agent 卡 → ah daemon 自动 STUCK 检测 + escalate 给 master, ≤310s (orchestrator 300s threshold + 10s margin)
- **100% Session Resume**: Claude 重启后带 `--continue` 因果物理一致性 (PR-6 已闭合, 这里仅 e2e regression 锁)
- **物理校验**: 测试结束后校验 `ccbd.sqlite` 历史、tmux session 彻底销毁、sandbox 物理清理。

## §9 PR 闭合 Roadmap (分组 A-G, 估算性)

按 §3 群A 痛点 + §4 群D 缺陷 + §5 介入点根因, 分 7 组:

### 组 A: master client dogfood 前提
- A1 真 completion detection (废弃 dispatch_and_complete_job test seam): provider-aware MarkerMatcher 链入 agent_io 读 pane 流, BUSY → IDLE 真状态机驱动
- A2 master client API (ah ask --wait subscribe push, 已有 in-process pubsub → 持久外部 push)
- 估: 2-3 PR, 600-1000 LOC

### 组 B: 完成通知 push 持久层
- B1 持久化事件流接口 (SSE / Unix socket / named pipe 选型)
- B2 客户端 subscribe + auto reconnect
- B3 e2e 度量 push 延迟 ≤500ms p95
- 估: 2 PR, 400-700 LOC

### 组 C: stuck detection 多信号
- C1 pane_diff_watcher (已有 30s tick / 300s threshold) 扩多信号: 内容 hash + log mtime + provider-aware
- C2 escalate channel 给主控 (push event 整合到组 B 流)
- 估: 1-2 PR, 300-500 LOC

### 组 D: slash command 投递路径 (Bug X 等价)
- D1 keystroke direct send (不走 paste buffer) per-provider
- D2 测试: ah ask "/clear" → provider 真当 slash 处理
- 估: 1 PR, 200-300 LOC

### 组 E: pane/provider 健康度多层探测
- E1 InitProbe 完善: tmux alive + provider 协议层 ready + completion detector 串联
- E2 e2e 锁 "pane alive ≠ provider alive" 真路径
- 估: 1-2 PR, 400-600 LOC (含 §4 #5 / §3 pane alive ≠ provider alive)

### 组 F: tmux scope + tmpdir lifecycle
- F1 ah daemon SIGKILL 后 scope 真清 e2e 验证 (systemd BindsTo 已设计)
- F2 /tmp 工作目录 lifecycle e2e
- 估: 1 PR, 200-400 LOC

### 组 G: e2e dogfooding 测试本身 (实际验组 A-F)
- G1 tests/ah_dogfooding.rs SOP-08 13 步闭环测试
- G2 监控面板与 RELEASE_NOTES 自动生成
- 估: 1 PR, 300-500 LOC

## §10 风险与 Scope Guard

- **风险**: push 延迟受 SQLite WAL 模式写入延迟干扰。**缓解**: 优先使用 in-memory broadcast 触发异步通知。
- **风险**: fake provider marker 过于理想化。**缓解**: 支持 regex 模糊匹配以适应真实 provider 的不可见 ANSI 字符。
- **Scope Guard**: PR-X 组 A/B 只做单向 push, 不涉及双向交互式 console。
- **Scope Guard**: 维持 master Claude 与 ah daemon 的 1:1 绑定关系, 不在本项目解决多主控并发抢占。
