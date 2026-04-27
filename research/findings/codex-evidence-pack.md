# Codex Evidence Pack: Reference Materials

生成时间：2026-04-26

范围：
- `docs/agent-cli-knowledge-base/`
- `research/sessions/home-sevenx/markdown/`
- `research/sessions/agent-harness/markdown/`
- `research/candidates/`

## 类 1：三家 CLI Agent 文档行为规范摘录

| 维度 | Provider | 文档原文 | docs file:line |
| --- | --- | --- | --- |
| resume 命令 | codex | `resume       Resume a previous interactive session (picker by default; use --last to continue the` | `docs/agent-cli-knowledge-base/codex/cli-reference.md:35` |
| sandbox 命令 | codex | `sandbox      Run commands within a Codex-provided sandbox` | `docs/agent-cli-knowledge-base/codex/cli-reference.md:31` |
| sandbox 模式 | codex | `-s, --sandbox <SANDBOX_MODE>` / `Select the sandbox policy to use when executing model-generated shell commands` | `docs/agent-cli-knowledge-base/codex/cli-reference.md:88-89` |
| completion | codex | `completion   Generate shell completion scripts` | `docs/agent-cli-knowledge-base/codex/cli-reference.md:30` |
| exec resume | codex | `Usage: codex exec resume [OPTIONS] [SESSION_ID] [PROMPT]` | `docs/agent-cli-knowledge-base/codex/cli-reference.md:240` |
| output stream | codex | `Codex writes formatted output by default. Add \`--json\` to receive newline-delimited JSON events (one per state change).` | `docs/agent-cli-knowledge-base/codex/cli-reference.md:2710` |
| auth credentials | codex | `Remove saved credentials for both API key and ChatGPT authentication. This command has no flags.` | `docs/agent-cli-knowledge-base/codex/cli-reference.md:2730` |
| hooks lifecycle | claude-code | `Hooks fire at specific points during a Claude Code session. When an event fires and a matcher matches, Claude Code passes JSON context about the event to your hook handler.` | `docs/agent-cli-knowledge-base/claude-code/hooks.md:23` |
| hooks events | claude-code | `PreToolUse - Before a tool call executes. Can block it` | `docs/agent-cli-knowledge-base/claude-code/hooks.md:38` |
| session start sources | claude-code | `resume - --resume, --continue, or /resume` | `docs/agent-cli-knowledge-base/claude-code/hooks.md:744` |
| hook stdin | claude-code | `For example, a \`PreToolUse\` hook for a Bash command receives this on stdin:` | `docs/agent-cli-knowledge-base/claude-code/hooks.md:539` |
| hook exit 2 | claude-code | `**Exit 2** means a blocking error. Claude Code ignores stdout and any JSON in it. Instead, stderr text is fed back to Claude as an error message.` | `docs/agent-cli-knowledge-base/claude-code/hooks.md:563` |
| PreToolUse decision | claude-code | `PreToolUse hooks can control whether a tool call proceeds.` | `docs/agent-cli-knowledge-base/claude-code/hooks.md:1078` |
| PostToolUse input | claude-code | `PostToolUse hooks fire after a tool has already executed successfully. The input includes both tool_input and tool_response.` | `docs/agent-cli-knowledge-base/claude-code/hooks.md:1249` |
| resume flag | gemini-cli | `--resume: Resume a previous session. Use "latest" for most recent or index number.` | `docs/agent-cli-knowledge-base/gemini-cli/cli-reference.md:72` |
| output format | gemini-cli | `--output-format: The format of the CLI output. Choices: text, json, stream-json` | `docs/agent-cli-knowledge-base/gemini-cli/cli-reference.md:77` |
| slash /clear | gemini-cli | `### \`/clear\`` | `docs/agent-cli-knowledge-base/gemini-cli/slash-commands.md:108` |
| slash /compress | gemini-cli | `### \`/compress\`` | `docs/agent-cli-knowledge-base/gemini-cli/slash-commands.md:127` |
| slash /resume | gemini-cli | `### \`/resume\`` | `docs/agent-cli-knowledge-base/gemini-cli/slash-commands.md:355` |
| at commands | gemini-cli | `At commands are used to include the content of files or directories as part of` | `docs/agent-cli-knowledge-base/gemini-cli/slash-commands.md:517` |
| shell mode | gemini-cli | `## Shell mode and passthrough commands (\`!\`)` | `docs/agent-cli-knowledge-base/gemini-cli/slash-commands.md:561` |
| hooks SessionStart | gemini-cli | `Fires on application startup, resuming a session, or after a \`/clear\` command.` | `docs/agent-cli-knowledge-base/gemini-cli/hooks.md:427` |
| auth API key | gemini-cli | `To authenticate and use Gemini CLI with a Gemini API key:` | `docs/agent-cli-knowledge-base/gemini-cli/auth-and-quotas.md:89` |
| sandbox enable order | gemini-cli | `1. **Command flag**: \`-s\` or \`--sandbox\`` | `docs/agent-cli-knowledge-base/gemini-cli/sandboxing.md:236` |

