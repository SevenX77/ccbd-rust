# Tasks: prompt-handler Phase 1 实施

## 概述

本 phase 目标是让 agent 遇到已知 prompt 自动跳过，遇到未知 prompt 进入 `PROMPT_PENDING` 等主控裁判，而不是被 supervisor 误判死亡。

## 任务依赖图

```text
T1 Schema/Action
  ├─> T2 KB IO/Seed
  │     └─> T3 Matcher
  │           └─> T4 Hash-Gating
  │                 └─> T5 Recursion
  │                       ├─> T7 Event
  │                       └─> T8 Integration
  └─> T6 State
        ├─> T7 Event
        └─> T9 RPC/CLI

T10 Tests/E2E docs depends on T1-T9.

Parallel: T6 可与 T1-T3 并行；T9 的 CLI 参数解析测试可与 T8 并行，但 RPC handler 接入需等 T6/T7。
```

## 详细任务

### T1: 定义 Prompt KB schema 与 Action 白名单

- **目标**: 在 `src/prompt_handler/` 建立 Phase 1 最小数据模型，能表达正则 fingerprint、动作序列、category、confidence、used_count，并保留 Phase 2 字段的反序列化兼容。
- **输入**: `design.md` §3.2/§6/§9.1 Q4-Q5，`Cargo.toml` 现有 `serde` / `serde_json` / `regex` 依赖。
- **输出**: 新增 `src/prompt_handler/mod.rs`、`src/prompt_handler/schema.rs`；在 `src/lib.rs` 暴露模块；key APIs: `PromptCase`、`PromptKb`、`PromptFingerprint`、`PromptAction`、`ValidatedAction`.
- **验收 (DoD)**:
  - [ ] 先写 failing unit test: 反序列化 design 示例、拒绝 shell 注入字符、接受基础 keysym 与 `yes/no/agree` literal。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] 异常必须返回 typed `CcbdError` 或 prompt-handler 专用 error，经 `From` 映射；禁止静默吞错。
  - [ ] schema 校验失败需 `tracing::warn!` 写明 reason 与 impact。
  - [ ] 每个 side-effect 前后要有 `tracing::info!`；本任务若无 side-effect，测试需断言纯解析不触发 IO。
  - [ ] 调用现有 API 前必须 `rg` 验证签名；本任务需记录 `serde`/`regex` 用法，不靠记忆写跨模块调用。
  - [ ] 新模块有 module-level docstring；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] unit test 覆盖 schema 兼容、action 白名单、非法字符拒绝。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: 使用现有 `regex::Regex`，不引入 `regex_lite`，因为 design 已要求 Multiline/Dotall 兼容字段且 `Cargo.toml` 已有 `regex`。
- **预估时间**: 0.5 day
- **风险**: Phase 1 逻辑不使用全量字段，但 schema 若直接拒绝未知字段会阻断 Phase 2 兼容。

### T2: 实现 KB 读写、文件锁与内置种子预案

- **目标**: 从 `~/.ccb/prompt-cases.json` 加载用户 KB，不存在时写入内置 default cases，支持 codex update 与通用 trust path。
- **输入**: T1 schema，`directories` 路径习惯，`nix` 文件锁依赖，design.md §3.1/§9.1 Q1/Q3。
- **输出**: 新增 `src/prompt_handler/kb.rs`、`src/prompt_handler/seeds.rs`；key APIs: `load_or_bootstrap_kb(path)`, `save_kb_atomic(path, kb)`, `default_cases()`.
- **验收 (DoD)**:
  - [ ] 先写 failing unit test: 文件不存在 bootstraps、用户 case 优先于内置、坏 JSON 返回 typed error、原子写不留下半文件。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] `load_or_bootstrap_kb` / `save_kb_atomic` 接受路径参数，不硬编码 `~/.ccb/prompt-cases.json`；
        单元测试用 `tempfile::TempDir` 隔离；
        测试运行不写真实 `~/.ccb`。
  - [ ] 读/写/lock/unlock KB 前后必须 `tracing::info!`，错误时 `tracing::warn!` 或 `tracing::error!` 写明 reason 与 impact。
  - [ ] 禁止 `catch` 后忽略；所有 IO / serde / lock 错误必须 typed error 传播。
  - [ ] 调用文件锁、路径解析、atomic rename 前必须 `rg` / 查文档验证现有项目习惯和依赖版本。
  - [ ] 新模块有 module-level docstring；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] unit test 覆盖 bootstrap、load、save、lock 路径；integration test 可用 tempfile 模拟 home。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: Phase 1 做进程内可验证的 file lock + atomic temp rename；fingerprint/action 冲突阻塞的精细 union merge 留给 Phase 2。
