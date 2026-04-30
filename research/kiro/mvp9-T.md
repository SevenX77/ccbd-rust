# Kiro Tasks: MVP 9 (项目启动器与生命周期回收)

> 文档定位：MVP 9 由 Codex 逐项实施的原子任务清单。本文基于 mvp9-R.md / mvp9-D.md，将 Launcher、Lifecycle Reconcile、Layout、Job Cancel、Doctor/Logs/Config
> CLI 收尾拆分成可独立验证的任务。MVP 9 是 ccbd-rust 迈向 1.0 的交付层补全阶段。
>
> **Plan-Review 修订记录（2026-04-30 Gemini Round 1）**：
> - 🔴 T1.3 加事务边界纪律：pid 探活分阶段 A/B/C/D，禁止 OS 探活在 SQLite 事务内
> - 🔴 T2.4 加 cancel_requested 跳过 is_prompt_only_reply（否则 cancel 永远不收尾）
> - 🟡 T2.2 加 daemon 端 per-session window 创建锁 + 升级预估 M→L
> - 🟢 T0.4 加 env 透传机制说明
>
> 范围约束：严格按 MVP9 R/D 落地：实现 ccb.toml、ccb start、项目级 session.kill、启动期 reconcile、tmux layout、job.cancel、ccb doctor/logs/config validate。
> 不引入无限嵌套 layout、不重写 MVP8 mailbox、不破坏既有 agent.* / job.* 语义。
>
> 实施顺序说明：本文按 4 个物理 stage 拆分：G9.0 Launcher、G9.1 Lifecycle/Reconcile、G9.2 Layout/Cancel、G9.3 Doctor/CLI parity。每个 stage 末尾都有 commit
> checkpoint。由于 Layout 是最大风险，G9.2 明确要求同步修补受影响的 MVP6/7/8 acceptance tests。

———

## 0. 总览

### 0.1 Stage 划分

| Stage | 主题 | 目标 | Checkpoint |
|---|---|---|---|
| G9.0 | Launcher | ccb.toml 解析、ccb start 批量启动、失败回滚 | ccb start 能按配置并行 spawn 多 agent |
| G9.1 | Reconcile + Kill | session.kill、启动期 pid 探活、资源级联清理 | 项目级 kill 和 daemon restart 自愈可测 |
| G9.2 | Layout + Cancel | 1:N window:pane 布局、session.apply_layout、job.cancel | 多 agent 同 window 分屏，queued/dispatched job 可取消 |
| G9.3 | Doctor + Logs + Config | ccb doctor、ccb logs、ccb config validate、CLI parity | 1.0 CLI 命令集完整 |

### 0.2 任务依赖图

graph TD
  subgraph G90[G9.0 Launcher]
    T01[T0.1 config model + toml parser]
    T02[T0.2 config discovery + validation]
    T03[T0.3 CLI module split]
    T04[T0.4 ccb start orchestration]
    T05[T0.5 launcher acceptance]
    T06[T0.6 G9.0 commit]
  end

  subgraph G91[G9.1 Lifecycle + Reconcile]
    T11[T1.1 session DB helpers]
    T12[T1.2 session.kill RPC]
    T13[T1.3 startup pid reconcile]
    T14[T1.4 resource cleanup hardening]
    T15[T1.5 lifecycle acceptance]
    T16[T1.6 G9.1 commit]
  end

  subgraph G92[G9.2 Layout + Cancel]
    T21[T2.1 tmux layout primitives]
    T22[T2.2 shared session window spawn]
    T23[T2.3 session.apply_layout RPC]
    T24[T2.4 jobs cancel schema + DB]
    T25[T2.5 job.cancel RPC + Ctrl-C]
    T26[T2.6 layout/cancel acceptance + old test patch]
    T27[T2.7 G9.2 commit]
  end

  subgraph G93[G9.3 Doctor + CLI parity]
    T31[T3.1 ccb doctor]
    T32[T3.2 ccb logs]
    T33[T3.3 ccb config validate]
    T34[T3.4 help/version/ps polish]
    T35[T3.5 G9.3 commit]
  end

  T01 --> T02 --> T03 --> T04 --> T05 --> T06
  T06 --> T11 --> T12 --> T13 --> T14 --> T15 --> T16
  T16 --> T21 --> T22 --> T23 --> T24 --> T25 --> T26 --> T27
  T27 --> T31 --> T32 --> T33 --> T34 --> T35

### 0.3 Layout 风险说明

MVP9 最大风险是 tmux 语义从 Agent = Window = Pane 改成 Session = Window, Agent = Pane。

必须同步检查并修补这些测试/假设：

- tests/mvp6_acceptance.rs
    - 可能假设每个 agent.spawn 创建独立 window。
    - 修补方向：只断言 agent 有唯一 pane、fifo reader 正常、输出可读，不再断言 window 数等于 agent 数。
