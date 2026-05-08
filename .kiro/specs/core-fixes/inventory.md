# Inventory: ccbd-rust core-fixes 现状代码地图

本文件由 a3 (Claude) 在 2026-05-08 撰写,纯**事实清单 + file:line 锚点**,不含方案建议。供 a2 (Gemini) 写 `research.md` / `design.md` 时直接 jump-to-source。

不写"应该怎么改",只写"当前是什么、在哪、什么 commit 加的"。

---

## 0. 顶层架构事实

| 事实 | 位置 |
|---|---|
| ccbd 二进制入口 | `src/bin/ccbd.rs:13` `main()` |
| ccb-rust CLI 二进制入口 | `src/bin/ccb-rust.rs:101` `main()` |
| RPC handler 总表 | `src/rpc/router.rs` (350 行) + `src/rpc/handlers.rs` (2058 行) |
| state_dir 解析 (XDG-only,不和 project_root 关联) | `src/env.rs:3-18` `resolve_state_dir()` |
| 共享 tmux session 名称 (硬编码常量) | `src/tmux/mod.rs:15` `SESSION_NAME = "ccbd-agents"` |
| 项目配置文件结构 | `src/cli/config.rs:8-48` `ProjectConfig / MasterConfig / AgentConfig / LayoutConfig` |
| 当前 agent state 集合 (无 WAITING_FOR_ACK) | 用例: `src/db/state_machine.rs:29` `"SPAWNING" \| "BUSY"`; `src/db/system.rs:413` `('SPAWNING', 'BUSY', 'IDLE')`; 转 STUCK/UNKNOWN/CRASHED/KILLED 散在各处 |

---

## 1. R1: 进程生命周期追踪 + 物理隔离 (Bug A & F)

### 1.1 当前 tmux 调用模型 (单 Session 多 Window/Pane)

* **Server 创建 / Session 复用**: `src/tmux/session.rs:55-88` `ensure_session_sync` — `tmux new-session -d -s <name> -c <cwd> -x 200 -y 60`,只在 session 不存在时创建。`-x 200 -y 60` 是建立后的初始尺寸,**没有跟 `set-option window-size manual` 配合**,client attach 后会被 reflow。
* **Server scope 包装**: `src/tmux/session.rs:84` `scope::wrap_in_scope("tmux", ..., &self.scope_policy)` → `src/tmux/scope.rs:20-43` `wrap_in_scope` 用 `systemd-run --user --scope --collect --unit=ccbd-tmux-<8hex> --slice=ccbd-agents.slice [--property=BindsTo=ccbd-rust.service]`。`BindsTo` 仅在 `detect_self_in_service()` 命中 `/proc/self/cgroup` 含 `ccbd-rust.service` 时附加 (`src/tmux/scope.rs:50-60` `detect_scope_policy`,`src/tmux/scope.rs:71-75`)。
* **新 Window 创建**: `src/tmux/session.rs:90-126` `spawn_window_sync` — `tmux new-window -d -t <session>: -n <window> -c <cwd> -P -F #{pane_id} -- <cmd>`。同时尝试复用初始 window (`reusable_initial_window_sync` 检 `0 / bash / sh / zsh / fish` 单 window 单 pane 状态)。
* **现有 Window 内 Split**: `src/tmux/session.rs:247-273` `split_window_sync` / `split_window_with_spec_sync` (调 `build_split_window_args` `src/tmux/session.rs:729-768`),`tmux split-window -d [-h|-v] [-p N] -t <target_or_parent_pane> -c <cwd> -P -F #{pane_id} -- <cmd>`。
* **kill 路径 (按粒度递增)**:
  * `kill_pane` `src/tmux/session.rs:356-363`
  * `kill_window` `src/tmux/session.rs:374-382`
  * `kill_session_window` `src/tmux/session.rs:384-386` (调 `kill_window(SESSION_NAME, session_id)` — 注意它只杀 Window,不是 tmux Session)
  * 没有 `kill_session_sync` 抽象;唯一直接 `tmux kill-session -t <SESSION_NAME>` 在 `src/bin/ccbd.rs:122-124` (daemon shutdown)
* **server-wide 杀**: `src/bin/ccbd.rs:128-132` `tmux -L <socket> kill-server` (daemon shutdown 路径)

### 1.2 哪些代码假设"single shared session"