- **预估时间**: 0.5 day
- **风险**: `~/.ccb` 与当前项目 state dir 不同，测试必须允许注入 KB path，避免污染开发者真实 home。

### T3: 实现静态正则 Matcher 与 case 选择

- **目标**: 对 sanitized pane 文本运行 KB 正则匹配，返回第一条可执行 case 与动作序列，支持内置与用户 case 优先级。
- **输入**: T1/T2，`src/db/jobs.rs::strip_ansi_escapes` 可复用性需 grep 确认可见性，design.md §2.2/§9.2(a)。
- **输出**: 新增 `src/prompt_handler/matcher.rs`；key APIs: `match_prompt(provider, pane_text, kb) -> MatchOutcome`.
- **验收 (DoD)**:
  - [ ] 先写 failing unit test: codex update 命中 skip、trust path 命中 accept、动态进度噪点 sanitization 后 hash 稳定、用户 case 覆盖内置。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] regex 编译失败必须 typed error + `tracing::warn!`，并说明跳过该 case 对自动化的 impact。
  - [ ] match 开始、命中、不命中、跳过 invalid case 都要 `tracing::info!`。
  - [ ] 调用 `strip_ansi_escapes` 或复制 sanitization 前必须 grep 验证可见性；不能凭记忆引用私有函数。
  - [ ] 新模块有 module-level docstring；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] unit test 覆盖匹配优先级、regex flags 存储兼容、invalid regex 降级。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: Matcher 只做本地确定性判断，不调用 LLM；未知 prompt 由后续状态与事件任务处理。
- **预估时间**: 0.5 day
- **风险**: capture-pane 文本可能包含 ANSI 或 shell 提示符噪声，sanitization 过强会误伤 prompt 内容。

### T4: 实现 Hash-Gating 4 级分流

- **目标**: 将 prompt 扫描前置过滤为 4 级：画面没变跳过、IDLE marker 跳过、正则预案匹配、全部不命中进入未知 prompt。
- **输入**: T3 matcher，`src/pane_diff/mod.rs::is_meaningful_diff`，`src/marker` matcher / parser registry，design.md §9.1 Q8/§9.2(f)。
- **输出**: 新增 `src/prompt_handler/gating.rs`；key APIs: `classify_capture(agent_ctx, previous_hash, capture) -> PromptGateDecision`.
- **验收 (DoD)**:
  - [ ] 先写 failing unit test: same hash -> skip、IDLE marker -> skip、regex match -> action、unknown -> pending candidate。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] hash 计算、IDLE marker 检查、regex matcher、unknown 分支均需 `tracing::info!`；异常 `warn/error` 写 reason 与 impact。
  - [ ] 所有失败返回 typed result，不允许静默降级成 skip。
  - [ ] 调用 `pane_diff::is_meaningful_diff`、marker matcher、parser registry 前必须 grep 源码验证签名和语义。
  - [ ] 新模块有 module-level docstring；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] unit test 覆盖 4 级分流；integration test 覆盖 marker-like 文本不误报 unknown。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: 使用 `sha2` 对 sanitized 文本 hash；不把 raw capture hash 作为唯一依据，避免时间戳/进度条导致轮询风暴。
- **预估时间**: 0.5 day
- **风险**: IDLE marker 与 prompt 文本可能同时存在，优先级必须按 design 固定，避免自动按键打断正常完成态。

### T5: 实现多层 prompt 递归处理 Max Depth 3

