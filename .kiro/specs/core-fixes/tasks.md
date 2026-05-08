# Tasks: ccbd-rust Core Fixes

## 概述
- spec 来源: requirements.md / research.md / design.md v3 / inventory.md
- 总 task 数: 41
- 总估时: 45.5 工时
- 实施顺序: Phase 1 -> Phase 2 -> Phase 3

## 实施顺序图 (DAG)
```text
Phase 1: T1.1.1 -> T1.1.2 -> T1.1.3 -> T1.1.4
         T3.1.1 -> T3.1.2 -> T3.1.3 -> T1.5.1
         T1.2.1, T1.3.1, T1.4.x, T3.2.x, T3.3.1 可并行
Phase 2: T2.1.1 -> T2.1.2 -> T2.2.1 -> T2.4.x -> T2.5.1
Phase 3: T4.1.x, T4.2.x, T4.3.x
```

## Phase 1: Isolation (R1 + R3)

### T1.1.1: 定义 agent/master session 命名函数
- **依据**: design.md §1.1 + requirements.md R1.1
- **改动**: 在 tmux 模块保留 socket 逻辑, 将 `SESSION_NAME` 的共享语义替换为 `agent_session_name(agent_id)` / `master_session_name(project_id)`。
- **锚点**:
  - 改前: `src/tmux/mod.rs:15` `SESSION_NAME = "ccbd-agents"`
  - 改后: `src/tmux/mod.rs:15` 暴露 `agent_<id>` / `master_<project_id>` 生成逻辑
- **依赖**: 无；可并行 with T1.2.1, T1.3.1
- **估时**: 1h
- **测试**:
  - 单元: `src/tmux/mod.rs` tests
  - 集成: `tests/r1_session_naming.rs`
  - E2E: 不适用
- **验收**: `cargo test tmux::tests::*session_name*` pass, 命名不再返回 `ccbd-agents`
- **置信度**: A

### T1.1.2: 新增 kill_session_sync/async
- **依据**: design.md §1.1 + requirements.md R1.2
- **改动**: 在 `TmuxServer` 中封装 `tmux kill-session -t <name>`, 并提供 async 包装供清理链调用。
- **锚点**:
  - 改前: `src/tmux/session.rs:356` 仅有 `kill_pane_sync`; `src/tmux/session.rs:384` 仅杀 shared window
  - 改后: `src/tmux/session.rs:384` 附近新增 `kill_session_sync` 和 async wrapper
- **依赖**: T1.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/tmux/mod.rs` 或 `src/tmux/session.rs` tmux integration tests
  - 集成: `tests/r1_session_lifecycle.rs`
  - E2E: 不适用
- **验收**: 创建临时 session 后调用 kill_session, `tmux has-session` 返回非 0
- **置信度**: A

### T1.1.3: agent cleanup 改杀独立 session
- **依据**: design.md §1.1 + requirements.md R1.2
- **改动**: `cleanup_agent_runtime_resources` 从按 pane 清理改为对 `agent_<agent_id>` 执行 `kill_session_sync`, 其余 fifo/sandbox/marker 清理维持。
- **锚点**:
  - 改前: `src/agent_io/registry.rs:50` cleanup 内杀 pane
  - 改后: `src/agent_io/registry.rs:50` cleanup 内杀 `agent_<agent_id>` session
- **依赖**: T1.1.2
- **估时**: 1h
- **测试**:
  - 单元: `src/agent_io/registry.rs` cleanup tests
  - 集成: `tests/r1_session_lifecycle.rs`
  - E2E: 真 tmux cleanup smoke
- **验收**: agent CRASHED/KILLED 后 `agent_<id>` session 不存在, fifo/sandbox 仍被清理
- **置信度**: A

### T1.1.4: daemon shutdown 遍历 active sessions 清理
- **依据**: design.md §1.1 + requirements.md R1.2
- **改动**: `cleanup_tmux_resources` 不再 kill `ccbd-agents`, 改从 DB 查询 ACTIVE/非终态 agents 和 master session, 逐个 kill `agent_<id>` / `master_<project_id>`。
- **锚点**:
  - 改前: `src/bin/ccbd.rs:122` 直接 `kill-session -t SESSION_NAME`
  - 改后: `src/bin/ccbd.rs:122` 遍历 DB session 名并逐一 kill
- **依赖**: T1.1.2
- **估时**: 2h
- **测试**:
  - 单元: `src/bin/ccbd.rs` cleanup helper tests
  - 集成: `tests/r1_shutdown_cleanup.rs`
  - E2E: daemon SIGTERM 后 tmux `agent_*`/`master_*` 全无
- **验收**: `cargo test --test r1_shutdown_cleanup` pass
- **置信度**: A

### T1.2.1: ensure_session 锁定 PTY 尺寸
- **依据**: design.md §1.2 + requirements.md R1.3
- **改动**: `ensure_session_sync` 使用 `new-session -d -s <name> -c <cwd> -x 150 -y 60`, 创建后执行 `set-option -t <name> window-size manual`。
- **锚点**:
  - 改前: `src/tmux/session.rs:55` `-x 200 -y 60`, 无 manual window-size
  - 改后: `src/tmux/session.rs:55` `-x 150 -y 60` 后设置 `window-size manual`
- **依赖**: 无；可并行 with T1.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/tmux/mod.rs` command-shape tests
  - 集成: `tests/r1_pty_size.rs`
  - E2E: attach 不改变后台 pane 宽度
