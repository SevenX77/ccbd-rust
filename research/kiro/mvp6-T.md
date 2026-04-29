# Kiro Tasks: MVP 6 (拨乱反正)

> **文档定位**：MVP 6 由 Codex 逐项实施的原子任务清单。每个任务必须独立编译、独立验证。严格按 `mvp6-D.md` 落地，禁止引入 MVP6 范围外能力（不修真 codex/gemini idle marker、不做 OAuth 物化、不做 mailbox/queue —— 这些留 MVP7-9）。**三个子阶段独立 commit**：G6.0 / G6.1 / G6.2 各一个 commit。

---

## 1. 任务依赖与执行图谱

```mermaid
graph TD
  subgraph G0[G6.0 CLI Skeleton]
    T01[T0.1 Cargo.toml 双 binary 拆分]
    T02[T0.2 src/main.rs -> src/bin/ccbd.rs]
    T03[T0.3 src/bin/ccb.rs ping 子命令]
    T04[T0.4 src/bin/ccb.rs ps 子命令 + tabled 输出]
    T05[T0.5 G6.0 commit]
  end

  subgraph G1[G6.1 Tmux Wrapper]
    T11[T1.1 src/tmux/error.rs TmuxError + CcbdError 映射]
    T12[T1.2 src/tmux/pane.rs TmuxPaneId 类型]
    T13[T1.3 src/tmux/session.rs TmuxServer + ensure_session_sync]
    T14[T1.4 spawn_window_sync / get_pane_pid_sync]
    T15[T1.5 pipe_pane_to_fifo_sync / send_keys_literal_sync / kill_pane_sync]
    T16[T1.6 async wrapper 套壳 (spawn_db)]
    T17[T1.7 src/tmux/tests 单测]
    T18[T1.8 G6.1 commit]
  end

  subgraph G2[G6.2 Spawn Surgery]
    T21[T2.1 新建 src/agent_io/ + state_machine async wrapper（pty 仍存在）]
    T22[T2.2 重写 handle_agent_spawn / send 走 agent_io（caller 切换，pty 不再被调用）]
    T23[T2.3 切除 src/pty/ + 移除 portable-pty]
    T24[T2.4 改造 mvp2/3/4 acceptance harness Ctx 构造点]
    T25[T2.5 新增 mvp6_acceptance.rs（含 send-keys + 双步 newline 验证）]
    T26[T2.6 G6.2 commit]
  end

  T01 --> T02 --> T03 --> T04 --> T05
  T05 --> T11
  T11 --> T12 --> T13 --> T14 --> T15 --> T16 --> T17 --> T18
  T18 --> T21
  T21 --> T22 --> T23 --> T24 --> T25 --> T26
```

---

## 2. 原子任务定义（G6.0 CLI Skeleton）

### T0.1: Cargo.toml 双 binary 拆分 + bin stub 立即可 build（Round 2 修订）

* **依赖前置**: 无
* **设计输入**: `mvp6-D.md §2 + Round 2 反馈：T0.1 应让 cargo build 立刻通过`
* **输出产物**: `Cargo.toml` 修改 + 创建 `src/bin/ccbd.rs` + `src/bin/ccb.rs` stub
* **执行步骤**:
  1. 不动 `[package]` 段
  2. `[dependencies]` 新增 `clap = { version = "4.5", features = ["derive"] }` / `tabled = "0.15"` / `nix = { version = "0.28", features = ["fs"] }` / `sha2 = "0.10"`
  3. **暂不删 `portable-pty`**（T2.3 才删 —— 见 G6.2 重排说明）
  4. 文件末尾新增：
     ```toml
     [[bin]]
     name = "ccbd"
     path = "src/bin/ccbd.rs"

     [[bin]]
     name = "ccb"
     path = "src/bin/ccb.rs"
     ```
  5. 立即创建两个 bin stub（保证 cargo build 这一步就通过）：
     - `mkdir -p src/bin`
     - `git mv src/main.rs src/bin/ccbd.rs`（保留原 daemon 入口）
     - 创建 `src/bin/ccb.rs` 含最简 main：
       ```rust
       fn main() {
           println!("ccb stub - implemented in T0.3 / T0.4");
           std::process::exit(0);
       }
       ```