- **目标**: 执行动作后重新 capture，并最多递归处理 3 层 prompt；超过深度转未知 prompt/PROMPT_PENDING。
- **输入**: T3/T4，`src/tmux/session.rs::send_keys_literal` / `send_keys_keysym` / `capture_pane`，design.md §9.1 Q6/§9.2(b)。
- **输出**: 新增 `src/prompt_handler/runner.rs`；key APIs: `handle_prompt_chain(agent_id, pane_id, provider, max_depth=3) -> PromptRunOutcome`.
- **验收 (DoD)**:
  - [ ] 先写 failing unit test: 单层自动跳过、两层 trust->update、depth=4 转 pending、非法 action 不执行。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] 每次 send key/literal、capture、depth increment、递归结束前后必须 `tracing::info!`。
  - [ ] action 执行失败、capture 失败、depth exceeded 必须 typed error 或 pending outcome + `warn/error` reason/impact。
  - [ ] 调用 `tmux::session::{send_keys_literal,send_keys_keysym,capture_pane}` 前必须 grep 验证签名和 ownership 类型。
  - [ ] 新模块有 module-level docstring；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] unit test 用 fake executor 覆盖递归；integration test 用 tmux 可选 fixture 验证 key sequence 顺序。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: Runner 依赖可替换 executor trait，unit test 不强依赖真实 tmux；真实 tmux 行为放 integration/e2e。
- **预估时间**: 1 day
- **风险**: 动作后画面刷新有延迟，必须用短暂稳定窗口而不是立即判定无变化。

### T6: 增加 PROMPT_PENDING 状态与派发阻塞规则

- **目标**: 在 agent 状态机中新增 `PROMPT_PENDING`，未知 prompt 时可 CAS 进入该状态，orchestrator 不再向该 agent 派新 job。
- **输入**: `src/db/state_machine.rs` 状态常量与 transition helpers，`src/orchestrator/mod.rs` 只查询 `IDLE` 派发，`src/db/jobs.rs` 有 `IDLE`/`UNKNOWN` 队列逻辑，design.md §5.1/§9.2(c)。
- **启动相关入口**: `src/bin/ccbd.rs` 调 `db::system::reconcile_startup_with_tmux_socket`；
  `src/db/system.rs` 的 startup reconcile 当前只扫描 `SPAWNING` / `WAITING_FOR_ACK` / `BUSY` / `IDLE`。
- **输出**: 修改 `src/db/state_machine.rs`、必要的 `src/db/jobs.rs`/`src/orchestrator/mod.rs` 测试；key APIs: `STATE_PROMPT_PENDING`, `mark_agent_prompt_pending`.
- **验收 (DoD)**:
  - [ ] 先写 failing unit test: active 状态可转 `PROMPT_PENDING`、`PROMPT_PENDING` 不能被 job claim/dispatch、resolve 后回到 `IDLE` 或 `BUSY` 的规则明确。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] daemon 启动 reconcile 路径显式处理 `PROMPT_PENDING`:
        保持原状态，不重启 agent，不派 queued job，等主控 `resolve_prompt`。
        unit test 覆盖 daemon 重启场景模拟。
  - [ ] 状态转移和拒绝派发必须有 `tracing::info!`；非法转移用 typed `CcbdError::AgentWrongState` 并 `warn` reason/impact。
  - [ ] 不允许通过宽泛 SQL 更新静默覆盖 `CRASHED`/`KILLED`。
  - [ ] 调用 `transit_agent_state*`、job dispatch API 前必须 grep 验证签名和事务语义。
  - [ ] 新增状态相关 module/doc 注释；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] unit test 覆盖 transition、active-state predicate、dispatch exclusion；integration test 覆盖 pending agent 不消费 queued job。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: `PROMPT_PENDING` 不是 active state，也不是 terminal state；watcher 不应把它当可派发或可自动恢复状态。
- **预估时间**: 0.5 day
- **风险**: startup reconcile 是 `PROMPT_PENDING` 的必处理点，不是可选优化；
  若重启后丢失 pending 语义，会误派 job、误 reset 或把仍活着的 agent 标为 `CRASHED`。

### T7: 发出 UNKNOWN_PROMPT_DETECTED 事件