## 类 2：18 天 Corpus Design Observations

### O-01 OAuth scope 记录
- **类别**: A4 协议
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-20-session.md:82`
- **原文**: > `Scopes: ['user:file_upload', 'user:inference', 'user:mcp_servers', 'user:profile', 'user:sessions:claude_code']`

### O-02 checkpoint resume 参数
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:350`
- **原文**: > `thread_id: Optional thread_id for checkpoint resume.`

### O-03 trace 输出路径
- **类别**: A6 观测
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:360`
- **原文**: > `- ``trace_path``: Path to trace.json (if TracingCallback active)`

### O-04 run_id 写入
- **类别**: A1 数据一致性
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:420`
- **原文**: > `# Write .run_id for potential resume`

### O-05 unexpected failure 清理 run_id
- **类别**: A1 数据一致性
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:437`
- **原文**: > `# Clean up .run_id on unexpected failure to avoid corrupted resume`

### O-06 auto-checkpointer failure
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:761`
- **原文**: > `13:35:32 WARNING graph_agent.core.harness: [Harness] Auto-checkpointer failed, running without: cannot import name 'override' from 'typing' (/Library/Frameworks/Python.framework/Versions/3.10/lib/python3.10/typing.py)`

### O-07 trace saved
- **类别**: A6 观测
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:822`
- **原文**: > `13:37:13 INFO graph_agent.callbacks.tracing: [TracingCallback] Saved trace to output/test_run/traces/5e6e37686e6a_summary.json`

### O-08 并发工具路径
- **类别**: A2 并发
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:1551`
- **原文**: > `Agent 确实非常聪明地调用了 DeerFlow 原生的 task_tool 并发派发子任务。`

### O-09 手写并发 grep 命中
- **类别**: A2 并发
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:1987`
- **原文**: > `/Users/sevenx/Documents/coding/AI-story-forge/tests/skills/adaptation_v1_sandbox//skill_workspace/tools/beat_dispatcher.py:61:    with ThreadPoolExecutor(max_workers=4) as executor:`

### O-10 sandbox acquire
- **类别**: A3 沙盒
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:2044`
- **原文**: > `60	            sandbox_id = self._acquire_sandbox(thread_id)`

### O-11 sandbox release
- **类别**: A3 沙盒
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:2055`
- **原文**: > `71	            get_sandbox_provider().release(sandbox_id)`

### O-12 background task status
- **类别**: A6 观测
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:1709`
- **原文**: > `143	            logger.info(f"[trace={trace_id}] Task {task_id} status: {result.status.value}")`

### O-13 task_running event
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:1720`
- **原文**: > `154	                        "type": "task_running",`

### O-14 子图隔离需求
- **类别**: A6 观测
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:2978`
- **原文**: > `53	    # 注：必须生成一个新的 thread_id 或隔离的 trace_dir 以防日志污染`

### O-15 并发隔离执行
- **类别**: A2 并发
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:2993`
- **原文**: > `68	*   **并发支持 (Parallel Execution)**：如果大模型在一次思考中（Parallel Tool Calling）同时调用了三次 \`ask_producer_review\`（比如针对三场不同的戏），框架底层必须能够并发拉起三个隔离的 \`GraphAgentHarness\`，极大提升吞吐量。`

### O-16 nested observability
- **类别**: A6 观测
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:2994`
- **原文**: > `69	*   **嵌套 Tracing (Nested Observability)**：日志系统应支持层级化。在 UI 或 JSONL trace 中，主 Agent 的工具调用记录下，应该能点开看到一个完整的子 Agent 执行流树形图（Tree View）。`

### O-17 context guard decision
- **类别**: A1 数据一致性
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:4498`
- **原文**: > `- **决策**：保留 AH 版本（有 \`runtime.context or {}\` 防御）`

### O-18 tool metrics decision
- **类别**: A6 观测
- **引用**: `research/sessions/agent-harness/markdown/2026-04-20-session.md:4506`
- **原文**: > `- **决策**：合并 SF 的 \`[ToolMetrics]\` 日志`

