# Dogfood M3a Logic Explained

## §1 PR 范围

M3a 对应 dogfood-5, 只落 B4 slash command keystroke:

- `agent.send` 收到单行 slash command 时, 不再走 tmux paste-buffer。
- slash command 通过 tmux direct `send-keys -l` 按字符送入 provider TUI, 再发送 Enter。
- 普通 prompt、多行文本、非 `/` 开头文本继续使用原 paste-buffer + delayed Enter 路径。

M3a 不改 M1 completion reader / marker path, 不改 M2 stuck / pane_diff / event.subscribe path, 不做 B5 health check, 不做 B6 真 stdout dogfood 全套。

## §2 新增字段与函数

| 字段 / 函数 | 文件:line | 中文 logic 解释 | 类型 / 签名 |
|---|---:|---|---|
| `send_text_to_pane` slash branch | `src/agent_io/writer.rs:6-14` | agent 输入统一入口。先判断文本是否是单行 slash command; 是则直接走 keystroke helper 并提前返回。 | `pub async fn(...) -> Result<()>` |
| paste-buffer fallback | `src/agent_io/writer.rs:16-46` | 普通消息路径保持原逻辑: `load_buffer`、`paste_buffer`、删除 buffer、等待 `CCB_TMUX_ENTER_DELAY`、发送 Enter, 可选第二次 Enter。 | existing branch |
| `send_slash_command_keystroke` | `src/agent_io/writer.rs:49-59` | slash command 专用投递。遍历 `slash_cmd.chars()`, 每个字符调用 tmux `send_keys_literal`, 最后调用 `send_enter`。不经 paste-buffer。 | `pub async fn(Arc<TmuxServer>, TmuxPaneId, &str) -> Result<()>` |
| `tmux.send_keys_literal` | `src/agent_io/writer.rs:55` | 对单个字符调用 tmux literal send-keys, 避免 slash command 被 bracketed paste 当作普通大段输入。 | async tmux call |
| `tmux.send_enter` | `src/agent_io/writer.rs:57` | slash command 字符送完后提交给 provider TUI。 | async tmux call |
| `is_single_line_slash_command` | `src/agent_io/writer.rs:61-63` | slash 判定条件: `text.starts_with('/')`, 不包含 `\n` 或 `\r`, trim 后非空。 | private fn |
| slash allow examples | `src/agent_io/writer.rs:99-104` | unit test 锁 `/clear`, `/new`, `/help` 都走 slash 判定。M3a 不做 provider-specific mapping。 | test coverage |
| multiline rejection | `src/agent_io/writer.rs:106-112` | unit test 锁非 slash、多行 slash、CR slash、空文本不走 keystroke。 | test coverage |
| `agent_id` 参数 | `src/agent_io/writer.rs:8` | slash branch 不使用 `agent_id`; 普通 paste-buffer branch 仍用它生成 tmux buffer name。 | `&str` |
| `pane` 参数 | `src/agent_io/writer.rs:9,51` | slash 和 paste 两条路径共用同一个 tmux pane 目标, 不改变 pane 注册语义。 | `TmuxPaneId` |
| `text` 参数 | `src/agent_io/writer.rs:10` | send 输入原文。slash 判定只读原文, 不 trim 后再发送, 避免改写用户命令。 | `String` |
| `sanitize_buffer_name` | `src/agent_io/writer.rs:72-83` | 只服务 paste-buffer 分支。slash branch 不创建 buffer, 因此不触碰 buffer 命名/删除逻辑。 | private fn |
| `env_float` | `src/agent_io/writer.rs:65-70` | 只服务 paste-buffer Enter delay。slash keystroke 直接 Enter, 不复用 paste 延迟。 | private fn |
| fixture slash branch | `tests/fixtures/mock_dogfood_provider.sh:102-111` | fake provider 读取一行 input 后, 若首字符是 `/`, 输出 slash ack 并回到 prompt, 不进入 job-id idle marker 分支。 | bash branch |
| `<<ah-slash-ack:cmd=$cmd>>` | `tests/fixtures/mock_dogfood_provider.sh:108` | slash 投递成功的可观测 marker。M3a test 用它证明 `/clear` 被 fake provider 按 slash command 收到。 | stdout marker |
| ordinary job branch | `tests/fixtures/mock_dogfood_provider.sh:113-119` | 非 slash 输入仍解析 job id, 输出 work/done/`<<ah-idle:job-id=X>>`; M1 completion fixture 行为保留。 | bash branch |
| `test_slash_command_keystroke_delivery` | `tests/ah_dogfooding.rs:534-567` | M3a 的 ignored dogfood test。先证明 fixture ack, 再证明 writer 已有 keystroke helper。 | test fn |

## §3 M3a test logic

### T2.1 `test_slash_command_keystroke_delivery`

准备状态:

- test 启动 `tests/fixtures/mock_dogfood_provider.sh`。
- stdin 写入 `/clear`。
- 读取 provider stdout。
- 再读取 `src/agent_io/writer.rs` 源码, 确认 step 4 已落 `send_slash_command_keystroke`。

assert:

- stdout 包含 `<<ah-slash-ack:cmd=/clear>>`。
- writer source 包含 `send_slash_command_keystroke`。

src 路径:

- `agent.send` 仍进入 `handle_agent_send`, 该 handler 不关心 transport。
- handler 调 `agent_io::send_text_to_pane`。
- `send_text_to_pane` 对 `/clear` 命中 `is_single_line_slash_command`。
- `send_slash_command_keystroke` char-by-char 调 tmux literal send, 然后 Enter。
- fake provider 看到 `/clear`, 输出 slash ack。

红灯到绿灯:

- Step 3 时 fixture ack 已存在, 但 writer source 不含 `send_slash_command_keystroke`, test fail。
- Step 4 增加 writer helper 与 entry detect 后, 同一 test pass。
- 这个 test 是 M3a 最小闭环; dogfood-8 再补真实 provider stdout reader 链路。

## §4 跟 M1/M2 兼容

- M1 reader / matcher 锁未改: `src/agent_io/reader.rs` 与 `src/marker/matcher.rs` 不在 M3a 改动范围。
- M1 `event.subscribe` / completion marker path 未改。
- M2 `pane_diff` / `state_machine` / `pubsub` / stuck filter 未改。
- `src/rpc/handlers.rs` 未改: request_id、idempotency、state CAS、`command_received` event 语义不变。
- 普通消息 paste-buffer 路径不动, 包括 `CCB_TMUX_ENTER_DELAY` 与 `CCB_TMUX_SECOND_ENTER_DELAY`。
- 多行消息不走 keystroke, 避免 escape 编码和 provider TUI 行编辑差异风险。

## §5 不在 M3a 的部分

- multi-line slash 不实现: 多行、大文本、含 CR/LF 的输入仍走 paste-buffer。
- provider-specific slash mapping 不实现: `/clear`, `/new`, `/help` 当前走统一 keystroke 行为。
- B5 `health_check.rs` / multi-layer probe 留 M3b。
- B6 真 stdout fake provider dogfood 全套留 M4/dogfood-8。
- slash command ack 目前由 fake provider test 锁住; 后续 dogfood-8 需要用真实 provider stdout reader 链路锁完整端到端。