- **目标**: 未知 prompt 进入 pending 时写入事件系统，让主控可以 watch 到 pane 文本、block reason 和候选动作上下文。
- **输入**: T4/T6，`src/db/events.rs::insert_event`，`src/rpc/handlers.rs::format_events`，design.md §5.2/§9.2(d)。
- **输出**: 新增 `src/prompt_handler/events.rs` 或合并到 runner；key APIs: `emit_unknown_prompt_detected(agent_id, payload)`.
- **验收 (DoD)**:
  - [ ] 先写 failing unit test: event_type 为 `UNKNOWN_PROMPT_DETECTED`，payload 包含 `pane_screenshot`、`suggested_action: null`、`block_reason`、`capture_hash`、`depth`。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] 事件写入前后必须 `tracing::info!`；失败必须 typed error + `tracing::error!` 写明主控不可见的 impact。
  - [ ] 不允许把事件失败吞掉后仍假装 pending 成功；pending 状态与 event 写入需尽量同事务或明确补偿策略。
  - [ ] 调用 `db::events::insert_event_sync/insert_event` 前必须 grep 验证签名；确认当前项目事件存储是 SQLite `events` 表，不是独立 `events.jsonl`。
  - [ ] 新模块有 module-level docstring；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] unit test 覆盖 payload schema、事件失败传播；integration test 覆盖 `agent.watch` 能看到事件。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: design 中的 `events.jsonl` 在当前代码落地为 SQLite `events` 表；不新增第二套事件落盘格式。
- **预估时间**: 0.5 day
- **风险**: pane 文本可能很长，payload 应截断到可读大小并记录截断标记，避免事件表膨胀。

### T8: 接入 WAITING_FOR_ACK / STUCK 前扫描路径

- **目标**: 将 prompt-handler 接入 `spawn_new_capture_seed` 的 visual diff 路径和 `marker_timer` STUCK 前检查，使 prompt 不再被误判为 ACK 变化或进程死亡。
- **输入**: T4/T5/T7，`src/rpc/handlers.rs::spawn_new_capture_seed`，`src/marker/mod.rs::spawn_marker_timer_task`，`src/agent_io` pane registry，design.md §1.1/§2.1。
- **存活检查入口**: 本仓库未命名 `ensure_active_pane_alive`；
  需排查 `tmux has-session` 封装（如 `src/tmux/session.rs::ensure_session_sync`）
  以及 pane diff / agent_io / monitor 中所有会把 pane/session 异常升级为 `STUCK` 或 `CRASHED` 的路径。
- **输出**: 修改 `src/rpc/handlers.rs`、必要时修改 `src/marker/timer.rs`；key behavior: visual diff 先跑 prompt scan，prompt handled 后继续 ACK 循环，unknown 则 pending 并停止该 job 派发链。
- **验收 (DoD)**:
  - [ ] 先写 failing integration test: WAITING_FOR_ACK 时出现 codex update 文本会发送 skip action 而非转 `BUSY`/`STUCK`；未知 prompt 转 `PROMPT_PENDING`。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] supervisor 周期存活检查（典型 `ensure_active_pane_alive` / 当前 `tmux has-session` 探活路径）
        豁免 `PROMPT_PENDING` 状态：pending agent 不参与 pane_dead 触发的 retry；
        integration test 覆盖 pending 期间模拟 `tmux has-session` 短暂失败不会触发 `CRASHED`。
  - [ ] 每次扫描触发、跳过、自动处理、pending、恢复 ACK 循环都必须 `tracing::info!`；异常 `warn/error` 写 reason/impact。
  - [ ] prompt-handler 失败不得静默变成 `CRASHED`；必须保留原有 fallback 并记录 prompt scan failure。
  - [ ] 调用 `spawn_new_capture_seed` 相关上下文、`capture_pane`、`send_keys`、marker timer 前必须 grep 验证签名与生命周期。
  - [ ] 新接入点有必要注释说明状态机意图；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] integration test 覆盖 ACK visual diff、STUCK 前扫描、scan failure fallback。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: Phase 1 优先接入 ACK/STUCK 关键路径；周期 IDLE/BUSY 轻量扫描如时间不足可只做最小后台 loop，但必须不影响派发。
