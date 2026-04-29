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
    T21[T2.1 切除 src/pty/ + Cargo.toml 移除 portable-pty]
    T22[T2.2 src/agent_io/ reader_task + TMUX_PANE_MAP]
    T23[T2.3 重写 handle_agent_spawn]
    T24[T2.4 改写 handle_agent_send 走 send_keys_literal]
    T25[T2.5 改造 mvp2/3/4 acceptance]
    T26[T2.6 新增 mvp6_acceptance.rs]
    T27[T2.7 G6.2 commit]
  end

  T01 --> T02 --> T03 --> T04 --> T05
  T05 --> T11
  T11 --> T12 --> T13 --> T14 --> T15 --> T16 --> T17 --> T18
  T18 --> T21
  T21 --> T22 --> T23 --> T24 --> T25 --> T26 --> T27
```

---

## 2. 原子任务定义（G6.0 CLI Skeleton）

### T0.1: Cargo.toml 双 binary 拆分

* **依赖前置**: 无
* **设计输入**: `mvp6-D.md §2`
* **输出产物**: `Cargo.toml` 修改
* **执行步骤**:
  1. 不动 `[package]` 段
  2. `[dependencies]` 新增 `clap = { version = "4.5", features = ["derive"] }` / `tabled = "0.15"` / `nix = { version = "0.28", features = ["fs"] }`
  3. **暂不删 `portable-pty`**（T2.1 才删）
  4. 文件末尾新增：
     ```toml
     [[bin]]
     name = "ccbd"
     path = "src/bin/ccbd.rs"

     [[bin]]
     name = "ccb"
     path = "src/bin/ccb.rs"
     ```
* **独立验收**: `cargo check`（应该会失败 —— src/bin/ccbd.rs 还没有），但 `cargo metadata --format-version 1 | jq '.packages[0].targets'` 能列出两个 bin target

### T0.2: src/main.rs → src/bin/ccbd.rs

* **依赖前置**: T0.1
* **设计输入**: `mvp6-D.md §2.3`
* **输出产物**: `src/bin/ccbd.rs`（新文件，内容来自原 main.rs）；`src/main.rs` 删除
* **执行步骤**:
  1. `mkdir -p src/bin`
  2. `git mv src/main.rs src/bin/ccbd.rs`
  3. 检查 `src/lib.rs` 是否 pub mod 暴露所有 ccbd 启动期需要的 db / rpc / monitor / sandbox / marker / error / env 模块（mvp5 已抽过，确认即可）
  4. `cargo check` 应该编译通过（src/bin/ccb.rs 还没文件会报错；先 stub 一个空 main 让 build 过）：
     ```rust
     // src/bin/ccb.rs (stub)
     fn main() { println!("ccb stub"); }
     ```
* **独立验收**: `cargo build --release` 通过；`target/release/ccbd` 与 `target/release/ccb` 都存在；跑 `target/release/ccbd` 行为不变（dev 模式启动可工作）

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

### T1.5: pipe_pane_to_fifo_sync + send_keys_literal_sync + kill_pane_sync

* **依赖前置**: T1.4
* **设计输入**: `mvp6-D.md §4.5`
* **输出产物**: 同上
* **执行步骤**:
  1. `pipe_pane_to_fifo_sync(&self, pane: &TmuxPaneId, fifo: &Path) -> Result<(), CcbdError>`
     - `tmux pipe-pane -t <pane> -O 'cat > <fifo_path>'`
  2. `send_keys_literal_sync(&self, pane: &TmuxPaneId, text: &str) -> Result<(), CcbdError>`
     - `tmux send-keys -t <pane> -l <text>`
  3. `kill_pane_sync(&self, pane: &TmuxPaneId) -> Result<(), CcbdError>`
     - `tmux kill-pane -t <pane>`
  4. 单测覆盖 send-keys 后 capture-pane 能看到字面字符 + kill-pane 后 list-panes 不含该 pane
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

### T1.7: src/tmux/tests 集成测试

* **依赖前置**: T1.6
* **设计输入**: `mvp6-D.md §4`
* **输出产物**: `src/tmux/mod.rs` 内 `mod tests` 或单独 `src/tmux/tests.rs`
* **执行步骤**:
  1. test_full_lifecycle：
     - 起一个 TmuxServer
     - ensure_session
     - spawn_window 跑 `sleep 5`
     - get_pane_pid 验证 > 0
     - mkfifo + pipe_pane_to_fifo
     - send_keys_literal "echo hello\n"
     - 从 fifo 读出含 "echo hello" 字符
     - kill_pane
  2. test_pane_id_parse：覆盖 T1.2 单测
  3. test_pid_parse_error：故意构造 invalid output
* **独立验收**: `cargo test tmux::tests` 全过

### T1.8: G6.1 commit

* **依赖前置**: T1.1 - T1.7
* **执行步骤**:
  1. `cargo test --quiet` 全绿（mvp1-5 测试 + 新 tmux 测试）
  2. `git add src/tmux/ src/error.rs src/lib.rs`
  3. commit message: `feat(mvp6): G6.1 tmux wrapper module`
* **独立验收**: 单 commit + 全测绿

---

## 4. 原子任务定义（G6.2 Spawn Surgery）

### T2.1: 切除 src/pty/ + Cargo.toml 移除 portable-pty

* **依赖前置**: T1.8
* **设计输入**: `mvp6-D.md §6.1 + §9.1`
* **输出产物**: src/pty/ 删除；Cargo.toml 移除依赖
* **执行步骤**:
  1. `rm -rf src/pty/`（先 verify 当前 use 路径——src/lib.rs 移除 `pub mod pty;`，src/rpc/handlers.rs 等的 use 暂时注掉，等 T2.2-T2.3 接回）
  2. Cargo.toml 删 `portable-pty = "0.8.1"` 行
  3. `cargo build` 应该编译失败（因为 handlers.rs 等还在调 pty::*）—— 这是预期，T2.2/T2.3 修复
  4. 提示：可以临时 stub 一个 `src/agent_io/mod.rs` 空模块占位让 lib.rs 可编译，T2.2 再实充
* **独立验收**: `grep -rn 'portable-pty\|portable_pty' Cargo.toml src/ tests/` 返回 0 行；Cargo.lock regenerate

### T2.2: src/agent_io/ - reader_task + TMUX_PANE_MAP

* **依赖前置**: T2.1
* **设计输入**: `mvp6-D.md §6.2 + §6.3 + Q6`
* **输出产物**: `src/agent_io/{mod.rs, reader.rs, writer.rs, registry.rs}`
* **执行步骤**:
  1. `src/agent_io/registry.rs`：定义 `TMUX_PANE_MAP: LazyLock<Arc<Mutex<HashMap<String, TmuxPaneId>>>>` + register/get/remove 函数（参考 mvp3 PARSER_REGISTRY 风格）
  2. `src/agent_io/reader.rs`：`spawn_agent_io_reader_task(agent_id, fifo: tokio::fs::File, db, parser_handle) -> JoinHandle`
     - 内部 `tokio::spawn(async move { ... })`
     - BufReader 读 FIFO，每 chunk 进 vt100 parser，写 output_chunk event（async wrapper）
     - marker matcher.scan 检测 IDLE 转移（async wrapper）
     - 错误退出时清理 TMUX_PANE_MAP
  3. `src/agent_io/writer.rs`：`pub async fn send_to_pane(tmux: &TmuxServer, agent_id: &str, text: String) -> Result<...>`，从 TMUX_PANE_MAP 拿 pane_id 后调 tmux.send_keys_literal
  4. `src/agent_io/mod.rs` re-export
  5. `src/lib.rs` 加 `pub mod agent_io;`
* **独立验收**: `cargo build` 通过（agent_io 模块可独立编译）

### T2.3: 重写 handle_agent_spawn

* **依赖前置**: T2.2
* **设计输入**: `mvp6-D.md §5`
* **输出产物**: `src/rpc/handlers.rs` 改写
* **执行步骤**:
  1. 修改 `Ctx` 结构：新增 `pub tmux_server: TmuxServer` 字段（src/bin/ccbd.rs 启动时构造）
  2. handle_agent_spawn 按 D §5.2 7 步流程改写：
     - 拼装 bwrap + agent_command full_cmd
     - mkfifo 创建命名管道（用 nix::unistd::mkfifo + spawn_db）
     - tmux ensure_session + spawn_window + get_pane_pid + pipe_pane_to_fifo（任一失败 cleanup）
     - pidfd_open + spawn_agent_pidfd_watch_task（不变）
     - insert_agent
     - tokio::fs::OpenOptions 以 RW 模式打开 FIFO（关键：不能 O_RDONLY 否则阻塞，见 D §5.3）
     - spawn_agent_io_reader_task
     - TMUX_PANE_MAP.insert
     - spawn_marker_timer_task
  3. 失败回滚 cleanup 闭包：kill_pane + remove_file + 不 insert_agent
* **独立验收**: `cargo build` 通过；`cargo test --test mvp2_acceptance` AC1-AC5 应该红（acceptance 还没改），但 lib unit 测试 + 新 agent_io 测试通过

### T2.4: handle_agent_send 改 send_keys_literal

* **依赖前置**: T2.3
* **设计输入**: `mvp6-D.md §5.4`
* **输出产物**: handlers.rs::handle_agent_send 改写
* **执行步骤**:
  1. 取消原 portable-pty master fd write
  2. 改为：`agent_io::writer::send_to_pane(&ctx.tmux_server, agent_id, text).await?`（内部调 tmux.send_keys_literal）
  3. 后续 record_send_progress 事务保留（mvp5 现状）
* **独立验收**: 同 T2.3，lib 测试通过

### T2.5: 改造 mvp2/3/4/5 acceptance

* **依赖前置**: T2.4
* **设计输入**: `mvp6-D.md §7`
* **输出产物**: tests/mvp{2,3,4}_acceptance.rs 改造
* **执行步骤**:
  1. mock_agent.sh 零修改
  2. 测试代码：用 ccbd::tmux::TmuxServer 在 setup 期 ensure_session（如果框架已自动起就跳过）
  3. 各 acceptance 测试的 spawn 调用走新流程（handlers.rs 内部已切，对外 RPC 不变）—— 一般无需改测试代码
  4. 业务断言（state_change / output_chunk 内容 / events 顺序）不允许改
  5. 跑 `cargo test --quiet` 全绿
* **独立验收**: mvp2/3/4 acceptance 全绿（业务断言保持）

### T2.6: 新增 mvp6_acceptance.rs

* **依赖前置**: T2.5
* **设计输入**: `mvp6-D.md §7.4`
* **输出产物**: tests/mvp6_acceptance.rs
* **执行步骤**:
  1. `test_no_portable_pty`：grep Cargo.lock + Cargo.toml 验证依赖移除
  2. `test_tmux_pane_created`：spawn 后调 tmux list-windows -t ccbd-agents 看到 window
  3. `test_pane_pid_matches_agent_pid`：spawn 后 SQLite agents.pid == tmux #{pane_pid}
  4. `test_pipe_pane_drains_to_fifo`：send 已知文本 → 等 events 出现 output_chunk
  5. `test_kill_pane_triggers_crashed`：tmux kill-pane → state_change to CRASHED
  6. （可选）`test_ccb_ping_smoke` / `test_ccb_ps_smoke`：cargo run --bin ccb ping，stdout 含 "ok=true"
* **独立验收**: `cargo test --test mvp6_acceptance` 全过

### T2.7: G6.2 commit

* **依赖前置**: T2.1 - T2.6
* **执行步骤**:
  1. `cargo test --quiet` 全绿（含 mvp6_acceptance）
  2. 跑 D §3.6 风格的验收脚本：grep portable-pty 0 行、ccb ping/ps smoke 通过
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
| T2.7 commit 后失败 | `git reset --soft HEAD~1` 回到 G6.1 末态—**保留 G6.0 + G6.1 收益**，重大失败时整 mvp6 回 mvp5 末态 commit `4f4b829` |

阶段独立 commit 是防灾设计——任一阶段红了不影响其他已 commit 阶段产出。
