# Kiro Requirements: MVP 7 (语义对接与物化挂载 / The Semantic & Auth Pivot)

> **文档定位**：本文件是 ccbd-rust 从“跑通底层基建”迈向“接管真实 LLM-CLI 负载”的决定性阶段（MVP 7）的官方 R (Requirements) 规格。本阶段直接针对 MVP6 暴露出的三个生产级 Blocking Risks，解决多行粘贴、TUI 语义检测、以及沙盒 Auth 物化问题。

---

## 0. 立项背景与边界共识

### 0.1 为什么必须做这个 MVP（核心驱动）
在 MVP6 完结时，`ccbd-rust` 展现了完美的底层 tmux 拓扑和并发隔离，但面对真实的 Codex / Gemini / Claude 负载时，直接被判定为 `production_ready = false`。原因在于系统的**“业务语义”仍停留在 Bash 时代**：
1. **输入崩塌**：将多行代码拆分并逐行发送 `Enter` 的策略，会彻底破坏 TUI（Terminal UI）的输入逻辑，导致 Agent 收到的指令支离破碎。
2. **鉴权真空**：沙盒隔离得太干净，把宿主的 `~/.config` 和凭证全部墙在外面，导致 Gemini / Claude 刚启动就卡死在 `Waiting for authentication...`。
3. **眼盲症**：TUI 会在屏幕中央画框，甚至带有游标跳动动画，而 MVP1-5 遗留的“只看最后一行是否包含正则”逻辑，对 TUI 的闲置状态（IDLE）完全失效（触发 Timeout）。

用户已下达明确指令：“替代现在的 ccb 并且比他更好”。要做到这点，必须在 Rust 侧复刻并超越旧 Python ccb 中的 `manifest.py` (Provider 配置) 和 `profiles.py` (语义检测) 能力。本 MVP 的使命就是让真 Codex 和 Gemini CLI 在 playground 的 smoke test 中全绿。

### 0.2 本 MVP **不做**的事（留给 MVP8/MVP9）
- **高阶任务编排**：Mailbox / 异步 Job 队列 / `ccb ask` / `ccb pend` / `ccb watch` 等等，统统推迟到 MVP8。本阶段依然只暴露出底层的 `agent.send` 和 `agent.read`。
- **环境对账与自启**：多项目自动 `reconcile` 唤起缺失 Agent，以及 `ccb start` 自动 4-pane 布局，推迟到 MVP9。
- **动态 TOML Provider 加载**：本阶段不引入支持用户自定义 Provider 的 TOML 加载器，仅针对官方 1st-party Providers (codex/gemini/claude/bash) 进行内部硬编码。

### 0.3 与上下游 MVP 的关系
- **承上（MVP6）**：完全继承 tmux 隔离与 FIFO pipe-pane 读取机制，不动物理拓扑。
- **启下（MVP8）**：只有当 Agent 的 IDLE 状态被精准识别且输入无损提交时，MVP8 的 Mailbox 队列才有基座可言。

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 7 验收必须全部通过：

1. **AC1 [Atomic Paste-Buffer] 多行原子发送**：`agent.send` 废弃现有的 `split('\n') + keysym(Enter)` 实现。必须重构为：将文本加载入 tmux 的隔离 buffer，并通过 `paste-buffer -p`（带 Bracketed Paste 标记）一次性发送至目标 pane。多行长文本必须作为一个原子块送入，中间不得触发任何意料外的子提交。
2. **AC2 [Auth Materialization] 凭证安全物化**：引入 Provider 生命周期钩子，在拼装 `bwrap` args 时，根据 Provider 种类注入特定的 `--ro-bind` 参数（例如将宿主的 `~/.config/gemini/` 或 `~/.codex/` 挂载入沙盒的同等位置），确保 `unsafe_no_sandbox=false` 模式下真 Agent 能获取到已认证的 Token。
3. **AC3 [2D Screen Semantic] TUI 语义感知**：放弃单一的“最后一行正则匹配”。利用 `vt100::Parser` 驻留的 200x200 屏幕缓冲区，支持对全屏内容的 2D 扫描（Screen Regex），并结合“屏幕视觉稳定 N 毫秒（Observed Stability）”机制，准确识别出 Codex 的 `>_` 和 Gemini 的 `✦` 提示符。
4. **AC4 [Provider Manifest Registry] 静态配置表**：Rust 侧内置一个强类型的 `ProviderManifest` 静态映射表（类似旧版 `profiles.py`），为 `bash`, `codex`, `gemini`, `claude` 提供各自的 Sandbox Mount Specs, Idle Detection Specs 和 Terminal Envs。
5. **AC5 [True Smoke Tests Green] 真实现场全绿**：在 playground 环境下，执行 `smoke-codex.sh` 和 `smoke-gemini.sh`，不仅必须能够 SPAWNING -> IDLE，且进行发送多行代码查询后，必须能再次安全回落至 IDLE。`STARTUP_MARKER_TIMEOUT` 和鉴权失败的报错彻底消失。