## 类 3：7 候选项目 Code Reference 索引

### tamux

**PTY 处理**:
- `research/candidates/tamux/crates/amux-daemon/src/pty_session.rs:11` — 引入 portable-pty。原文：`use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};`
- `research/candidates/tamux/crates/amux-daemon/src/pty_session.rs:27` — 定义 PTY session 结构。原文：`pub struct PtySession {`

**Session 管理**:
- `research/candidates/tamux/crates/amux-daemon/src/session_manager.rs:25` — 定义 SessionManager。原文：`pub struct SessionManager {`
- `research/candidates/tamux/crates/amux-daemon/src/session_manager.rs:93` — 构造带 history 的 session manager。原文：`pub fn new_with_history(history: Arc<HistoryStore>, pty_channel_capacity: usize) -> Arc<Self> {`

**持久化**:
- `research/candidates/tamux/crates/amux-daemon/src/plugin/persistence.rs:25` — 定义 plugin persistence 构造入口。原文：`pub fn new(history: Arc<crate::history::HistoryStore>) -> Self {`

### overstory

**SQLite 存储**:
- `research/candidates/overstory/src/sessions/store.ts:2` — session store 文件说明。原文：` * SQLite-backed session store for agent lifecycle tracking.`
- `research/candidates/overstory/src/sessions/store.ts:75` — 建表语句。原文：`CREATE TABLE IF NOT EXISTS sessions (`
- `research/candidates/overstory/src/sessions/store.ts:196` — 创建 SessionStore。原文：`export function createSessionStore(dbPath: string): SessionStore {`

**兼容迁移**:
- `research/candidates/overstory/src/sessions/compat.ts:67` — SQLite authoritative 规则。原文：` * 1. If sessions.db exists AND has rows, open it directly (SQLite is authoritative).`
- `research/candidates/overstory/src/sessions/compat.ts:76` — 打开兼容 session store。原文：`export function openSessionStore(overstoryDir: string): {`

**进程 / tmux**:
- `research/candidates/overstory/src/worktree/process.ts:63` — headless subprocess spawn 说明。原文：` * Spawn a headless agent subprocess directly via Bun.spawn().`
- `research/candidates/overstory/src/worktree/process.ts:84` — spawnHeadlessAgent 函数。原文：`export async function spawnHeadlessAgent(`

### batty

**Console / TTY**:
- `research/candidates/batty/src/console_pane.rs:21` — console pane run 入口。原文：`pub fn run(`
- `research/candidates/batty/src/console_pane.rs:240` — 发送 message。原文：`fn send_message(&self, message: &str, stdout: &mut impl Write) -> Result<()> {`
- `research/candidates/batty/src/console_pane.rs:418` — raw terminal 结构。原文：`struct RawTerminal {`

**状态分类**:
- `research/candidates/batty/src/shim/classifier.rs:102` — classify 函数。原文：`pub fn classify(agent_type: AgentType, screen: &vt100::Screen) -> ScreenVerdict {`
- `research/candidates/batty/src/shim/classifier.rs:112` — 带 confidence 的 classify。原文：`pub fn classify_with_confidence(agent_type: AgentType, screen: &vt100::Screen) -> Classification {`
- `research/candidates/batty/src/shim/classifier.rs:355` — context exhausted 检测。原文：`fn detect_context_exhausted(content: &str) -> bool {`

**Codex shim tests**:
- `research/candidates/batty/src/shim/tests_codex.rs:45` — Codex mock spawn helper。原文：`/// Spawn a Codex SDK shim with a mock bash script as the sentinel process.`
- `research/candidates/batty/src/shim/tests_codex.rs:153` — SendMessage spawn 说明。原文：`// The SendMessage will cause the runtime to spawn \`codex exec --json ...\``

### ccswarm

**PTY / terminal**:
- `research/candidates/ccswarm/crates/ai-session/src/core/pty.rs:1` — PTY core 文件说明。原文：`//! PTY (Pseudo-Terminal) management`
- `research/candidates/ccswarm/crates/ai-session/src/core/pty.rs:10` — PtyHandle 结构。原文：`pub struct PtyHandle {`
- `research/candidates/ccswarm/crates/ai-session/src/core/pty.rs:40` — spawn_command。原文：`pub async fn spawn_command(&self, cmd: CommandBuilder) -> Result<()> {`
- `research/candidates/ccswarm/crates/ai-session/examples/pty_test.rs:10` — PTY example main。原文：`async fn main() -> Result<()> {`