- tests/mvp7_acceptance.rs
    - marker / provider readiness 依赖 pane capture，不应依赖 window 名。
    - 修补方向：通过 agent_io::pane_id(agent_id) 定位 pane。
- tests/mvp8_acceptance.rs
    - serial queue / watch / pend 只应关心 pane 和 DB job 状态。
    - 修补方向：多 agent 同 session 时不要假设 tmux window 名为 agent_id。
- src/tmux/session.rs 单测
    - spawn_window 仍可保留底层 primitive，但新增 shared-window split primitive 后要补对应单测。
- src/rpc/handlers.rs::handle_agent_spawn
    - 当前直接 spawn_window(SESSION_NAME, agent_id, ...)。G9.2 要改为：第一个 agent 创建项目 window，后续 agent split pane。

———

## 1. 原子任务定义（G9.0 Launcher：配置与批量启动）

### T0.1: 定义 ccb.toml 配置模型与 TOML 解析

- 文件路径:
    - 修改：Cargo.toml
    - 新建：src/cli/config.rs
    - 修改：src/bin/ccb.rs
- 输入:
    - mvp9-R.md AC1
    - mvp9-D.md §2
    - 现有 src/bin/ccb.rs
- 输出:
    - ProjectConfig
    - AgentConfig
    - LayoutConfig
    - TOML parse API：load_project_config(path) -> Result<ProjectConfig, CliError>
- 依赖: 无
- 执行步骤:
    1. 在 Cargo.toml 增加 toml 依赖。
    2. 新建 src/cli/config.rs，定义 serde 可反序列化 struct：
        - version: String
        - layout: Option<String>
        - env: HashMap<String, String>
        - agents: BTreeMap<String, AgentConfig>
    3. 支持 agent 级 env override。
    4. 校验 version == "1"。
    5. 校验 layout ∈ single|stack|grid，缺省为 grid。
    6. 校验 agent id 非空、只包含 ASCII alnum / _ / -。
    7. 校验 provider 非空。
- 验收:
    - cargo test --bin ccb cli::config --quiet 通过。
    - 单测覆盖 valid config、unknown layout、empty agents、bad agent id。
    - cargo build --quiet 通过。
- 预估: M

### T0.2: 实现 config 查找与 validate 基础能力

- 文件路径:
    - 修改：src/cli/config.rs
    - 修改：src/bin/ccb.rs
- 输入:
    - mvp9-D.md §2.2
- 输出:
    - find_config(start_dir) -> Result<PathBuf, CliError>
    - validate_project_config(config) -> Vec<Diagnostic>
- 依赖: T0.1
- 执行步骤:
    1. 实现查找顺序：
        - CCB_CONFIG_PATH
        - 从 cwd 向上查找 ccb.toml
    2. 若未找到，返回清晰错误并提示创建 ccb.toml。
    3. validate 不连接 daemon，只做本地静态检查。
    4. 为后续 ccb config validate 复用同一套逻辑。
- 验收:
    - 单测用 tempdir 覆盖 cwd 向上查找。
    - 单测覆盖 CCB_CONFIG_PATH 优先级。
    - cargo test --bin ccb cli::config --quiet 通过。
- 预估: S

### T0.3: 拆分 CLI 模块，控制 src/bin/ccb.rs 体积

- 文件路径:
    - 修改：src/bin/ccb.rs
    - 新建：src/cli/mod.rs
    - 新建：src/cli/rpc_client.rs
    - 新建：src/cli/output.rs
- 输入:
    - 现有 src/bin/ccb.rs
    - mvp9-D.md §3
- 输出:
    - rpc_call 移入 cli::rpc_client
    - common output helpers 移入 cli::output
    - ccb.rs 只保留 Clap enum 和 dispatch skeleton
- 依赖: T0.1, T0.2
- 执行步骤:
    1. 将现有 CliError、rpc_call、socket resolve 移入可复用模块。
    2. 保持现有 ping/ps/ask/pend/watch 行为不变。
    3. 避免一次性重写 CLI；仅做机械拆分。
- 验收:
    - cargo test --quiet 通过。
    - ccb --help 构建正常。
    - 现有 CLI 单元行为不回归。
- 预估: M

### T0.4: 实现 ccb start 批量启动编排与失败回滚

- 文件路径:
    - 修改：src/bin/ccb.rs
    - 新建：src/cli/start.rs
    - 修改：src/cli/rpc_client.rs
- 输入:
    - mvp9-R.md AC1
    - mvp9-D.md §3
    - handle_session_create
    - handle_agent_spawn
- 输出:
    - CLI 子命令：ccb start [--config PATH] [--wait]
    - 成功输出 session id 和 agent spawn summary
    - 失败时调用 session.kill(session_id, force=true) 回滚