* **独立验收**: `cargo build --release` 通过；`target/release/ccbd` 与 `target/release/ccb` 都存在；`cargo metadata` 列出双 bin target；现有 daemon 测试不受影响

### T0.2: ~~src/main.rs → src/bin/ccbd.rs~~ （Round 2 修订：合并到 T0.1）

T0.1 已合并 main.rs 移动 + ccb stub 创建。本任务保留为 noop，直接进 T0.3。

### T0.3: ccb ping 子命令

* **依赖前置**: T0.2
* **设计输入**: `mvp6-D.md §3.3`
* **输出产物**: `src/bin/ccb.rs` 完整化
* **执行步骤**:
  1. clap derive struct 定义 `Cli { cmd: Cmd }`，subcommand 含 `Ping`
  2. socket path 解析（按 §3.1 优先级：CCB_SOCKET env > XDG fallback；dev 模式 fallback 到 `target/dev_state/ccbd.sock`）
  3. `cmd_ping`：调 `system.dump`，parse 响应，print socket 路径 + sessions/agents 数量
  4. 错误处理：socket 不存在 → 红色提示 + exit(1)；ECONNREFUSED → exit(1)；JSON 错 → exit(3)
* **独立验收**:
  - daemon 没起：`./target/release/ccb ping` 输出红色错误，exit 1
  - daemon 起着：`scripts/start-daemon.sh && ./target/release/ccb ping`，stdout 含 "ok=true socket=..."，exit 0

### T0.4: ccb ps 子命令 + tabled 输出

* **依赖前置**: T0.3
* **设计输入**: `mvp6-D.md §3.4`
* **输出产物**: `src/bin/ccb.rs` 加 `cmd_ps`
* **执行步骤**:
  1. clap subcommand 加 `Ps`
  2. 调 `system.dump`
  3. 用 `tabled::Tabled` derive 给 AgentRow（agent_id / provider / state / sub_state / pid 等列）
  4. 用 `tabled::Table::new(rows)` 生成表格 + println
  5. 表格底部 println hint：`💡 To inspect agents live: tmux -L ccbd-<hash> attach -t ccbd-agents`（hash 计算可暂时 hardcode 或从 dump.daemon_meta 拿，本期对 hash 计算不严格——只要 hint 字段类型正确即可）
* **独立验收**: 同 T0.3 但跑 `./target/release/ccb ps`，输出表格 + hint

### T0.5: G6.0 commit

* **依赖前置**: T0.1 - T0.4
* **设计输入**: `mvp6-D.md §12`
* **输出产物**: 一个 git commit
* **执行步骤**:
  1. `cargo build --release` 通过
  2. daemon 起后 `ccb ping` / `ccb ps` 都能正常返回
  3. `cargo test --quiet` 全绿（不影响现有测试）
  4. `git add Cargo.toml Cargo.lock src/bin/ src/lib.rs`（如有）
  5. commit message: `feat(mvp6): G6.0 CLI skeleton (ccb ping / ps)`
* **独立验收**: 单 commit + `ccb ping` / `ccb ps` 跑通 + `cargo test` 全绿

---

## 3. 原子任务定义（G6.1 Tmux Wrapper）

### T1.1: src/tmux/error.rs