**Lifecycle / health**:
- `research/candidates/ccswarm/crates/ccswarm/tests/e2e_cli_test.rs:286` — health command 测试。原文：`fn test_health_command() {`
- `research/candidates/ccswarm/crates/ccswarm/tests/mockall_tests.rs:315` — session lifecycle tests 模块。原文：`mod session_lifecycle_tests {`
- `research/candidates/ccswarm/crates/ccswarm/tests/mockall_tests.rs:425` — healthy session shutdown 条件。原文：`// Session should NOT be shut down if it's healthy`

**执行 pipeline**:
- `research/candidates/ccswarm/crates/ccswarm/src/execution/pipeline.rs:10` — TaskPipeline 结构。原文：`pub struct TaskPipeline {`
- `research/candidates/ccswarm/crates/ccswarm/src/execution/pipeline.rs:38` — batch processing 函数。原文：`pub async fn process_batch<F, Fut>(`

### cli-agent-orchestrator

**Terminal / tmux**:
- `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/services/terminal_service.py:80` — terminal 创建函数。原文：`def create_terminal(`
- `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/services/terminal_service.py:288` — terminal input 发送函数。原文：`def send_input(`
- `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/clients/tmux.py:253` — paste 发送函数。原文：`def send_keys_via_paste(self, session_name: str, window_name: str, text: str) -> None:`

**Sessions / API**:
- `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/services/session_service.py:44` — session 创建函数。原文：`def create_session(`
- `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/api/main.py:302` — API create_session。原文：`async def create_session(`

**Providers**:
- `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/providers/claude_code.py:55` — Claude provider class。原文：`class ClaudeCodeProvider(BaseProvider):`
- `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/providers/codex.py:113` — Codex provider class。原文：`class CodexProvider(BaseProvider):`
- `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/providers/gemini_cli.py:141` — Gemini provider class。原文：`class GeminiCliProvider(BaseProvider):`

### metaswarm

**平台安装**:
- `research/candidates/metaswarm/cli/metaswarm.js:38` — Claude 安装函数。原文：`function installClaude() {`
- `research/candidates/metaswarm/cli/metaswarm.js:55` — Codex 安装函数。原文：`function installCodex() {`
- `research/candidates/metaswarm/cli/metaswarm.js:111` — Gemini 安装函数。原文：`function installGemini() {`

**项目 setup**:
- `research/candidates/metaswarm/cli/metaswarm.js:127` — setupProject。原文：`function setupProject(platformFlag) {`
- `research/candidates/metaswarm/cli/metaswarm.js:228` — initCommand。原文：`async function initCommand(args) {`

**Conversation extraction**:
- `research/candidates/metaswarm/skills/setup/scripts/beads-fetch-conversation-history.ts:5` — session file extraction 说明。原文：` * Extracts conversation history from Claude Code session files for self-reflection.`
- `research/candidates/metaswarm/skills/setup/scripts/beads-fetch-conversation-history.ts:246` — parseConversationFile。原文：`function parseConversationFile(`
- `research/candidates/metaswarm/skills/setup/scripts/beads-fetch-conversation-history.ts:405` — output object。原文：`const output: OutputData = {`

### agent-orchestrator

**tmux resolution**:
- `research/candidates/agent-orchestrator/packages/web/server/tmux-utils.ts:87` — session ID validation。原文：`export function validateSessionId(sessionId: string): boolean {`
- `research/candidates/agent-orchestrator/packages/web/server/tmux-utils.ts:144` — resolveTmuxSession。原文：`export function resolveTmuxSession(`
- `research/candidates/agent-orchestrator/packages/web/server/tmux-utils.ts:151` — tmux exact match 注释。原文：`// Without =, tmux uses prefix matching: "ao-1" would match "ao-15"`

**WebSocket terminal**:
- `research/candidates/agent-orchestrator/packages/web/server/mux-websocket.ts:61` — SessionBroadcaster。原文：`export class SessionBroadcaster {`
- `research/candidates/agent-orchestrator/packages/web/server/mux-websocket.ts:237` — TerminalManager。原文：`class TerminalManager {`
- `research/candidates/agent-orchestrator/packages/web/server/mux-websocket.ts:313` — attach tmux via node-pty。原文：`const pty = ptySpawn(this.TMUX, ["attach-session", "-t", tmuxSessionId], {`

