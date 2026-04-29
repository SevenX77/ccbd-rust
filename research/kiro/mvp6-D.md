# Kiro Design: MVP 6 (拨乱反正 / The Tmux Pivot & CLI Skeleton)

> **文档定位**：本文件是 ccbd-rust MVP 6 阶段的官方 D (Design) 规格。基于 `mvp6-R.md` 的边界要求，为 Codex 实施提供**无歧义落地蓝图**。核心：切除 `portable-pty`，重建 Tmux 控制平面，引入 `ccb` CLI 骨架客户端。

---

## 1. 总体路线图与依赖拓扑

本阶段采用三步走的外科手术式迭代，安全检查点（Safe Checkpoints）隔离变更风险。每个子阶段独立可回滚——任一阶段失败，只回退到该 checkpoint，不影响前阶段成果。

```mermaid
graph TD
    subgraph G6.0 CLI Skeleton
        T0[Cargo.toml 拆分 bin/ccb 与 bin/ccbd] --> T1[实现 ccb ping]
        T1 --> T2[实现 ccb ps]
        T2 --> SC1(((Checkpoint: ccb 能连通 daemon)))
    end

    subgraph G6.1 Tmux Wrapper
        T3[创建 src/tmux/ 抽象] --> T4[TmuxSession / TmuxPaneId 类型]
        T4 --> T5[实现 new_window / display_pid / pipe_pane / send_keys / kill_pane]
        T5 --> SC2(((Checkpoint: cargo test tmux::tests 能起停 pane)))
    end

    subgraph G6.2 Spawn Surgery
        SC1 --> T6
        SC2 --> T6[切除 src/pty/ 模块及 portable-pty 依赖]
        T6 --> T7[重写 rpc/handlers.rs::handle_agent_spawn]
        T7 --> T8[新建 src/agent_io/ 改造 reader task 适配 FIFO]
        T8 --> T9[改造 mvp2/3/4 acceptance + 新增 mvp6_acceptance]
        T9 --> SC3(((Checkpoint: 端到端 acceptance 全绿 + AC4 手动 attach 验证)))
    end
```

---

## 2. Cargo.toml 双 binary 拆分

### 2.1 现状

单 `[package] name = "ccbd"`，`src/main.rs` 既是 daemon 入口又混杂部分核心逻辑（rpc::Ctx 构造、env::resolve_state_dir 调用等）。`Cargo.toml` 含 `portable-pty = "0.8.1"` 依赖。

### 2.2 目标设计

将核心逻辑抽到 `src/lib.rs`（已存在的 lib 模式继续保留），产出**双可执行文件**。

**Cargo.toml diff**：

```toml
[package]
name = "ccbd"             # crate 名保持不变（向后兼容 cargo 历史）
version = "0.1.0"
edition = "2024"

[dependencies]
# 现有依赖保留 ...
# tokio / serde / rusqlite / thiserror / tracing / directories / uuid / libc / which

# === MVP6 新增 ===
clap = { version = "4.5", features = ["derive"] }
tabled = "0.15"           # ccb ps 终端表格输出
nix = { version = "0.28", features = ["fs"] }   # mkfifo / unistd

# === MVP6 移除 ===
# portable-pty = "0.8.1"  # 删除该行

# === 新增 binaries 配置 ===
[[bin]]
name = "ccbd"
path = "src/bin/ccbd.rs"

[[bin]]
name = "ccb"
path = "src/bin/ccb.rs"
```

### 2.3 物理代码移动

```bash
mkdir -p src/bin
git mv src/main.rs src/bin/ccbd.rs
# src/lib.rs 已存在（mvp5 已抽过），继续 pub mod 暴露 db / rpc / monitor / sandbox / marker / error / env / tmux / agent_io 等
```

**`src/bin/ccbd.rs`** 内容等同原 `src/main.rs`——daemon 入口 + 启动序列 + 信号处理。

**`src/bin/ccb.rs`** 是 MVP6 新增的 CLI 客户端入口（详见 §3）。

---

## 3. `bin/ccb.rs` 客户端二进制结构

`ccb` 是无状态 JSON-RPC client，通过 Unix Domain Socket 接 daemon。

### 3.1 Socket 路径协商

```rust
fn resolve_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("CCB_SOCKET") {
        return PathBuf::from(p);
    }
    let dirs = directories::ProjectDirs::from("", "", "ccbd")
        .expect("XDG state dir not resolvable");
    dirs.state_dir()
        .expect("state_dir not available")
        .join("ccbd.sock")
}
```