- 依赖: T0.3
- 执行步骤:
    1. 新增 Clap subcommand Start.
    2. 解析 config 并创建 session：
        - session.create { project_id, absolute_path, master_pid }
    3. 对每个 configured agent 调 agent.spawn。
    4. Spawn 初版可串行实现；若使用并行，必须保证错误聚合和回滚清晰。
    5. 任一 spawn 失败：
        - 调 session.kill(force=true)。
        - 输出失败 agent 和原始 RPC error。
    6. --wait 时轮询 agent.watch 或 DB state，直到所有 agent IDLE 或 timeout。
    7. 暂不做 layout；G9.2 接 session.apply_layout。
    8. **【Plan-Review Minor】env 透传**：config 里 `[env]` global map + `[agents.X.env]` override 必须打包传给 agent.spawn。当前 RPC schema 已有 `sandbox_overrides` 字段，但 sandbox_overrides 是 bwrap 配置不是 env_vars。**建议**：让 agent.spawn 加一个新 optional 字段 `extra_env_vars: HashMap<String, String>`，CLI 在 ccb start 把合并后的 env map 塞进去；daemon 端透传给 systemd-run --setenv 或 env_state。如果觉得改 RPC 太大，可以先放进 sandbox_overrides 的扩展字段下做兜底，G9.0 不强求完整闭环。
- 验收:
    - CLI unit test 使用 fake rpc client 覆盖 all-or-nothing。
    - 手工 smoke：ccb start --config examples/ccb.toml 能发出 session.create + N 次 agent.spawn。
    - cargo build --quiet 通过。
- 预估: L

### T0.5: 新增 launcher acceptance 与示例配置

- 文件路径:
    - 新建：tests/mvp9_acceptance.rs
    - 新建：examples/ccb.toml
- 输入:
    - mvp9-D.md §9.1
    - G9.0 output
- 输出:
    - test_launcher_config_parse_and_batch_spawn
    - examples/ccb.toml
- 依赖: T0.4
- 执行步骤:
    1. 复用 MVP8 Harness：temp DB、temp state_dir、unsafe no sandbox。
    2. 构造 2-3 个 bash agent 的 config。
    3. 直接调用 CLI start orchestration helper 或 RPC handlers，不要求启动真实 ccb 进程。
    4. 断言：
        - 创建一个 session。
        - 所有 configured agents 写入 DB。
        - spawn 失败时调用回滚路径。
- 验收:
    - cargo test --test mvp9_acceptance --quiet 通过。
    - cargo test --quiet 不回归。
- 预估: M

### T0.6: G9.0 commit checkpoint

- 文件路径: 全部 G9.0 修改
- 输入: T0.1 - T0.5
- 输出: 一个 stage commit
- 依赖: T0.5
- 验收:
    1. cargo fmt
    2. cargo build --quiet
    3. cargo test --test mvp9_acceptance --quiet
    4. cargo test --quiet
    5. commit message: feat(mvp9): G9.0 add config launcher and ccb start
- 预估: XS

———

## 2. 原子任务定义（G9.1 Reconcile + session.kill）

### T1.1: 增加 session 查询与摘要 DB helpers

- 文件路径:
    - 修改：src/db/sessions.rs
    - 修改：src/db/schema.rs（仅 struct/helper，不改 DDL，除非 D 文档后续要求 session state）
    - 修改：src/db/system.rs
- 输入:
    - mvp9-R.md AC3/AC4
    - mvp9-D.md §5
    - 现有 sessions / agents 表
- 输出:
    - query_session_sync
    - query_sessions_sync
    - query_session_agents_sync
    - SessionSummary
- 依赖: T0.6
- 执行步骤:
    1. 增加按 session_id 查询 session 的 sync/async helper。
    2. 增加查询 session 下所有 agent 的 helper。
    3. 增加 session.list 所需 summary：
        - session id
        - project id
        - project absolute path
        - agent count
        - active agent count
    4. 避免改旧 schema；若需要 session lifecycle 状态，先在 helper 层用 agent state 聚合表达。
- 验收:
    - cargo test --lib db::sessions --quiet 通过。
    - 单测覆盖 empty session、active count、unknown session。
- 预估: S

### T1.2: 实现 session.kill RPC 与 router 注册

- 文件路径:
    - 修改：src/rpc/handlers.rs
    - 修改：src/rpc/router.rs
    - 修改：src/db/system.rs
- 输入:
    - mvp9-R.md AC3
    - mvp9-D.md §5.2
    - 现有 cascade_kill_session_agents_sync
    - 现有 handle_agent_kill
- 输出:
    - RPC：session.kill { session_id, force }
    - Router method "session.kill"
