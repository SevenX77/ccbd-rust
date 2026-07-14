# ah PR-1a Tasks: Evidence Statemachine

## §0. 数据校验声明

本任务清单只覆盖 PR-1a：design §3.1 状态机拦截与 §3.3 Done 前强制；PR-1b 的 §3.2 Read-first hook 只保留 stub。

已读：
- `.kiro/specs/ah-evidence-statemachine/design.md:1-129`
- `/tmp/a1-pr1a-tasks-brief.md`

已 grep / verify：
- `src/db/schema.rs:45` 现有 `evidence` 表，`src/db/schema.rs:59` 现有 `jobs` 表。
- `src/db/state_machine.rs:258` `mark_agent_idle_matched_sync` 是 marker matched 后转 `IDLE` 的同步入口。
- `src/db/state_machine.rs:281` 查询当前 dispatched job，`src/db/state_machine.rs:312` 直接调用 `mark_job_completed_conn_sync`。
- `src/db/jobs.rs:288` `DISPATCHED -> COMPLETED` 更新点，`src/db/jobs.rs:405` dispatched job 查询点。
- `src/agent_io/writer.rs:14` 已有 tmux paste 写入机制，可用于调用层注入 `SYSTEM DENY`。
- `tests/mvp12_r2_dispatcher_lifecycle.rs:61`、`tests/dispatch_atomicity.rs:133` 已有 `mark_agent_idle_matched` 相关回归锚点。

## §1. Tasks 全景图

| Phase | 内容 | 核心 T 数 | 估算 | 依赖前置 |
| :--- | :--- | :--- | :--- | :--- |
| Phase 0 | 准备与 baseline 核验 | 2 | 0.5h | main 最新 |
| Phase 1 | tests-first 红灯 | 3 | 1.5h | Phase 0 |
| Phase 2 | schema + DB evidence 扩展 | 3 | 1.5h | Phase 1 |
| Phase 3 | 状态机拦截实施 | 3 | 2h | Phase 2 |
| Phase 4 | Done 前 TDD 强制 | 2 | 1.5h | Phase 3 |
| Phase 5 | 集成验证与交付 | 5 | 1h | Phase 4 |

## §2. 详细 Tasks

### Phase 0: 准备

- [ ] **T0.1 切实施分支**
  - 从最新 `main` 切 `feat/ah-evidence-statemachine-pr-1a`。
  - verify main HEAD 为 brief 指定 `1dfa61e` 或记录实际更新后的 HEAD。

- [ ] **T0.2 复核 design baseline**
  - 复核 design §3.1、§3.3、§6.1、§6.2。
  - grep 并记录实施锚点：
    - `src/db/state_machine.rs:258` `mark_agent_idle_matched_sync`
    - `src/db/state_machine.rs:281` dispatched job 查询
    - `src/db/state_machine.rs:312` job completed 更新
    - `src/db/schema.rs:45` `evidence` 表
    - `src/agent_io/writer.rs` tmux 写入机制

### Phase 1: tests-first 红灯

- [ ] **T1.1 写状态机拦截 failing test**
  - 新增或扩展 PR-1a 专用测试，seed 一个 `BUSY` agent + `DISPATCHED` code-change job。
  - 不插入 `diff_generated` / `mtime_changed` / `test_passed` evidence。
  - 触发 marker matched。
  - 断言：
    - agent 不得转 `IDLE`。
    - job 不得转 `COMPLETED`。
    - 调用层可观察到 `SYSTEM DENY: Missing physical evidence` 注入请求或 PTY 末尾文本。

- [ ] **T1.2 写 Done 前 TDD failing test**
  - seed 一个需要测试证据的 dispatched job。
  - 插入非 `test_passed` evidence 或完全不插 evidence。
  - 触发 marker matched。
  - 断言 job 保持 `DISPATCHED`，不得变 `COMPLETED`。

- [ ] **T1.3 串行验红并提交红灯**
  - 单条运行：
    - `CARGO_BUILD_JOBS=1 cargo test --test <pr1a_test_name> -- --test-threads=1`
  - 确认 T1.1/T1.2 是预期红灯。
  - commit: `test(pr-1a): failing tests for evidence enforcement`

### Phase 2: schema + DB 扩展

- [ ] **T2.1 扩充现有 `evidence` 表**
  - 在 `src/db/schema.rs` 对现有 `evidence` 表新增字段：
    - `job_id TEXT NULL REFERENCES jobs(id) ON DELETE CASCADE`
    - `evidence_type TEXT`
    - `subject_path TEXT`
    - `payload TEXT`
  - 增加索引：
    - `(agent_id, job_id, evidence_type)`
    - `(agent_id, evidence_type, subject_path)`，供后续 PR-1b Read-first 使用。
  - 迁移纪律：已有 l3 evidence 记录字段保持 NULL，不破坏 `PENDING/SEALED/REVIEWED/DISCARDED` 语义。

- [ ] **T2.1b 在 `src/db/mod.rs` 接入旧库迁移执行链**
  - 不能只改 `src/db/schema.rs`；旧用户已存在的 DB 不会重新执行 `CREATE TABLE` 字段定义。
  - 在 `init()` 的现有迁移链后追加 `migrate_evidence_records_columns(&conn)?`。
  - 按现仓 pattern 实现 `ALTER TABLE evidence ADD COLUMN ...`：
    - 参考 `src/db/mod.rs:46-49` 的 `migrate_*` 调用链。
    - 参考 `src/db/mod.rs:54-112` 对 duplicate column 的容忍处理。
  - 迁移字段覆盖 T2.1 的 `job_id` / `evidence_type` / `subject_path` / `payload`。
  - 补旧库升级测试：先创建旧 schema evidence 表并插入 l3 evidence，再调用 `init()`，断言旧记录保留、新字段为 NULL、不会崩。