dev 模式（`CCB_ENV=dev`）下 fallback 到 `<CARGO_MANIFEST_DIR>/target/dev_state/ccbd.sock` 跟 daemon 端对齐。

### 3.2 CLI 拓扑

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ccb", version, about = "Claude Code Bridge CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Probe the daemon liveness
    Ping,
    /// List all sessions / agents / pending evidence
    Ps,
}
```

### 3.3 `ccb ping` 实现

发 `system.dump`（mvp4 已实现），从 result 取 daemon 元数据：

```rust
fn cmd_ping() -> Result<(), Box<dyn Error>> {
    let socket = resolve_socket_path();
    if !socket.exists() {
        eprintln!("\x1b[31mccbd daemon is not running\x1b[0m at {}", socket.display());
        eprintln!("Start it with: scripts/start-daemon.sh");
        std::process::exit(1);
    }
    let result = rpc_call(&socket, "system.dump", &json!({}))?;
    println!("ok=true socket={}", socket.display());
    if let Some(sessions) = result.get("sessions").and_then(|v| v.as_array()) {
        println!("sessions={} agents={}",
            sessions.len(),
            result.get("agents").and_then(|v| v.as_array()).map_or(0, |a| a.len())
        );
    }
    Ok(())
}
```

### 3.4 `ccb ps` 实现

```rust
fn cmd_ps() -> Result<(), Box<dyn Error>> {
    let socket = resolve_socket_path();
    let dump = rpc_call(&socket, "system.dump", &json!({}))?;

    // 用 tabled 格式化 agents 表格
    #[derive(tabled::Tabled)]
    struct AgentRow {
        agent_id: String,
        provider: String,
        state: String,
        sub_state: String,
        pid: String,
    }

    let rows: Vec<AgentRow> = dump.get("agents")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(|a| AgentRow { ... }).collect())
        .unwrap_or_default();

    let tmux_socket = format!("ccbd-{}", state_dir_hash());
    println!("{}", tabled::Table::new(rows));
    println!();
    println!("\x1b[2m💡 To inspect agents live: tmux -L {} attach -t ccbd-agents\x1b[0m", tmux_socket);
    Ok(())
}
```

### 3.5 错误处理

| 场景 | 输出 | exit code |
|---|---|---|
| socket 文件不存在 | 红色提示 "ccbd daemon is not running at {path}" | 1 |
| socket 存在但 connect ECONNREFUSED | "ccbd daemon socket exists but not accepting connections" | 1 |
| RPC 返回 JSON-RPC error | "RPC error: {error_code}: {details}" | 2 |
| 解析 JSON 失败 | "invalid JSON response from daemon" | 3 |

不允许 panic / unwrap 在生产路径。

---

## 4. `src/tmux/` 模块设计（Rust tmux wrapper）

宿主机 tmux 二进制的强类型防腐层。所有 tmux 命令通过 `std::process::Command` 同步调用，由 Tokio `spawn_blocking`（已是 mvp5 标准模式）包裹。

### 4.1 目录结构

```
src/tmux/
├── mod.rs        # 顶层 TmuxServer 入口 + 配置 + Tokio async wrappers
├── session.rs    # TmuxSession 抽象 + ensure_session
├── pane.rs       # TmuxPaneId + window/pane lifecycle ops
└── error.rs      # TmuxError 枚举 + map 到 CcbdError
```

### 4.2 强类型抽象

```rust
// src/tmux/mod.rs
pub struct TmuxServer {
    socket_name: String,         // "ccbd-<state_dir_hash>"
}

// src/tmux/pane.rs
pub struct TmuxPaneId(pub String);   // e.g. "%1" / "%17"

