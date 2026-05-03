# Kiro Requirements: MVP 11 (Real-World Parity / 生产级对齐)

> **Plan-Review 修订记录（2026-05-03 Round 2）**：
> - [P0.1, P1.8] 彻底废弃 Anchor 的 `PartOf` 绑定。改为 Agent Scope `BindsTo=ccbd-session-<session_id>.service`，由 Systemd 内核级自动级联销毁 Agent。明确 Fallback 探测模式。
> - [P0.2] cascade_kill 引入基于 `sessions` 表 `status='KILLED'` 的 SQLite 级原子 CAS 短路保护，根除 TOCTOU 竞态。
> - [P0.3, P1.3] 清除测试描述中残留的“软跳过 / return”字样。确认 CI 跳过只允许使用显式环境变量（`CCB_TEST_SKIP_REAL_PROVIDER=1`），其余情况缺依赖直接 `panic!`。
> - [P1.6] `SendKeysVerified` 协议同步了 `retry_fallback_keys` 字段以支持退避按键。
> - [P0.1] 废弃 `PartOf=ccbd-rust.service`。改用 (b) 模式：agent.scope `BindsTo=ccbd-session-<session_id>.service`。Anchor service 完全独立，不绑 daemon。详见 D §5 Q1 重写。
> - [新增 R-6.4] 同时返工 mvp10 G10.0 中 agent.scope BindsTo 目标，从 `ccbd-rust.service` 改为 `ccbd-session-<session_id>.service`。R-* 矩阵 R-6 行已对应更新。

> **Plan-Review 修订记录（2026-05-03 Round 1）**：
> - [P0.1] 统一使用 `ccbd-session-<session_id>.service` 作为 Anchor 命名，并强化其与 Daemon 的系统级绑定（`PartOf`）语义。
> - [P0.2] 取消测试软跳过。强制要求 AC1-3 端到端测试为硬门槛，本地默认必跑，CI 需显式 opt-out。
> - [P1.4] AC5 补充 Gemini 及其他缺失的 legacy 测试返工要求。
> - [P1.5] 强化 AC4，明确拆分出触发链和唯一回收源的独立测试要求。

> **文档定位**：本文件是 ccbd-rust MVP 11 阶段的官方 R (Requirements) 规格。本阶段旨在解决 MVP 10 暴露出的致命架构与工程纪律缺陷（即“立刻死”与“Provider 适配层真空化”问题）。本 MVP 将彻底重构 Launcher 生命周期模型，并全量复刻 Python ccb 的 Provider 启动与交互适配语义，确保 ccbd-rust 在真实生产环境下具备与 Python 原版 100% 的等效能力。

---

## 0. 立项背景与边界共识

### 0.1 为什么必须做这个 MVP（核心驱动）
在 MVP 10 交付后进行的真实环境部署中，暴露了两个严重脱离真实业务场景的缺陷：
1. **立刻死（Master PID 模型错误）**：`ccb start` 作为一个瞬态 CLI，将自身的 PID 作为 `master_pid` 传给 Daemon。CLI 退出后，Daemon 的 `master_watch` 协程立刻触发 `cascade_kill_session_agents`，导致刚刚拉起的所有 Provider 瞬间被杀。
2. **Provider 真空化（交互瘫痪）**：Rust 侧的 `ProviderManifest` 仅百行，丢失了 Python ccb 中多达 2000 余行的适配层逻辑（包含 50+ 环境变量透传、启动期 `Enter` 等待、交互式弹窗处理、以及重试清理机制）。这导致真实的 Codex/Claude 在启动时弹出的升级提示、授权提示等交互界面下完全卡死。

这不仅是功能遗漏，更是由于过往 MVP 的“AC 抽象化”、“用 Bash 模拟测试”以及“缺少真实环境 Parity 评估维度”等工程纪律问题积累导致的结构性塌方。

### 0.2 核心边界
- **In-scope**:
  - 彻底重构 `master_pid` 生命周期模型（R-1）。
  - 重新设计并实装复杂的 `ProviderManifest` 协议（R-2）。
  - 返工并强化 Acceptance Tests，强制引入真实 Provider 端到端测试（R-3, R-6）。
  - R 文档强制加入“Python ccb behavior mapping”章节（R-4）。
  - Plan Review Rubrics 新增第八维度（R-5）。