* **依赖前置**: T0.5
* **设计输入**: `mvp6-D.md §4.6 + §8`
* **输出产物**: `src/tmux/error.rs`，`src/error.rs` 加新变体
* **执行步骤**:
  1. 新建 `src/tmux/error.rs`，定义 `TmuxError` 枚举（BinaryNotFound / CommandFailed / ParsePaneId / ParsePid / Io）
  2. `src/error.rs::CcbdError` 新增 `TmuxNotFound` 与 `TmuxCommandFailed { cmd, stderr, exit }` 变体
  3. `to_rpc_error()` 加分支返回 error_code `TMUX_NOT_FOUND` / `TMUX_COMMAND_FAILED`
  4. `From<TmuxError> for CcbdError` 实现转换
  5. 新增 round-trip 单测：构造 TmuxCommandFailed → to_rpc_error → 校验 code/error_code/details
* **独立验收**: `cargo test error::tests` 通过，新增 round-trip 测试通过

### T1.2: src/tmux/pane.rs - TmuxPaneId

* **依赖前置**: T1.1
* **设计输入**: `mvp6-D.md §4.2`
* **输出产物**: `src/tmux/pane.rs`
* **执行步骤**:
  1. `pub struct TmuxPaneId(pub String);`
  2. `impl TmuxPaneId { pub fn parse(s: &str) -> Result<Self, TmuxError>; }` —— 不以 `%` 开头返回 ParsePaneId
  3. derive `Clone, Debug, PartialEq, Eq, Hash`
  4. 单测：parse `"%1"` ok / parse `"abc"` err
* **独立验收**: `cargo test tmux::pane::tests` 通过

### T1.3: src/tmux/session.rs - TmuxServer + ensure_session_sync

* **依赖前置**: T1.2
* **设计输入**: `mvp6-D.md §4.2 / §4.5`
* **输出产物**: `src/tmux/session.rs`，`src/tmux/mod.rs` re-export
* **执行步骤**:
  1. 新建 `src/tmux/mod.rs` + `pub mod error; pub mod pane; pub mod session;`
  2. `src/lib.rs` 加 `pub mod tmux;`
  3. `pub struct TmuxServer { socket_name: String }`，构造函数 `TmuxServer::new(state_dir: &Path) -> Self`（hash state_dir 拿 socket_name）
  4. `ensure_session_sync(&self, session_name: &str, cwd: &Path) -> Result<(), CcbdError>`：
     - 先 `tmux -L <socket> has-session -t <session>` 检测
     - 不存在则 `tmux -L <socket> new-session -d -s <session> -c <cwd> -x 200 -y 60`
     - exit code 0 = 成功；非 0 = TmuxCommandFailed
  5. derive Clone（要 Arc<Mutex<>> 还是直接 Clone 都可，按需选择最简单的）
  6. 单测：`ensure_session_sync` 调真 tmux 二进制 + 检测 socket 文件存在 + cleanup 杀 server
* **独立验收**: `cargo test tmux::session::tests` 通过

### T1.4: spawn_window_sync + get_pane_pid_sync

* **依赖前置**: T1.3
* **设计输入**: `mvp6-D.md §4.3 / §4.5`
* **输出产物**: `src/tmux/session.rs` 加方法（或 src/tmux/pane.rs 看实施分配）
* **执行步骤**:
  1. `spawn_window_sync(&self, session: &str, window: &str, cwd: &Path, cmd: &[&str]) -> Result<TmuxPaneId, CcbdError>`：
     - 构造 `tmux -L ... new-window -d -t <session>: -n <window> -c <cwd> -P -F #{pane_id} -- <cmd...>`
     - stdout 捕获 → trim → `TmuxPaneId::parse`
  2. `get_pane_pid_sync(&self, pane: &TmuxPaneId) -> Result<i32, CcbdError>`：
     - `tmux -L ... display-message -p -t <pane> #{pane_pid}`
     - stdout → trim → parse i32 → ParsePid 错误处理
  3. 单测：起 session + spawn `sleep 30` window + display-message 拿 pid + 验证 pid > 0 + kill-pane cleanup
* **独立验收**: `cargo test tmux::*` 全绿

### T1.5: pipe_pane_to_fifo_sync + send_keys_* + kill_pane_sync（Round 1 双模式 send 采纳）