impl TmuxPaneId {
    pub fn parse(s: &str) -> Result<Self, TmuxError> {
        if s.starts_with('%') {
            Ok(Self(s.to_string()))
        } else {
            Err(TmuxError::ParsePaneId(s.to_string()))
        }
    }
}
```

### 4.3 同步 API 集合（pub(crate) 给 db 模块同款 _sync 风格）

```rust
impl TmuxServer {
    pub(crate) fn ensure_session_sync(&self, session_name: &str, cwd: &Path) -> Result<(), CcbdError>;
    pub(crate) fn spawn_window_sync(&self, session: &str, window: &str, cwd: &Path, cmd: &[&str]) -> Result<TmuxPaneId, CcbdError>;
    pub(crate) fn get_pane_pid_sync(&self, pane: &TmuxPaneId) -> Result<i32, CcbdError>;
    pub(crate) fn pipe_pane_to_fifo_sync(&self, pane: &TmuxPaneId, fifo: &Path) -> Result<(), CcbdError>;
    pub(crate) fn send_keys_literal_sync(&self, pane: &TmuxPaneId, text: &str) -> Result<(), CcbdError>;
    pub(crate) fn kill_pane_sync(&self, pane: &TmuxPaneId) -> Result<(), CcbdError>;
}
```

### 4.4 Async wrapper（mvp5 spawn_db helper 同款）

```rust
impl TmuxServer {
    pub async fn ensure_session(&self, session_name: String, cwd: PathBuf) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::ensure_session", move || {
            server.ensure_session_sync(&session_name, &cwd)
        }).await
    }
    pub async fn spawn_window(&self, session: String, window: String, cwd: PathBuf, cmd: Vec<String>) -> Result<TmuxPaneId, CcbdError>;
    pub async fn get_pane_pid(&self, pane: TmuxPaneId) -> Result<i32, CcbdError>;
    pub async fn pipe_pane_to_fifo(&self, pane: TmuxPaneId, fifo: PathBuf) -> Result<(), CcbdError>;
    pub async fn send_keys_literal(&self, pane: TmuxPaneId, text: String) -> Result<(), CcbdError>;
    pub async fn kill_pane(&self, pane: TmuxPaneId) -> Result<(), CcbdError>;
}
```

注：`spawn_db` 名字略不准（不只是 db 操作了），可考虑改名为 `spawn_blocking_op` 或新增 `tmux::spawn_tmux_op` 同款 helper——交由实施者按命名清晰度决定（详见 §10 Q5）。

### 4.5 命令构造模板（关键命令）

```rust
// ensure_session
Command::new("tmux")
    .args(["-L", &self.socket_name, "new-session", "-d", "-s", session_name,
           "-c", &cwd.display().to_string(), "-x", "200", "-y", "60"])

// spawn_window
Command::new("tmux")
    .args(["-L", &self.socket_name, "new-window", "-d",
           "-t", &format!("{session}:"),
           "-n", window,
           "-c", &cwd.display().to_string(),
           "-P", "-F", "#{pane_id}",
           "--"])
    .args(cmd_args)
// stdout 是 "%N\n"，parse 成 TmuxPaneId

// get_pane_pid
Command::new("tmux")
    .args(["-L", &self.socket_name, "display-message",
           "-p", "-t", &pane.0,
           "#{pane_pid}"])
// stdout 是 "PID\n"，parse 成 i32

// pipe_pane_to_fifo
Command::new("tmux")
    .args(["-L", &self.socket_name, "pipe-pane",
           "-t", &pane.0, "-O",
           &format!("cat > {}", fifo.display())])

// send_keys_literal (-l 模式不解析 keysyms)
Command::new("tmux")
    .args(["-L", &self.socket_name, "send-keys",
           "-t", &pane.0, "-l", text])

// kill_pane
Command::new("tmux")
    .args(["-L", &self.socket_name, "kill-pane", "-t", &pane.0])
```

### 4.6 错误处理（`src/tmux/error.rs`）

```rust
pub enum TmuxError {
    BinaryNotFound,          // tmux 不在 PATH
    CommandFailed { cmd: String, stderr: String, exit: i32 },
    ParsePaneId(String),     // pane_id 不以 "%" 开头
    ParsePid(String),        // pane_pid 不是整数
    Io(std::io::Error),
}