- **验收**: `tmux display -t <session> -p '#{window_width}x#{window_height}'` 为 `150x60`
- **置信度**: A

### T1.3.1: 统一 tmux scope BindsTo 服务名
- **依据**: design.md §1.3 + requirements.md R1.2
- **改动**: 将 tmux scope 中 `BindsTo=ccbd-rust.service` 改为 `BindsTo=ccbd.service`, 与 sandbox systemd wrapper 对齐。
- **锚点**:
  - 改前: `src/tmux/scope.rs:55` 使用 `ccbd-rust.service`
  - 改后: `src/tmux/scope.rs:55` 使用 `ccbd.service`
- **依赖**: 无；可并行 with T1.2.1
- **估时**: <30min
- **测试**:
  - 单元: `src/tmux/scope.rs` tests
  - 集成: `tests/mvp10_acceptance.rs`
  - E2E: systemd-run 下 scope property 包含 `BindsTo=ccbd.service`
- **验收**: `cargo test mvp10` pass 且断言新服务名
- **置信度**: A

### T1.3.2: master_watch 归零后触发 daemon 自杀
- **依据**: design.md §1.3 + requirements.md R1.2
- **改动**: 保留 master pidfd cascade, 增加 active_agents 归零检测和 5s grace 后 `system.shutdown` 路径。
- **锚点**:
  - 改前: `src/monitor/master_watch.rs:7` 只 cascade kill agents
  - 改后: `src/monitor/master_watch.rs:7` cascade 后判断 active agent 数并请求 shutdown
- **依赖**: T1.1.4
- **估时**: 2h
- **测试**:
  - 单元: `src/monitor/master_watch.rs` tests
  - 集成: `tests/r1_master_exit_shutdown.rs`
  - E2E: 杀 master PID 后 5s 内 daemon 退出
- **验收**: `cargo test --test r1_master_exit_shutdown` pass
- **置信度**: A

### T1.3.3: 增加 daemon auto_shutdown_on_master_exit 配置
- **依据**: design.md §1.3 + requirements.md R1.2
- **改动**: 在 daemon 配置读取链中增加 `[daemon] auto_shutdown_on_master_exit = true`, 并让 T1.3.2 受该开关控制。
- **锚点**:
  - 改前: `src/cli/config.rs:8` 项目配置无 daemon auto shutdown 字段
  - 改后: `src/cli/config.rs:8` 或 daemon config 模块新增开关及默认值
- **依赖**: T1.3.2
- **估时**: 1h
- **测试**:
  - 单元: `src/cli/config.rs` config parse tests
  - 集成: `tests/r1_master_exit_shutdown.rs`
  - E2E: 配置 false 时 master 退出不杀 daemon
- **验收**: config 默认 true, false 可禁用自杀
- **置信度**: A