- **预估时间**: 1 day
- **风险**: supervisor/pane 存活检查与 ACK/STUCK 扫描是两个独立路径，都必须覆盖；
  漏掉任一路径，pending agent 仍可能被误判死亡。
  `spawn_new_capture_seed` 现在在 `rpc/handlers.rs`，接入过重可能让 RPC 模块继续膨胀；若抽 helper，保持改动最小。

### T9: 实现 agent.resolve_prompt RPC 与 ccb prompt resolve CLI

- **目标**: 主控可通过 RPC 指定 action 并选择是否保存到 KB，从 `PROMPT_PENDING` 解阻塞 agent。
- **输入**: T1/T2/T5/T6/T7，`src/rpc/router.rs` method whitelist，`src/rpc/handlers.rs` handler 模式，`src/bin/ccb-rust.rs` clap command，design.md §5.3/§9.2(e)。
- **输出**: 修改 `src/rpc/router.rs`、`src/rpc/handlers.rs`、`src/bin/ccb-rust.rs`；可新增 `src/cli/prompt.rs`；key APIs: RPC `agent.resolve_prompt(agent_id, action, save_to_kb)`，CLI `ccb-rust prompt resolve <agent> <action> [--save-to-kb]`.
- **验收 (DoD)**:
  - [ ] 先写 failing unit test: RPC method whitelist、缺字段报错、非 pending agent 拒绝、合法 action 执行后回到 `IDLE`、`save_to_kb=true` 写入 KB。
  - [ ] 最少实现让测试 green，再 refactor；遵守 TDD 红绿循环。
  - [ ] RPC request、action validate、send key、KB save、state transition、reply emit 前后都必须 `tracing::info!`。
  - [ ] 所有 bad params / bad state / IO failure 用 typed error；禁止返回成功但 action 未执行。
  - [ ] 调用 `rpc::router`、`rpc::handlers`、`cli::rpc_client`、tmux send APIs 前必须 grep 验证签名。
  - [ ] 新 CLI 模块有 module-level docstring；`CLAUDE.md` / `README.md` 不在本任务更新，列入末尾统一清单。
  - [ ] unit test 覆盖 router/handler/CLI 参数；integration test 覆盖 pending -> resolve -> unblocked。
  - [ ] cargo fmt / clippy / build 在实现任务时通过。
- **关键决策**: CLI 名称按现有 binary 落地为 `ccb-rust prompt resolve`；如发布别名 `ccb`，文档中同步说明。
- **预估时间**: 1 day
- **风险**: `save_to_kb=true` 需要从 pending payload 构造 fingerprint，Phase 1 应限定为保存主控明确给出的 action + 当前 capture hash/regex 草案，避免生成过宽正则。

### T10: Phase 1 验收测试与完成清单

- **目标**: 汇总单元、集成、手动 e2e 覆盖，确认 design.md §9.2 Phase 1 (a)-(f) 全部落地。
- **输入**: T1-T9，现有 tests 风格，`scripts/` e2e 习惯，真实 a3/codex update prompt 复现路径。
- **输出**: 新增/修改 focused tests 与 `docs/mvp13-e2e-checklist.md` 或新增 prompt-handler e2e checklist；不在本 task 改 `CLAUDE.md` / `README.md`，只记录待更新项。
- **验收 (DoD)**:
  - [ ] 先写 failing acceptance/integration test 覆盖完整路径，再补最少实现或测试 fixture 调整；遵守 TDD 红绿循环。
  - [ ] 所有新测试失败时必须暴露 typed error / assert message，不允许 timeout 后无诊断。
  - [ ] 增加 mock provider integration test fixture:
        用 shell 脚本（`tests/fixtures/mock_prompt_provider.sh` 或类似）在 pane 里 `echo` 出真实 prompt 文本（codex update / trust path），不依赖 codex CLI 真实版本。
        integration test 跑这个 fixture 验证 prompt-handler 自动 skip 路径。
        手动 e2e 仍然跑真实 codex update 一次作为最终验收。
  - [ ] e2e 脚本每个 side-effect，包含启动 daemon、发送 prompt、resolve、读取事件，都要有可追踪 log。
  - [ ] 实施前 grep 现有 tests/scripts 命名与 helper 签名，复用 `tests/common`，不凭记忆写新 harness。
  - [ ] 新测试模块有 module-level docstring 或文件头说明；`CLAUDE.md` / `README.md` 更新列入末尾统一清单。
  - [ ] cargo test 单元 + 集成全过。
  - [ ] 手动 e2e 覆盖 codex update prompt 自动跳过（a3 跑）、未知 prompt 进 `PROMPT_PENDING`、`ccb-rust prompt resolve` 解阻塞。
  - [ ] cargo fmt / clippy / build 全过。