impl From<TmuxError> for CcbdError {
    fn from(e: TmuxError) -> Self {
        match e {
            TmuxError::BinaryNotFound => CcbdError::EnvironmentNotSupported {
                details: "tmux binary not found in PATH".into()
            },
            TmuxError::CommandFailed { cmd, stderr, exit } => CcbdError::TmuxCommandFailed {
                cmd, stderr, exit
            },
            // ...
        }
    }
}
```

`CcbdError` 新增两个变体：`TmuxCommandFailed { cmd, stderr, exit }` 映射 `error_code="TMUX_COMMAND_FAILED"`，`TmuxNotFound` 映射 `error_code="TMUX_NOT_FOUND"`（为运维诊断保留）。

---

## 5. `handle_agent_spawn` 核心流程重写

### 5.1 当前 mvp5 末态流程

```rust
// 简化版
async fn handle_agent_spawn(...) {
    let env_state = ...;
    let bwrap_args = build_args(&sandbox_dir, &overrides)?;
    let agent_command = compose_command(provider, ...);  // bash / mock_agent.sh / 等

    // portable-pty 起 PTY
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(...)?;
    let mut child = pair.slave.spawn_command(...)?;
    let pid = child.process_id().unwrap();

    // pidfd 注册
    let pidfd = pidfd_open(pid)?;
    spawn_agent_pidfd_watch_task(...);

    // 起 PTY reader（mvp5 PTY exempt 模式：spawn_blocking 闭包内 sync DB）
    spawn_pty_reader_task(pair.master, ...);

    // marker timer
    spawn_marker_timer_task(...);

    insert_agent_async(...).await?;
}
```

### 5.2 MVP6 重写后流程

```rust
async fn handle_agent_spawn(...) {
    let env_state = ...;
    let bwrap_args = build_args(&sandbox_dir, &overrides)?;
    let agent_command = compose_command(provider, ...);

    // === 1. 拼装最终 cmd（bwrap 包裹 + agent_command）===
    let mut full_cmd: Vec<String> = vec!["bwrap".into()];
    full_cmd.extend(bwrap_args);
    full_cmd.push("--".into());
    full_cmd.extend(agent_command);

    // === 2. 准备 FIFO 路径 ===
    let fifo_dir = ctx.state_dir.join("pipes");
    tokio::fs::create_dir_all(&fifo_dir).await?;
    let fifo_path = fifo_dir.join(format!("{}.fifo", agent_id));
    // mkfifo 通过 nix（同步调用，spawn_blocking 包）
    crate::db::common::spawn_db("agent_io::mkfifo", {
        let fifo_path = fifo_path.clone();
        move || -> Result<(), CcbdError> {
            nix::unistd::mkfifo(&fifo_path, nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR)
                .map_err(|e| CcbdError::TmuxCommandFailed {
                    cmd: format!("mkfifo {}", fifo_path.display()),
                    stderr: e.to_string(),
                    exit: -1,
                })?;
            Ok(())
        }
    }).await?;

    // === 3. Tmux Spawn (关键原子区) ===
    let tmux = ctx.tmux_server.clone();
    tmux.ensure_session("ccbd-agents".into(), session_dir.clone()).await?;
    let pane_id = tmux.spawn_window(
        "ccbd-agents".into(),
        agent_id.to_string(),
        session_dir.clone(),
        full_cmd,
    ).await?;

    // 失败回滚：以下任一步失败必须 kill_pane + 删 FIFO
    let cleanup = || async {
        let _ = tmux.kill_pane(pane_id.clone()).await;
        let _ = tokio::fs::remove_file(&fifo_path).await;
    };

    let pid = match tmux.get_pane_pid(pane_id.clone()).await {
        Ok(p) => p,
        Err(e) => { cleanup().await; return Err(e); }
    };

    if let Err(e) = tmux.pipe_pane_to_fifo(pane_id.clone(), fifo_path.clone()).await {
        cleanup().await;
        return Err(e);
    }

    // === 4. pidfd 注册 + 写 SQLite ===
    let pidfd = pidfd_open(pid)?;  // 已是 mvp2 现状
    spawn_agent_pidfd_watch_task(agent_id.clone(), pidfd, ...);
    db::agents::insert_agent(ctx.db.clone(), agent_id.clone(), ..., Some(pid as i64)).await?;

    // === 5. I/O 挂载（替代 spawn_pty_reader_task）===
    // FIFO 必须 RW 打开，否则 reader 会阻塞——见 5.3
    let fifo_file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)            // 关键：保留 writer 端阻止 EOF
        .open(&fifo_path)
        .await?;
    spawn_agent_io_reader_task(agent_id.clone(), fifo_file, ctx.db.clone(), parser_handle.clone());

    // === 6. 注册 pane_id 到内存映射（替代 PTY_MAP）===
    TMUX_PANE_MAP.insert(agent_id.clone(), pane_id);

    // === 7. marker timer ===
    spawn_marker_timer_task(agent_id, TimerKind::Startup, db.clone(), parser_handle);

    Ok(json!({ "state": "SPAWNING", "pid": pid }))
}
```

### 5.3 FIFO open 的关键陷阱与正确顺序（Round 1 反馈采纳）

Linux 命名管道有两个语义陷阱：

1. **reader 端 `open(O_RDONLY)` 会阻塞直到有 writer 端打开**——反之亦然
2. **writer 端最后一个 close 时 reader 会 EOF**——如果 reader 持有的是 RDONLY 句柄

**正确顺序**（必须严格遵守，不可调换）：

```
1. mkfifo()           # daemon 创建命名管道
2. daemon open(RW)    # daemon 自己同时持有读+写两端 fd（O_RDWR）
                      # 此时 daemon 已是 writer 之一，所以 reader 端不会阻塞
                      # 反之亦然：daemon 也是 reader 之一，writer 不会阻塞
