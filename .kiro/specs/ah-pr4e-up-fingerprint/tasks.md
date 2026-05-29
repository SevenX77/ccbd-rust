# ah PR4e Tasks: Fingerprint Audit & Alignment (`ah up`)

## 0. 数据校验声明

本任务清单基于 `.kiro/specs/ah-pr4e-up-fingerprint/design.md` lock（commit `3ac2f49`），只实现配置指纹审计与声明式对齐，不扩展 rules/skills，不依赖 PR-1c。

现有代码锚点：

- DB sessions 当前无 `config_hash`：`src/db/schema.rs:8-15`，`Session` struct 在 `src/db/schema.rs:107-115`。
- DB agents 当前无 `config_hash`：`src/db/schema.rs:17-29`，`Agent` struct 在 `src/db/schema.rs:117-130`。
- session 写入路径：`src/db/sessions.rs:17-35,202-212`。
- agent 写入路径：`src/db/agents.rs:7-28,132-145`。
- ah.toml schema：`MasterConfig` 在 `src/cli/config.rs:23-36`，`AgentConfig` 在 `src/cli/config.rs:63-72`。
- CLI `Cmd` enum 当前无 `Up`：`src/bin/ah.rs:32-97`。
- RPC router 当前无 realign：`src/rpc/router.rs:13-34,71-85`。
- master spawn 入口：`src/rpc/handlers.rs:208-296`，物化调用在 `src/rpc/handlers.rs:233-241`。
- agent spawn 入口：`src/rpc/handlers.rs:317-475`，物化调用在 `src/rpc/handlers.rs:352-362`，agent insert 在 `src/rpc/handlers.rs:463-475`。
- PR4d provisioning barrier：`prepare_home_layout_with_extensions` 在 `src/provider/home_layout.rs:62-91`；Claude path `src/provider/home_layout.rs:110-113`；Codex path `src/provider/home_layout.rs:156-162`。
- BUSY/state machine 语义：`mark_agent_idle_matched_outcome_sync` 在 `src/db/state_machine.rs:278-330`。
- kill/event 原语：`mark_agent_killed` 在 `src/db/agents_lifecycle.rs:128-145`；`insert_event` 在 `src/db/events.rs:100-109`。

依赖声明：

- 依赖 PR-1a evidence statemachine，已 merge。
- 依赖 PR4d auto-provisioning git plugins，已 merge，cache layout 为 `$XDG_CACHE_HOME/ah/cache/git/<host>/<owner>/<repo>/<ref>/`。
- 不依赖 PR-1c read-first hook RPC；PR4e 只关心配置物化与运行指纹。

实测 grep：

- `rg -n "config_hash|CREATE TABLE IF NOT EXISTS sessions|CREATE TABLE IF NOT EXISTS agents|pub struct Session|pub struct Agent" src/db/schema.rs`
- `rg -n "insert_session_sync|insert_agent_sync|handle_session_spawn_master_pane|handle_agent_spawn|prepare_home_layout_with_extensions|mark_agent_idle_matched_outcome_sync|mark_agent_killed|insert_event" src`
- `rg -n "enum Cmd|METHODS|session.spawn_master_pane|agent.spawn|sha2|serde_json|tabled" src Cargo.toml`

## 1. Phase 矩阵