- **关键决策**: 自动测试用 fixture/fake executor 保证稳定，真实 codex CLI 弹窗只作为手动 e2e，不让 CI 依赖外部 CLI 版本。
- **预估时间**: 0.5 day
- **风险**: codex update prompt 是否出现受本机 CLI 版本影响，手动 e2e 需要保留可注入 fake pane 文本的替代验证。

## Phase 1 完结条件 (acceptance criteria)

- [ ] `cargo test` 单元 + 集成全过。
- [ ] `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo build` 全过。
- [ ] 手动 e2e 覆盖：codex update prompt 自动跳过（a3 跑）。
- [ ] 手动 e2e 覆盖：未知 prompt 进入 `PROMPT_PENDING`，主控能从事件看到 `UNKNOWN_PROMPT_DETECTED`。
- [ ] 手动 e2e 覆盖：`ccb-rust prompt resolve <agent> <action>` 能执行动作并解阻塞。
- [ ] design.md §9.2 Phase 1 的 (a)-(f) 全部落地：静态正则 + seeds、多层递归、`PROMPT_PENDING`、unknown event、resolve RPC/CLI、Hash-Gating 4 级。

## Out of scope (本 phase 显式不做)

- LLM 调用与 Haiku 4.5 / 主控 fallback（Phase 3）。
- Schema 全字段逻辑使用：`created_at` / `last_used_at` / `created_by` / `regex_flags` / `trigger_state` 留给 Phase 2；Phase 1 仅做兼容存储/解析。
- 多 ccbd 写冲突精细处理、fingerprint 模糊匹配 cleanup、冲突阻塞官方 action 裁决（Phase 2）。
- Action 导致 process 死的 Dangerous/Invalid 防误杀闭环（Phase 2）。
- vt100 parser 颜色/属性识别（Phase 3）。
- README / CLAUDE.md 正式用户文档更新；Phase 1 末期统一补。

## Phase 1 完成后必须做的事

- [ ] 更新 `README.md`：新增 prompt-handler 能力、KB 路径、`prompt resolve` 示例、已知限制。
- [ ] 更新 `CLAUDE.md` 或对应 agent 操作说明：遇到 `UNKNOWN_PROMPT_DETECTED` 的主控处理流程。
- [ ] 补充 KB schema 文档：字段说明、action 白名单、Phase 2/3 预留字段。
- [ ] 补充运维排障说明：如何查看 pending agent、unknown prompt event、KB 写入失败日志。
- [ ] 复盘是否需要把 `spawn_new_capture_seed` 中 prompt 扫描逻辑抽离，降低 `src/rpc/handlers.rs` 体积。

---

# PR4a Tasks: 生命周期主干 + can-input 确认探针 + 确定性兜底循环

## 范围说明

PR4a supersede 旧 §10 的 HandledSet / visible-only / 两阶段截断方案。PR4a 只做确定性、可本地测试的生命周期主干：用 outcome 判据推进状态，用现有内置 regex prompt-handler 做清障动作，用 can-input 确认 READY/DONE。不做 `prompt_experience` DB 自学习表，不做 Anthropic/Haiku LLM 慢路径；这些留给 PR4b。

## PR4a 任务依赖图

```text
P4A-T1 OutcomePredicate 接口
  ├─> P4A-T2 can-input 探针 + BSpace + ProviderManifest 字段
  │     ├─> P4A-T3 SPAWNING lifecycle loop cutover
  │     └─> P4A-T5 BUSY/DONE 判据接入
  ├─> P4A-T4 PromptRecurrentLoop 复用现有 regex handler
  │     ├─> P4A-T3 SPAWNING 期可清障
  │     └─> P4A-T6 PROMPT_PENDING 允许 SPAWNING
  └─> P4A-T7 cutover tests + PR5 遗留断言清理
```