### T1.4.1: 移除 agent.spawn layout hint 路由
- **依据**: design.md §1.4 + requirements.md R1.1
- **改动**: 删除 `has_layout_hint` 分支和 split 优先逻辑, agent.spawn 始终进入独立 `agent_<id>` session 创建路径。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:348` layout hint 决定 split/spawn 路径
  - 改后: `src/rpc/handlers.rs:348` 只调用 agent 独立 session spawn
- **依赖**: T1.1.1, T3.1.3
- **估时**: 2h
- **测试**:
  - 单元: `src/rpc/handlers.rs` spawn tests
  - 集成: `tests/r1_session_naming.rs`
  - E2E: agent.spawn 后 `tmux ls` 出现 `agent_<id>`
- **验收**: agent.spawn 不再调用 `split_window*`
- **置信度**: A

### T1.4.2: 删除 CLI grid split plan 和 RPC 字段
- **依据**: design.md §1.4 + requirements.md R1.1
- **改动**: 删除 `split_plan_for_layout`, `layout_direction` 等启动 RPC 参数, CLI start 只发送 master/agent 基础 spawn 请求。
- **锚点**:
  - 改前: `src/cli/start.rs:234` `split_plan_for_layout`
  - 改后: `src/cli/start.rs:80` 不再生成 split plan 或 layout params
- **依赖**: T1.4.1
- **估时**: 2h
- **测试**:
  - 单元: 删除/替换 `src/cli/start.rs:336` layout tests
  - 集成: `tests/r1_start_no_layout.rs`
  - E2E: `ccb-rust start` 不发送 layout hints
- **验收**: `rg "layout_direction|split_plan_for_layout" src tests` 无生产引用
- **置信度**: A

### T1.4.3: 移除 LayoutConfig::Grid 默认和兼容逻辑
- **依据**: design.md §1.4 + requirements.md R1.1
- **改动**: 移除 `LayoutConfig::Grid`, 调整 default layout 和配置解析测试, 保留 single/stack 或将 layout 标记废弃。
- **锚点**:
  - 改前: `src/cli/config.rs:45` enum 包含 Grid; `src/cli/config.rs:164` 默认 Grid
  - 改后: `src/cli/config.rs:45` 无 Grid 物理布局语义
- **依赖**: T1.4.2
- **估时**: 1h
- **测试**:
  - 单元: `src/cli/config.rs` tests
  - 集成: `tests/r1_start_no_layout.rs`
  - E2E: 旧 `layout = "grid"` 给出明确迁移提示或兼容 no-op
- **验收**: `cargo test cli::config` pass
- **置信度**: A

### T1.4.4: 删除 grid layout 测试文件
- **依据**: design.md §1.4 + requirements.md R1.1
- **改动**: 删除 `tests/mvp12_grid_layout.rs`, 并移除 `mvp9_acceptance.rs` 中依赖 shared session target 的断言。
- **锚点**:
  - 改前: `tests/mvp12_grid_layout.rs:1` 覆盖 split/grid
  - 改后: `tests/r1_session_naming.rs:1` 覆盖独立 session
- **依赖**: T1.4.2, T1.4.3
- **估时**: 1h
- **测试**:
  - 单元: 不适用
  - 集成: `tests/r1_session_naming.rs`
  - E2E: 不适用
- **验收**: `cargo test --test r1_session_naming` pass, grid 文件不再编译
- **置信度**: A

### T3.1.1: Session struct 增加 absolute_path
- **依据**: design.md §3.1 + requirements.md R3.1
- **改动**: 为 `src/db/schema.rs` 的 `Session` 添加 `absolute_path: String`, 与 `db/sessions.rs` 查询结构对齐。
- **锚点**:
  - 改前: `src/db/schema.rs:86` `Session` 缺少 `absolute_path`
  - 改后: `src/db/schema.rs:86` `Session` 带 `absolute_path`
- **依赖**: 无；可并行 with T1.1.1
- **估时**: <30min
- **测试**:
  - 单元: `src/db/sessions.rs` tests
  - 集成: `tests/r3_absolute_path.rs`
  - E2E: 不适用
- **验收**: session 查询返回 absolute_path 字段
- **置信度**: A

### T3.1.2: query_session_by_id_sync JOIN projects
- **依据**: design.md §3.1 + requirements.md R3.1
- **改动**: 将 `query_session_by_id_sync` 改为 JOIN `projects`, SELECT `projects.absolute_path` 并填入 `Session`。
- **锚点**:
  - 改前: `src/db/sessions.rs:78` 查询 session 未补齐 project absolute_path
  - 改后: `src/db/sessions.rs:78` JOIN `projects ON sessions.project_id = projects.id`
- **依赖**: T3.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/db/sessions.rs` query tests
  - 集成: `tests/r3_absolute_path.rs`
  - E2E: 不适用
- **验收**: `query_session_by_id_sync` 对 session id 返回真实 project root
- **置信度**: A

### T3.1.3: master spawn 使用 session.absolute_path
- **依据**: design.md §3.1 + requirements.md R3.1
- **改动**: `handle_session_spawn_master_pane` 将 `master_cwd` 从 `project_id` 改为 `session.absolute_path`, session 名改 `master_<project_id>`。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:146` `session.project_id.clone().into()`
  - 改后: `src/rpc/handlers.rs:146` `PathBuf::from(session.absolute_path)`
- **依赖**: T1.1.1, T3.1.2
- **估时**: 1h
- **测试**:
  - 单元: `src/rpc/handlers.rs` master spawn tests
  - 集成: `tests/r3_master_cwd.rs`
  - E2E: master 执行 `pwd` 输出 project root
- **验收**: master pane CWD 不是 `~` 或 state_dir, 而是 absolute_path
- **置信度**: A

### T3.1.4: agent spawn tmux cwd 使用 session.absolute_path
- **依据**: design.md §3.1 + requirements.md R3.2
- **改动**: `handle_agent_spawn` 保留 `session_dir` 作为 sandbox/fifo 资源路径, 但 tmux `ensure_session`/`spawn_window` 的 `-c` 统一用 `session.absolute_path`。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:282` `session_dir` 同时作 sandbox 和 tmux cwd
  - 改后: `src/rpc/handlers.rs:330` `ensure_session(agent_<id>, absolute_path)`; `src/rpc/handlers.rs:355` spawn cwd 为 absolute_path