- **不在范围内**:
  - MVP 1-6 已沉淀的基础设施（如 SQLite DB 范式、Agent/Job 核心状态机流转、bwrap Sandbox 隔离路径等）保持不变。
  - 不引入新的 RPC 方法，仅调整现有 `session.create` 等接口的载荷或底层处理逻辑。

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 11 验收必须全部通过（本阶段起，**全面禁止**使用 `bash` 或 `ccbd_test_helper` 代替核心 Provider 走 AC）：

**测试硬门槛警告**：以下 AC1-AC3 默认必跑。本地开发环境必须预装 `codex`/`gemini`/`claude` 二进制及对应 API Keys。若依赖缺失，测试直接 **`panic!` (FAIL)**（绝不允许 silent return 软跳过）。CI 环境可通过传入显式参数（如 `CCB_TEST_SKIP_REAL_PROVIDER=1`）进行 opt-out。

1. **AC1 [Codex Real-World 端到端]**：在清空了宿主遗留状态（fresh sandbox & no existing processes）的环境下，执行 `ccb start` 拉起真实 **codex** CLI。系统必须能自动：
   - 全量注入所需的鉴权及配置环境变量（50+ env passthrough）。
   - 识别并绕过 Codex 启动时可能出现的 `Update now / Skip` 交互提示（通过自动输入跳过）。
   - 成功进入 `IDLE` 状态。
   - 随后连续执行 2 个不同的 `ccb ask` 任务（如：“echo 1” 和 “echo 2”）。
   - 断言：任务均能返回 `COMPLETED`，且 `reply_text` 包含预期的执行结果，最终 Agent 安全回落至 `IDLE`，全过程**不得有任何人手干预**。若出现卡死或超时，则判定为 FAIL。
2. **AC2 [Gemini Real-World 端到端]**：在同等 fresh sandbox 环境下，拉起真实 **gemini** CLI。
   - 必须通过 `startup_sequence` 里的模拟 `Enter`（或 `SecondEnter`）机制与 Escape + C-u 成功度过其冷启动缓冲期。
   - 处理 2 个真实的问答任务。
   - 断言：正常返回 `COMPLETED`，无卡死，最终回落 `IDLE`。
3. **AC3 [Claude Real-World 端到端]**：在同等 fresh sandbox 环境下，拉起真实 **claude** CLI。
   - 必须验证 `CCB_CLAUDE_MD_MODE=route` 等特化环境变量已正确透传。
   - 处理 2 个真实的问答任务。
   - 断言：正常返回 `COMPLETED`，无卡死，最终回落 `IDLE`。
4. **AC4 [Launcher Detach 生命周期验证]**：执行 `ccb start` 拉起 Session 后，必须验证其生命周期已与 CLI 进程彻底解绑：
   - **(a) 触发链截断断言**：当 `ccb start` 提交 `session.create` 后立即退出，验证 daemon 端**没有**为该 session 创建 `master_pidfd_watch_task`（检查 daemon 日志，或调用诊断接口确认 `monitor::contains("master:<session_id>") == false`）。
   - **(b) 唯一回收源断言**：验证 cascade_kill 的回收路径有且仅有两条等价语义：
     (1) 显式 `ccb kill --session <id>` 通过 daemon 调 `systemctl --user stop ccbd-session-<session_id>.service` → 由于 agent.scope `BindsTo=ccbd-session-<session_id>.service`，systemd 内核自动 stop 全部 agent.scope（无需 daemon 跑 sigkill 循环）；daemon 的 SessionWatch 检测到 anchor inactive 后同步 DB 标记 KILLED；
     (2) 用户**手动**执行 `systemctl --user stop ccbd-session-<id>.service`（不经 daemon），路径同上。

     证明 cascade_kill_session_agents 函数本身从"主动 SIGKILL agents"退化为"DB 状态同步 + tmux pane 清理"。彻底拔除 PID 退出"立刻死"逻辑。
5. **AC5 [Legacy MVP Acceptance Test 真 Provider 返工]**：mvp7 / mvp8 / mvp9 的 acceptance test 套件中，凡是涉及 provider spawn / agent ready / ask flow 的测试用例，必须**至少有一个变种**用真 CLI 跑通端到端（fresh sandbox + 完整 startup_sequence + 真 ask）。原 bash 测试可保留作 sanity check，但不能作为唯一判据。具体强制要求：
   - `mvp7_acceptance.rs::test_true_codex_smoke_idle_roundtrip` / `test_true_gemini_smoke_idle_roundtrip` 必须重写，移除 `CCBD_UNSAFE_NO_SANDBOX=1` 依赖，使用新的 Manifest 协议真实通过。
   - 增加 `mvp8_real_codex.rs::test_true_codex_ask_pend_roundtrip`（将原 bash 版本的 `test_pend_blocks_until_completed` 升级为真实 Codex 流转）。
   - 增加 `mvp9_real_codex.rs::test_launcher_config_parse_and_batch_spawn_real`（使用 TOML 配置批量拉起真实 Codex 与 Claude 并验证双双到达 IDLE）。