| 调用点 | 用法 | file:line |
|---|---|---|
| `ensure_session(SESSION_NAME, …)` | master pane 创建前 | `src/rpc/handlers.rs:144` |
| `ensure_session(SESSION_NAME, session_dir)` | 每次 agent.spawn 入口 | `src/rpc/handlers.rs:330` |
| `window_exists(SESSION_NAME, …)` | agent.spawn 路由判断 | `src/rpc/handlers.rs:338` |
| `spawn_window(SESSION_NAME, window_name, …)` | 第一个 agent (无 layout hint) | `src/rpc/handlers.rs:355` |
| `kill_window(SESSION_NAME, agent_id)` / `(SESSION_NAME, project_id)` | session.kill force 路径 | `src/rpc/handlers.rs:99, 104` |
| `kill_session_window(session.id)` | session.kill force 路径 | `src/rpc/handlers.rs:107-109` |
| daemon shutdown | `src/bin/ccbd.rs:122-132` | 同上 |
| `format!("{}:{project_id}", SESSION_NAME)` | session_window_target 计算 | `src/rpc/handlers.rs:51-53` |

### 1.3 进程生命周期 (current 实装)

* **Daemon 主循环 + 信号处理**: `src/bin/ccbd.rs:76-116` `run_until_shutdown`,SIGTERM / SIGINT 触发 `cleanup_tmux_resources` (118-142)。**没有 master-pid 监控 + cascade kill 自身的逻辑** — daemon 只在自己被显式 kill 时清。
* **Agent pidfd 监控**: `src/monitor/agent_watch.rs:9-50` `spawn_agent_pidfd_watch_task`。pidfd readable → `mark_agent_crashed_with_exit` + 清 marker / agent_io / pidfd registry。
* **Master pidfd 监控** (mvp15 957dbf5): `src/monitor/master_watch.rs:7-53` `spawn_master_pidfd_watch_task`。master pane pid 退出 → `cascade_kill_session_agents` + `kill_pane` 每个 agent。
* **Master pidfd 注册点**: `src/rpc/handlers.rs:160-186` (`handle_session_spawn_master_pane`) — 取 pane_pid → `pidfd_open` → register + spawn task。
* **systemd-scope 绑定**: `src/sandbox/systemd.rs:8-40` `wrap_command` 给 agent 加 `--property=BindsTo=ccbd.service`(仅 `under_systemd=true` 时);`systemd.rs:42-61` `master_command` 给 master 加 `--slice=ccb-<project>-ccbd-workspace.slice` 但**没有** `BindsTo`。
* **systemd anchor 服务名**: `src/tmux/scope.rs:55` 用 `ccbd-rust.service`;`src/sandbox/systemd.rs:29` 用 `ccbd.service` — **两个名字不一致** (mvp11 G11.1 commit `f91d0c8` 切到 `ccbd.service`,但 tmux scope 还在用 `ccbd-rust.service`)。
* **Agent CRASHED 标记**: `src/db/agents_lifecycle.rs:50` 内调用 `cleanup_agent_runtime_resources`(内含 `kill_pane_sync`)。
* **Cascade kill on session.kill**: `src/rpc/handlers.rs:71-131` `handle_session_kill` (含 master_pane / agents / session_window 多步杀);`src/db/system.rs:126-178` `cascade_kill_session_agents_sync`。
* **Cleanup on agent cleanup**: `src/agent_io/registry.rs:50-86` `cleanup_agent_runtime_resources` — 杀 pane + rm fifo + rm sandbox_dir + 清 marker / parser / monitor。socket 名通过 `OnceLock<String>` 全局静态 (`src/agent_io/registry.rs:15-19` `set_tmux_socket_name`,在 `src/bin/ccbd.rs:42` 注册)。

### 1.4 已有的 mvp10/13 startup reconcile 抗孤儿

* **Phase A-D 实施**: `src/db/system.rs:380-647`
  * Phase A `startup_reconcile_phase_a_select_candidates` (403): 选 SPAWNING/BUSY/IDLE 的 agents
  * Phase B `startup_reconcile_phase_b_probe_pids` (444): `kill(pid, 0)` 探活
  * Phase B2 `startup_reconcile_phase_b2_prepare_alive_io` (475): 探 fifo 是否能 open
  * Phase C `startup_reconcile_phase_c_crash_dead` (533): 死 pid 标 CRASHED + fail dispatched jobs + cleanup runtime resources
  * Phase D `startup_reconcile_phase_d_reregister_alive` (577): 活 pid 重 attach (重建 pidfd / parser / matcher / reader / marker timer)
* **Orphan systemd-run scope cleanup** (mvp13 7f54533 + 48bdadc): `src/db/system.rs:262-305` `reconcile_orphan_scopes_sync` + `is_own_ccbd_scope` (369),只清 description 含 `@<daemon_marker>` 且不在 active sessions/agents 引用列表里的 scope。默认 dry-run (266 行检 `CCBD_RECONCILE_FORCE`)。
* **Stale tmux socket sweep**: `src/db/system.rs:696-753` `sweep_stale_tmux_sockets_sync` — 扫 `/tmp/tmux-<uid>/` 下 `ccbd-*` 前缀 socket,有 ls-sessions 响应保留,无响应删。
* **Doctor 检查 tmux 孤儿**: mvp10 `8c0d21d`、`70109736` (CI 工作流)、`scripts/cleanup_orphan_tmux.sh`。