**Lifecycle / process startup**:
- `research/candidates/agent-orchestrator/packages/web/server/direct-terminal-ws.ts:20` — terminal server factory。原文：`export function createDirectTerminalServer(tmuxPath?: string): DirectTerminalServer {`
- `research/candidates/agent-orchestrator/packages/web/server/start-all.ts:25` — spawnProcess。原文：`function spawnProcess(`
- `research/candidates/agent-orchestrator/packages/web/server/start-all.ts:101` — direct terminal auto-restart 行。原文：`spawnProcess("direct-terminal", "node", [resolve(__dirname, "direct-terminal-ws.js")], { restart: true });`

## 类 2 续：ccbd-rust 相关 corpus observations

### O-19 paste-buffer 后 Enter 仍可能未提交
- **类别**: A4 协议
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:20596`
- **原文**: > | ① Message stuck in recipient's input, never submitted | **NOT FIXED** | `lib/terminal_runtime/tmux_send.py` sequence is identical to v5: `load-buffer` → `paste-buffer -p` → `sleep 0.5` → `send-keys Enter`. No post-send verify, no retry on alternate keycodes. Live test: job `job_178c3bcc81ce` stuck in `mailbox_state: delivering / queue_depth: 1`; pane input remained at idle prompt |

### O-20 投递序列包含 load-buffer、paste-buffer、send-keys Enter
- **类别**: A4 协议
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:20624`
- **原文**: > load-buffer → paste-buffer -p → sleep 0.5s → send-keys Enter

### O-21 send-keys Enter 返回不等于 Enter 已注册
- **类别**: A4 协议
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:20626`
- **原文**: > (returns after `send-keys Enter` whether Enter registered or not)

### O-22 双 Enter fork 记录在 tmux_send.py
- **类别**: A4 协议
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:20947`
- **原文**: > Fork `bedf12c` 在 `tmux_send.py` `_paste_via_buffer` 里：第一次 `send-keys Enter` 后，如果 env `CCB_TMUX_SECOND_ENTER_DELAY > 0`，sleep 那么多秒再发一次 Enter。

### O-23 bracketed-paste 时序 race 命中
- **类别**: A4 协议
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:20972`
- **原文**: > 2. **bracketed-paste 时序 race**：某些 CLI 在"粘贴开始-结束"标记中间把 Enter 视为多行换行

### O-24 Completion hook never fires 对应 fallback polling
- **类别**: A6 观测
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:20598`
- **原文**: > | ③ Completion hook never fires | **FIXED** | v6.0.0 "Gemini Multi-Round Completion" = fallback polling; v6's `lib/completion/tracker.py` + `ReplyCandidateKind.FALLBACK_TEXT` provides ranked reply candidates |

### O-25 completion timeout fallback polling 被列为目标
- **类别**: A6 观测
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:20611`
- **原文**: > - Completion timeout fallback polling (v6's Multi-Round Completion does this).

### O-26 anchor_seen=false 关联 Codex first-prompt ingestion bug
- **类别**: A6 观测
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-25-session.md:867`
- **原文**: > **Codex current state diagnosis**: Tested in fresh scope — CCB bridge shows "started, waiting for Claude commands" but Codex TUI shows only welcome banner. `anchor_seen: false`. This is **CCB ↔ Codex v0.124.0 first-prompt ingestion bug**, NOT WebSocket. Separate from OpenAI WebSocket drop.

### O-27 agent activity markers 包含 Planning 和 Thinking
- **类别**: A6 观测
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:4948`
- **原文**: > 107	5. The agent 活动检测 shall 在 pane 末 10 行（`tmux capture-pane -p -S -10`）grep `AGENT_ACTIVITY_MARKERS` 集合（`Planning` / `Thinking` / `Generating` / `Calling` / `Loading` / `✦` / Braille spinner 字符等）

### O-28 看到 Planning/Thinking/spinner 后停止 retry
- **类别**: A6 观测
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:4751`
- **原文**: > **修复**：retry 前必须先检查 agent 是否已经在干活（`Planning/Thinking/spinner` 等迹象），有 → 停止 retry，转去等 completion。