- [ ] **T2.2 扩展 `src/db/evidence.rs` API**
  - 增加 `insert_evidence_record_sync(agent_id, job_id: Option<&str>, evidence_type, subject_path: Option<&str>, payload)`。
  - 增加 `query_evidence_for_job_sync(job_id, evidence_type)` 或 `has_job_evidence_sync(job_id, &[types])`。
  - 保留现有 unknown/l3 evidence CRUD API 行为不变。

- [ ] **T2.3 DB 单测转绿**
  - 覆盖：
    - nullable `job_id` round-trip。
    - `job_id = ?` 查询自然排除 `job_id IS NULL` 的 Read-first 预备记录。
    - 现有 l3 evidence helper 测试不回归。
  - 单条运行：
    - `CARGO_BUILD_JOBS=1 cargo test --lib evidence -- --test-threads=1`

### Phase 3: 状态机拦截实施

- [ ] **T3.1 在 `mark_agent_idle_matched_sync` 加 evidence gate**
  - 仅当存在当前 dispatched job 时启用 gate；无 dispatched job 的 init/idle marker 保持原语义。
  - 查询当前 job 的 evidence：
    - 至少存在 `mtime_changed` 或 `diff_generated`。
    - 对需要测试的任务，后续 Phase 4 再要求 `test_passed`。
  - 无 evidence 时：
    - agent 保持原 active state，优先保持 `BUSY`。
    - job 保持 `DISPATCHED`。
    - 不调用 `mark_job_completed_conn_sync`。
    - 返回可被调用层识别的拒绝结果。

- [ ] **T3.2 在调用层注入 `SYSTEM DENY`**
  - 不在 DB 层直接操作 PTY。
  - 在 `mark_agent_idle_matched` async wrapper 或其调用方识别拒绝结果。
  - 通过现有 `agent_io/writer.rs` / tmux paste 机制向 agent pane 注入：
    - `SYSTEM DENY: Missing physical evidence. You must output a git diff or test result before finishing.`
  - 确保注入失败不会把 job 错误标为 completed。

- [ ] **T3.3 T1.1 转绿**
  - 单条运行：
    - `CARGO_BUILD_JOBS=1 cargo test --test <pr1a_test_name> <state_machine_gate_test> -- --test-threads=1`

### Phase 4: Done 前 TDD 强制

- [ ] **T4.1 实施 `test_passed` evidence gate**
  - 对代码修改类 / TDD 类 job，`COMPLETED` 前必须存在 `evidence_type = 'test_passed'` 且 `job_id = ?`。
  - 不引入 `READY_FOR_MERGE` 状态；现有真实状态仍为 `DISPATCHED` / `COMPLETED`。
  - 若 job 类型暂未结构化，先以明确的测试 fixture / dispatch 标记实现，不用 prompt 文本正则做长期契约。

- [ ] **T4.2 T1.2 转绿**
  - 单条运行：
    - `CARGO_BUILD_JOBS=1 cargo test --test <pr1a_test_name> <tdd_gate_test> -- --test-threads=1`

### Phase 5: 集成验证与交付

- [ ] **T5.1 全库单测**
  - 单条运行：
    - `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`
  - 预期不低于 PR4b baseline：`388+ passed`。

- [ ] **T5.2 mvp12 home layout 回归**
  - 单条运行：
    - `CARGO_BUILD_JOBS=1 cargo test --test mvp12_home_layout -- --test-threads=1`
  - 预期：`4 passed`。

- [ ] **T5.3 mvp2 acceptance 关键回归**
  - 单条运行：
    - `CARGO_BUILD_JOBS=1 cargo test --test mvp2_acceptance -- --test-threads=1`
  - 重点确认现仓真实 ac 编号 `ac2`-`ac6` 不回归；`ac2` / `ac3a` 当前 ignored 时按测试输出记录。

- [ ] **T5.4 按文件名提交**
  - `git add` 必须逐个列出改动文件，禁止 `git add -A` / `git add .`。
  - commit PR-1a 实施。

- [ ] **T5.5 推分支并开 PR**
  - push `feat/ah-evidence-statemachine-pr-1a`。
  - 创建 PR：
    - base: `main`
    - title: `feat(pr-1a): evidence statemachine 物理验证拦截`
  - 不 merge，等待 user 拍板。

## §3. PR-1b Stub

PR-1b: design §3.2 Read-first hook 依赖 PR4c `PreToolUse` hook 注入基建。等 PR4c 实施完成后，回头补 PR-1b tasks：
- Claude `settings.json` hooks 合并。
- hook 脚本 / 小二进制。
- agent_id/session_id env 注入。
- ccbd socket 查询。
- `Read` evidence 写入。
- `Edit` / `Write` 未读先写的 exit 2 或 JSON deny 真测试。

本 PR-1a 不拆、不实现 PR-1b。

## §4. 风险 + 注意点

- design §2 的“扩体现有”按“扩充现有”理解。
- PR-1a 查询 Done 类 evidence 时必须使用 `job_id = ?`，自然排除 PR-1b 预备的 `job_id IS NULL` Read-first 记录。
- `evidence` 表扩展是 `[BREAKING]` DB migration：实施时必须验证旧库升级平滑，旧 l3 evidence 记录保留且新增字段为 NULL。
- `SYSTEM DENY` 注入不要放在 DB 层；DB 层只做判断和状态变更，PTY 写入由调用层完成。
- 任何 cargo 验证必须串行：`CARGO_BUILD_JOBS=1`，需要 test runner 串行时加 `-- --test-threads=1`。