- **依赖**: T1.1.1, T3.1.2
- **估时**: 2h
- **测试**:
  - 单元: `src/rpc/handlers.rs` agent spawn tests
  - 集成: `tests/r3_agent_cwd.rs`
  - E2E: agent 执行 `pwd` 输出 project root 或 sandbox `/workspace`
- **验收**: 裸模式 tmux pane CWD 为 absolute_path, sandbox 模式 bwrap 后为 `/workspace`
- **置信度**: A

### T3.2.1: bwrap build_args 接收 project_root 并绑定 /workspace
- **依据**: design.md §3.2 + requirements.md R3.3
- **改动**: 调整 `bwrap::build_args` 签名或调用链, 区分 `sandbox_dir` 与 `project_root`, 将真实 project root bind 到 `/workspace`。
- **锚点**:
  - 改前: `src/sandbox/bwrap.rs:74` `--bind <sandbox_dir> /workspace`
  - 改后: `src/sandbox/bwrap.rs:74` `--bind <project_root> /workspace`
- **依赖**: T3.1.2
- **估时**: 2h
- **测试**:
  - 单元: `src/sandbox/bwrap.rs` build_args tests
  - 集成: `tests/r3_bwrap_workspace.rs`
  - E2E: sandbox agent 可读 project root 文件
- **验收**: build args 中 `/workspace` 左侧为 absolute project root
- **置信度**: A

### T3.2.2: bwrap 增加 --chdir /workspace
- **依据**: design.md §3.2 + requirements.md R3.3
- **改动**: 在 bwrap 参数流中加入 `--chdir /workspace`, 确保进程进入 sandbox 后 CWD 固定。
- **锚点**:
  - 改前: `src/sandbox/bwrap.rs:26` build_args 全文无 `--chdir`
  - 改后: `src/sandbox/bwrap.rs:74` bind 后追加 `--chdir /workspace`
- **依赖**: T3.2.1
- **估时**: <30min
- **测试**:
  - 单元: `src/sandbox/bwrap.rs` build_args tests
  - 集成: `tests/r3_bwrap_workspace.rs`
  - E2E: sandbox agent `pwd` 为 `/workspace`
- **验收**: `build_args` 包含连续参数 `--chdir`, `/workspace`
- **置信度**: A

### T3.2.3: .git 默认只读绑定
- **依据**: design.md §3.2 + requirements.md R3.3
- **改动**: 当 project root 下存在 `.git`, 默认对其生成 `--ro-bind <project_root>/.git /workspace/.git`, 避免 agent 修改 git 元数据。
- **锚点**:
  - 改前: `src/sandbox/bwrap.rs:11` `RoBind` 仅处理显式 overrides/manifest
  - 改后: `src/sandbox/bwrap.rs:74` workspace bind 后为 `.git` 添加 ro-bind 规则
- **依赖**: T3.2.1
- **估时**: 1h
- **测试**:
  - 单元: `src/sandbox/bwrap.rs` build_args `.git` tests
  - 集成: `tests/r3_bwrap_workspace.rs`
  - E2E: sandbox 内 `.git` 不可写
- **验收**: `.git` 存在时 args 含 `--ro-bind`
- **置信度**: A

### T3.2.4: 配置 additional_ro_binds
- **依据**: design.md §3.2 + requirements.md R3.3
- **改动**: 在 `ccb.toml` 配置结构增加 `[sandbox] additional_ro_binds = []`, 并映射为 bwrap 安全只读挂载。
- **锚点**:
  - 改前: `src/cli/config.rs:8` 无 sandbox ro bind 配置
  - 改后: `src/cli/config.rs:8` 增加 sandbox config 并传入 `SandboxOverrides`
- **依赖**: T3.2.1
- **估时**: 2h
- **测试**:
  - 单元: `src/cli/config.rs`, `src/sandbox/bwrap.rs`
  - 集成: `tests/r3_additional_ro_binds.rs`
  - E2E: 自定义 ro bind 在 sandbox 内可读不可写
- **验收**: TOML 数组可解析且 forbidden path 仍被拒绝
- **置信度**: A