- 依赖: T1.1
- 执行步骤:
    1. 新增 handle_session_kill.
    2. 验证 session 存在。
    3. 找出 session 下所有 active agent。
    4. 对每个 agent：
        - 优先 pidfd SIGKILL。
        - 标记 DB 为 KILLED，保留历史日志。
        - cancel marker timer。
        - shutdown reader。
        - remove monitor registry。
        - remove parser registry。
    5. force=true 时忽略单个 pane/process cleanup 的 NotFound 类错误，继续清理其他资源。
    6. 返回 killed count。
- 验收:
    - handler 单测覆盖 unknown session、active agents killed、repeat kill idempotent。
    - router 单测证明 method 已注册。
    - cargo test --lib rpc::handlers::tests::test_handle_session_kill --quiet 通过。
- 预估: M

### T1.3: 强化 daemon startup reconcile 的 pid 探活

- 文件路径:
    - 修改：src/db/system.rs
    - 修改：src/bin/ccbd.rs（如需调整调用顺序）
    - 可能修改：src/monitor/mod.rs
- 输入:
    - mvp9-R.md AC4
    - mvp9-D.md §5.1
    - 现有 reconcile_startup_sync
- 输出:
    - 启动期扫描 SPAWNING/BUSY/IDLE agents。
    - 对每个 agent pid 做 pidfd_open 或 kill(pid, 0) 探活。
    - dead pid -> CRASHED / dispatched job FAILED / state_change event。
- 依赖: T1.2
- 执行步骤:
    1. 保留现有 master pidfd reconcile。
    2. 将 active agent reconcile 从“全部 active 直接 CRASHED”改为“逐个 pid 探活”。
    3. **【Plan-Review Blocker #1】事务边界纪律**：绝不能把 `kill(pid, 0)` 这种 OS 级耗时探活放在 SQLite 事务里。必须分两阶段：
        - 阶段 A（事务）：`SELECT id, pid, state FROM agents WHERE state IN ('SPAWNING','BUSY','IDLE')`，立即提交事务，把候选列表读到 Rust Vec。
        - 阶段 B（无事务）：在 Rust 层遍历 Vec，对每个 pid 调用 `nix::sys::signal::kill(Pid::from_raw(pid), None)`，把死的收到 dead_list、活的收到 alive_list。
        - 阶段 C（事务）：对 dead_list 在一个事务里 mark CRASHED + cascade dispatched job → FAILED + remove registry。
        - 阶段 D（无事务）：对 alive_list 重新订阅 reader / 注册 pidfd / 启动相应 marker timer。
    4. dead pid：
        - mark CRASHED，reason STARTUP_RECONCILE_DEAD_PID。
        - cascade dispatched job -> FAILED。
        - remove agent_io / marker / parser / monitor registry。
    5. alive pid：
        - 注册 pidfd 到 monitor。
        - 若 FIFO 文件存在，重新启动 reader。
        - 若 agent state 是 BUSY，重新启动 Busy marker timer。
        - 若 state 是 SPAWNING，重新启动 Startup marker timer。
    6. 无法恢复 reader/fifo 时降级为 UNKNOWN 或 CRASHED，不要让 job 永久 DISPATCHED。
- 验收:
    - cargo test --lib db::system --quiet 通过。
    - 单测覆盖 dead pid agent -> CRASHED。
    - 单测覆盖 alive pid agent 不被误标 CRASHED。
    - MVP8 restart reconcile 测试仍通过。
- 预估: L

### T1.4: 清理 tmux pane/window 与 sandbox/fifo 残留

- 文件路径:
    - 修改：src/tmux/session.rs
    - 修改：src/agent_io/registry.rs
    - 修改：src/rpc/handlers.rs
    - 修改：src/db/system.rs
- 输入:
    - mvp9-D.md §5.2
    - 现有 cleanup_spawn_resources
    - 现有 agent_io::shutdown_reader
- 输出:
    - kill_window
    - kill_session_window
    - cleanup_agent_runtime_resources(agent_id)
- 依赖: T1.3
- 执行步骤:
    1. 在 tmux wrapper 增加 kill_window(session, window)。
    2. session.kill 完成 agent DB 标记后，kill 项目 window。
    3. 清理 FIFO 文件。
    4. 清理 sandbox dir。
    5. 所有 cleanup helper 必须 tolerate NotFound。
- 验收:
    - tmux unit test 覆盖 kill-window。
    - acceptance 覆盖 session.kill 后 tmux window 不存在。
    - cargo test --quiet 通过。
- 预估: M

### T1.5: Lifecycle acceptance tests

- 文件路径:
    - 修改：tests/mvp9_acceptance.rs
    - 可能修改：tests/mvp2_acceptance.rs
- 输入:
    - mvp9-D.md §9.2 / §9.3
- 输出:
    - test_session_kill_cleans_resources
    - test_reconcile_cleans_dead_pids_on_boot