### 1.5 R1 相关现存 commit

| commit | 文件 | 关联 |
|---|---|---|
| `188b6fc` mvp10 G10.0 | `src/tmux/scope.rs` | tmux server 包 systemd-run scope |
| `eb93d75` mvp10 G10.1 | `src/db/system.rs` | graceful shutdown + reconcile stale socket sweep |
| `09e75cf` mvp11 G11.0 | `src/db/system.rs:262-305` | systemd anchor + DB CAS cascade |
| `f91d0c8` mvp11 G11.1 | `src/sandbox/systemd.rs:29` | agent BindsTo 切到 `ccbd.service`(但 `src/tmux/scope.rs:55` 没切) |
| `7f54533` mvp13 | `src/db/system.rs:262` | startup reconcile orphan scope cleanup (Bug A 残留) |
| `48bdadc` mvp13 | `src/db/system.rs:266, 287` | reconcile cross-daemon isolation + dry-run |
| `65c6503` Bug-E fix | `src/agent_io/registry.rs:50-86` | cleanup 时杀 pane (前缺) |
| `957dbf5` mvp15 | `src/monitor/master_watch.rs`, `src/rpc/handlers.rs:160-186` | master pidfd 监控 + ccb-rust attach |
| `6739f6a` (R1 反向目标) | `src/rpc/handlers.rs:146`, `src/cli/start.rs:253` | master pane 用 project_id 作 cwd + 40% width;**R1 想反向** (架构层走 1-session-per-CLI 后,这种共享 grid 模型不再适用) |
| `f44d45a` | `src/rpc/handlers.rs:348-357` | layout hint 路由,split 优先于 spawn_window |

---

## 2. R2: 状态机防抖 + "双保险" (Bug D & E)

### 2.1 当前状态枚举 + 转换图

* **schema**: `src/db/schema.rs:21` `state TEXT NOT NULL` (无枚举约束),`state_version INTEGER` 用做 CAS。
* **观察到的 state 字符串集合** (grep 全仓):
  * 入口/正常流: `SPAWNING` `BUSY` `IDLE` `UNKNOWN` `STUCK` `KILLED` `CRASHED`
  * sub_state: `'Matched'`(IDLE 子态 — `src/db/state_machine.rs:56`)、`'Asserted'`(L3 assert — `src/rpc/handlers.rs:881`)
  * **没有** `WAITING_FOR_ACK` / `DEBOUNCE` / `ACKED` 等防抖中间态
* **转换实施位置**:
  * SPAWNING → IDLE/UNKNOWN: `src/provider/init_probe_task.rs:44-150` (probe 满 STEADY_COUNT=2 进 IDLE,deadline 进 UNKNOWN)
  * SPAWNING/BUSY → IDLE (marker 匹配): `src/db/state_machine.rs:12-89` `mark_agent_idle_matched_sync`
  * BUSY → STUCK (pane diff): `src/db/state_machine.rs:100-158` `mark_agent_stuck_sync`
  * SPAWNING/BUSY → UNKNOWN (marker timeout): `src/db/state_machine.rs:160-244` `mark_agent_unknown_sync`
  * IDLE → BUSY (job dispatch): `src/orchestrator/mod.rs:94` `update_agent_state(... "BUSY")`
  * IDLE → BUSY (handle_agent_send): `src/rpc/handlers.rs:786` 直接 reply "BUSY",但实际 state 转换在 reader 路径
  * 任意 → KILLED: `src/db/agents_lifecycle.rs` `mark_agent_killed_sync`
  * 任意 → CRASHED: `src/monitor/agent_watch.rs:34` `mark_agent_crashed_with_exit`

### 2.2 当前防抖机制 (零散非状态机式)

* **stability_ms 字段**: `src/provider/manifest.rs:18` `pub stability_ms: u64`。codex / gemini / claude 全为 `300` ms (manifest.rs:173, 190, 207),bash / unknown 为 `0` (146, 228)。
* **stability_ms 消费点**: `src/agent_io/reader.rs:52-94` 在 `MatchResult::Matched` 但 `pending_stability_match` 模式下,等 `stability` 时长内若 fifo 又 readable 就消重新匹配 (skip_scan_after_stability_noise=true,丢一次 scan);超时无新输入才 commit `mark_agent_idle_matched`。
* **"capture_baseline" 防抖** (handle_agent_send 路径): 
  * `src/rpc/handlers.rs:737` `let capture_baseline = ctx.tmux_server.capture_pane(...).await.ok();` — send 之前 snapshot
  * `src/rpc/handlers.rs:776-784` `spawn_new_capture_seed(...)` — 启动 5 秒后台任务
  * `src/rpc/handlers.rs:1010-1100` `spawn_new_capture_seed`: 每 100ms 一次 capture,直到出现"非 baseline 前缀"内容(屏幕换页)或 5s deadline。新捕获内容能匹配 marker 才 mark IDLE。
