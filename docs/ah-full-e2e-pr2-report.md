# ah Full E2E PR-2 Report

## §1 PR-2 范围与对齐目标

PR-2 承接 PR-1 Grand Tour 主线，专注 DRIFT + NEW 两类动态生命周期分支。落地对象是 5-case matrix：ENV Drift、HOOKS Drift、PLUGINS Drift、NO_CHANGE 幂等、NEW Agent。PR-2 明确不覆盖 ORPHAN / BUSY / ERROR；`SKIPPED_BUSY` 与 `FORCE_REALIGN` 也留给 PR-3 的 BUSY/ERROR 专项，避免把 drift/new 主线和异常生命周期混在同一个 PR。

## §2 测试拓扑

新增 1 个 ignored 测试：`grand_tour_drift_new_matrix`，位于 `tests/ah_full_e2e_drift.rs`。该测试复用 PR-1 风格的 in-process RPC harness：temp DB、temp state dir、temp project dir、隔离 tmux server、`dispatch` JSON-RPC helper。

串接顺序是固定长链路，不重置 DB / state / tmux：

1. baseline setup：`session.create` + `session.spawn_master_pane` + `agent.spawn`
2. `case_01_env_drift`
3. `case_02_hooks_drift`
4. `case_03_plugins_drift`
5. `case_04_no_change`
6. `case_05_new_agent`

测试用 fake `claude` binary 注入 temp `PATH`，让真实 Claude provider manifest 走 `prepare_home_layout_with_extensions` 物化路径，同时不依赖真实 Claude CLI 或真实 token。

## §3 实施摘要

- ENV Drift：给 `a1.env` 增加 `GRAND_TOUR_DRIFT_ENV=v2`，显式传入 `session.realign` payload，断 per-agent `status=REALIGNED`、`event=drift_realigned`、PID 变化、旧 PID 退出、`agents.config_hash` 变化。
- HOOKS Drift：写 host hook `hooks/pr2-audit-v2.sh`，把 `PreToolUse` hook 放入 payload，断 per-agent `status=REALIGNED`、reason 含 `hooks changed`、`.claude/settings.json` 存在、`.claude/hooks/pr2-audit-v2.sh` symlink 指向 host hook。
- PLUGINS Drift：写 host plugin cache `.claude/plugins/cache/pr2-claude-audit/plugin.json`，把 plugin 放入 payload，断 per-agent `status=REALIGNED`、reason 含 `plugins changed`、cache symlink 和 enabled symlink 都指向 host cache、`settings.json` 中 `enabledPlugins.pr2-claude-audit=true`。
- NO_CHANGE：用 T6 结束后的 ENV+HOOKS+PLUGINS 叠加态再次 realign，断 per-agent `status=NO_CHANGE`、PID 不变、config_hash 不变、`drift_realigned` 与 `agent_spawned` 事件计数不增长。
- NEW Agent：在已有 session 追加 `a2` block，payload 同时包含 `a1` 和 `a2`，断 per-agent `a2.status=NEW`、`action=spawned`、`agent_a2` 到 IDLE、`agent_spawned.reason=NEW`、`a1` PID 不变、`a2` sandbox 和 `.claude/CLAUDE.md` 存在。

## §4 物理断言风格

PR-2 延续“物理断言”风格，不只看 RPC 文本：

- `assert_symlink_target` 使用 `std::fs::read_link`，验证 hook/plugin symlink 的真实目标。
- `.claude/CLAUDE.md` 用 `assert_sandbox_file` 验证 NEW agent 的 Claude rules 物化。
- Plugin 双 symlink 都验证：`.claude/plugins/cache/<name>` 与 `.claude/plugins/<name>`。
- `.claude/settings.json` 用 JSON pointer 校验：`/enabledPlugins/pr2-claude-audit == true`。
- Provider HOME 不在 `state_dir/sandboxes/<session>/<agent>` 直下，而在 `$XDG_CACHE_HOME/ah/sandboxes/<hash>`；测试 helper 复刻 `home_layout` 的 sandbox hash 规则解析 `.claude/*` 路径。

## §5 关键发现

- `session.realign` 的 REALIGNED 路径会 `delete_agent` 后重新 `agent.spawn`。SQLite FK cascade 会清旧 agent events；因此 drift case 不能假设 `a1` 的 `drift_realigned` 事件计数 1→2→3 累加，只能断最新事件或当前事件存在。
- NO_CHANGE 路径不同：handler 命中 per-agent `handlers.rs:470` 后直接返回，不 delete/reinsert agent，所以事件计数前后不增长的断言有效。
- `drift_reason` 真输出包含 `env changed` / `hooks changed` / `plugins changed`，因此 PR-2 对 hooks/plugins reason 保留强断言。
- `materialize_claude_plugins` 同时物化 `.claude/plugins/cache/<name>` 和 `.claude/plugins/<name>` 两条 symlink，两条都在 PR-2 中断言。
- RPC status 必须锚定 `statuses[]` per-agent entry：`handlers.rs:461 NEW`、`:470 NO_CHANGE`、`:516 REALIGNED`。不能误取 master/session 顶层 `handlers.rs:401` 的 `NO_CHANGE`。

## §6 验证

- Host verify: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1` -> 4 passed, 0 failed, finished in 2.08s.
- Default lane: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --test-threads=1` -> 3 passed, 1 ignored.
- Commit 序列建议：PR-2 spec lock -> T1 red skeleton -> T2/T3 harness+fixtures -> T4 ENV -> T5/T6 hooks/plugins -> T7/T8/T9 final cases+report。
- a2 audit focus: per-agent status 锚定、payload 显式包含 env/hooks/plugins、NEW payload 同时包含 a1+a2。
- a3 audit focus: read_link 物理断言、PID race retry、NO_CHANGE event count 不增长、ORPHAN/BUSY/ERROR 不进入 PR-2。

## §7 PR-3 Future Scope

PR-3 继续覆盖 ORPHAN / BUSY / ERROR：

- ORPHAN：agent block 删除后的 audit-only 与 force cleanup。
- BUSY：`SKIPPED_BUSY` 非 force 路径。
- FORCE_REALIGN：BUSY + force 的 kill/rebuild 路径。
- ERROR：provider crash/recovery 与 evidence/prompt 扩展链路。

PR-2 不实现这些分支，避免当前 5-case DRIFT + NEW matrix 的断言边界被异常生命周期稀释。