---

## 2. 状态机激活范围 (Delta)

核心状态节点及 CAS 流转协议 **不变**。
发生变化的是 **“感知侧 (Sensor)” 判定 UNKNOWN / BUSY -> IDLE 的触发条件**：
- **Before**：仅依靠基于 stream 的单行正则 `MarkerMatcher` 瞬间命中。
- **After**：变为 `Matcher` 初步命中屏幕特征，且触发 `StabilityTimer`。只有当屏幕在这 N 毫秒内不再接收到新字节变更，才最终 `assert` 为 IDLE 状态（有效规避 LLM 生成代码期间偶然吐出 `>_` 字符串导致的 False IDLE）。

---

## 3. R-* 需求切割矩阵更新 (Scope Definitions)

| Req ID | Description | MVP 1-6 状态 | MVP 7 更新状态 | 备注 |
|---|---|---|---|---|
| **R-SEND-1** | PTY 多行安全提交 | 🔴 Broken | 🟢 **Full** | 引入 `tmux load-buffer` + `paste-buffer -p` 原子提交 |
| **R-MARKER-1** | Agent 闲置语义探测 | 🟡 Partial | 🟢 **Full** | 从单维正则演进为 2D 屏幕感知 + 视觉防抖稳定 (Stability) 检测 |
| **R-AUTH-1** | Provider 凭证沙盒挂载 | ⚪ N/A | 🟢 **Full** | MVP7 新增：按 Manifest 规范安全挂载宿主 `.config` 等凭据目录 |
| R-TMUX-1 | Tmux Pane 托管 | 🟡 Partial | 🟡 Partial | 维持原状。 |
| R-STATE-* | 核心状态机韧性 | 🟢 Full | 🟢 Full | 保持不动。 |

---

## 4. 范围分阶段（实施视角）

建议按以下三个独立的物理边界进行实施，每个阶段独立可测。

### G7.0：Atomic Tmux Paste (消除 Send 破损)
- 修改 `src/agent_io/writer.rs`。
- 实现安全的 `tmux load-buffer -b <uuid> -` (通过 `stdin` pipe 写入避免 shell 逃逸漏洞) 以及 `tmux paste-buffer -p -t <pane> -b <uuid>`。
- **安全检查点**：接受含有 Bash 循环控制的多行字符串，验证没有被截断。

### G7.1：Provider Registry & Auth Materialization (消除鉴权破损)
- 创建 `src/provider/manifest.rs` 定义 `ProviderManifest`。
- 提取宿主常见 Auth 目录，并在 `handle_agent_spawn` -> `bwrap::build_args` 环节按需追加 `--ro-bind` 参数。
- **安全检查点**：`smoke-gemini.sh` 启动不再卡死在 OAuth 界面（能在沙盒内读到宿主 token）。

### G7.2：TUI 2D Semantic Sensor (消除眼盲)
- 修改 `src/marker/matcher.rs` 和 `src/marker/timer.rs`。
- 将 `vt100::Parser` 的 `screen().contents_formatted()` 暴露出来支持正则匹配。
- 引入 Observed Stability Timer（命中后暂存结果，需经历 `CCB_OBSERVED_POLL_MS` 无新输出才转正为 IDLE）。
- **安全检查点**：`smoke-codex.sh` 和 `smoke-gemini.sh` 能够完美识别 TUI Prompt 并在回复结束后准确转移回 IDLE 状态。