* **idle_anti_pattern**: `src/provider/manifest.rs:174, 191` codex `"(?m)^\s*[\u{2800}-\u{28FF}◦●○]\s+Working\s"`,gemini `"[\u{2800}-\u{28FF}]"`(braille spinner)。`src/marker/matcher.rs:73-77` 在 prompt regex 命中后再过 anti_regex,命中 anti 则 `NoMatch`。
* **prompt-only reply swallow**: `src/db/state_machine.rs:43, 91-98` `is_prompt_only_reply` — reply 长度 ≤ 4 且为已知 prompt 字符则不算完成,事务回滚。
* **"idle_scan_enabled" 阀门**: `src/agent_io/reader.rs:151` 由 init_probe_task `Arc<AtomicBool>` 控制,init probe 没 STEADY 之前 reader 不扫 marker (`src/provider/init_probe_task.rs:18, 28-39, 75` 设 `Ordering::SeqCst` true)。

### 2.3 PaneDiffWatcher 实装

* **模块**: `src/pane_diff/mod.rs` 全 342 行
* **核心结构**: `PaneDiffObservation`(28),`AgentDiffState { last_meaningful_text, last_meaningful_at }`(13),`PaneDiffTickResult` (33)
* **入口循环**: `src/pane_diff/mod.rs:73-85` `pane_diff_watcher_loop` — 默认 30s tick (`DEFAULT_WATCH_INTERVAL` 9 行)。
* **Tick 实施**: `src/pane_diff/mod.rs:87-126` `pane_diff_watcher_tick` — 查 BUSY agents → capture_pane → 喂给 `process_pane_diff_observations` → 超过 `DEFAULT_STUCK_THRESHOLD = 300s` (10 行) 没实质变化则 `mark_agent_stuck`。
* **去 spinner / Thinking / 时间戳**: `src/pane_diff/mod.rs:129-149` `sanitize_for_diff` (用 braille `\u{2800}-\u{28FF}` regex、ascii spinner regex、thinking regex、trailing time regex)。
* **"实质性变化"判定**: `src/pane_diff/mod.rs:151-167` `is_meaningful_diff` — sanitize 后字符长度 +8 或者前缀关系破裂。
* **启动**: `src/orchestrator/mod.rs:21-24` 在 orchestrator task 旁起一个 `pane_diff_watcher_loop`。

### 2.4 MarkerMatcher 实装

* **模块**: `src/marker/matcher.rs` (193 行)
* **构造**: `src/marker/matcher.rs:30-51` `new` / `from_manifest`
* **scan 主体**: `src/marker/matcher.rs:58-79` — 有两种模式
  * `IdleDetectionMode::LineEndRegex` (默认 / bash): 反向看末尾 5 行,失败再看末尾 20 行
  * `IdleDetectionMode::ObservedStability` (codex/gemini/claude): 全屏 regex
* **prompt regex / anti regex**: `src/marker/matcher.rs:88-95` 硬编码 codex `(?m)^›\s`、gemini `Type your message or @path/to/file`、claude `(?m)^❯\s*$`、其他 `[\$#>✦]\s*$`。

### 2.5 BUSY timeout / Marker timer

* **TIMEOUT 常量**: `src/marker/timer.rs:9-12` `STARTUP_TIMEOUT = 10s`,`BUSY_TIMEOUT = 10800s = 3h` (兜底 STUCK)。
* **timer 行为**: `src/marker/timer.rs:28-121` watch::Sender / oneshot::Sender 控制,Startup 超时 → mark UNKNOWN + 写 evidence;Busy 超时 → mark STUCK。

### 2.6 R2 相关现存 commit

| commit | 关联 |
|---|---|
| `3da9cef` "fix(B-2)" | reply text capture — 更大 vt100 屏 + fallback + less aggressive prompt swallow (即 `is_prompt_only_reply` 改写) |
| `e03a32b` mvp11 G11.2 | `StartupSequenceEngine` (init_probe_task) 实装,SPAWNING 阶段不扫 marker |
| `f91d0c8` mvp11 G11.1 | manifest 协议升级,加 `idle_anti_pattern` / `idle_detection_mode` / `stability_ms` |
| (没有 commit 引入 WAITING_FOR_ACK 状态) | — |

---

## 3. R3: CWD + 沙盒挂载校准 (Master CWD Bug + Bug C & G)

### 3.1 project_root 在系统中的流向 (从 CLI 到 tmux)