- 依赖: T1.4
- 执行步骤:
    1. 启动 session + 2 个 bash agents。
    2. 调 session.kill(force=true)。
    3. 断言：
        - agents 状态为 KILLED。
        - monitor registry 清空。
        - reader shutdown。
        - tmux project window 已销毁。
    4. 构造 dead pid 或短生命周期 child pid。
    5. 调 reconcile，断言 agent 变 CRASHED，dispatched job 变 FAILED。
- 验收:
    - cargo test --test mvp9_acceptance --quiet 通过。
    - cargo test --test mvp2_acceptance --quiet 不回归。
- 预估: M

### T1.6: G9.1 commit checkpoint

- 文件路径: 全部 G9.1 修改
- 输入: T1.1 - T1.5
- 输出: 一个 stage commit
- 依赖: T1.5
- 验收:
    1. cargo fmt
    2. cargo test --lib db::system db::sessions --quiet
    3. cargo test --test mvp9_acceptance --quiet
    4. cargo test --quiet
    5. commit message: feat(mvp9): G9.1 add session kill and startup reconcile
- 预估: XS

———

## 3. 原子任务定义（G9.2 Layout + job.cancel）

### T2.1: 新增 tmux layout primitives

- 文件路径:
    - 新建：src/tmux/layout.rs
    - 修改：src/tmux/mod.rs
    - 修改：src/tmux/session.rs
- 输入:
    - mvp9-R.md AC2
    - mvp9-D.md §4.2
    - tmux native layouts：even-vertical, even-horizontal, tiled
- 输出:
    - LayoutKind::{Single, Stack, Grid}
    - select_layout(window_target, LayoutKind)
    - list_panes(window_target)
- 依赖: T1.6
- 执行步骤:
    1. 新建 tmux/layout.rs，解析 layout string。
    2. 实现 select-layout wrapper：
        - single no-op
        - stack -> even-vertical
        - grid -> tiled
    3. 增加 list-panes -F "#{pane_id}" wrapper。
    4. 所有 tmux command error 继续使用 CcbdError::TmuxCommandFailed。
- 验收:
    - tmux unit test 覆盖 layout parser。
    - tmux integration test 在 temp server 中创建 3 panes 后 apply tiled 成功。
    - cargo test --lib tmux --quiet 通过。
- 预估: M

### T2.2: 将 agent.spawn 从 per-agent window 改为 shared session window + split pane

- 文件路径:
    - 修改：src/rpc/handlers.rs
    - 修改：src/tmux/session.rs
    - 修改：src/agent_io/registry.rs（如需保存 window target）
    - 可能修改：src/db/sessions.rs
- 输入:
    - mvp9-D.md §4.1
    - 现有 handle_agent_spawn
- 输出:
    - 第一个 agent spawn 创建 session project window。
    - 后续 agent spawn 在同一 window 中 split pane。
    - 每个 agent 仍注册唯一 pane_id。
- 依赖: T2.1
- 执行步骤:
    1. 定义项目 window name：ccb:<project_id> 或稳定等价命名。
    2. 从 session_id 查询 project id / absolute path。
    3. 若 window 不存在：
        - create window with first agent command。
    4. 若 window 存在：
        - split-window -d -t <window> -c <cwd> -P -F "#{pane_id}" -- <cmd>。
    5. 保持 fifo、reader、parser、pidfd 注册逻辑不变。
    6. spawn 失败 rollback 必须只清理本 agent 的 pane/fifo/sandbox，不杀整个 session。
    7. **【Plan-Review Major #1】Window 创建并发竞争**：当 ccb start 用 `join_all` 并发发 N 个 agent.spawn，所有请求会同时检查 "window 存在吗" 然后竞相 new-window，导致 tmux 报错。Daemon 端必须有同步序列化机制：在 handle_agent_spawn 中加 `tokio::sync::Mutex<HashMap<session_id, Arc<Mutex<()>>>>` 锁住"同一 session 的 window 创建/查询/split"路径——同一 session_id 的 spawn 串行排队、不同 session_id 不互锁。或者更简单：CLI 端 ccb start 第一个 agent 串行 spawn（等返回再并发剩余）—— T0.4 必须做这个串行化。**两种修法二选一，T-doc 推荐前者（daemon 内部锁），原因是它对未来其他 caller（比如直接调 RPC 的脚本）也安全**。
    8. spawn 失败 rollback 时，如果 split 完发现是首个 pane 死了，window 会自动消失；下次 spawn 要正确处理"window 没了"重建路径（不是 idempotent 假设 window 永远在）。
- 验收:
    - 新单测：同一 session spawn 两个 agents，pane_id 不同但 window target 相同。
    - **新单测：3 个 agent.spawn 并发请求同一 session，全部成功，最终 1 个 window + 3 个 pane（覆盖 Plan-Review Major #1）。**
    - test_handle_agent_spawn_returns_idle_and_inserts_pid 仍通过。
    - cargo test --lib rpc::handlers --quiet 通过。