* **依赖前置**: T1.4
* **设计输入**: `mvp6-D.md §4.5 + §5.4`
* **输出产物**: 同上
* **执行步骤**:
  1. `pipe_pane_to_fifo_sync(&self, pane: &TmuxPaneId, fifo: &Path) -> Result<(), CcbdError>`
     - `tmux pipe-pane -t <pane> -O 'cat > <fifo_path>'`
  2. `send_keys_literal_sync(&self, pane: &TmuxPaneId, text: &str) -> Result<(), CcbdError>`
     - `tmux send-keys -t <pane> -l <text>`（`-l` literal mode 不解析 keysym）
  3. `send_keys_keysym_sync(&self, pane: &TmuxPaneId, keysym: &str) -> Result<(), CcbdError>` — **Round 1 新增**
     - `tmux send-keys -t <pane> <keysym>`（无 `-l`，让 tmux 解析 keysym 如 `Enter` / `C-c` / `Tab`）
  4. `kill_pane_sync(&self, pane: &TmuxPaneId) -> Result<(), CcbdError>`
     - `tmux kill-pane -t <pane>`
  5. 单测必须覆盖：
     - `send_keys_literal_sync("hello\n")` 后 capture-pane 能看到字面 `hello\n`（验证 literal 行为）
     - `send_keys_keysym_sync("Enter")` 后 shell pane 能识别为换行（spawn bash 后 send `echo hi` literal + send `Enter` keysym → capture-pane 含 `hi`）
     - `kill_pane_sync` 后 `list-panes` 不含该 pane
* **独立验收**: `cargo test tmux::*` 全绿

### T1.6: async wrapper

* **依赖前置**: T1.5
* **设计输入**: `mvp6-D.md §4.4`
* **输出产物**: `src/tmux/session.rs` async 方法
* **执行步骤**:
  1. 每个 sync 函数对应一个 async wrapper：`pub async fn ensure_session(&self, ...) -> Result<...>`
  2. async wrapper 内 `crate::db::common::spawn_db("tmux::ensure_session", move || self.ensure_session_sync(...))`
  3. 注意 self 的 borrow / clone：TmuxServer 需 derive Clone，闭包内 move
* **独立验收**: `cargo build` 通过；async wrapper 单测（用 #[tokio::test]）

### T1.7: src/tmux/tests 集成测试（Round 2 修订：与新双步 send 策略一致）

* **依赖前置**: T1.6
* **设计输入**: `mvp6-D.md §4 + §5.4.2`
* **输出产物**: `src/tmux/mod.rs` 内 `mod tests`
* **执行步骤**:
  1. **环境前置**：测试开头 `which::which("tmux").expect("tmux binary required for tmux module tests; install via apt-get install tmux on CI")` —— 失败给清晰错信息（Round 2 non-blocking 采纳）
  2. test_full_lifecycle：
     - 起 TmuxServer
     - ensure_session
     - spawn_window 跑 `sleep 5`
     - get_pane_pid 验证 > 0
     - mkfifo + pipe_pane_to_fifo
     - **使用 send_keys_literal "echo hello" + send_keys_keysym "Enter"**（按新双步策略）
     - 从 fifo 读出含 "echo hello" 字符
     - kill_pane
  3. test_pane_id_parse：覆盖 T1.2 单测
  4. test_pid_parse_error：故意构造 invalid output
  5. test_compute_socket_name_deterministic：调 `compute_socket_name` 两次同 state_dir 应返同一字符串
* **独立验收**: `cargo test tmux::tests` 全过 + tmux 缺失时给明确错信息

### T1.8: G6.1 commit

* **依赖前置**: T1.1 - T1.7
* **执行步骤**:
  1. `cargo test --quiet` 全绿（mvp1-5 测试 + 新 tmux 测试）
  2. `git add src/tmux/ src/error.rs src/lib.rs`
  3. commit message: `feat(mvp6): G6.1 tmux wrapper module`