---

## 2. 状态机激活范围 (Delta)

本 MVP 针对 Agent 和 Job 的核心状态机节点保持不变，但对全局**生命周期回收机制**作重大扩充：
- **`sessions` 表扩充状态字段**：为 `sessions` 表新增 `status` 字段（`ACTIVE` / `KILLED` / `CLOSED`）。`cascade_kill_session_agents` 操作必须基于 `UPDATE sessions SET status='KILLED' WHERE id=? AND status='ACTIVE'` 进行 SQLite 级原子 CAS。这根除了从前依赖轮询 Agent 数量造成的 TOCTOU 竞态漏洞。
- Agent 在 `SPAWNING` 阶段的驻留逻辑将变厚（由单纯等待 `MarkerTimer` 转变为执行 `startup_sequence` 状态机后再进入稳态检测）。

---

## 3. R-* 需求切割矩阵 (Scope Definitions)

| Req ID | Description | MVP 1-10 状态 | MVP 11 更新状态 | 备注 |
|---|---|---|---|---|
| **R-1** | Launcher 重构与生命周期解绑 | 🔴 严重缺陷 | 🟢 **Full** | 废弃当前 PID 直接绑定模式，引入长驻锚。 |
| **R-2** | ProviderManifest 深度语义升级 | 🔴 真空化 | 🟢 **Full** | 引入 `env_passthrough`, `startup_sequence`, `readiness_timeout_s` 及 `interactive_prompt_handlers`。 |
| **R-3** | Parity 端到端 Acceptance Tests | 🔴 虚假全绿 | 🟢 **Full** | 必须覆盖真实 codex/gemini/claude 的全自动流转。 |
| **R-4** | R 文档 Behavior Mapping 章节 | ⚪ N/A | 🟢 **Full** | 本文 §7 已落实。 |
| **R-5** | Plan Review Rubrics 升级 | ⚪ N/A | 🟢 **Full** | 引入第 8 维度 `real_provider_parity`。 |
| **R-6** | MVP 1-10 局部返工定界 | ⚪ N/A | 🟢 **Full** | 明确推翻 mvp7(G7.1) 与 mvp9(G9.0) 的部分设计；新增 R-6.4：将 MVP10 Agent Scope `BindsTo` 从 Daemon 改绑至 Session Anchor。 |

---

## 4. 范围分阶段（实施视角）

### G11.0：Launcher 与 Master 生命周期重构 (R-1 & R-6)
- **目标**：解决“立刻死”。
- **实施**：重构 `handle_session_create` 和 `master_watch.rs`。采用 R-1 决断的系统级长驻锚方案。
- **Checkpoint**：运行 `ccb start` 并确认其退出后，后台 Agent 依然存活，`ccb ps` 显示 IDLE 而非 KILLED。

### G11.1：Manifest 协议重构与环境透传 (R-2 & R-6)
- **目标**：解决“Provider 适配层真空化”的基础底座。
- **实施**：
  - 重写 `src/provider/manifest.rs`，加入新的深层语义字段。
  - 重写 `src/sandbox/systemd.rs` 与 `bwrap.rs`，解析 `env_passthrough` 并从宿主中捞取指定的 50+ 个白名单环境变量注入沙盒。
- **Checkpoint**：在沙盒内运行真实 Provider 时，使用 `env` 命令可查看到完整的 `ANTHROPIC_*`, `GOOGLE_*` 等变量。

### G11.2：启动序列引擎与交互防御器 (R-2)
- **目标**：解决 TUI 启动期的各种阻碍与眼盲问题。
- **实施**：
  - 在 `agent_io` 或 `marker` 子系统中，引入一个前置的 `StartupSequenceEngine`，负责按 Manifest 设定执行休眠、`Enter` 发送、以及 `Escape + C-u` 的清理重试。
  - 引入 `InteractivePromptInterceptor`，正则扫描输出，遇到 `Update now` 等定义在 `interactive_prompt_handlers` 里的模式时，自动发送预设响应。