### T3.3.1: home_layout 参数重命名为 sandbox_dir
- **依据**: design.md §3.3 + requirements.md R3.3
- **改动**: 将 `prepare_home_layout(provider_name, project_root)` 的第二形参重命名为 `sandbox_dir`, 不改变物化行为。
- **锚点**:
  - 改前: `src/provider/home_layout.rs:33` 参数名 `project_root`
  - 改后: `src/provider/home_layout.rs:33` 参数名 `sandbox_dir`
- **依赖**: 无；可并行 with T3.2.1
- **估时**: <30min
- **测试**:
  - 单元: `tests/mvp12_home_layout.rs`
  - 集成: 不适用
  - E2E: 不适用
- **验收**: `cargo test --test mvp12_home_layout` pass
- **置信度**: A

### T1.5.1: 联调 master/agent 独立 session + absolute_path
- **依据**: design.md §5.1 + requirements.md R1.1, R3.1, R3.2
- **改动**: 增加跨 R1/R3 集成测试, 验证 master 用 `master_<project_id>`, agent 用 `agent_<agent_id>`, 两者 cwd 均来自 project absolute_path。
- **锚点**:
  - 改前: `tests/mvp9_acceptance.rs:502` 断言 `ccbd-agents:<session_id>`
  - 改后: `tests/r1_r3_isolation_cwd.rs:1` 断言独立 session 与 CWD
- **依赖**: T3.1.3, T3.1.4, T1.4.1
- **估时**: 2h
- **测试**:
  - 单元: 不适用
  - 集成: `tests/r1_r3_isolation_cwd.rs`
  - E2E: 真 tmux start smoke
- **验收**: `cargo test --test r1_r3_isolation_cwd` pass
- **置信度**: A

## Phase 2: State Machine (R2)

### T2.1.1: 增加 WAITING_FOR_ACK 状态常量/判断
- **依据**: design.md §2.1 + requirements.md R2.1
- **改动**: 统一状态字符串定义或 helper, 增加 `WAITING_FOR_ACK`, 避免散落硬编码。
- **锚点**:
  - 改前: `src/db/schema.rs:21` state 为 TEXT 且代码无 ACK 状态
  - 改后: `src/db/state_machine.rs:29` 附近统一识别 `WAITING_FOR_ACK`
- **依赖**: Phase 1 完成
- **估时**: 1h
- **测试**:
  - 单元: `src/db/state_machine.rs` tests
  - 集成: `tests/r2_waiting_for_ack.rs`
  - E2E: 不适用
- **验收**: 新状态可写入 DB 且 helper 判断通过
- **置信度**: A

### T2.1.2: agent.send IDLE -> WAITING_FOR_ACK 使用 CAS
- **依据**: design.md §2.1 + requirements.md R2.1
- **改动**: 在 `handle_agent_send` 发送前执行 atomic CAS, 仅允许 `IDLE` 进入 `WAITING_FOR_ACK`, 失败立即返回 BUSY。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:737` capture baseline 后继续发送; `src/rpc/handlers.rs:786` 直接 reply BUSY
  - 改后: `src/rpc/handlers.rs:737` send 前 CAS 至 `WAITING_FOR_ACK`
- **依赖**: T2.1.1
- **估时**: 2h
- **测试**:
  - 单元: `src/rpc/handlers.rs` state transition tests
  - 集成: `tests/r2_waiting_for_ack.rs`
  - E2E: 并发 agent.send 只有一个进入 send_text
- **验收**: 第二个并发 send 返回 BUSY 且未写 tmux pane
- **置信度**: A

### T2.2.1: spawn_new_capture_seed 改 50ms + meaningful diff
- **依据**: design.md §2.2 + requirements.md R2.2
- **改动**: 将 capture 轮询从 100ms 改为 50ms, 退出条件改为 `is_meaningful_diff` 或 `stability_ms` 窗口满足。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:1010` 每 100ms 判断非 baseline 前缀
  - 改后: `src/rpc/handlers.rs:1010` 每 50ms 复用 `src/pane_diff/mod.rs:151` meaningful diff
- **依赖**: T2.1.2
- **估时**: 2h
- **测试**:
  - 单元: `src/rpc/handlers.rs` capture seed tests
  - 集成: `tests/r2_ack_visual_diff.rs`
  - E2E: 旧 marker 残留不会秒回 IDLE
- **验收**: `cargo test --test r2_ack_visual_diff` pass
- **置信度**: A