### O-29 Gemini hook 期望时 settle_window 降级到 30s
- **类别**: A6 观测
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:20238`
- **原文**: > **TD-008 修复：CCB Gemini detector 真空期吞响应**（2026-04-23 同日开 spec + 实施 + 部署 + 验证）—— 简化 Kiro spec `.kiro/specs/ccb-gemini-detector-suppression/`。Gemini analyst 架构咨询定根因：不是 hook 路径缺失（已有 `poll_exact_hook`），而是 `AnchoredSessionStabilityDetector` 2s 稳定窗口在 Gemini "content 已写但 toolCalls 未补"的真空期赢过 hook（hook 实测 ~4s 才 fire）。修法**参数化降级**：`is_hook_expected=True`（Gemini + req_id）时 settle_window 2s → 30s，让 hook 优先；无 req_id 场景行为不变。

### O-30 pane_state alive 绑定状态输出
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-26-session.md:128`
- **原文**: > binding: status=bound runtime=tmux:%4 session=/home/sevenx/coding/agent-harness/.ccb/.codex-a1-session source=provider-session workspace=/home/sevenx/coding/agent-harness terminal=tmux socket=None socket_path=/home/sevenx/coding/agent-harness/.ccb/ccbd/tmux.sock pane=%4 active_pane=%4 pane_state=alive marker=CCB-a1-3429a8e8

### O-31 tmux cleanup 输出 owned/active/orphaned/killed
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-26-session.md:7793`
- **原文**: > tmux_cleanup: socket=/home/sevenx/coding/agent-harness/.ccb/ccbd/tmux.sock owned=%1,%4,%3,%2 active=%1,%4,%3,%2 orphaned=- killed=-

### O-32 completion_reason 可记录 pane_dead
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-26-session.md:7962`
- **原文**: > - **a1 Codex**: `failed` (`completion_reason: pane_dead`) — pane 在处理时挂了。`ccb ps` 显示现在 pane 又 alive,可能 ccbd 自愈拉起了。

### O-33 pane alive 不等于 codex 进程可用
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-26-session.md:8016`
- **原文**: > ⏺ 发现问题:Codex pane 一直在 dump `No saved session found with ID 019db764...` —— 是 codex CLI 在死循环 retry 一个失效的 session ID。pane 技术上 alive 但 codex 进程根本没起来。这就是上次 reply 为空的根因 —— ccbd 把 pane status 当成 codex status,实际是错位的。

### O-34 runtime metadata 包含 reconcile_state
- **类别**: A5 lifecycle
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-25-session.md:6993`
- **原文**: > 21	        'reconcile_state',

### O-35 runtime metadata 包含 restart_count
- **类别**: A5 lifecycle
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-25-session.md:6994`
- **原文**: > 22	        'restart_count',

### O-36 bashrc 缺失 CCB 配置项
- **类别**: A3 沙盒
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-14-session.md:1364`
- **原文**: > ⏺ 现在清楚了。`.bashrc` 里缺少：`CCB_REPLY_LANG`、`cc`/`ccb` 完整 alias 模板、`proxy-on/off`、`_reset_mouse_tracking`。同时文件和 JSON 也要同步。一并处理：

### O-37 Gemini sandbox BeforeAgent hook 注入 settings.json
- **类别**: A3 沙盒
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:18005`
- **原文**: > 26	1. When CCB ccbd 启动 Gemini agent 时, the system shall 把 BeforeAgent hook 注入 sandbox `~/.gemini/settings.json`，命令为 **`/usr/bin/timeout 5s <python3> bin/ccb-provider-finish-hook --event start ...`**（复用现有脚本，只加新参数；**外层 `timeout 5s` 防止 hook 进程挂死整个 Gemini CLI**——见 [Gemini review 风险 4]）

### O-38 Gemini sandbox hook projection 目标
- **类别**: A3 沙盒
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:18001`
- **原文**: > 22	**Objective:** 把 BeforeAgent hook projection 到 Gemini sandbox，触发后写一张签收回执（reception artifact），结构与 finish-hook 完全镜像。

### O-39 生产 sandbox 需要 /usr/bin/timeout
- **类别**: A3 沙盒
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:10529`
- **原文**: > 1. 生产 sandbox 有 `/usr/bin/timeout`（已验证）

### O-40 MemoryMax=5G 用作 cgroup 硬上限
- **类别**: A3 沙盒
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-23-session.md:701`
- **原文**: > - `MemoryMax=5G`：硬上限，超过时 cgroup 内 OOM kill 最耗内存的进程

### O-41 不要停下，决策问题先问 Gemini
- **类别**: A4 协议
- **引用**: `research/sessions/agent-harness/markdown/2026-04-24-session.md:10559`
- **原文**: > ❯ 不要停下,决策问题先问Gemini

### O-42 禁止问是否继续
- **类别**: A4 协议
- **引用**: `research/sessions/agent-harness/markdown/2026-04-24-session.md:15018`
- **原文**: > ❯ 不允许在问我要不要继续这种蠢问题了. 你唯一可以停下来问我的,只有在你和Gemini辩论3轮后依旧没有统一,才能问我. 否则直到把所有需求做完前,不要停. 把这点作为铁律写进全局claude.md,优先级放最高

