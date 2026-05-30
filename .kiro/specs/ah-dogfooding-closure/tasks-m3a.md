# Tasks M3a: ah dogfooding closure slash keystroke

## §1 PR scope 与度量目标

M3a scope: design §4 的 `dogfood-5`, 只做 B4 slash command keystroke。

M3a 继承 M1/M2: `event.subscribe`、marker completion、stuck push、`InterventionCounters { cancel, capture, poll }` 均已存在; 本 PR 不改这些主线, 只补 `agent.send` slash transport。

M3a 目标:
- `src/agent_io/writer.rs:6-43` 当前所有 send 都走 tmux `load_buffer` + `paste_buffer` + delayed Enter。
- 新规则: message 首字符为 `/` 且单行时, 走 tmux keystroke direct send; 其他消息继续走 paste-buffer + Enter。
- `src/rpc/handlers.rs:1170-1296` 的 `handle_agent_send` 保持 request_id、idempotency、state CAS、event 记录语义, 不直接关心 transport。

M3a 度量:

- slash command 投递成功率 100%。fake claude/codex/gemini 对 `/clear` 输出 slash ack, test 断言能收到 `<<ah-slash-ack:cmd=/clear>>`。

## §2 TDD 任务列表

### T1 mock_dogfood_provider.sh 扩 slash ack

文件: `tests/fixtures/mock_dogfood_provider.sh`。

依赖: 现 fixture `tests/fixtures/mock_dogfood_provider.sh:100-113` 读一行 input, 输出 work/done/idle marker。

内容: 在 read loop 中检测 `line` 首字符为 `/`, 输出 `mock_dogfood_provider[$provider]: slash cmd=/clear` 与 `<<ah-slash-ack:cmd=/clear>>`; claude/codex/gemini 共用同一逻辑。

验收:
- shell 直接运行 fixture, 输入 `/clear`, stdout 含 `<<ah-slash-ack:cmd=/clear>>`。
- 输入普通 `job-id:X` 仍输出 `<<ah-idle:job-id=X>>`。

### T2 红灯 test: slash command keystroke delivery

文件: 扩 `tests/ah_dogfooding.rs`。

新增 test: `test_slash_command_keystroke_delivery`。

流程: 安装/启动 fake provider 或用 Harness 挂接 tmux pane, 对 agent 调 `agent.send` text=`/clear`, 读取 provider output / event / pane, assert `<<ah-slash-ack:cmd=/clear>>` 出现。

红灯原因:
- 当前 `send_text_to_pane` 对 slash 仍走 paste-buffer + delayed Enter。
- fake provider slash ack test 要求 direct keystroke path; 实现前不会观察到 slash ack 或 transport marker。

验收:
- `cargo test --test ah_dogfooding -- --include-ignored --test-threads=1` 中 M1 5 + M2 4 继续 PASS, M3a 1 个 FAIL。

### T3 src/agent_io/writer.rs 增 keystroke 分支

文件: `src/agent_io/writer.rs`。

内容:
- 新增 helper `is_single_line_slash_command(text: &str) -> bool`:
  - `text.starts_with('/')`
  - 不包含 `\n` / `\r`
  - trim 后非空
- 新增 `send_slash_command_keystroke(tmux, pane, slash_cmd: &str)`:
  - 使用 tmux direct send-keys / literal keystroke API。
  - 不调用 `load_buffer` / `paste_buffer`。
  - 发送 command 后再 Enter。
- 修改 `send_text_to_pane`:
  - 若 `is_single_line_slash_command(&text)`, 走 keystroke branch 并 return。
  - 其他文本保留现 paste-buffer + `CCB_TMUX_ENTER_DELAY` + optional second Enter 完整逻辑。

per-provider mapping:
- M3a 可先用统一 allowlist/helper: `/clear`, `/new`, `/help`。
- 若 provider 需要差异, 新增 `slash_map` helper, 但不把 provider manifest 改造扩大到 M3a 之外。

验收:
- `test_slash_command_keystroke_delivery` 绿。
- writer 既有 sanitize buffer tests 继续绿。

### T4 红灯变绿

T3 实施后重跑:

- `cargo test --test ah_dogfooding -- --include-ignored --test-threads=1`
- 期望 M1 5 + M2 4 + M3a 1 = 10 PASS。

### T5 a3 audit

- a3 audit: PM 替身 + scope drift。
- 核 M3a 只做 B4 slash keystroke, 不碰 B5 health / B6 dogfood full e2e。
- 核普通多行 prompt 仍走 paste-buffer, 没引入 multiline keystroke 编码风险。

### T6 docs 同步

文件: `docs/engine/dogfood-m3a/logic-explained.md`。

内容:
- 字段级翻译 `is_single_line_slash_command`, `send_slash_command_keystroke`, slash allowlist/mapping, fake provider slash ack。
- 明确 M3a 与 M1/M2 差异: M1/M2 是 wait/event/stuck, M3a 只改 input transport。

### T7 PR report

文件: `docs/reports/pr-dogfood-m3a.md`。

内容:
- 背景、scope、变更、测试、audit、风险、后续。
- 明确 M3a 指标: slash command 投递成功率 100%。

## §3 验收门槛

- `cargo test --test ah_dogfooding -- --include-ignored --test-threads=1`: M1 5 + M2 4 + M3a 1 = 10 PASS。
- `cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`: PASS。
- `CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1`: PASS。
- slash command 投递成功率 100%: fake provider 对 `/clear` 输出 ack, test 断言通过。
- grep verify: writer 有 slash keystroke branch; fixture 有 `ah-slash-ack`; 普通 paste-buffer path 仍存在。

## §4 scope guard

- 只允许改 B4 相关: `tests/fixtures/mock_dogfood_provider.sh`, `tests/ah_dogfooding.rs`, `src/agent_io/writer.rs`。
- `src/rpc/handlers.rs` 原则上不改; 若 audit 证明需要, 只能做 transport 参数透传, 不改 idempotency/state 语义。
- 不改 M1/M2 锁定主线: `reader.rs`, `marker/matcher.rs`, `pane_diff/mod.rs`, `state_machine.rs`, `pubsub.rs`, `event.subscribe` handlers。
- 不引入 multi-line keystroke; 多行、大文本、含 escape 的消息继续 paste-buffer。
- 不实施 B5 health check。
- 不实施 B6 真 stdout dogfood 全套。