### T2.2.2: ACK 完成后 WAITING_FOR_ACK -> BUSY
- **依据**: design.md §2.2 + requirements.md R2.3
- **改动**: capture seed 任务确认视觉变化后直接 `update_agent_state(..., "BUSY")`, 再允许 MarkerMatcher 重新结束任务。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:1010` 新捕获内容能匹配 marker 时 mark IDLE
  - 改后: `src/rpc/handlers.rs:1010` ACK 成功先 mark BUSY, completion 仍走 marker 路径
- **依赖**: T2.2.1
- **估时**: 1h
- **测试**:
  - 单元: `src/rpc/handlers.rs` ack completion tests
  - 集成: `tests/r2_waiting_for_ack.rs`
  - E2E: `ccb ps` 可观察 WAITING_FOR_ACK 后进入 BUSY
- **验收**: ACK 后状态不是 IDLE, 而是 BUSY
- **置信度**: A

### T2.2.3: ACK 失败映射 STUCK/CRASHED
- **依据**: design.md §2.1 + requirements.md R2.2
- **改动**: `TmuxCommandFailed` 或 pane/pid 死亡时, 从 `WAITING_FOR_ACK` 转 `STUCK` 或 `CRASHED`, 并写 evidence。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:1010` capture 失败只结束后台轮询
  - 改后: `src/rpc/handlers.rs:1010` 捕获失败触发状态回退
- **依赖**: T2.2.1
- **估时**: 1h
- **测试**:
  - 单元: `src/rpc/handlers.rs` failure tests
  - 集成: `tests/r2_ack_failure.rs`
  - E2E: kill pane during ACK 后状态 CRASHED/STUCK
- **验收**: ACK 失败不会永久停在 WAITING_FOR_ACK
- **置信度**: A

### T2.3.1: L3 assert 允许覆盖 WAITING_FOR_ACK
- **依据**: design.md §2.3 + requirements.md R2.3
- **改动**: 调整 `state_machine_assert` 的 guard, L3 evidence 可从 `WAITING_FOR_ACK` 直接 assert 到 IDLE。
- **锚点**:
  - 改前: `src/db/state_machine_assert.rs:1` assert 路径未识别 ACK 态
  - 改后: `src/db/state_machine_assert.rs:1` ACK 态接受高权威 evidence 覆盖
- **依赖**: T2.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/db/state_machine_assert.rs` tests
  - 集成: `tests/mvp4_acceptance.rs`
  - E2E: 不适用
- **验收**: `WAITING_FOR_ACK -> IDLE/Asserted` 成功且 state_version 增加
- **置信度**: A

### T2.3.2: jobs status 适配 ACK 但不改 schema
- **依据**: design.md §2.3 + requirements.md R2.1
- **改动**: 保持 `status TEXT`, 只调整 Rust 状态处理与 job_update 通知对 ACK 态的显示/过滤。
- **锚点**:
  - 改前: `src/db/jobs.rs:196` 只 claim `IDLE`/`UNKNOWN`
  - 改后: `src/db/jobs.rs:196` claim 仍排除 `WAITING_FOR_ACK`, 其他状态显示不 panic
- **依赖**: T2.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/db/jobs.rs` tests
  - 集成: `tests/r2_jobs_ack_guard.rs`
  - E2E: orchestrator 不抢跑 ACK agent
- **验收**: ACK agent 不被 claim, schema 无 migration
- **置信度**: A

### T2.4.1: marker match guard 支持 ACK
- **依据**: design.md §2.4 + requirements.md R2.3
- **改动**: `mark_agent_idle_matched_sync` 的允许状态加入 `WAITING_FOR_ACK`, 使 LLM 快速响应可被合法完成。
- **锚点**:
  - 改前: `src/db/state_machine.rs:56` `IN ('SPAWNING', 'BUSY')`
  - 改后: `src/db/state_machine.rs:56` `IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY')`
- **依赖**: T2.1.1
- **估时**: <30min
- **测试**:
  - 单元: `src/db/state_machine.rs` tests
  - 集成: `tests/r2_state_guard_audit.rs`
  - E2E: 不适用
- **验收**: ACK 态 marker match 可转 IDLE
- **置信度**: A

### T2.4.2: stuck timeout guard 支持 ACK
- **依据**: design.md §2.4 + requirements.md R2.2
- **改动**: `mark_agent_stuck_sync` 从只允许 BUSY 改为 BUSY 或 WAITING_FOR_ACK。
- **锚点**:
  - 改前: `src/db/state_machine.rs:128` `state = 'BUSY'`
  - 改后: `src/db/state_machine.rs:128` `state IN ('BUSY', 'WAITING_FOR_ACK')`
- **依赖**: T2.1.1
- **估时**: <30min
- **测试**:
  - 单元: `src/db/state_machine.rs` tests
  - 集成: `tests/r2_state_guard_audit.rs`
  - E2E: 不适用
- **验收**: ACK 超时可转 STUCK
- **置信度**: A

### T2.4.3: unknown timeout guard 支持 ACK
- **依据**: design.md §2.4 + requirements.md R2.2
- **改动**: `mark_agent_unknown_sync` 允许 `WAITING_FOR_ACK` 超时转 UNKNOWN。
- **锚点**:
  - 改前: `src/db/state_machine.rs:194` `IN ('SPAWNING', 'BUSY')`
  - 改后: `src/db/state_machine.rs:194` `IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY')`
