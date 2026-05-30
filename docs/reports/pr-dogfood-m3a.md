# PR Report: feat(ah-dogfood): M3a dogfood-5 — B4 slash command keystroke

## §1 PR 标题与摘要

PR 标题: `feat(ah-dogfood): M3a dogfood-5 — B4 slash command keystroke`

本 PR 落地 ah dogfooding closure 的 M3a 子集: B4 slash command keystroke。`agent.send` 对单行 `/clear` 这类 slash command 改走 tmux direct keystroke, 普通消息继续走 paste-buffer。目标是让 provider 把 slash command 当命令处理, 并用 fake provider slash ack 锁住 100% 投递成功。

## §2 背景与 scope

M2 PR #29 已合 main, M3a 基于 M2:

- M1 已有 completion marker 与 `event.subscribe`。
- M2 已有 stuck push 与 0 poll 指标。
- M3a 只补 input transport 中的 slash command 分支。

M3a scope:

- `src/agent_io/writer.rs`: 单行 `/` 开头消息走 keystroke。
- `tests/fixtures/mock_dogfood_provider.sh`: `/clear` 输出 `<<ah-slash-ack:cmd=/clear>>`。
- `tests/ah_dogfooding.rs`: 新增 M3a slash delivery test。

不在 scope: multi-line slash、provider-specific mapping、B5 health check、B6 真 stdout dogfood 全套。

## §3 变更摘要

Commit 范围:

- `1c90a51 test(ah-dogfood): M3a step 3 tests-first — T1 fixture slash ack + T2.1 红灯`
- `ef12910 feat(ah-dogfood): M3a step 4 — send_slash_command_keystroke (T2.1 red→green)`

改动统计:

- Step 3 tests-first: 2 文件, +43。
- Step 4 src impl: 1 文件, +36 / -1。
- 合计: 约 +79 / -1。

变更点:

- fixture: slash input 输出 `ah-slash-ack`。
- test: `test_slash_command_keystroke_delivery` 锁 ack + writer keystroke helper。
- src: `send_text_to_pane` 增 slash branch; 新增 `send_slash_command_keystroke` 与 `is_single_line_slash_command`。

## §4 测试与验证

```bash
cargo test --test ah_dogfooding -- --include-ignored --test-threads=1
```

结果: 10 passed。覆盖 M1 5 + M2 4 + M3a 1。

```bash
cargo test --lib pane_diff
```

结果: 16 passed。

```bash
cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1
```

结果: 4 passed。

```bash
CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1
```

结果: passed。

grep verify:
`rg "send_slash_command_keystroke|is_single_line_slash_command|ah-slash-ack|load_buffer|paste_buffer" src/agent_io/writer.rs tests` 覆盖 slash branch、ack marker 与普通 paste path 保留。

## §5 audit 结果

M3a 改动小: 1 个 src 文件 + 1 个 fixture + 1 个 test。跳详细 a3 audit, 主控亲审 verify。

主控自审:

- `src/rpc/handlers.rs` 未改, `handle_agent_send` 语义不漂移。
- M1 reader / marker 未改。
- M2 pane_diff / state_machine / pubsub / event.subscribe 未改。
- 普通 paste-buffer path 仍存在。
- 多行 slash 不命中 keystroke。

## §6 已知限制与后续

M3a 度量兑现:

- slash command 投递成功率 100%: `test_slash_command_keystroke_delivery` PASS。

已知限制:

- M3a 只支持 single-line slash。
- multi-line slash 和 provider-specific mapping 留后续。
- fake provider ack 锁的是最小投递行为; dogfood-8 需要补真 stdout reader 全链路。

后续:

- M3b: B5 health check / multi-layer probe。
- M4: B6 真 stdout fake provider dogfood 全套。

## §7 风险 / breaking

无 BREAKING。

- 非 `/` 开头消息继续走 paste-buffer。
- 含 `\n` / `\r` 的多行消息继续走 paste-buffer。
- RPC / CLI / SQLite schema 全继承不变。
- 主要风险是 provider-specific slash 差异尚未抽象; 当前统一 keystroke 足够覆盖 `/clear` 最小指标。