## P4A-T1: 新增生命周期 outcome 判据接口

- **类型**: 新增。
- **目标**: 定义 `OutcomePredicate` 契约，让通用循环能复测“当前阶段是否到达目标状态”，不再用“某段屏幕文字是否消失”作为成功判据。
- **主要改动点**:
  - 新增 `src/lifecycle/` 或 `src/provider/lifecycle.rs`，定义 `OutcomePredicate`、`OutcomeKind`、`OutcomeResult`。
  - `src/provider/init_probe_task.rs:44` 的启动循环改为调用 SPAWNING→READY predicate，而不是直接 `probe.detect`。
  - `src/rpc/handlers.rs:490` 保留启动 task 挂点，但传入 manifest/provider 所需的 outcome 配置。
- **验收**:
  - SPAWNING predicate 能委托 can-input 确认 READY。
  - BUSY/DONE predicate 能组合“输出稳定”与 can-input 复核。
  - 单测覆盖 predicate 未达成时返回可诊断状态，不直接推进 DB 状态。

## P4A-T2: can-input 确认探针

- **类型**: 新增 + cutover。
- **目标**: 实现安全 can-input：先确认不是对话框，再发可配置 probe 字符，抓屏确认 probe 出现在输入框，发送 `BSpace` 删除，再二次确认 probe 消失。
- **主要改动点**:
  - `src/prompt_handler/schema.rs:167` 将 `BSpace`/`Backspace` 加入 keysym 白名单。
  - `src/tmux/session.rs:349` 复用 `send_keys_keysym_sync` 发送 `BSpace`；不要用 Enter 作为 readiness probe。
  - `src/provider/manifest.rs:5` 在 `ProviderManifest` 增加 `input_probe_literal: &'static str`，为 codex/gemini/claude/bash 填默认值。
  - 新增 can-input helper，必须复用 `TmuxPromptIo::send_key_literal` / `send_key_keysym` 能力。
- **验收**:
  - 对 `Do you trust...1) Yes 2) No`、`Update available...1) Update 2) Skip` capture 不发送裸 probe 字符。
  - ready 输入框 capture 下发送 probe 字符、看到回显、发送 `BSpace`、二次 capture 确认删除。
  - probe 字符只来自 manifest，不在逻辑里硬编码。

## P4A-T3: SPAWNING lifecycle loop cutover

- **类型**: cutover。
- **目标**: 替换 `init_probe_task.rs` 的纯扫文字循环；SPAWNING 期如果被已知 prompt 卡住，也能执行内置动作清障，然后复测 can-input。
- **主要改动点**:
  - `src/provider/init_probe_task.rs:44` 改成：capture visible → classify dialog/ready → PromptRecurrentLoop 清障 → OutcomePredicate 复测 → 成功后 `mark_agent_idle_matched`。
  - `src/rpc/handlers.rs:457` reader 仍先注册 parser/fifo；`idle_scan_enabled` 不再作为“启动期不能点框”的硬阻塞。
  - `src/provider/init_probe.rs` 旧 `InitGateProbe::detect` 退化为 ready-candidate/diagnostic helper，不能作为最终成功判据。
- **验收**:
  - 启动期已知 trust/update prompt 能被处理，不再等到 timeout UNKNOWN。
  - 清障后必须通过 can-input 才能 IDLE。
  - 超时仍进入 `UNKNOWN` 或 `PROMPT_PENDING`，不能无限 loop。

## P4A-T4: PromptRecurrentLoop 确定性版本

- **类型**: 新增 + cutover。
- **目标**: 建立通用循环的 PR4a 子集：读屏 → 内置 regex 选动作 → 执行动作 → 等稳定 → 复测 outcome → max_depth 封顶。PR4a 不接 DB/LLM 慢路径。
- **主要改动点**:
  - 复用 `src/prompt_handler/integration.rs:41`、`src/prompt_handler/runner.rs:108`、`src/prompt_handler/gating.rs:38`，但 runner 结果必须接受 outcome predicate 复测。
  - 对同一 dialog 区域指纹连续无效 action 要停止重发并升级，而不是继续按到 depth_exceeded。
  - 指纹提取只取对话框区域，不使用整屏 `sanitize_pane_text` 作为快路径 key，避免 footer/quota/memory 状态栏噪点打穿。