- 预估: **L**（Plan-Review 调整：原标 M，含锁竞争 + rollback 边界处理升 L）

### T2.3: 实现 session.apply_layout RPC 与 CLI start 接入

- 文件路径:
    - 修改：src/rpc/handlers.rs
    - 修改：src/rpc/router.rs
    - 修改：src/cli/start.rs
    - 修改：src/bin/ccb.rs
- 输入:
    - mvp9-D.md §3.1 / §4.2 / §8
- 输出:
    - RPC：session.apply_layout { session_id, layout }
    - Router method "session.apply_layout"
    - ccb start 在 spawn 全成功后调用 apply_layout
- 依赖: T2.2
- 执行步骤:
    1. Handler 验证 session 存在。
    2. 根据 session project id 构造 window target。
    3. 调 tmux::layout::apply_layout。
    4. 返回 { "status": "ok", "layout": "grid" }。
    5. CLI start 在所有 agent spawn 成功后调用此 RPC。
- 验收:
    - router 单测覆盖 method 注册。
    - handler 单测覆盖 invalid layout。
    - mvp9 acceptance 可观察 tmux panes 在同一 window。
- 预估: M

### T2.4: 扩展 jobs schema 支持 CANCELLED 与 cancel_requested

- 文件路径:
    - 修改：src/db/schema.rs
    - 修改：src/db/jobs.rs
    - 修改：src/db/state_machine.rs
- 输入:
    - mvp9-R.md AC5
    - mvp9-D.md §6.2
- 输出:
    - jobs 表新增 cancel_requested INTEGER NOT NULL DEFAULT 0
    - Job struct 新增 cancel_requested: bool 或 i64
    - queued cancel helper
    - dispatched cancel request helper
    - idle completion hook 支持 CANCELLED
- 依赖: T2.3
- 执行步骤:
    1. 修改 DDL：新增 cancel_requested 字段。
    2. 更新所有 SELECT jobs row mapper。
    3. 增加 helper：
        - mark_queued_job_cancelled_sync
        - request_dispatched_job_cancel_sync
        - mark_job_cancelled_conn_sync
    4. mark_agent_idle_matched_sync 查到 dispatched job 后：
        - 若 cancel_requested = true，最终写 CANCELLED。
        - 否则按 MVP8 原逻辑 COMPLETED。
    5. 确保 job.wait 对 CANCELLED 也视为 terminal。
    6. **【Plan-Review Blocker #2】绕过 is_prompt_only_reply 拦截**：MVP8 round-2 在 mark_agent_idle_matched_sync 加了 `is_prompt_only_reply` guard（reply 仅含 prompt 字符时 swallow 这次 IDLE 转移）。Ctrl-C 后 Agent 输出 `^C\n$ ` 会被它当作 prompt-only reply 吞掉，导致 cancel 永远不收尾，job 永远 DISPATCHED。**修法**：在 mark_agent_idle_matched_sync 里查到的 dispatched job 若 `cancel_requested = true`，**必须跳过 is_prompt_only_reply 检查**，直接走 CANCELLED 路径（reply_text 可以为空）。新增 `cancel_requested + prompt_only_reply` 的单测专门覆盖此场景。
- 验收:
    - cargo test --lib db::jobs --quiet 通过。
    - 单测覆盖 QUEUED -> CANCELLED。
    - 单测覆盖 DISPATCHED + cancel_requested + IDLE -> CANCELLED。
    - 单测覆盖 cancel_requested=true + prompt-only reply 不被 swallow（Plan-Review Blocker #2）。
- 预估: M

### T2.5: 实现 job.cancel RPC 和 Ctrl-C 注入

- 文件路径:
    - 修改：src/rpc/handlers.rs
    - 修改：src/rpc/router.rs
    - 修改：src/tmux/session.rs
    - 修改：src/agent_io/writer.rs（可选）
    - 修改：src/bin/ccb.rs
- 输入:
    - mvp9-D.md §6
    - mvp9-R.md AC5
- 输出:
    - RPC：job.cancel { job_id }
    - CLI：ccb cancel <job_id>
    - tmux send Ctrl-C primitive
- 依赖: T2.4
- 执行步骤:
    1. 增加 tmux wrapper：send_ctrl_c(pane).
    2. Handler 查询 job。
    3. 若 job QUEUED：
        - DB 直接 CANCELLED。
        - notify job update。
    4. 若 job DISPATCHED:
        - 设置 cancel_requested = 1。
        - 找 agent pane。
        - 发送 Ctrl-C。
        - 返回 { "status": "CANCEL_REQUESTED" }。
    5. 若 job 已 terminal：
        - 幂等返回当前 status。
    6. 若 missing job：
        - IpcInvalidRequest.
    7. CLI ccb cancel 打印最终/请求状态。