- **Checkpoint**：即使 Codex 有升级提示，ccbd 也能自动按键跳过，最终安稳进入 IDLE 稳态。

### G11.3：Parity Acceptance Tests 落实 (R-3)
- **目标**：用硬性的自动化测试锁定成果。
- **实施**：编写 `tests/mvp11_parity_codex.rs`, `_gemini.rs`, `_claude.rs`。
- **Checkpoint**：在有真实 Provider 环境的 CI/开发机上，测试全部独立全绿。

---

## 5. 跟前后 MVP 的接口约束

- **Plan Review Rubrics 升级 (R-5)**：
  自本 MVP 起，所有的 Plan Review 过程必须采用 **8 维度 Rubrics**。
  新增维度：`real_provider_parity`（生产级真实对齐度）。
  - **判据**：该设计是否能 100% 映射/覆盖对应阶段 Python ccb 的所有边界处理和环境变量透传？测试用例是否脱离了 mock 依赖而直面了真实的 Provider CLI？
  - **红线**：一旦此维度得分 `≤ 5`（满分通常为10），则整个 Plan Review 立即判定为 **FAIL**，无论其他维度多高。实施者必须重新设计对齐方案。
- **RPC 变更**：
  `session.create` 的入参可能需要调整，将 `master_pid` 从强制传入调用端 CLI 的 PID 改为由 Daemon 根据决断策略（见 §6）自行生成或绑定。

---

## 6. 核心架构决断 (Architectural Decisions)

### 决断 1：Launcher 生命周期重构策略 (R-1)
> 候选方案分析：
> - **A (session.detach 与 systemd 锚)**：使用一个 `systemd-run --user --unit=ccbd-session-<project> --remain-after-exit` 创建一个虚拟的服务单元作为 Session 的实体锚点。Daemon 改用 `systemctl is-active` 查询其生命周期。此方案极其稳健，完全契合 MVP 10 构建的 cgroup/systemd 治理体系，无遗留进程，生命周期由 OS 直接背书。
> - **B (fork sentinel)**：CLI fork 一个睡死的 dummy process 作为 master。太脏，遗留无效进程树，且与 systemd 原生生态不融合。
> - **C (attach tmux)**：强制 attach。但这破坏了用户“后台启动服务”的诉求，限制了自动化脚本的集成。

**最终方案：(b) Agent BindsTo Anchor 模式**（Round 2 重审后从 (a) PartOf 升级到 (b)）

**Rationale (Round 2 重审)**：原 Round 1 选 (a) `PartOf=ccbd-rust.service` 把 anchor 绑死在 daemon 上，但有致命 Restart 陷阱——daemon panic + systemd `Restart=on-failure` 自动重启时，PartOf 不会同步停 anchor，会留下幽灵 anchor service。改用 (b)：

- **Anchor 完全独立**：`systemd-run --user --unit=ccbd-session-<session_id>.service --remain-after-exit /usr/bin/true`，**不绑** daemon。daemon 死亡不影响 anchor 存活。
- **Agent 反向绑 Anchor**：mvp10 G10.0 的 agent.scope `BindsTo=ccbd-rust.service` 在 mvp11 范围内改成 `BindsTo=ccbd-session-<session_id>.service`（即 anchor unit name）。这是 R-6.4 显式返工。
- **真 Detach**：daemon 重启时，anchor + agent.scope 全部存活；daemon 通过 reconcile 路径（扫 DB 已存在的 ACTIVE session + 检查 anchor unit 存在性）重新 attach SessionWatch。
- **回收路径** `cascade_kill_session_agents` 退化为 “DB 状态 CAS + tmux pane 清理”——内核级 BindsTo 已经做完进程杀戮。
- **分层 ScopePolicy fallback**：复用 mvp10 的探测模式（生产 / 开发 / 受限三档），见 D §3.3。