| Phase | 目标 | 文件 | LOC | Tests-first 红灯命令 | 验收门 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **M1 Schema + CLI audit skeleton** | 增加 `config_hash` 基建、sorted-key hash、`ah up` CLI 骨架与 dry audit 输出。 | + `src/provider/fingerprint.rs`; M `src/provider/mod.rs`, `src/db/schema.rs`, `src/db/sessions.rs`, `src/db/agents.rs`, `src/bin/ah.rs`, `src/cli/mod.rs`; + `src/cli/up.rs`; + `tests/pr4e_up_fingerprint.rs` | 220-320 | `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint no_change_reports_up_to_date -- --test-threads=1` | NO_CHANGE 红灯转绿；lib 单测 hash 稳定。 |
| **M2 session.realign / agent.realign RPC + spawn hash commit** | 注册 realign RPC，spawn 成功后写运行 hash，实现 DRIFT plugins/hooks 和 master drift 对齐入口。 | M `src/rpc/router.rs`, `src/rpc/handlers.rs`, `src/db/sessions.rs`, `src/db/agents.rs`, `src/db/events.rs` | 220-300 | `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint plugin_drift_realigns_agent hook_drift_realigns_agent master_drift_audit_only_by_default master_drift_force_triggers_realign -- --test-threads=1` | DRIFT plugins/hooks/master audit/force 三类转绿，新 hash 写入 DB。 |
| **M3 State machine integration: BUSY / ORPHAN / NEW** | 区分 NEW / DRIFT / ORPHAN，BUSY 默认 skip，`--force` 才中断。 | M `src/cli/up.rs`, `src/rpc/handlers.rs`, `src/db/agents_lifecycle.rs`, `src/db/events.rs` | 180-260 | `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint orphan_agent_is_reported busy_agent_skip_and_force_realign new_agent_is_spawned -- --test-threads=1` | ORPHAN、BUSY skip/force、NEW agent spawn 转绿。 |
| **M4 E2E + regression** | 全局回归，确保 PR4d/PR4c/PR-1a 不退化并整理 ship。 | M tests only if needed for fixtures; no new product scope | 80-120 | `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint -- --test-threads=1` | PR4e 全绿；PR4d 5/5、PR4c 6/6、PR-1a 3/3、lib passed。 |

## 2. Tests-First 红灯方案

- [ ] **T1 NO_CHANGE baseline**
  - description: `ah.toml` 不变时 `ah up` 输出 no drift，DB 不写新 hash，不触发 kill/spawn。
  - files: + `tests/pr4e_up_fingerprint.rs`; M `src/cli/up.rs`; M `src/provider/fingerprint.rs`。
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint no_change_reports_up_to_date -- --test-threads=1`
  - depends_on: none.

- [ ] **T2 DRIFT plugins changed**
  - description: Agent plugins 列表增加一项时输出 DRIFT，调用 `agent.realign`，重建后写入新 `agents.config_hash`。
  - files: + `tests/pr4e_up_fingerprint.rs`; M `src/rpc/handlers.rs`; M `src/db/agents.rs`; M `src/provider/home_layout.rs` only if fixture wiring needs extra output.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint plugin_drift_realigns_agent -- --test-threads=1`
  - depends_on: T1.

- [ ] **T3 DRIFT hooks changed**
  - description: Agent hook spec 改变时输出 `hooks changed`，重建后 provider settings 指向新 hook。
  - files: + `tests/pr4e_up_fingerprint.rs`; M `src/rpc/handlers.rs`; M `src/cli/up.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint hook_drift_realigns_agent -- --test-threads=1`
  - depends_on: T1.

- [ ] **T4 ORPHAN deleted agent**
  - description: DB 中存在 agent，但新 `ah.toml` 删除该 agent；`ah up` 默认输出 ORPHAN 并提示，不 kill、不 spawn；`--force` 才执行清理策略。
  - files: + `tests/pr4e_up_fingerprint.rs`; M `src/cli/up.rs`; M `src/rpc/handlers.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint orphan_agent_is_reported -- --test-threads=1`
  - depends_on: T1.

- [ ] **T5 BUSY skip + --force**
  - description: Agent 为 BUSY 且 drift 时默认 `SKIPPED_BUSY` 不 kill；`--force` 时调用 kill + full rebuild。
  - files: + `tests/pr4e_up_fingerprint.rs`; M `src/rpc/handlers.rs`; M `src/db/agents_lifecycle.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint busy_agent_skip_and_force_realign -- --test-threads=1`
  - depends_on: T2.