- 验收:
    - handler 单测覆盖 queued/dispatched/completed/missing。
    - tmux test 验证 sleep 10 被 Ctrl-C 中断。
    - cargo test --lib rpc::handlers --quiet 通过。
- 预估: L

### T2.6: Layout/Cancel acceptance，并修补受影响旧测试

- 文件路径:
    - 修改：tests/mvp9_acceptance.rs
    - 修改：tests/mvp6_acceptance.rs
    - 修改：tests/mvp7_acceptance.rs
    - 修改：tests/mvp8_acceptance.rs
    - 修改：src/tmux/mod.rs tests（如有）
- 输入:
    - mvp9-D.md §9
    - T2.1 - T2.5
- 输出:
    - test_launcher_batch_spawn_and_layout
    - test_job_cancel_queued
    - test_job_cancel_dispatched_sends_sigint
    - Old acceptance tests updated for shared-window semantics
- 依赖: T2.5
- 执行步骤:
    1. mvp9 layout test：
        - create session。
        - spawn 3 bash agents。
        - apply grid。
        - assert 3 panes in one session window。
    2. queued cancel test：
        - long first job + queued second job。
        - cancel second。
        - assert CANCELLED and never dispatched。
    3. dispatched cancel test：
        - submit sleep 10; echo should_not_finish。
        - wait DISPATCHED。
        - cancel。
        - assert returns to IDLE before 10s。
        - assert job CANCELLED。
    4. Patch old tests:
        - remove window-name assumptions。
        - use pane ids and DB states as assertions。
    5. Avoid weakening behavioral assertions.
- 验收:
    - cargo test --test mvp6_acceptance --quiet
    - cargo test --test mvp7_acceptance --quiet
    - cargo test --test mvp8_acceptance --quiet
    - cargo test --test mvp9_acceptance --quiet
    - all 0 failed.
- 预估: L

### T2.7: G9.2 commit checkpoint

- 文件路径: 全部 G9.2 修改
- 输入: T2.1 - T2.6
- 输出: 一个 stage commit
- 依赖: T2.6
- 验收:
    1. cargo fmt
    2. cargo test --lib tmux db::jobs rpc::handlers --quiet
    3. cargo test --test mvp6_acceptance --quiet
    4. cargo test --test mvp7_acceptance --quiet
    5. cargo test --test mvp8_acceptance --quiet
    6. cargo test --test mvp9_acceptance --quiet
    7. cargo test --quiet
    8. commit message: feat(mvp9): G9.2 add shared layout and job cancel
- 预估: XS

———

## 4. 原子任务定义（G9.3 Doctor + Logs + Config Validate）

### T3.1: 实现 ccb doctor

- 文件路径:
    - 新建：src/cli/doctor.rs
    - 修改：src/bin/ccb.rs
- 输入:
    - mvp9-R.md AC6
    - mvp9-D.md §7.1
    - 现有 sandbox/env check
- 输出:
    - CLI：ccb doctor [--json]
    - diagnostic rows with status/info/suggestion
- 依赖: T2.7
- 执行步骤:
    1. 检查 system binaries：
        - tmux
        - bwrap
        - systemd-run
    2. 检查 daemon socket：
        - exists
        - connectable
        - system.dump succeeds
    3. 检查 provider auth hints：
        - ~/.codex/auth.json
        - ~/.anthropic
        - ~/.claude
        - ~/.config/gemini
    4. 检查 cwd / .ccb / state dir 权限。
    5. 默认 human-readable table；--json 输出 machine-readable diagnostics。
    6. 不做 destructive repair，除非后续 D 文档新增 --fix。
- 验收:
    - unit test 覆盖 diagnostic aggregation。
    - manual smoke：缺 daemon 时有清楚建议。
    - cargo build --quiet 通过。
- 预估: M

### T3.2: 实现 ccb logs <agent>

- 文件路径:
    - 新建：src/cli/logs.rs
    - 修改：src/bin/ccb.rs
- 输入:
    - mvp9-R.md AC6
    - mvp9-D.md §7.2
    - RPC agent.read
- 输出:
    - CLI：ccb logs <agent_id> [--since-event-id N] [--raw-json]
- 依赖: T3.1
- 执行步骤:
    1. 调 agent.read(since_event_id=0)。
    2. 默认只打印 output_chunk.payload.text。
    3. state_change 用分割线打印。
    4. --raw-json 输出完整 event JSON。
    5. 打印最后 seen seq_id，方便用户后续 tail。
- 验收:
    - CLI helper 单测覆盖 payload parse。
    - smoke：对 bash agent 执行 echo 后 logs 能打印原始输出。
    - cargo test --bin ccb --quiet 通过。