### O-43 纠正 continuation questions 的 iron rule
- **类别**: A4 协议
- **引用**: `research/sessions/agent-harness/markdown/2026-04-24-session.md:17863`
- **原文**: > - **User feedback on continuation questions** — "不允许在问我要不要继续这种蠢问题了". Saved as `feedback_no_continuation_questions.md`. Iron rule already in `~/.claude/CLAUDE.md` top.

### O-44 用户指出 master 视野太窄
- **类别**: A4 协议
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-25-session.md:8109`
- **原文**: > ❯ 你的视野太窄了而且非常短视，不适合做设计，只能沿着设计好的路径推进项目。让Gemini全局考虑，重新设计

### O-45 stupid question 命中主控需求
- **类别**: A4 协议
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-22-session.md:16430`
- **原文**: > 这种stupid question。我需要主控帮我思考和回答这种问题，像一个项目经理帮我推进项目进度知道项目完成

### O-46 SIGKILL 后 ccbd 自动 respawn
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-26-session.md:1002`
- **原文**: > - ccbd binding healthy,a2 进程已 SIGKILL + ccbd 自动 respawn(node PID 3990595→4138634,memory 339→242 MB,确认是干净 fresh 进程)

### O-47 Gemini SIGTERM ignored 后使用 SIGKILL
- **类别**: A5 lifecycle
- **引用**: `research/sessions/agent-harness/markdown/2026-04-26-session.md:5778`
- **原文**: > - **Gemini SIGTERM ignored**: had to use SIGKILL (kill -9) on PID

### O-48 Master Claude death 记录为 Node.js self-crashed
- **类别**: A5 lifecycle
- **引用**: `research/sessions/home-sevenx/markdown/2026-04-25-session.md:861`
- **原文**: > - **Master Claude death root cause**: Confirmed NOT OOM (free 3.8G available, no cgroup-kill log). Tool_use at 16:40:27 had no tool_result — Claude Node.js process self-crashed. Anthropic upstream bug requiring Q1 instrumentation (installed).

## 类 3 续：候选项目 code references 加深

### tamux 补充

- **PTY spawn / portable_pty**: `research/candidates/tamux/crates/amux-daemon/src/pty_session.rs:88` — `PtySession::spawn` 创建 PTY。原文：`/// Spawn a new PTY with the given shell and dimensions.`
- **PTY child process**: `research/candidates/tamux/crates/amux-daemon/src/pty_session.rs:139` — slave 端 spawn command。原文：`let child = Arc::new(std::sync::Mutex::new(pair.slave.spawn_command(cmd)?));`
- **PTY stdin write**: `research/candidates/tamux/crates/amux-daemon/src/pty_session.rs:214` — 向 PTY 写 raw bytes。原文：`/// Write raw bytes into the PTY's stdin.`
- **bwrap sandbox**: `research/candidates/tamux/crates/amux-daemon/src/sandbox.rs:18` — Linux sandbox 类型。原文：`/// Linux: uses bubblewrap (bwrap) for mount namespace isolation.`
- **sandbox fallback**: `research/candidates/tamux/crates/amux-daemon/src/sandbox.rs:153` — 无 sandbox binary 时 passthrough。原文：`"sandbox: no sandbox binary found, using passthrough (commands run without isolation)"`

### overstory 补充

- **mail SQLite store**: `research/candidates/overstory/src/mail/store.ts:1` — mail storage 文件说明。原文：`* SQLite-backed mail storage for inter-agent messaging.`
- **messages schema**: `research/candidates/overstory/src/mail/store.ts:46` — messages 表 DDL。原文：`const CREATE_TABLE = \``
- **WAL/busy_timeout**: `research/candidates/overstory/src/mail/store.ts:181` — 并发访问配置。原文：`// Configure for concurrent access from multiple agent processes.`
- **tmux isolation socket**: `research/candidates/overstory/src/worktree/tmux.ts:14` — 独立 tmux server socket。原文：`* Dedicated tmux server socket name for agent session isolation.`
- **tmux create session**: `research/candidates/overstory/src/worktree/tmux.ts:116` — createSession 函数。原文：`export async function createSession(`

### batty 补充