- [ ] **T6 master DRIFT audit-only + --force**
  - description: `sessions.config_hash` 与 Master expected hash 不同，`ah up` 默认只报告 master DRIFT、不重启 master pane；`--force` 才触发全量重启 master pane，不只处理 agents。
  - files: + `tests/pr4e_up_fingerprint.rs`; M `src/rpc/handlers.rs`; M `src/db/sessions.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint master_drift_audit_only_by_default master_drift_force_triggers_realign -- --test-threads=1`
  - depends_on: T1.

- [ ] **T7 NEW agent spawn**
  - description: 新 `ah.toml` 有 agent block，但 DB 无该 `agent_id`；`ah up` 标记 NEW 并 spawn 新 agent，不走 realign/kill。
  - files: + `tests/pr4e_up_fingerprint.rs`; M `src/cli/up.rs`; M `src/rpc/handlers.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint new_agent_is_spawned -- --test-threads=1`
  - depends_on: T1.

## 3. 详细 Tasks

### M1: Schema + `ah up` CLI skeleton

- [ ] **T8 Add config_hash DB columns and structs**
  - description: 给 sessions/agents 增加 nullable `config_hash TEXT`，扩展 schema structs 与 query row mapping。
  - files: M `src/db/schema.rs:8-29,107-130`; M `src/db/sessions.rs:17-35,202-212`; M `src/db/agents.rs:7-28,132-145`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --lib db::tests::test_init_schema -- --test-threads=1`
  - depends_on: T1.

- [ ] **T9 Add DB hash helpers**
  - description: 新增 session/agent hash update/query helpers，保持 insert APIs 兼容旧调用。
  - files: M `src/db/sessions.rs`; M `src/db/agents.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --lib config_hash -- --test-threads=1`
  - depends_on: T8.

- [ ] **T10 Add deterministic fingerprint module**
  - description: 实现 sorted-key `serde_json` deterministic serialization + SHA256，不实现完整 RFC 8785。
  - files: + `src/provider/fingerprint.rs`; M `src/provider/mod.rs`; uses `src/provider/extensions.rs:4-9`, `src/cli/config.rs:23-72`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --lib provider::fingerprint -- --test-threads=1`
  - depends_on: T8.

- [ ] **T11 Add CLI `ah up` skeleton**
  - description: 在 `Cmd` enum 增加 `Up { force }`，新增 `src/cli/up.rs`，先实现 parse + expected hash + no-change report。
  - files: M `src/bin/ah.rs:32-97`; M `src/cli/mod.rs:1-8`; + `src/cli/up.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint no_change_reports_up_to_date -- --test-threads=1`
  - depends_on: T10.

### M2: realign RPC + spawn hash commit

- [ ] **T12 Register realign RPC methods**
  - description: 注册 `session.realign` / `agent.realign`，新增 handlers stub，router unknown-method tests 更新。
  - files: M `src/rpc/router.rs:13-34,71-85`; M `src/rpc/handlers.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --lib rpc::router -- --test-threads=1`
  - depends_on: T10.

- [ ] **T13 Commit hash after master spawn**
  - description: `handle_session_spawn_master_pane` 物化与 pane spawn 成功后写 `sessions.config_hash`。
  - files: M `src/rpc/handlers.rs:208-296`; M `src/db/sessions.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint master_spawn_persists_config_hash -- --test-threads=1`
  - depends_on: T9, T12.

- [ ] **T14 Commit hash after agent spawn**
  - description: `handle_agent_spawn` 物化与 `insert_agent` 成功后写 `agents.config_hash`。
  - files: M `src/rpc/handlers.rs:317-475`; M `src/db/agents.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint agent_spawn_persists_config_hash -- --test-threads=1`
  - depends_on: T9, T12.

- [ ] **T15 Implement DRIFT realign pipeline for agents**
  - description: `agent.realign` 执行 Stage 1-5：verify hash, gate, destroy, reconstruct, commit。
  - files: M `src/rpc/handlers.rs`; M `src/db/agents.rs`; M `src/db/events.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint plugin_drift_realigns_agent hook_drift_realigns_agent -- --test-threads=1`
  - depends_on: T13, T14.