* **采集**: `src/cli/start.rs:35-46` `start_from_options` 接 `options.cwd: PathBuf`(由 `src/bin/ccb-rust.rs:157` `std::env::current_dir()?` 提供)。
* **传给 RPC**: `src/cli/start.rs:48-77` `start_project` 用 `project_root.file_name() → project_id`(只取 basename),发 `session.create { project_id, absolute_path }`。
* **存到 DB**: `src/rpc/handlers.rs:55-69` `handle_session_create` → `src/db/sessions.rs:21-33, 196-206` `create_session_sync` 落 `projects(id, absolute_path)` + `sessions(id, project_id, ...)`。
* **session.spawn_master_pane 派 cwd**:
  * `src/rpc/handlers.rs:142` `let tmux_cmd = systemd::master_command(&session.project_id, cmd, ...);`
  * `src/rpc/handlers.rs:146` **`let master_cwd: PathBuf = session.project_id.clone().into();`** ← project_id 是 basename,被当 PathBuf 用。tmux 见到相对路径会按 ccbd 自身 CWD 解析。
  * `src/rpc/handlers.rs:147-155` `spawn_window(SESSION_NAME, project_id, master_cwd, tmux_cmd)`
  * **会话 ensure 用 state_dir**: `src/rpc/handlers.rs:144` `ensure_session(SESSION_NAME, ctx.state_dir.clone())` — 第一次 ensure 时,session 本身的 cwd 是 `~/.local/state/ccbd`,不是 project_root。
* **agent.spawn 派 cwd**:
  * `src/rpc/handlers.rs:282-286` `let session_dir = sandbox_guard.path() OR ctx.state_dir.clone()` — sandbox dir(若启用 bwrap)或裸 state_dir,**绝不**等于 `session.absolute_path`。
  * `src/rpc/handlers.rs:330` `ensure_session(SESSION_NAME, session_dir.clone())` 第一次 agent ensure 时拿 sandbox/state_dir 当 cwd。
  * `src/rpc/handlers.rs:349-356` 三种 spawn 路径(split-with-spec / split / spawn_window) 都用 `session_dir` 作 `-c`。
* **session.absolute_path 的实际下游**: 全仓只有 `system_dump` (`src/db/system.rs:38-54`) 和 `session.list` (`src/rpc/handlers.rs:206-222`) 读出来回 RPC 给 CLI 显示;**没有**任何 spawn / cwd 路径用它。

### 3.2 state_dir 解析 (跟 project_root 无关)

* **resolve_state_dir**: `src/env.rs:3-18` 走 `directories::ProjectDirs::from("", "", "ccbd").state_dir()`(典型 `~/.local/state/ccbd/`),`CCB_ENV=dev` 时走 `target/dev_state`。**不读 project_root**。
* **使用方**: `src/bin/ccbd.rs:34` `dir = env::resolve_state_dir()` → `ctx.state_dir`,所有 sandbox / fifo / sqlite 都落在这。
* **TmuxServer 创建**: `src/bin/ccbd.rs:41` `Arc::new(TmuxServer::new(&dir))` — server 的 state_dir 也是 XDG 路径 (用于 `compute_socket_name`)。

### 3.3 bwrap workspace 挂载

* **build_args 实装**: `src/sandbox/bwrap.rs:26-98` `build_args(sandbox_dir, ...)` — sandbox_dir 是 `state_dir/sandboxes/<session_id>/<agent_id>` (`src/sandbox/path.rs:7-21` `resolve_sandbox_dir`)。
* **关键挂载点**:
  * `src/sandbox/bwrap.rs:74-78` `--bind <sandbox_dir> /workspace` — **`<sandbox_dir>` 不是 project_root**,是 ccbd 自己造的小目录
  * `src/sandbox/bwrap.rs:65-69` `--bind <home_overrides.home_root> /home/agent` — 这是 prepare_home_layout 的产物,跟 project_root 无关
  * `src/sandbox/bwrap.rs:73-75` `--setenv HOME /home/agent`
* **prepare_home_layout 的 project_root 参数**: `src/provider/home_layout.rs:33-36` 接 `project_root: &Path`,但 `src/sandbox/bwrap.rs:33` 实际传的是 `sandbox_dir`(就是 `prepare_home_layout(provider_name, sandbox_dir)`)。这是命名误导:project_root 这个参数名实际接收的是 sandbox_dir。
* **chdir / cwd 设置在 bwrap 命令里**: 全文搜不到 `--chdir /workspace`(grep `chdir` `src/sandbox/bwrap.rs` 无结果) — bwrap 启动后 cwd 沿用 spawn 时的 cwd(即 tmux pane 拿到的 `session_dir`)。

### 3.4 R3 相关现存 commit