- **验收**:
  - 同一 prompt 连续重复 capture 时，同一 action 不无限重发。
  - 屏幕变成 ready 后只做 can-input，不再 fire 已处理 prompt action。
  - `max_depth=3` 封顶后进入 pending/unknown，不活锁。

## P4A-T5: IDLE→WORKING 与 WORKING→DONE 判据接入

- **类型**: cutover。
- **目标**: 把 PR5 的 ACK→BUSY 机制并入生命周期主干：IDLE 发 job 后进 WAITING_FOR_ACK；出现 working/diff 信号进 BUSY；DONE 必须输出稳定且 can-input 通过。
- **主要改动点**:
  - `src/rpc/handlers.rs:1072` 的 `spawn_new_capture_seed` 保留为 ACK/working 信号来源，但结果要走 OutcomePredicate。
  - `src/orchestrator/mod.rs:152` 的 ACK stability fallback 要与 visual diff 路径合并语义，避免双状态机。
  - `src/agent_io/reader.rs:57` 的 marker stability 回 IDLE 前加 can-input 复核。
  - `src/pane_diff/mod.rs:73` 继续作为 STUCK 检测，不作为 DONE 成功判据。
- **验收**:
  - WAITING_FOR_ACK→BUSY 只发生一次，reason 明确。
  - BUSY→IDLE 必须完成 job reply collection 后再 IDLE。
  - 旧 PR5 “Gemini 跳过 WAITING_FOR_ACK / ACK→BUSY 不可见”断言被新 lifecycle 测试覆盖。

## P4A-T6: PROMPT_PENDING 放开 SPAWNING

- **类型**: cutover。
- **目标**: 启动期未知 prompt 能进入 `PROMPT_PENDING`，等主控 resolve；不再只能 `INIT_PROBE_TIMEOUT -> UNKNOWN`。
- **主要改动点**:
  - `src/db/state_machine.rs:193` allowed states 增加 `STATE_SPAWNING`。
  - `src/prompt_handler/integration.rs:153` 的 pending transition 同步允许 `SPAWNING`。
  - `src/db/system.rs:415` reconcile 保持 `PROMPT_PENDING`，不得重启/误杀/派 job。
- **验收**:
  - SPAWNING agent 未知 prompt 可 CAS 到 `PROMPT_PENDING` 并写事件。
  - `PROMPT_PENDING` 仍不是 active/terminal，orchestrator 不派新 job。
  - `agent.resolve_prompt` 后按是否有 dispatched job 回 IDLE/BUSY。

## P4A-T7: cutover 测试迁移与 PR5 遗留断言清理

- **类型**: cutover。
- **目标**: 把旧 init-probe/PR5/§10 测试迁移到 lifecycle contract，删除会与新主干冲突的双状态机断言。
- **主要改动点**:
  - `tests/mvp12_init_probe.rs` 和 `src/provider/init_probe.rs` 单测改为 ready-candidate/diagnostic，不再断言最终 readiness。
  - `tests/r2_waiting_for_ack.rs`、`tests/mvp12_r2_dispatcher_lifecycle.rs` 保留 WAITING_FOR_ACK 互斥与 job completion 核心，删除依赖旧 ACK 双路径的断言。
  - 新增 PR4a lifecycle contract tests：重复 prompt 不无限重发、dialog 不发裸 probe、ready can-input probe+BSpace、真实 update/trust 快照 pin。
- **验收**:
  - `cargo test` 全绿。
  - 新测试不依赖真实 codex/gemini/claude，只用 fake IO / fixture capture。
  - PR6b 再做真 3-provider/VPS 达成门。

## PR4a 完成条件

- [ ] PR4a lifecycle contract tests 全绿。
- [ ] `cargo test` 全绿。
- [ ] `cargo fmt --check` 通过。
- [ ] `DESIGN.md` §10 标记旧方案 superseded，并写入生命周期主干。
- [ ] 明确记录 PR4b out of scope：`prompt_experience` 表、LLM 慢路径、HTTP client、成功率自学习。