---

## 5. 跟前后 MVP 的接口约束

- **JSON-RPC schema**：绝对不可变。客户端发送请求的方式和载荷保持一致。
- **SQLite 范式**：保持不变。`output_chunk` 依旧作为字节流存档，不受 2D Screen 机制影响。

---

## 6. 核心架构决断 (Architectural Decisions / Open Questions)

此部分直接回应计划评审期间存在的 4 个架构不确定性：

### 决断 1：多行 Send 机制选择
**决定：采用 `tmux load-buffer -b <name> -` (stdin 传递) + `tmux paste-buffer -p`**。
**理由**：
- 为什么不用 Shell 命令行传参 (`load-buffer -b name "text"`)？因为超长代码（比如 50KB）会超出 `execve` 的参数大小限制 (ARG_MAX)，且包含复杂的引号、控制字符易导致逃逸漏洞。
- 必须通过生成 Rust `Command` 并开启 `.stdin(Stdio::piped())` 将要发送的 text 以流的方式喂给 tmux buffer。
- `-p` 参数至关重要 (Bracketed Paste)，它能告诉 Codex/Gemini TUI “这是一整块粘贴的输入，不要对中间的换行符执行回车运算”。

### 决断 2：Per-Provider Config 的表达方式
**决定：使用静态硬编码的 `LazyLock<HashMap<&'static str, ProviderManifest>>` (Rust 原生枚举/结构体)。**
**理由**：
- `ccbd-rust` 作为高稳态系统，首要支持的是第一方固化 Provider（bash, codex, claude, gemini）。
- 引入外部 TOML 读取会立刻陷入“路径发现、文件权限、格式验证、热更新重载”的泥潭，严重拉伸工期。
- 采用原生的 Rust 结构体，不仅拥有编译期类型检查，在未来 MVP 若需扩充时，再从 TOML 解析后 merge 进这个注册表即可（分离了机制与策略）。

### 决断 3：TUI Marker 检测的实现策略
**决定：`vt100` 全屏正则扫描 (Screen Regex) + 输出稳定防抖 (Observed Stability Timer)**。
**理由**：
- 旧 CCB 针对 codex 用的 `ANCHORED_SESSION_STABILITY` 是通过读取 agent 落盘的 `session.json` 来判断状态。此法需要深入各家 Agent 源码的内部文件协议，且强耦合，不适合 Rust L2 守护进程维护。
- `vt100` crate 在后台时刻维护着一个 200x200 的 Terminal Screen Buffer。无论 prompt 是画在最后一行，还是像 Gemini 那样悬浮在画面中下部，只需对 `screen().contents()` 跑一次正则即可捕获特定占位符（如 `>_` 或 `✦`）。
- 为了防止模型恰好吐出包含 Prompt 特征的正常代码，必须加入时间防抖：当正则命中后，`ccbd` 等待 N 毫秒（如 300ms）。如果在等待期 PTY 收到了新字节，则取消命中状态（判定为模型仍在输出）；若 N 毫秒内毫无动静，才确认转入 IDLE。

### 决断 4：Auth 物化范围与无沙盒模式处理
**决定：基于 Manifest 配置，精确 RO 挂载；`unsafe_no_sandbox` 模式仅需确保 `$HOME` 环境变量传递。**
**理由**：
- 直接把整个 `~` 挂进 `bwrap` 过于危险且打破隔离初衷。必须在 `ProviderManifest` 中定义具体的数组（例如 Gemini 需挂载 `~/.config/gemini/` 和 `~/.config/gcloud/`）。`bwrap` 启动时动态通过 `std::path::Path::new` 检查存在性，若存在则拼接 `--ro-bind <src> <src>`。
- 当处于 `unsafe_no_sandbox` 模式（MVP2/4 测试场景）时，进程本来就跑在宿主真实用户空间，拥有全部权限。此时无需挂载，只需在 `Command` 中 `env_clear()` 之后重新补齐当前的 `HOME`, `USER`, `PATH` 即可。