- **Ping/Pong health**: `research/candidates/batty/src/team/daemon/health/ping_pong.rs:1` — shim health monitoring 文件说明。原文：`//! Periodic Ping/Pong health monitoring for shim handles.`
- **stale handle detection**: `research/candidates/batty/src/team/daemon/health/ping_pong.rs:40` — secs_since_last_pong 超时分类。原文：`handle.secs_since_last_pong().and_then(|secs_since_pong| {`
- **socketpair shim IPC**: `research/candidates/batty/src/team/daemon/shim_spawn.rs:1` — shim spawn 文件说明。原文：`//! Shim subprocess spawning: create a socketpair, fork/exec \`batty shim\`,` 
- **fd 3 IPC handoff**: `research/candidates/batty/src/team/daemon/shim_spawn.rs:188` — child socket 通过 fd 3 传递。原文：`// Pass child socket as fd 3`
- **console send message**: `research/candidates/batty/src/console_pane.rs:240` — console pane 调用 batty send。原文：`fn send_message(&self, message: &str, stdout: &mut impl Write) -> Result<()> {`

### ccswarm 补充

- **PTY handle fields**: `research/candidates/ccswarm/crates/ai-session/src/core/pty.rs:9` — PTY handle 结构。原文：`/// Handle to a PTY`
- **PTY spawn command**: `research/candidates/ccswarm/crates/ai-session/src/core/pty.rs:39` — 在 PTY 中 spawn command。原文：`/// Spawn a command in the PTY`
- **PTY write**: `research/candidates/ccswarm/crates/ai-session/src/core/pty.rs:60` — 向 PTY 写数据。原文：`/// Write data to the PTY`
- **read timeout**: `research/candidates/ccswarm/crates/ai-session/src/core/pty.rs:135` — read_with_timeout。原文：`/// Read data from PTY with timeout (for testing)`
- **parallel batch pipeline**: `research/candidates/ccswarm/crates/ccswarm/src/execution/pipeline.rs:37` — 并行 batch 处理。原文：`/// Process tasks in parallel batches`

### cli-agent-orchestrator 补充

- **terminal workflow**: `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/services/terminal_service.py:91` — create_terminal 工作流说明。原文：`This function orchestrates the complete terminal creation workflow:`
- **metadata persistence**: `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/services/terminal_service.py:143` — terminal metadata 入库。原文：`# Step 3: Persist terminal metadata to database`
- **failure cleanup**: `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/services/terminal_service.py:216` — create_terminal 失败清理。原文：`except Exception as e:`
- **paste enter count**: `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/services/terminal_service.py:295` — send_input 使用 tmux paste buffer。原文：`"""Send input to terminal via tmux paste buffer.`
- **bracketed paste**: `research/candidates/cli-agent-orchestrator/src/cli_agent_orchestrator/clients/tmux.py:253` — send_keys_via_paste。原文：`def send_keys_via_paste(self, session_name: str, window_name: str, text: str) -> None:`

### metaswarm 补充

- **Claude install**: `research/candidates/metaswarm/cli/metaswarm.js:38` — Claude Code 安装函数。原文：`function installClaude() {`
- **Codex install dir**: `research/candidates/metaswarm/cli/metaswarm.js:55` — Codex CLI 安装函数。原文：`function installCodex() {`
- **Codex skill symlink**: `research/candidates/metaswarm/cli/metaswarm.js:81` — skills symlink 段。原文：`// Symlink skills`
- **Gemini extension install**: `research/candidates/metaswarm/cli/metaswarm.js:111` — Gemini CLI 安装函数。原文：`function installGemini() {`
- **conversation JSONL parse**: `research/candidates/metaswarm/skills/setup/scripts/beads-fetch-conversation-history.ts:246` — 解析 conversation file。原文：`function parseConversationFile(`

### agent-orchestrator 补充

- **session id validation**: `research/candidates/agent-orchestrator/packages/web/server/tmux-utils.ts:83` — session id 校验说明。原文：`* Validate a session ID format.`
- **tmux binary detection**: `research/candidates/agent-orchestrator/packages/web/server/tmux-utils.ts:100` — findTmux 函数。原文：`export function findTmux(`
- **exact tmux match**: `research/candidates/agent-orchestrator/packages/web/server/tmux-utils.ts:150` — 精确匹配 tmux session。原文：`// Try exact match first using = prefix for exact matching (e.g., "ao-orchestrator")`
- **TerminalManager map**: `research/candidates/agent-orchestrator/packages/web/server/mux-websocket.ts:237` — TerminalManager 类。原文：`class TerminalManager {`
- **node-pty attach-session**: `research/candidates/agent-orchestrator/packages/web/server/mux-websocket.ts:312` — attach tmux session。原文：`// Spawn PTY`