| commit | 关联 |
|---|---|
| `6739f6a` | `src/rpc/handlers.rs:146` 把 master_cwd 从 `state_dir` 改成 `project_id` (basename)。**部分修复**了 master pane,但仍非 absolute_path |
| `fa6712b` mvp11 | bwrap auth mount path translation to sandbox HOME (`src/sandbox/bwrap.rs:177-220`) |
| `0254acd` mvp13 stage 0-4 | 主线 sandbox 实施 + 195→241 测试 |
| `4f5f31d` mvp13 | sandbox onboarding mirror + .codex/auth.json symlink → copy |
| `910ca5f` mvp13 | bwrap nested-sandbox HOME fallback |
| `75b4378` mvp13 | bwrap mount SSL CA bundle |
| `71995a5` B-1 | sandbox onboarding mirror for all providers |

* **没有 commit** 把 `session.absolute_path` 串到 `agent.spawn` 的 tmux cwd 或 bwrap `--bind /workspace` 上。

---

## 4. R4: ccb.toml master cmd 多参数 (Bug B)

### 4.1 当前解析

* **配置结构**: `src/cli/config.rs:20-26` `MasterConfig { cmd: String (#[serde(default = default_master_cmd)]), enabled: bool }`。
* **default 值**: `src/cli/config.rs:168-170` `fn default_master_cmd() -> String { "claude".into() }`。
* **当前项目自身配置**: `ccb.toml` 第 8 行 `[master] cmd = "claude"`。
* **传到 spawn**: `src/cli/start.rs:88` `cmd: config.master.cmd.clone()` → RPC `session.spawn_master_pane params.cmd`。
* **systemd 包装**: `src/sandbox/systemd.rs:42-61` `master_command(project_id, cmd, env_state)` 把 `cmd` 字符串原样塞进 `["systemd-run", "--user", "--scope", [...], "--", "sh", "-lc", cmd]`。`sh -lc <cmd>` 处理空格分词,**多参数 cmd 已经能跑通**(例如 `cmd = "claude --dangerously-skip-permissions --continue /remote-control"` 的字符会传给 sh)。
* **pane title 用 cmd 第一个 word**: `src/rpc/handlers.rs:156` `cmd.split_whitespace().next().unwrap_or(cmd)` — 命名 `master (claude)` 。

### 4.2 R4 相关现存 commit

* (无) — `master cmd` 字段以 `String` 形式实装,没有任何 commit 把它升级成 `Vec<String>` / `argv: ["claude", "--continue", ...]` 形态。

### 4.3 brief tmp-lifecycle-brief.md 现状对照

仓库根有 `tmp-lifecycle-brief.md`(2026-05-07 写,4848 字节)— 内容与 `requirements.md` 早期 R1+R4 版本基本对应,直接说"only fix two bugs"。但本次 spec 已扩展成 R1-R4 四条,该 brief 已过时,**未删除**。

---

## 5. 测试现状

### 5.1 acceptance / unit 测试矩阵

| 文件 | 主要覆盖 |
|---|---|
| `tests/mvp2_acceptance.rs` | session.create + agent.spawn + bwrap baseline (R3 sandbox 入口,但是 sandbox_dir 路径,不是 project_root) |
| `tests/mvp3_acceptance.rs` | agent_send / agent_read 基础流 |
| `tests/mvp4_acceptance.rs` | UNKNOWN 状态 + evidence + assert_state (R2 当前防抖之上的 L3 assert 路径) |
| `tests/mvp6_acceptance.rs` | agent_send + tmux scope policy |
| `tests/mvp7_acceptance.rs` | reader stability_ms (= R2 当前防抖核心) |
| `tests/mvp7_real_codex.rs` / `mvp7_real_gemini.rs` | 真 provider smoke,需要 OAuth 凭证 |
| `tests/mvp8_acceptance.rs` / `mvp8_real_codex.rs` | (后续验证) |
| `tests/mvp9_acceptance.rs` | start_project + master + grid + cancel |
| `tests/mvp9_real_codex_claude.rs` | 真 codex/claude 双机 |
| `tests/mvp10_acceptance.rs` | tmux server systemd-run scope wrap (R1 当前 lifecycle 部分) |
| `tests/mvp11_acceptance.rs` | systemd anchor service + cascade |
| `tests/mvp11_real_*.rs` | 真 codex/gemini/claude provider mvp11 验证 |
| `tests/mvp12_grid_layout.rs` | grid split 确定性 |
| `tests/mvp12_init_probe.rs` | init_probe 4 个 provider 检测 |
| `tests/mvp12_home_layout.rs` | provider home materialization |
| `tests/mvp12_r2_dispatcher_lifecycle.rs` | R-2 (dispatcher) idle_match → notify 路径 |

### 5.2 真 provider 测试 gate

* `tests/common/mod.rs:6-37` `hard_gate(binary)` — 需 OAuth 文件 (`~/.codex/auth.json` / `~/.gemini/oauth_creds.json`) + tmux + bwrap + systemd-run。`12584ef` commit 把"API key 检测" 改成 OAuth 文件检测。
* CCB 项目自带 `ccb.toml` 用 a1=codex / a2=codex / a3=gemini / a4=claude (所有 grid).