- **依赖**: T2.1.1
- **估时**: <30min
- **测试**:
  - 单元: `src/db/state_machine.rs` tests
  - 集成: `tests/r2_state_guard_audit.rs`
  - E2E: 不适用
- **验收**: ACK 超时可转 UNKNOWN
- **置信度**: A

### T2.4.4: recovery scan 包含 ACK
- **依据**: design.md §2.4 + requirements.md R2.2
- **改动**: startup reconcile Phase A/B/C 的 candidate SQL 将 `WAITING_FOR_ACK` 纳入恢复扫描和 crash recovery。
- **锚点**:
  - 改前: `src/db/system.rs:413` 和 `src/db/system.rs:549` `IN ('SPAWNING', 'BUSY', 'IDLE')`
  - 改后: `src/db/system.rs:413` 和 `src/db/system.rs:549` 加入 `WAITING_FOR_ACK`
- **依赖**: T2.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/db/system.rs` startup reconcile tests
  - 集成: `tests/mvp8_acceptance.rs`
  - E2E: daemon restart 后 ACK agent 被恢复/标死
- **验收**: startup reconcile 不漏 ACK agent
- **置信度**: A

### T2.4.5: agent.send reply 返回转换结果
- **依据**: design.md §2.4 + requirements.md R2.1
- **改动**: `handle_agent_send` reply 不再固定 BUSY, 根据 CAS/ACK 转换结果返回 `WAITING_FOR_ACK` 或拒绝时 `BUSY`。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:786` reply 固定 `"BUSY"`
  - 改后: `src/rpc/handlers.rs:786` reply 使用实际 state
- **依赖**: T2.1.2
- **估时**: <30min
- **测试**:
  - 单元: `src/rpc/handlers.rs` tests
  - 集成: `tests/r2_waiting_for_ack.rs`
  - E2E: `ccb send` 输出 ACK 状态
- **验收**: 首个 send 返回 `WAITING_FOR_ACK`, 并发 send 返回 `BUSY`
- **置信度**: A

### T2.5.1: ACK 态 RPC 互斥复用 IDLE guard
- **依据**: design.md §2.1, §2.4 + requirements.md R2.1
- **改动**: 将 `handle_agent_send` 的入口状态检查与 `handle_agent_assert_state` 的 `state != "IDLE"` guard 语义对齐, ACK 态拒绝第二条 send。
- **锚点**:
  - 改前: `src/rpc/handlers.rs:868` L3 assert 前检查 `state != "IDLE"`; send 路径未复用同等互斥
  - 改后: `src/rpc/handlers.rs:737` send 入口复用 IDLE-only 互斥
- **依赖**: T2.1.2, T2.4.5
- **估时**: 1h
- **测试**:
  - 单元: `src/rpc/handlers.rs` concurrent send tests
  - 集成: `tests/r2_send_mutex.rs`
  - E2E: 两个并发 `agent.send` 只有一个进入 tmux writer
- **验收**: WAITING_FOR_ACK 下 `agent.send` 返回 BUSY, 未调用 `send_text_to_pane`
- **置信度**: A

## Phase 3: Config (R4)

### T4.1.1: 更新推荐 ccb.toml master cmd
- **依据**: design.md §4.1 + requirements.md R4.1
- **改动**: 将项目模板/示例中的 `[master] cmd` 更新为 `claude --dangerously-skip-permissions --continue /remote-control`, 保留 `enabled = true`。
- **锚点**:
  - 改前: `ccb.toml:8` `cmd = "claude"`
  - 改后: `ccb.toml:8` `cmd = "claude --dangerously-skip-permissions --continue /remote-control"`
- **依赖**: Phase 2 完成；可并行 with T4.2.1
- **估时**: <30min
- **测试**:
  - 单元: `src/cli/config.rs` custom master cmd tests
  - 集成: `tests/r4_master_cmd.rs`
  - E2E: 真 Claude CLI 可按配置启动
- **验收**: config loader 读取完整字符串且不截断参数
- **置信度**: A

### T4.1.2: 验证 sh -lc 复杂 argv 透传
- **依据**: design.md §4.1 + requirements.md R4.1
- **改动**: 为 `systemd::master_command` 增加带引号/多参数命令测试, 明确现有 `sh -lc` 是设计约束。
- **锚点**:
  - 改前: `src/sandbox/systemd.rs:42` `master_command` 已原样封装 cmd, 测试只覆盖简单 `claude`
  - 改后: `src/sandbox/systemd.rs:242` tests 覆盖完整 cmd 字符串
