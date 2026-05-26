# Prompt Handler Phase 1 Acceptance

## 完结条件

- [ ] `cargo test` 单元 + 集成全过。- **自动化 (integration test 覆盖)**；当前仍有既有 `tests/mvp11_acceptance.rs::test_systemctl_stop_anchor_triggers_cascade` 失败，非 prompt-handler T10 引入。
- [ ] `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo build` 全过。- **自动化 (CI / 手动跑)**。
- [ ] 手动 e2e 覆盖：codex update prompt 自动跳过（a3 跑）。- **派 a3 e2e**。
- [ ] 手动 e2e 覆盖：未知 prompt 进入 `PROMPT_PENDING`，主控能从事件看到 `UNKNOWN_PROMPT_DETECTED`。- **integration test 覆盖, 真实可选**。
- [ ] 手动 e2e 覆盖：`ah prompt resolve <agent> <action>` 能执行动作并解阻塞。- **integration test 覆盖, 真实可选**。
- [ ] design.md §9.2 Phase 1 的 (a)-(f) 全部落地：静态正则 + seeds、多层递归、`PROMPT_PENDING`、unknown event、resolve RPC/CLI、Hash-Gating 4 级。

## Design §9.2 对应

- (a) 静态正则匹配 + 内置种子预案：T1+T2 `dfbac2c`。
- (b) 多层 prompt 递归 Max Depth 3：T5 `658d075`。
- (c) `PROMPT_PENDING` agent 状态：T6 `ae6afeb`。
- (d) emit `UNKNOWN_PROMPT_DETECTED` 事件：T7+T8 `53f4ef8`。
- (e) RPC `agent.resolve_prompt(agent_id, action, save_to_kb: bool)` + CLI：T9 `011d995`。
- (f) Hash-Gating 4 级分流，无 LLM 路径：T3+T4 `3402662`。
- T10 自动化验收、mock fixture、本文档：本轮提交。

## 自动化覆盖

- `tests/fixtures/mock_prompt_provider.sh` 用独立 shell fixture 输出 codex update、trust path、unknown EULA prompt 文本，不依赖真实 Codex CLI。
- `tests/prompt_handler_e2e.rs::known_codex_update_prompt_is_auto_skipped_in_tmux` 覆盖已知 codex update prompt 自动发送 `2` + `Enter`，agent 不进入 `PROMPT_PENDING` / `CRASHED`。
- `tests/prompt_handler_e2e.rs::unknown_prompt_enters_pending_emits_event_and_resolves_to_kb` 覆盖未知 prompt 进入 `PROMPT_PENDING`、写 `UNKNOWN_PROMPT_DETECTED` event、`agent.resolve_prompt` 解阻塞、`prompt-cases.json` 新增 case。
- `tests/prompt_handler_e2e.rs::pidfd_exit_does_not_crash_prompt_pending_agent` 覆盖 pidfd watcher 看到进程退出时不把 `PROMPT_PENDING` agent 改成 `CRASHED`。

## a3 真实 e2e 步骤

1. 起项目 ccbd，派一个 codex agent。
2. 等 Codex 启动到 0.129 升级提示，或用临时旧版本 / 清理 Codex version state 主动触发升级提示。
3. 验证 prompt-handler 自动 send `2` + `Enter`，agent 进入 `IDLE` 而不是 `CRASHED`。
4. 派一个 ask 任务给 agent，验证 reply 正常。
5. 可选模拟未知 prompt：pane 直接 send-keys 一些未知 EULA / trust 变体文案，验证 `PROMPT_PENDING` + `UNKNOWN_PROMPT_DETECTED` event。
6. CLI 调 `ah prompt resolve a1 --keys "1 Enter" --save-to-kb`，验证 agent 解阻塞 + KB 落地。

## 已知限制 + Phase 2/3 待办

- KB 路径用 state_dir 不是 `~/.ccb` 全局，Phase 2 可选迁移。
- Phase 1 没 LLM；Haiku / 主控 fallback 属于 Phase 3。
- mock fixture 覆盖稳定自动化路径；真实 Codex 升级路径仅 a3 手动跑一次。
- vt100 parser / 颜色识别留给 Phase 3。
- 多 ccbd 冲突 union-merge 留给 Phase 2。

## Phase 1 完成后必须做的事

- [ ] 更新 `README.md`：新增 prompt-handler 能力、KB 路径、`prompt resolve` 示例、已知限制。- 本轮不做。
- [ ] 更新 `CLAUDE.md` 或对应 agent 操作说明：遇到 `UNKNOWN_PROMPT_DETECTED` 的主控处理流程。- 本轮不做。
- [ ] 补充 KB schema 文档：字段说明、action 白名单、Phase 2/3 预留字段。- 本轮不做。
- [ ] 补充运维排障说明：如何查看 pending agent、unknown prompt event、KB 写入失败日志。- 本轮不做。
- [ ] 复盘是否需要把 `spawn_new_capture_seed` 中 prompt 扫描逻辑抽离，降低 `src/rpc/handlers.rs` 体积。- 本轮不做。