### 5.3 现存测试**没有覆盖**的区域

* **R1**: 没有 `kill master tmux pane → daemon 自杀 → ccbd-tmux scope 立即 systemd-stop` 这种端到端孤儿自检。`src/monitor/master_watch.rs` 的 unit test (97-137) 用一个 `sh -c sleep 0.2` 子进程模拟 master,确认 cascade 但没验真 tmux scope 终结。
* **R2**: 没有 "send 后立即被旧标志位假性 IDLE" 的回归测试 — 当前 stability_ms / spawn_new_capture_seed 路径没有 acceptance。`mvp7_acceptance.rs` 围绕 stability_ms 但只针对 noise scenario。
* **R3**: 没有验 master / agent **真实 cwd = project absolute path** 的测试。可以 grep 全仓 `pwd`/`getcwd` 在 tests/ 下无相关 assertion。
* **R4**: `cli/config.rs:230-271` 已有 master_config default + 自定义 cmd 解析的 unit test,但**没有**任何测试验证多参数 cmd "claude --continue /remote-control" 在 systemd-run 链中真能 splat 成 argv。

---

## 6. 上游 ccb-python bug 与本仓的对应关系

参考三份文档:
* `docs/upstream-ccb-bugs/gemini-dispatch-and-completion-bugs.md` (Bug X / Y / Z, fork 已修)
* `docs/upstream-ccb-bugs/installer-default-config-mismatch.md` (项目级默认配置硬编码不一致)
* `docs/upstream-ccb-bugs/tmux-scope-and-tmpdir-leak-bugs.md` (Bug A-F,scope 不绑定 ccbd 主进程 + mkdtemp leak + janitor 前缀错位 + 无 startup reconcile + agent restart loop)

### 6.1 与 ccbd-rust R1 重合

| 上游 bug | 本仓现状 |
|---|---|
| 上游 Bug A "scope-not-bound-to-ccbd" | ccbd-rust 用 `BindsTo=ccbd-rust.service`(`src/tmux/scope.rs:55`) 或 `BindsTo=ccbd.service`(`src/sandbox/systemd.rs:29`,**两个名字不一致**)。`detect_self_in_service` (`src/tmux/scope.rs:71-75`) 仅在 daemon 跑在 systemd unit 内才生效,日常 ssh 起 daemon 时**不挂 BindsTo**。 |
| 上游 Bug B "mkdtemp-leak-on-fork-failure" | 本仓有 `src/sandbox/path.rs:23-54` `SandboxDirGuard` (RAII Drop 自动 rmtree,2c7710c mvp13 修),覆盖 panic 路径。 |
| 上游 Bug C "janitor-prefix-mismatch" | ccbd-rust 没分布式 janitor,只有内置 `reconcile_orphan_scopes_sync` (`src/db/system.rs:262`),命名前缀通过 `is_own_ccbd_scope` 检 description `@<daemon_marker>` 而非 unit 名,不会出现两套独立命名。 |
| 上游 Bug D "no-startup-reconcile" | 已实施 `src/db/system.rs:380-647` (Phase A-D),mvp13 `7f54533` 加孤儿 scope 清理。 |
| 上游 Bug E "agent-restart-loop-without-cleanup" | `src/db/agents_lifecycle.rs:50, 113` 在标 CRASHED 时 `cleanup_agent_runtime_resources` (杀 pane / rm fifo / rm sandbox);commit `65c6503` Bug-E fix 直接对应。 |
| 上游 Bug F "flat-tmpdir-namespace" | ccbd-rust sandboxes 落在 `state_dir/sandboxes/<sess>/<agent>` (`src/sandbox/path.rs:15`),**不是** `/tmp` 顶层,自然避开。 |

### 6.2 与 R2 重合

| 上游 bug | 本仓现状 |
|---|---|
| 上游 Bug Y "completion-detector-misses-thinking-and-clear" | ccbd-rust 用 `idle_anti_pattern`(braille spinner regex,`src/provider/manifest.rs:174, 191`)抑制 Thinking 阶段被错认 IDLE;`spawn_new_capture_seed`(`src/rpc/handlers.rs:1010`) 用"屏幕换页"启发抓 `/clear` 后的新 prompt。但**没有** WAITING_FOR_ACK 显式状态机,完全靠 stability_ms 阀门。 |
| 上游 Bug X "slash-via-paste-not-recognized" | 本仓 send 路径使用 `tmux load-buffer + paste-buffer + send-keys Enter` (推断,`tmux/session.rs:450-525` 三个相关方法均存在)。**没有针对 slash 命令的 keystroke fallback** — 跟上游一样用 paste-buffer。 |
| 上游 Bug Z "autonew-cannot-find-gemini-session" | 本仓没 autonew 工具(autonew 是上游 python ccb 的产物),N/A。 |

### 6.3 与 R3 / R4 重合