### 决断 2：ProviderManifest 核心字段设计 (R-2)
**推荐设计**：
```rust
pub struct ProviderManifest {
    pub provider_name: &'static str,
    pub command: &'static [&'static str],
    
    /// 与 Python ccb 的 `runtime_env/control_plane.py` (50+ allowlist) 对应。
    /// 指定需要从宿主环境变量原样提取并注入的白名单前缀或全名。
    pub env_passthrough: &'static [&'static str],
    
    /// Magic env vars injected by ccbd into sandbox (固定值，与宿主无关).
    /// 对应 Python ccb 各 provider backend 中通过 dispatch_env_for_provider 注入的常量。
    /// 例如 CCB_CLAUDE_MD_MODE=route, CCB_TMUX_ENTER_DELAY=2.0 等。
    pub injected_env_vars: &'static [(&'static str, &'static str)],

    /// 与 Python `provider_core/init_gate.py` 中的 `CCB_*_READY_TIMEOUT_S` 对应。
    pub readiness_timeout_s: u32,

    /// 与 Python `terminal_runtime/tmux_send.py` 里的 `enter_delay` / `second_enter_delay` / `Escape + C-u` 机制对应。
    /// 在 agent spawn 后，在等待 marker 之前执行的一系列前置机械动作。
    pub startup_sequence: &'static [StartupStep],

    /// 与 Python 侧处理 Codex 升级提示等未显式抽象但散落各处的 TUI blocker 对应。
    /// 正则拦截器：若在屏幕中扫描到该 Regex，则自动发送应对的 keysym，并具备次数防死循环限制。
    pub interactive_prompt_handlers: &'static [PromptHandler],

    pub idle_detection_mode: IdleDetectionMode,
    pub marker_pattern: &'static str,
    pub stability_ms: u64,
}

pub enum StartupStep {
    WaitMs(u64),
    SendKeysVerified {
        keys: &'static str,
        /// 发完后等多久 capture-pane 看到这个 pattern 出现，超时则 retry
        verify_pattern: Option<&'static str>,
        verify_timeout_ms: u64,
        /// 与 Python `_verify_delivery` 失败后的 `CCB_VERIFY_RETRY_KEYCODES=Return,C-m` 备用机制对应
        retry_fallback_keys: Option<&'static [&'static str]>,
    },
    ClearLine {
        /// 纯机械动作（Escape + C-u）后，需要确认 pane 真的回到了空 prompt 状态再进下一步
        expected_after: Option<&'static str>,
    }, 
}

pub struct PromptHandler {
    pub pattern: &'static str,
    pub response_keys: &'static str,
    pub max_triggers: u32,
}
```

---

## 7. Python ccb behavior mapping (R-4)

为了杜绝功能阉割，本节提供确凿的 Python 侧与 Rust 侧行为映射对比基准，这是 Reviewer 验收代码的硬性依据：

| 行为类别 | Python ccb 原始实现定位 | ccbd-rust (MVP11) 对应落地策略 |
|---|---|---|
| **Master 生命周期** | `ccbd/app_runtime/lifecycle.py` 中 `app.serve_forever()` 维持了 Daemon 自身的存活，通过 `project.py` 中的引用进行管理。旧版并没有要求一个瞬时的 `ccb start` 来维系进程树。 | 采用决断 1 的 **Detach 语义**。`ccb start` 仅发送 RPC 并退出，取消 `src/monitor/master_watch.rs` 中将启动 CLI 的 PID 当作级联自毁信标的错误逻辑。 |
| **Provider 启动控制** | `provider_core/init_gate.py` 中的 `InitGate` 状态机，包含了 `INITIALIZING` 的分段退避轮询与 `deadline_s`。 | 映射为 Manifest 中的 `readiness_timeout_s` 和 `startup_sequence` 状态机。在进入稳定的 IDLE 检测前，必须跑完 Sequence 并防死锁。 |
| **Env Passthrough** | `runtime_env/control_plane.py` 中定义了 `_CONTROL_PLANE_ALLOWLIST`。 | 映射为 Manifest 的 `env_passthrough`。在 `src/sandbox/bwrap.rs` 和 `systemd.rs` 拼装参数时，遍历该列表并执行 `std::env::var` 提取宿主密钥（如 `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`）并注入。 |
| **Injected Env Vars** | Python 侧各 provider backend 通过 `dispatch_env_for_provider` 注入的常量控制开关。 | 映射为 Manifest 的 `injected_env_vars`。ccbd 依据配置主动注入（如 `CCB_CLAUDE_MD_MODE=route`, `CCB_TMUX_ENTER_DELAY=2.0`）到沙盒，无需宿主存在。 |
| **Interactive Prompt 处理** | `terminal_runtime/tmux_send.py` 处理了 `enter_delay`, `second_enter_delay` 甚至输入区防污染的 `Escape` + `C-u` 清理；发送后有 `verify_send_succeeded`。 | 映射为 Manifest 中的 `interactive_prompt_handlers` 和 `StartupStep` 状态机（包含带有 `verify_pattern` 和 `verify_timeout_ms` 的 `SendKeysVerified`，以及带有 `expected_after` 的 `ClearLine`）。 |