3. tmux pipe-pane     # 让 tmux 起 'cat > fifo' 作为额外 writer
4. agent_io reader    # daemon 用同一个 fd 进 BufReader 异步读
```

**为什么 Step 2 要先于 Step 3**：

- 如果先 Step 3（tmux 启动 cat 作 writer）→ cat 的 open(O_WRONLY) 会**阻塞**直到有 reader 端打开 fifo
- 但 daemon 这时还没 open，没有 reader 在场
- 死锁：tmux cat 阻塞在 open / daemon 等 spawn-pane 命令返回但永不返回
- **顺序倒转就死锁**——必须 daemon 先持 RW fd，再让 tmux 接 writer

**为什么持 O_RDWR 而非 O_RDONLY**：

- daemon 自己是 reader，但同时也作 writer 端持有引用 → tmux cat 进程退出时 writer 端引用未消失，reader 不会假 EOF
- daemon 持续 read 直到自己主动关闭 fd
- daemon shutdown 时关 fd → 真 EOF → reader_task 退出循环

### 5.3.1 实施要点

```rust
// 必须严格按此顺序，每步成功后才进下一步：
nix::unistd::mkfifo(&fifo_path, ...)?;                                   // (1)
let fifo_file = tokio::fs::OpenOptions::new()
    .read(true).write(true).open(&fifo_path).await?;                     // (2) RW open
ctx.tmux_server.pipe_pane_to_fifo(pane_id, fifo_path.clone()).await?;    // (3) tmux 接 writer
spawn_agent_io_reader_task(agent_id, fifo_file, db, parser);             // (4) reader task

### 5.4 `agent.send` 改造（Round 1 send-keys 语义验证补充）

mvp1-5 的 `agent.send` 通过 PTY master fd write。MVP6 改为：

```rust
async fn handle_agent_send(...) {
    let pane_id = TMUX_PANE_MAP.get(agent_id).ok_or(AgentNotFound)?.clone();
    send_text_to_pane(&ctx.tmux_server, &pane_id, text).await?;
    // 后续走原有事务逻辑（record_send_progress）
}
```

#### 5.4.1 `tmux send-keys` 模式选择 + 换行语义

`tmux send-keys` 有两种关键模式：
- **`-l` literal**：text 字面字节直发，不解析任何 keysym。`\n`（0x0A）当作普通字节
- **不加 `-l`** 默认模式：tmux 解析 keysym（如 `Enter` / `C-c` / `Tab`），其他字符按字面发

mvp1-5 的 `agent.send` 入参 `text` 通常以 `\n` 结尾触发 shell 行输入。**`\n` 字节（0x0A = `\r` 不是；是 LF）在 PTY 上是 cooked-mode "newline"**，等价于按 Enter。

**风险**：tmux send-keys 模式下 LF 是否被 PTY 视为 newline，取决于 PTY 的 cooked/raw mode 设置 + tmux 是否做特殊处理。**没有真实验证就当假设是危险的**。

#### 5.4.2 实施策略：分两步发送（Codex Round 1 fallback 建议采纳）

为保证 mock_agent.sh + bash + future 真 agent 都行为一致，**`send_text_to_pane` 实现采用安全分两步**：

```rust
async fn send_text_to_pane(tmux: &TmuxServer, pane: &TmuxPaneId, text: String) -> Result<(), CcbdError> {
    // 1. 拆分：去掉所有尾随 \n，但保留中间 \n
    let trimmed = text.trim_end_matches('\n');
    let trailing_newlines = text.len() - trimmed.len();

    // 2. literal 发送 trimmed 文本（含中间 \n）
    if !trimmed.is_empty() {
        tmux.send_keys_literal(pane.clone(), trimmed.to_string()).await?;
    }

    // 3. 每个尾随 \n 用 Enter keysym 发送（确保 cooked-mode 行触发）
    for _ in 0..trailing_newlines {
        tmux.send_keys_keysym(pane.clone(), "Enter".into()).await?;
    }
    Ok(())
}
```

新增 `tmux::TmuxServer::send_keys_keysym(pane, "Enter")` 方法：构造 `tmux send-keys -t <pane> Enter`（不加 `-l`，让 tmux 解析 keysym）。