- **依赖**: 无；可并行 with T4.1.1
- **估时**: <30min
- **测试**:
  - 单元: `src/sandbox/systemd.rs` tests
  - 集成: `tests/r4_master_cmd.rs`
  - E2E: 不适用
- **验收**: `sh -lc <full cmd>` 在 args 中保持单个 shell command 字符串
- **置信度**: A

### T4.2.1: attach 默认目标改为 agent_<agent_id>
- **依据**: design.md §4.2 + requirements.md R1.1
- **改动**: `ccb-rust attach <agent_id>` 映射到 `tmux attach -t agent_<agent_id>`, start 后提示不再指向 shared session。
- **锚点**:
  - 改前: `src/bin/ccb-rust.rs:282` attach 使用 `SESSION_NAME`
  - 改后: `src/bin/ccb-rust.rs:282` attach 使用 `agent_session_name(agent_id)`
- **依赖**: T1.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/bin/ccb-rust.rs:542` attach command tests
  - 集成: `tests/r4_attach_mapping.rs`
  - E2E: `ccb-rust attach a1` 进入 `agent_a1`
- **验收**: prepared tmux args 为 `attach -t agent_<id>`
- **置信度**: A

### T4.2.2: paste-buffer 风险登记为 deferred
- **依据**: design.md §4.2 + requirements.md R4.1
- **改动**: 在 spec/docs 中记录本次不改 `agent_io/writer.rs`, 将 slash paste-buffer 截断风险转入后续专向 spec。
- **锚点**:
  - 改前: `src/agent_io/writer.rs:1` send 路径仍为 paste-buffer/enter
  - 改后: `.kiro/specs/core-fixes/tasks.md:1` 明确 no-code deferred 项, 源码不变
- **依赖**: 无；可并行 with T4.1.1
- **估时**: <30min
- **测试**:
  - 单元: 不适用
  - 集成: 不适用
  - E2E: 后续 spec 覆盖
- **验收**: 本 spec 实施 PR 不包含 `agent_io/writer.rs` 行为改动
- **置信度**: A

### T4.3.1: master.cmd 为空时自动填默认 claude
- **依据**: design.md §4.3 + requirements.md R4.1
- **改动**: Daemon/CLI 启动时检测旧配置, 若 `[master] cmd` 为空字符串则用默认 `claude` 并给出迁移提示。
- **锚点**:
  - 改前: `src/cli/config.rs:20` `MasterConfig.cmd` 允许空字符串透传
  - 改后: `src/cli/config.rs:168` 默认与空值归一化为 `claude`
- **依赖**: T4.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/cli/config.rs` parse empty cmd tests
  - 集成: `tests/r4_master_cmd.rs`
  - E2E: 空 cmd 旧配置可启动 master
- **验收**: 空 `master.cmd` 不会生成空 `sh -lc ""`
- **置信度**: A

### T4.3.2: doctor 提示清理旧 ccbd-agents 孤儿 session
- **依据**: design.md §4.3 + requirements.md R1.2
- **改动**: `doctor` 检测旧共享 session `ccbd-agents`, 提示用户手动 `tmux -L <socket> kill-session -t ccbd-agents`。
- **锚点**:
  - 改前: `src/cli/doctor.rs:1` 无 core-fixes 旧 shared session 提示
  - 改后: `src/cli/doctor.rs:1` 增加 orphan shared session check
- **依赖**: T1.1.1
- **估时**: 1h
- **测试**:
  - 单元: `src/cli/doctor.rs` tests
  - 集成: `tests/r4_doctor_migration.rs`
  - E2E: 存在 `ccbd-agents` 时 doctor 输出清理建议
- **验收**: doctor 只提示, 不自动 kill 旧 session
- **置信度**: A

## 测试策略
- 单元测试: 跟随 src/ 实施, 默认 a1 自己写；DB 状态和命令 args 优先单元覆盖。
- 集成测试: 新增 `tests/r1_*`, `tests/r2_*`, `tests/r3_*`, `tests/r4_*`, 并修正受 shared session/grid 影响的旧测试。
- E2E 测试: 真 LLM CLI 跑, 覆盖 1-Session-per-CLI 启动 + ACK 完整链路 + master cmd 透传, a3 (Claude) 主笔, a1 fallback。

## 风险与回滚
- 每 Phase 实施前 commit 现状作回滚锚点。
- Phase 1 失败回滚: revert R1/R3 commits, 临时保留 master pane 模式和旧 `ccbd-agents` 管理。
- Phase 2 失败回滚: schema TEXT 不变, 回退 ACK 状态代码即可。
- Phase 3 失败回滚: ccb.toml 模板回退到无 master.cmd 参数, attach 暂回 shared session 提示。