- 预估: S

### T3.3: 实现 ccb config validate 与可选 migrate stub

- 文件路径:
    - 新建：src/cli/config_cmd.rs
    - 修改：src/bin/ccb.rs
    - 修改：src/cli/config.rs
- 输入:
    - mvp9-R.md AC6
    - mvp9-D.md §2
- 输出:
    - CLI：ccb config validate [--config PATH]
    - CLI：ccb config migrate stub 或 explicit “not implemented” message
- 依赖: T3.2
- 执行步骤:
    1. Add nested Clap subcommand:
        - Config { Validate, Migrate }
    2. validate 复用 T0.2 validate logic。
    3. 输出所有 warnings/errors。
    4. 若通过，exit 0。
    5. migrate 若不实现真实转换，必须清晰提示旧 JSON -> TOML 手工路径，不 silent success。
- 验收:
    - unit test 覆盖 validate exit behavior helper。
    - manual smoke：valid example returns ok。
    - cargo build --quiet 通过。
- 预估: S

### T3.4: CLI parity polish：ps、version、help、session list

- 文件路径:
    - 修改：src/bin/ccb.rs
    - 修改：src/cli/output.rs
    - 修改：src/rpc/handlers.rs
    - 修改：src/rpc/router.rs
- 输入:
    - mvp9-R.md AC6
    - mvp9-D.md §8
    - T1.1 SessionSummary
- 输出:
    - RPC：session.list
    - ccb ps shows sessions + agents cleanly
    - ccb version explicit subcommand or Clap version path documented
    - polished --help
- 依赖: T3.3
- 执行步骤:
    1. Add handle_session_list using T1.1 summary helper。
    2. Register "session.list" router method。
    3. Upgrade ccb ps to show session summary before agent table。
    4. Add explicit ccb version if product wants command parity; otherwise ensure ccb --version is documented in help.
    5. Review all new command help text for clarity.
- 验收:
    - router test for session.list。
    - cargo run --bin ccb -- --help manually reviewed。
    - cargo test --quiet 通过。
- 预估: M

### T3.5: G9.3 final checkpoint

- 文件路径: 全部 G9.3 修改
- 输入: T3.1 - T3.4
- 输出: final MVP9 stage commit
- 依赖: T3.4
- 验收:
    1. cargo fmt
    2. cargo build --quiet
    3. cargo test --test mvp9_acceptance --quiet
    4. cargo test --quiet
    5. Manual smoke:
        - ccb config validate --config examples/ccb.toml
        - ccb doctor
        - ccb start --config examples/ccb.toml --wait
        - ccb ps
        - ccb logs <agent_id>
        - ccb cancel <job_id>
        - ccb kill / session.kill
    6. commit message: feat(mvp9): G9.3 add doctor logs and config validation
- 预估: XS

———

## 5. MVP9 验收矩阵

| AC | 覆盖任务 | 自动测试 | 手工 smoke |
|---|---|---|---|
| AC1 Launcher / ccb start | T0.1-T0.5 | test_launcher_config_parse_and_batch_spawn | ccb start --config examples/ccb.toml --wait |
| AC2 Auto Layout | T2.1-T2.3, T2.6 | test_launcher_batch_spawn_and_layout | tmux attach 查看同 window 多 pane |
| AC3 Project Kill | T1.1-T1.5 | test_session_kill_cleans_resources | ccb kill --force |
| AC4 Boot Reconcile | T1.3-T1.5 | test_reconcile_cleans_dead_pids_on_boot | kill agent process, restart daemon |
| AC5 Job Control / Cancel | T2.4-T2.6 | test_job_cancel_queued, test_job_cancel_dispatched_sends_sigint | ccb cancel <job_id> |
| AC6 CLI Parity | T3.1-T3.4 | CLI helper unit tests | ccb doctor, ccb logs, ccb config validate, ccb --help |

———

## 6. 全量完成标准

MVP9 完成前必须满足：

1. cargo fmt 无 diff。
2. cargo build --quiet 通过。
3. cargo test --quiet 全部 0 failed。
4. cargo test --test mvp6_acceptance --quiet 通过。
5. cargo test --test mvp7_acceptance --quiet 通过。
6. cargo test --test mvp8_acceptance --quiet 通过。
7. cargo test --test mvp9_acceptance --quiet 通过。
8. examples/ccb.toml 可用于本地 smoke。
9. ccb --help 展示 1.0 命令集：
    - ping
    - ps
    - start
    - ask
    - pend
    - watch
    - cancel
    - kill
    - doctor
    - logs
    - config validate
    - version 或 --version
10. 不破坏 MVP8 mailbox 核心语义：
    - queued FIFO
    - serial-per-agent
    - job.wait
    - agent.watch
    - crash/unknown/killed job failure cascade.