#### 5.4.3 测试验证（mvp6_acceptance 必须覆盖）

```rust
// tests/mvp6_acceptance.rs
#[test]
fn test_send_text_with_trailing_newline_triggers_shell_eval() {
    // spawn bash agent
    // send "echo hello\n"
    // expect events 中 output_chunk 含 "hello"（说明 echo 真执行了，shell 响应了 newline）
    // 不能只验证 send 成功——必须看到 echo 输出
}

#[test]
fn test_send_text_with_ctrl_c_via_keysym() {
    // 后续可加：拓展 send 入参支持 keysym（mvp7 范围）
    // 本期至少覆盖 newline 通路
}
```

如果 acceptance 测试发现 fallback 方案有问题，回退到纯 `-l` 模式 + 改 mock_agent.sh 适配。

### 5.5 失败回滚原则

任一 spawn 子步骤失败 → 必须 cleanup：kill_pane + remove fifo + 不写 SQLite agent 记录。这是为了保证不留 zombie pane / 孤儿 FIFO 文件。

---

## 6. `src/pty/` 模块切除 + `src/agent_io/` 新增

### 6.1 切除 src/pty/

```bash
rm -rf src/pty/
```

`src/pty/mod.rs` 和 `src/pty/tasks.rs` 整体物理删除。`src/lib.rs` 移除 `pub mod pty;`。

### 6.2 新建 src/agent_io/

```
src/agent_io/
├── mod.rs        # 顶层导出 + TMUX_PANE_MAP 全局注册表
├── reader.rs     # spawn_agent_io_reader_task（替代 spawn_pty_reader_task）
└── writer.rs     # send_to_pane wrappers (薄封装 tmux send_keys_literal)
```

### 6.3 reader_task 关键改造

```rust
// src/agent_io/reader.rs
pub fn spawn_agent_io_reader_task(
    agent_id: String,
    fifo: tokio::fs::File,
    db: Db,
    parser: ParserHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {  // 注意：tokio::spawn（不是 spawn_blocking）
        let mut reader = tokio::io::BufReader::new(fifo);
        let mut buf = vec![0u8; 4096];
        loop {
            let n = match reader.read(&mut buf).await {
                Ok(0) => continue,        // EOF on RW-open FIFO 不会发生；保险
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(?e, agent_id, "fifo read failed");
                    break;
                }
            };
            // parser.lock() 是 std::sync::Mutex —— 闭包内仅同步访问后立刻 drop guard，
            // 不能跨 await。每次 process / scan 在独立 scope。
            let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
            {
                let mut p = parser.lock().unwrap();
                p.process(&buf[..n]);
            }

            // db::events::insert_event_async 签名（mvp5 末态）：
            //   pub async fn insert_event(
            //       db: Db,
            //       agent_id: String,
            //       request_id: Option<String>,
            //       event_type: String,
            //       payload: String,
            //   ) -> Result<i64, CcbdError>
            // 注意参数顺序：request_id 在 event_type 之前
            let _ = db::events::insert_event(
                db.clone(),
                agent_id.clone(),
                None,                         // request_id
                "output_chunk".into(),
                json!({"text": chunk}).to_string(),
            ).await;

            // marker matcher 检查（同步访问 parser，scope 内立刻 drop）
            let matched = {
                let p = parser.lock().unwrap();
                MarkerMatcher::default().scan(&p)
            };
            if matched == MatchResult::Matched {
                let _ = db::state_machine::mark_agent_idle_matched(
                    db.clone(),
                    agent_id.clone(),
                ).await;
                marker::registry::reset(&agent_id);
            }
        }
    })
}
```

**关键：** mvp5 的 `R-PTY-EXEMPT-1`（PTY reader 在 spawn_blocking 闭包内合法调 sync DB）**MVP6 后失效** —— 因为 reader 不再在 spawn_blocking 闭包内，是纯 async tokio task。所有 db 调用走 async wrapper（`mvp5` `_sync` / 同名 async 双层结构正常工作）。R-PTY-EXEMPT-1 在 R 文档里标记 "MVP6 后自然废弃"。

**重要：T2.2 必须先在 `src/db/state_machine.rs` 新增 `mark_agent_idle_matched` async wrapper**（mvp5 因 PTY exempt 故意没出，MVP6 reader 切 async 后必须出）。模板：