- [ ] **T16 Implement master drift audit/force policy**
  - description: `sessions.config_hash` 漂移时默认只审计并返回 master DRIFT；`--force` 时才走 master 全量重启：stop anchor + master spawn path + session hash commit。
  - files: M `src/rpc/handlers.rs:192-296`; M `src/db/sessions.rs`; M `src/db/events.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint master_drift_audit_only_by_default master_drift_force_triggers_realign -- --test-threads=1`
  - depends_on: T13, T15.

### M3: State machine integration, ORPHAN, NEW

- [ ] **T17 Add diff classifier: NO_CHANGE / DRIFT / ORPHAN / NEW / SKIPPED_BUSY**
  - description: `ah up` / RPC diff engine 区分 DB-only ORPHAN 与 config-only NEW，不把 NEW 错当 DRIFT。
  - files: M `src/cli/up.rs`; M `src/rpc/handlers.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint orphan_agent_is_reported new_agent_is_spawned -- --test-threads=1`
  - depends_on: T15.

- [ ] **T18 Implement ORPHAN handling**
  - description: config 删除 agent 时输出 ORPHAN；默认提示用户决定，不 kill、不 spawn；复用同一个 `--force` flag，和 master drift / BUSY 一致，force 才调用 kill/cleanup。
  - files: M `src/rpc/handlers.rs`; M `src/db/agents_lifecycle.rs:128-145`; M `src/db/events.rs:100-109`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint orphan_agent_is_reported -- --test-threads=1`
  - depends_on: T17.

- [ ] **T19 Implement NEW agent spawn**
  - description: config 有新 agent 而 DB 无该 id 时调用 agent spawn path，物化成功后写新 hash。
  - files: M `src/rpc/handlers.rs:317-475`; M `src/cli/up.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint new_agent_is_spawned -- --test-threads=1`
  - depends_on: T17.

- [ ] **T20 Implement BUSY skip and --force**
  - description: 若 agent `state = BUSY` 且无 force，输出 `SKIPPED_BUSY` 并写 `drift_skipped`；复用同一个 `--force` flag，和 master drift / ORPHAN 一致，force 才 kill + rebuild。
  - files: M `src/rpc/handlers.rs`; M `src/db/state_machine.rs` only if helper needed; M `src/db/events.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint busy_agent_skip_and_force_realign -- --test-threads=1`
  - depends_on: T15, T17.

### M4: E2E + regression

- [ ] **T21 PR4e full acceptance**
  - description: 7 个 PR4e tests-first 场景全部绿。
  - files: `tests/pr4e_up_fingerprint.rs`.
  - tests: `CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint -- --test-threads=1`
  - depends_on: T16, T18, T19, T20.

- [ ] **T22 Existing PR regression**
  - description: 确认 PR4d/PR4c/PR-1a/lib 不退化。
  - files: no product code unless regression found.
  - tests:
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4d_auto_provisioning -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1a_evidence_statemachine -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`
  - depends_on: T21.

- [ ] **T23 Diff hygiene and ship**
  - description: `cargo fmt`、`git diff --check`，按 by-name stage，提交 PR4e。
  - files: all changed PR4e files.
  - tests:
    - `cargo fmt`
    - `git diff --check`
  - depends_on: T22.

## 4. 风险与执行注意

- Master drift 不能只做 agent realign；必须覆盖 `sessions.config_hash` 漂移，并走 master pane 全量重启或明确 force gate。
- NEW agent 与 DRIFT agent 是不同状态：NEW 没有旧进程可 kill，应直接 spawn；DRIFT 需要先 gate/destroy/reconstruct。
- ORPHAN 默认策略要保守，避免用户误删配置导致运行 agent 被直接杀；若实现 force cleanup，必须有测试覆盖。
- BUSY 判断以 DB state 为准；不要通过 pane 文本猜测。
- Hash 输入必须是 raw spec，不能把 PR4d `cache_dir` 写进 hash。
- `sorted-key serde_json` 不是完整 RFC 8785；测试只要求 HashMap 顺序稳定、plugins 排序稳定、hook Vec 保序。