* **独立验收**: 单 commit + 全测绿

---

## 4. 原子任务定义（G6.2 Spawn Surgery）

**Round 1 重要修订**：原 T2.1-T2.7 顺序违反"每任务独立编译"原则——T2.1 删 pty 后到 T2.3 才能编译过。Round 2 重排为：先建兼容层 + caller 切换 + 最后删 pty。每步 cargo build 都通过。

### T2.1: 新增 src/agent_io/ + db state_machine async wrapper + shutdown_reader（Round 2 修订）

* **依赖前置**: T1.8
* **设计输入**: `mvp6-D.md §6.2 + §6.3 + §6.3.1 + Round 1/2 反馈`
* **输出产物**: `src/agent_io/{mod.rs,reader.rs,writer.rs,registry.rs}` + `src/db/state_machine.rs` 新增 async wrapper
* **执行步骤**:
  1. **新增 `db::state_machine::mark_agent_idle_matched` async wrapper**：mvp5 PTY exempt 期间故意没出，MVP6 必须出
  2. **新增 `src/agent_io/registry.rs`**：`AgentIoEntry` struct（含 pane_id + reader_handle + fifo_path）+ `TMUX_PANE_MAP: LazyLock<Arc<Mutex<HashMap<String, AgentIoEntry>>>>` + register/get/remove 函数
  3. **新增 `src/agent_io/reader.rs`**：`spawn_agent_io_reader_task(...)` 按 D §6.3 模板：
     - tokio::spawn 异步 task
     - std::sync::Mutex<vt100::Parser> 锁**不跨 await**——每次 process / scan 在独立 scope
     - `db::events::insert_event` 调用参数顺序：`(db, agent_id, None, "output_chunk".into(), payload)`
     - `db::state_machine::mark_agent_idle_matched` 走新 async wrapper
  4. **新增 `src/agent_io/writer.rs`**：`send_text_to_pane(tmux, pane, text)` 按 D §5.4.2 双步策略（split on \n + 每段 literal + 每个 \n 用 Enter keysym）
  5. **新增 `src/agent_io/mod.rs::shutdown_reader(agent_id)` async 函数**（Round 2 反馈采纳）：从 TMUX_PANE_MAP 移出 entry → abort reader_handle → await unwind → remove fifo file。详见 D §6.3.1
  6. `src/lib.rs` 加 `pub mod agent_io;`
  7. **不动**：src/pty/ 仍存在；handlers.rs / marker / 等 caller 还调 pty::*
* **独立验收**: `cargo build` 通过（pty + agent_io 共存）；`cargo test --lib agent_io::tests` 全绿（含 shutdown_reader 单测：起 reader → shutdown → 验证 reader_handle 已 abort + fifo 已删）

### T2.2: 重写 handle_agent_spawn + handle_agent_send 走 agent_io + handle_agent_kill 加 shutdown_reader 调用

* **依赖前置**: T2.1
* **设计输入**: `mvp6-D.md §5 + §5.3 + §5.4 + §6.3.1`
* **输出产物**: `src/rpc/handlers.rs` 改写
* **执行步骤**:
  1. `Ctx` 新增 `tmux_server: Arc<TmuxServer>` 字段
  2. **完整列出所有 Ctx 构造点**：
     - `src/bin/ccbd.rs` daemon 启动序列：`Arc::new(TmuxServer::new(&state_dir))`
     - `tests/mvp2_acceptance.rs` / `tests/mvp3_acceptance.rs` / `tests/mvp4_acceptance.rs` 内 helpers (`build_test_ctx` 或类似)
     - `src/rpc/handlers.rs` 内 `mod tests` 单测构造
     - 每个构造点都加 `tmux_server: Arc::new(TmuxServer::new(&state_dir))`
  3. **handle_agent_spawn 按 D §5 严格 8 步流程**（FIFO 顺序：mkfifo → **daemon RW open**（必须先于 pipe-pane） → tmux ensure_session → spawn_window → get_pane_pid → tmux pipe-pane → pidfd_open → insert_agent → spawn_reader_task → TMUX_PANE_MAP.insert AgentIoEntry → spawn_marker_timer_task）
  4. handle_agent_send 按 D §5.4.2 split on \n + literal + Enter keysym
  5. **handle_agent_kill 必须调 `agent_io::shutdown_reader(agent_id)` 关闭 reader 资源**（Round 2 反馈）
  6. **pidfd watcher task 退出前也调 `shutdown_reader(agent_id)`**（CRASHED 路径）—— 修改 src/monitor/agent_watch.rs
  7. **master_death cascade 也对 active agent 调 shutdown_reader**—— 修改 src/monitor/master_watch.rs
  8. 失败回滚 cleanup 闭包：覆盖**所有**失败点（get_pid / pipe_pane / pidfd_open / insert_agent / OpenOptions），任一失败必须 kill_pane + drop fifo_file + remove fifo + drop pidfd