```rust
// src/db/state_machine.rs (MVP6 新增)
pub async fn mark_agent_idle_matched(db: Db, agent_id: String) -> Result<usize, CcbdError> {
    crate::db::common::spawn_db("state_machine::mark_agent_idle_matched", move || {
        mark_agent_idle_matched_sync(&db, &agent_id)
    }).await
}
```

**Mutex 跨 await 防护**：上述 reader 代码用 `std::sync::Mutex<vt100::Parser>`（mvp3 现状），闭包内 `{ }` scope 显式控制 lock guard 生命周期，**禁止 lock guard 跨 `.await`**——否则 `MutexGuard: !Send` 会让 `tokio::spawn` 的 future 也 `!Send`，编译错。如果实施时发现编译错，要么用 scope 显式 drop，要么换 `tokio::sync::Mutex`（更重）。推荐 scope 方案。

### 6.4 调用方更新

| 文件 | 当前 use 路径 | MVP6 目标 |
|---|---|---|
| `src/bin/ccbd.rs` (原 main.rs) | 无 pty 引用 | 新增 tmux_server 初始化（在 rpc::Ctx 内） |
| `src/rpc/handlers.rs` | `crate::pty::*` | `crate::tmux::*` + `crate::agent_io::*` |
| `src/marker/timer.rs` | `crate::pty::*`（如有）| `crate::agent_io::*` |
| `src/marker/matcher.rs` | 同上 | 同上 |

---

## 7. mvp2/3/4/5 acceptance 测试改造

### 7.1 影响评估

`portable-pty` 直接读 master fd 与 tmux `pipe-pane -O 'cat > fifo'` 的输出**字节流应该一致**——两条路径都是从同一个 OS PTY 端流出的字节，包含 mock_agent.sh 的 ANSI escape codes（没有，因为 mock_agent.sh 是纯 echo）+ shell prompt（`$`）。`vt100::Parser` 解析逻辑、marker regex 对接、events 顺序与 payload 内容**预期完全兼容**。

### 7.2 改造点

```text
tests/mvp2_acceptance.rs:
  - 场景：spawn agent（用 mock_agent.sh）→ 等 IDLE → send 命令 → read events → kill
  - 改动：mock_agent.sh 自身零修改
  - 测试代码：assert spawn 后 agent 在 tmux pane（而非 portable-pty）
    可加新断言：调 tmux list-panes 看到对应 pane_id
  - 业务断言：state_change / events 顺序 / output_chunk 内容 全部不允许放宽

tests/mvp3_acceptance.rs / mvp4_acceptance.rs:
  - 同上：仅断言机制不变，仅 spawn 物理拓扑变了
  - vt100 marker / mark_agent_unknown / agent.assert_state 等高阶逻辑保持
```

### 7.3 CI 环境约束

acceptance 测试现在硬依赖宿主有 tmux 二进制。本地 dev 环境已具备（`which tmux` 通过），CI（GitHub Actions / GitLab CI）需 `apt-get install -y tmux` 或 docker image 内置。

### 7.4 新增 mvp6_acceptance.rs

覆盖 R AC2-AC5 的关键路径：

```text
tests/mvp6_acceptance.rs:
  - test_no_portable_pty_in_cargo_lock
    grep Cargo.lock 验证 portable-pty 不再存在
  - test_tmux_pane_created
    spawn agent 后调 tmux list-panes -t ccbd-agents 看到对应 window
  - test_pane_pid_matches_agent_pid
    spawn 后 SQLite agents.pid 与 tmux display-message #{pane_pid} 一致
  - test_pipe_pane_drains_to_fifo
    向 pane send 一段已知文本，验证 events 表里出现 output_chunk
  - test_kill_pane_triggers_crashed
    手动 tmux kill-pane → pidfd watcher 捕获 → state_change to CRASHED
  - test_ccb_ping_smoke / test_ccb_ps_smoke（如果方便）
    跑 cargo run --bin ccb ping，验证 stdout 含 "ok=true"
```

---

## 8. CcbdError 新增错误码

```rust
pub enum CcbdError {
    // 现有变体保留 ...
    TmuxNotFound,
    TmuxCommandFailed { cmd: String, stderr: String, exit: i32 },
}
```

| 错误码 | message | code | 触发场景 |
|---|---|---|---|
| `TMUX_NOT_FOUND` | "tmux binary not found in PATH" | -32000 | daemon 启动检测 / spawn 时 |
| `TMUX_COMMAND_FAILED` | "tmux command failed: {cmd}" | -32000 | tmux subprocess 退出码非 0 |