| 上游 bug | 本仓现状 |
|---|---|
| installer-default-config-mismatch | ccbd-rust 用 `ccb.toml`(TOML)而非上游 ccb.config 裸文本 + hardcoded default;`src/cli/config.rs:163-174` defaults (`grid` / `claude` / enabled=true);**新建项目时不写文件** (`find_config_with_env` `src/cli/config.rs:125-162` 找不到就报 "could not find ccb.toml")。**没有自动模板生成**,所以上游的硬编码错位问题在 ccbd-rust 不存在。 |
| Bug B (上游不同的 B,即 master cmd 多参数) | 上游 ccb 用 `cmd, a1:codex, a2:gemini, a3:claude` 这种自定义 csv 格式,本仓走 TOML + `[master] cmd = "..."` 字符串,通过 sh -lc 解析。 |

---

## 7. 备查:关键 enum / trait / 公共 API 索引

| 名称 | 位置 |
|---|---|
| `IdleDetectionMode` | `src/provider/manifest.rs:22-26` |
| `InitProbeKind` | `src/provider/manifest.rs:28-48` |
| `InitGateProbe` trait | `src/provider/init_probe.rs:9` |
| `SplitDirection` / `SplitSpec` | `src/tmux/session.rs:10-21` |
| `LayoutKind` | `src/tmux/layout.rs:4-39` |
| `ScopePolicy` / `UnitConfig` | `src/tmux/scope.rs:5-18` |
| `SandboxOverrides` / `RoBind` | `src/sandbox/bwrap.rs:11-23` |
| `EnvState` | `src/sandbox/mod.rs` (检 `under_systemd / unsafe_no_sandbox / bwrap_available / systemd_run_available`) |
| `MarkerMatcher` | `src/marker/matcher.rs:14-86` |
| `MarkerTimerHandle` / `TimerKind` | `src/marker/timer.rs:15-24` |
| `AgentDiffState` / `PaneDiffObservation` / `PaneDiffTickResult` | `src/pane_diff/mod.rs:13-36` |
| `ProviderManifest` | `src/provider/manifest.rs:5-20` |
| `Ctx { db, state_dir, env_state, tmux_server }` (RPC handler 共享 ctx) | `src/rpc/mod.rs` (导出),用法见 `src/bin/ccbd.rs:54-59` |
| `SESSION_NAME` 常量 | `src/tmux/mod.rs:15` `"ccbd-agents"` |
| `compute_socket_name(state_dir)` | `src/tmux/mod.rs:17-25` (sha256 前 16 hex,前缀 `ccbd-`) |
| `unit_name_for_socket(socket_name)` | `src/tmux/scope.rs:45-48` (前缀 `ccbd-tmux-` + 8 hex) |
| `agent_slice_for_project` / `workspace_slice_for_project` | `src/sandbox/systemd.rs:63-72` |
| `CcbdError` 关键变体 (R1/R3 相关) | `src/error.rs:12, 15, 39, 42` `AgentNotFound / AgentAlreadyExists / AgentUnexpectedExit / AgentWrongState / SandboxMountFailed / EnvironmentNotSupported / TmuxCommandFailed / IpcInvalidRequest` |

---

## 8. 备注 — 我读了但没头绪是否要进 inventory 的代码

(留给主控决定要不要补到 a2 brief)

* `src/provider/home_layout.rs` 全 692 行 — provider HOME 物化逻辑,跟 R3 sandbox path 间接相关 (.codex / .gemini / .claude 内容拷到 /home/agent),但本身不涉及 project_root cwd 校准。
* `src/db/jobs.rs` `claim_next_job` / `dispatched_jobs` 系列 — 调度状态机,跟 R2 状态枚举关联。但没有 ACK 子态。
* `src/db/agents_lifecycle.rs` — KILLED 写库 + cleanup 链。R1 cascade kill 间接走它。
* `src/orchestrator/pubsub.rs` 28 行 — broadcast channel,job_update / agent_output 通知。R2 设计若新增 ACK 通知会扩这里。
* `src/agent_io/writer.rs` / `agent_io/mod.rs` `send_text_to_pane` — 实际发送 text 到 pane 的入口,decompose 成 load_buffer + paste_buffer + send_enter (推断)。`paste_buffer -p` 是上游 Bug X 的根因;若 R2 / 后续要避开同坑需要 keystroke 路径。
* `src/cli/doctor.rs` 296 行 + `src/cli/logs.rs` 80 行 — 诊断/查日志 CLI 子命令,与 4 条 R 不直接相关。
* `src/db/state_machine_assert.rs` — `assert_state_to_idle` L3 evidence-driven assert 路径,被 `handle_agent_assert_state` (`src/rpc/handlers.rs:864-882`) 调用。R2 中 UNKNOWN→IDLE 的 L3 路径,跟新增 ACK 状态可能冲突。