* **独立验收**: `cargo build` 通过；mvp2/3/4/5 acceptance 编译通过（断言可能红——T2.4 改造后再绿）；`cargo test --lib` 全绿

### T2.3: 切除 src/pty/ + Cargo.toml 移除 portable-pty（最后一步）

* **依赖前置**: T2.2
* **设计输入**: `mvp6-D.md §6.1 + §9.1`
* **输出产物**: src/pty/ 删除；Cargo.toml 移除 portable-pty 依赖
* **执行步骤**:
  1. 验证：`grep -rn 'pty::' src/ tests/ --include='*.rs'` 应该 0 行（caller 已切到 agent_io，T2.2 完成后 pty 模块不被引用）
  2. `rm -rf src/pty/`
  3. `src/lib.rs` 删除 `pub mod pty;`
  4. Cargo.toml 删 `portable-pty = "0.8.1"`
  5. `cargo build` —— 因为 caller 已切，删 pty 不破坏 build
* **独立验收**:
  - `grep -rn 'portable-pty\|portable_pty' Cargo.toml src/ tests/` 返回 0 行
  - `cargo build` 通过
  - `cargo test --lib` 全绿（mvp1-5 lib unit test 都不依赖 pty）

### T2.4: 改造 mvp2/3/4 acceptance（acceptance harness 加 tmux_server 构造点）

* **依赖前置**: T2.3
* **设计输入**: `mvp6-D.md §7 + Round 1 Ctx 构造点反馈`
* **输出产物**: tests/mvp{2,3,4}_acceptance.rs 改造
* **执行步骤**:
  1. mock_agent.sh 零修改
  2. acceptance harness 内 build_test_ctx 类辅助函数加 `tmux_server: Arc::new(TmuxServer::new(&state_dir))` 字段（T2.2 已经在 Ctx struct 加了字段，这步是补 caller 构造）
  3. 各 acceptance test 的 spawn 调用走新流程（handlers.rs 内部已切到 agent_io，对外 RPC 不变）—— 业务断言代码无需改
  4. 业务断言（state_change / output_chunk 内容 / events 顺序）不允许改
  5. 跑 `cargo test --quiet` 全绿
* **独立验收**: mvp2/3/4 acceptance 全绿（业务断言保持）

### T2.5: 新增 mvp6_acceptance.rs（含 send-keys 验证 + Round 1 增强）