向后兼容：仅新增，不动既有错误码。

---

## 9. Cargo.lock 与依赖清理

### 9.1 移除 portable-pty

```toml
# Cargo.toml diff
- portable-pty = "0.8.1"
```

`cargo build` 后 Cargo.lock 自动 regen（移除 portable-pty 及其传递依赖 mio / nix-old 等）。

### 9.2 新增依赖审计

| crate | 用途 | 风险 |
|---|---|---|
| clap 4.5 | CLI argument parsing | 成熟、被 rust-lang 自己用、兼容性好 |
| tabled 0.15 | terminal table output | 仅 ccb ps 用，不进 daemon binary |
| nix 0.28 | mkfifo / unistd | mvp1-5 已用过 nix 子集，是延伸 |

无新外部 system 依赖（除 tmux binary 本身）。

---

## 10. Open Questions（已决断）

| # | 问题 | 决断 | 理由 |
|---|---|---|---|
| Q1 | tmux pipe-pane 输出怎么 drain？| **`tmux pipe-pane -O 'cat > <fifo>'` + `tokio::fs::File` 异步读 FIFO** | 最 Unix 哲学。mkfifo 建通道，tmux cat 写入，daemon tokio AsyncRead 流式读。无中间缓冲泄漏 |
| Q2 | tmux server socket 策略 | **自定义 socket: `tmux -L ccbd-<state_dir_hash>`** | 必须与用户个人 tmux 隔离；崩溃时可 `kill $(pidof tmux ...ccbd-...)` 清理而不影响用户 |
| Q3 | session / window 命名 | **单 session `ccbd-agents`，每 agent 一个 window 名 `<agent_id>`** | 用户 attach 后用 `Ctrl+B N` 遍历各 agent 现场 |
| Q4 | 用户 attach 入口暴露 | **`ccb ps` 输出底部加 hint** | DX 友好；不通过 daemon 包装 `ccb attach` 避免终端接管复杂度 |
| Q5 | spawn_db helper 改名？| **不改名，直接复用** | mvp5 `db::common::spawn_db("op_name", closure)` 已是通用 spawn_blocking helper，op_name 字符串支持任意命名空间（"tmux::spawn_window" 等）；改名会破坏 mvp5 现状 |
| Q6 | TMUX_PANE_MAP 数据结构 | **`Arc<Mutex<HashMap<String, TmuxPaneId>>>`，遵循 mvp3 PARSER_REGISTRY / mvp4 MARKER_TIMER_REGISTRY 同款 LazyLock 模式** | 风格统一 |
| Q7 | tmux 命令调用是否经 spawn_blocking？| **是，全部经 spawn_db 包装** | mvp5 R-RUNTIME-1 异步硬化要求；tmux subprocess 启动是阻塞 syscall |

---

## 11. 兼容性与实施时长

| 维度 | 兼容性 |
|---|---|
| RPC schema | 完全一致（仅可能新增 `system.ping` 方法） |
| 错误码 | 仅新增 `TMUX_NOT_FOUND` / `TMUX_COMMAND_FAILED`（向后兼容扩展）|
| 状态机 | 完全一致（物理拓扑变，语义不变）|
| schema | 完全一致 |
| mvp5 spawn_db 事务边界 | 保持，扩展用于 tmux 操作 |

**实施时长预期**：

| 子阶段 | 工作量 | 回滚成本 |
|---|---|---|
| G6.0 CLI Skeleton | 3-4 小时 | 低（删 src/bin/ccb.rs + 改 Cargo.toml）|
| G6.1 Tmux Wrapper | 4-6 小时 | 低（删 src/tmux/）|
| G6.2 Spawn Surgery | 6-8 小时 | 中（事务边界改动较大；若 acceptance 红，回到 G6.1 末态保留收益） |

总计 13-18 小时（按 AI vibecoding 节奏 1-2 天）。

---

## 12. 实施时回滚路径

阶段独立 commit（mvp5 同款）：

- 阶段 G6.0 commit：`feat(mvp6): G6.0 CLI skeleton (ccb ping / ps)`
- 阶段 G6.1 commit：`feat(mvp6): G6.1 tmux wrapper module`
- 阶段 G6.2 commit：`refactor(mvp6): G6.2 portable-pty -> tmux pivot for agent spawn`

任一 commit 后发现失败：`git reset --soft HEAD~1` 保留改动，检查后 checkout 丢弃。

mvp5 末态（commit `4f4b829`）作为 hard fallback——重大架构问题时整 mvp6 回退。