* **依赖前置**: T2.4
* **设计输入**: `mvp6-D.md §7.4 + §5.4.3`
* **输出产物**: tests/mvp6_acceptance.rs
* **执行步骤**:
  1. `test_no_portable_pty`：grep Cargo.lock + Cargo.toml 验证依赖移除
  2. `test_tmux_pane_created`：spawn 后调 tmux list-windows -t ccbd-agents 看到 window（with agent_id 命名）
  3. `test_pane_pid_matches_agent_pid`：spawn 后 SQLite `agents.pid` == `tmux display-message #{pane_pid}`
  4. `test_pipe_pane_drains_to_fifo`：send 已知文本 → 等 events 出现 output_chunk
  5. `test_send_text_with_trailing_newline_triggers_shell_eval`（**Round 1 新增**）：send `echo hello\n` → 等 output_chunk 含 `hello`（验证 D §5.4.2 双步发送的 newline 能真触发 shell 行解析）
  6. `test_kill_pane_triggers_crashed`：tmux kill-pane → state_change to CRASHED + tmux list-panes 不再含该 pane
  7. `test_pidfd_kill_cleans_tmux_pane`（**Round 1 non-blocking #6 采纳**）：通过 RPC `agent.kill` SIGKILL agent → 验证 tmux pane 也被自然关闭 / 不残留
  8. `test_ccb_ping_smoke`：跑 `target/release/ccb ping`，stdout 含 "ok=true socket=" + exit 0
  9. `test_ccb_ps_smoke`：跑 `target/release/ccb ps`，stdout 含表格头 + 底部含 "tmux -L" hint
* **独立验收**: `cargo test --test mvp6_acceptance` 全过

### T2.6: G6.2 commit

* **依赖前置**: T2.1 - T2.5
* **执行步骤**:
  1. `cargo test --quiet` 全绿（含 mvp6_acceptance）
  2. 跑验收脚本：`grep -rnE 'portable-pty|portable_pty' Cargo.toml src/ tests/` 返回 0 行；ccb ping/ps smoke 通过
  3. `git add src/ tests/ Cargo.toml Cargo.lock`
  4. commit message: `refactor(mvp6): G6.2 portable-pty -> tmux pivot for agent spawn`
* **独立验收**: 单 commit + 全测绿

---

## 5. 验收命令快速参考

```bash
# === 阶段 G6.0 ===
cargo build --release
ls target/release/ccbd target/release/ccb
./scripts/start-daemon.sh   # 现有脚本仍工作
./target/release/ccb ping
./target/release/ccb ps

# === 阶段 G6.1 ===
cargo test --lib tmux
which tmux   # 验证 binary 存在

# === 阶段 G6.2 ===
# AC2 portable-pty 切除
grep -rnE 'portable-pty|portable_pty' Cargo.toml src/ tests/   # 0 行

# AC3 tmux 接管 spawn
./scripts/start-daemon.sh
# spawn 一个 mock agent...
tmux -L ccbd-<hash> list-windows -t ccbd-agents

# AC4 用户可观测（手动）
# 在另一个终端跑：
tmux -L ccbd-<hash> attach -t ccbd-agents
# 应能看到 mock_agent 实时输出

# 总测试
cargo test --quiet
```

---

## 6. 失败回滚指南（采纳 mvp5 安全化标准）

**前置**：执行回滚前必 `git status` 确认仅 mvp6 范围内文件改动。

| 失败时机 | 回滚步骤 |
|---|---|
| T0.x 失败（未 commit）| `git checkout -- Cargo.toml src/` 回到 mvp5 末态 |
| T0.5 commit 后失败 | `git reset --soft HEAD~1` 检查再决定 |
| T1.x 失败（未 commit）| `git checkout -- Cargo.toml src/tmux/ src/error.rs src/lib.rs` 回到 G6.0 末态 |
| T1.8 commit 后失败 | `git reset --soft HEAD~1` 回到 G6.0 末态——**保留 G6.0 收益** |
| T2.x 失败（未 commit）| `git checkout -- src/ tests/ Cargo.toml Cargo.lock` 回到 G6.1 末态 |
| T2.6 commit 后失败 | `git reset --soft HEAD~1` 回到 G6.1 末态—**保留 G6.0 + G6.1 收益**，重大失败时整 mvp6 回 mvp5 末态 commit `4f4b829` |

阶段独立 commit 是防灾设计——任一阶段红了不影响其他已 commit 阶段产出。
