# ah PR-1b Tasks: Read-First Evidence Hook

## §0. 数据校验声明

本任务清单只覆盖 PR-1b：基于 PR4c 的显式 hooks 物化能力，实现 Read-before-Edit 证据写入、写前拦截，以及 hook 反向动态武装 PR-1a 物理证据 gate。PR-1b 不做 prompt 文本启发式 arming，不新增 rules/skills 字段，不做 PR4d/PR4e 外部资产 provisioning。

已读：
- `.kiro/specs/ah-pr1b-readfirst-hook/design.md:17-25`，继承字段/RPC 表：新增 `evidence.insert` 与 `job.mark_requires_evidence`。
- `.kiro/specs/ah-pr1b-readfirst-hook/design.md:31-35`，hook 运行上下文依赖 `CCB_JOB_ID` 与 `CCB_SOCKET`。
- `.kiro/specs/ah-pr1b-readfirst-hook/design.md:36-40`，Read 证据由 hook 检测 `Read` / `read_file` 后写入。
- `.kiro/specs/ah-pr1b-readfirst-hook/design.md:42-50`，写类工具先查 read evidence，再调用 `job.mark_requires_evidence(CCB_JOB_ID)` 动态武装。
- `.kiro/specs/ah-pr1b-readfirst-hook/design.md:51-63`，hook 使用 Python 解析 stdin/stdout，不依赖 jq。
- `.kiro/specs/ah-pr1b-readfirst-hook/design.md:67-78`，显式声明原则与 M1-M3 实施切片。
- `.kiro/specs/ah-pr1b-readfirst-hook/design.md:82-96`，tests-first 验收场景。

已 grep / verify：
- `src/db/schema.rs:45-58` 是 `evidence` table 当前真实定义，包含 `job_id` / `evidence_type` / `subject_path` / `payload`。
- `src/db/schema.rs:63-78` 是 `jobs` table 当前真实定义，包含 `requires_physical_evidence` / `requires_test_evidence`。
- `src/provider/extensions.rs:4-10` 已有 `ExtensionConfig { hooks, plugins }`；`src/provider/extensions.rs:12-64` 已有 `HookGroup` / `HookItem`。
- `src/db/evidence.rs:33-65` 已有 `insert_evidence_record_sync`，`src/db/evidence.rs:126-146` 已有 async wrapper。
- `src/db/evidence.rs:67-89` 已有 `has_job_evidence_sync`，但没有带 `subject_path` 过滤的 public async 查询 wrapper。
- `src/db/jobs.rs:324-339` / `src/db/jobs.rs:797-813` 已有 `set_job_evidence_requirements`，可服务 `job.mark_requires_evidence`。
- `src/rpc/router.rs:12-30` 方法白名单无 `evidence.insert` / `job.has_evidence` / `job.mark_requires_evidence`。
- `src/rpc/router.rs:67-85` dispatch match 无证据 RPC 分支。
- `src/rpc/handlers.rs:540-572` `handle_job_submit` 当前只 insert job；PR-1b 不在这里做 prompt 启发式 arming。
- `src/db/state_machine.rs:381-404` 已有 completion gate；PR-1b 要保持 PR-1a 的 physical evidence 类型契约，不把 read 当作完成物理证据替代 diff/mtime。
- `rg CCB_JOB_ID src tests` 无命中；PR-1b 必须新增 dispatch/job send 时的 `CCB_JOB_ID` 注入与 provider env passthrough。

## §1. Tasks 全景图

| Phase | 内容 | 核心 T 数 | 估算 | 依赖前置 |
| :--- | :--- | :--- | :--- | :--- |
| Phase 0 | 准备与源码锚定 | 2 | 0.5h | PR4c merged |
| Phase 1 | tests-first 红灯 | 6 | 2.5h | Phase 0 |
| Phase 2 | Evidence RPC + dynamic arming RPC | 6 | 2h | Phase 1 |
| Phase 3 | CCB_JOB_ID/CCB_SOCKET hook 上下文注入 | 4 | 1.5h | Phase 2 |
| Phase 4 | Python-based evidence-hook.sh | 6 | 2.5h | Phase 3 |
| Phase 5 | 显式 ah.toml 集成 + completion gate 回归 | 4 | 1h | Phase 4 |
| Phase 6 | 全局回归 + ship | 5 | 1h | Phase 5 |

## §2. 详细 Tasks

### Phase 0: 准备

- [ ] **T0.1 切实施分支**
  - Files: 无。
  - 从最新目标分支切 `feat/ah-pr1b-readfirst-hook`。
  - Acceptance Criteria:
    - `git status -sb` 显示当前分支正确。
    - 不基于未提交的 PR4c/H1 修复工作树实施。

- [ ] **T0.2 复核 design round 2 与源码锚点**
  - Files: `.kiro/specs/ah-pr1b-readfirst-hook/design.md`，`src/db/schema.rs`，`src/db/evidence.rs`，`src/db/jobs.rs`，`src/rpc/router.rs`，`src/rpc/handlers.rs`，`src/provider/manifest.rs`。
  - grep:
    - `rg -n "evidence\\.insert|job\\.has_evidence|job\\.mark_requires_evidence|insert_evidence_record|has_job_evidence|set_job_evidence_requirements|CCB_JOB_ID" src tests`
    - `rg -n "collect_spawn_env|ENV_PASSTHROUGH|extra_env_vars|agent.send|handle_agent_send|dispatch_job_to_agent" src`
  - Acceptance Criteria:
    - 实施记录使用实测 file:line，不沿用漂移行号。
    - 明确 PR-1b 不改 `handle_job_submit` 做 prompt 文本启发式 arming。

### Phase 1: tests-first 红灯

- [ ] **T1.1 写 evidence.insert RPC failing test**
  - Files: `tests/pr1b_readfirst_hook.rs` 或 `src/rpc/router.rs` tests。
  - 场景：seed session/agent/job 后，通过 JSON-RPC 调 `evidence.insert`。
  - Acceptance Criteria:
    - params: `agent_id`、`job_id`、`evidence_type="read"`、`subject_path`、`payload`。
    - result 返回 `{ "evidence_id": "evi_...", "recorded": true }`。
    - DB `evidence` 行包含 job/type/path/payload，且 `events.event_type == "evidence_recorded"`。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook evidence_insert_rpc_records_read -- --test-threads=1`

- [ ] **T1.2 写 job.has_evidence RPC failing test**
  - Files: `tests/pr1b_readfirst_hook.rs` 或 `src/rpc/router.rs` tests。
  - 场景：同一 job 下先无 read evidence，再插入 read evidence。
  - Acceptance Criteria:
    - `job.has_evidence` 支持 `job_id`、`evidence_type="read"`、`subject_path`。
    - 无证据返回 `{ "has_evidence": false }`，有证据返回 true。
    - 不把其它 job 或其它 path 的 read 误判为当前 job/path 证据。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook job_has_evidence_is_job_and_path_scoped -- --test-threads=1`

- [ ] **T1.3 写 job.mark_requires_evidence RPC failing test**
  - Files: `tests/pr1b_readfirst_hook.rs` 或 `src/rpc/router.rs` tests。
  - 场景：seed job 后，通过 JSON-RPC 调 `job.mark_requires_evidence`。
  - Acceptance Criteria:
    - params: `job_id`。
    - DB job `requires_physical_evidence == true`，`requires_test_evidence` 保持原值。
    - 重复调用幂等。
    - 不存在 job 返回 typed invalid request。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook job_mark_requires_evidence_sets_physical_gate -- --test-threads=1`

- [ ] **T1.4 写 CCB_JOB_ID env failing test**
  - Files: `tests/pr1b_readfirst_hook.rs`，必要时 `src/provider/manifest.rs` / `src/sandbox/systemd.rs` tests。
  - 场景：agent 收到 dispatched job 后，provider 命令 env 包含当前 job 上下文。
  - Acceptance Criteria:
    - `CCB_SOCKET` 保持透传。
    - 新增 `CCB_JOB_ID=<当前 dispatched job id>` 注入路径。
    - `CCB_JOB_ID` 不作为静态 agent spawn env 固定成旧 job；必须随 job dispatch/send 对应当前 job。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook dispatched_job_env_contains_ccb_job_id -- --test-threads=1`

- [ ] **T1.5 写 Python hook 协议 failing test**
  - Files: `tests/pr1b_readfirst_hook.rs`，`assets/hooks/evidence-hook.sh`。
  - 场景：用 fixture Unix socket 模拟 ccbd JSON-RPC，向脚本 stdin 喂 Claude/Gemini hook JSON。
  - Acceptance Criteria:
    - Claude `Read` 调 `evidence.insert`，`Edit` / `Write` / `MultiEdit` 调 `job.has_evidence` + `job.mark_requires_evidence`。
    - Gemini `read_file` 调 `evidence.insert`，`replace` / `write_file` 调 `job.has_evidence` + `job.mark_requires_evidence`。
    - 脚本通过 `python3 -c` 解析 JSON，不调用 jq。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook evidence_hook_protocol_python_read_and_write -- --test-threads=1`

- [ ] **T1.6 写 deny/allow 输出 failing test**
  - Files: `tests/pr1b_readfirst_hook.rs`。
  - Acceptance Criteria:
    - 无 read 证据时 Claude 输出 `hookSpecificOutput.permissionDecision == "deny"` 与 Evidence Required reason。
    - 无 read 证据时 Gemini 输出 design §3.4 的 `decision == "deny"` / `reason` / `systemMessage`。
    - 有 read 证据时输出 allow，且仍已调用 `job.mark_requires_evidence`。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook evidence_hook_deny_allow_outputs_match_provider_protocols -- --test-threads=1`

### Phase 2: Evidence RPC + Dynamic Arming RPC

- [ ] **T2.1 增加 path-aware evidence 查询 helper**
  - Files: `src/db/evidence.rs`。
  - Acceptance Criteria:
    - 新增 sync + async helper，支持 `job_id` / `evidence_type` / `subject_path` 精确查询。
    - 不破坏既有 `has_job_evidence_sync(conn, agent_id, job_id, evidence_types)` 调用方。

- [ ] **T2.2 实现 handle_evidence_insert**
  - Files: `src/rpc/handlers.rs`。
  - Acceptance Criteria:
    - 校验必填 `agent_id` / `job_id` / `evidence_type` / `subject_path`。
    - `evidence_type` 初期允许 `read`，其它类型沿用 PR-1a helper 但测试固定 read 路径。
    - 调用 `insert_evidence_record`，返回 `evidence_id` / `recorded`。

- [ ] **T2.3 实现 handle_job_has_evidence**
  - Files: `src/rpc/handlers.rs`。
  - Acceptance Criteria:
    - 校验 `job_id` / `evidence_type` / `subject_path`。
    - 返回 `{ "has_evidence": bool }`。
    - subject_path 以项目相对路径字符串比较；路径规范化策略在测试中固定。

- [ ] **T2.4 实现 handle_job_mark_requires_evidence**
  - Files: `src/rpc/handlers.rs`。
  - Acceptance Criteria:
    - 校验 `job_id`。
    - 调用 `set_job_evidence_requirements(db, job_id, true, <保留原 requires_test_evidence>)` 或等价 DB helper。
    - 返回 `{ "job_id": ..., "requires_physical_evidence": true }`。
    - 不做 prompt 文本扫描。

- [ ] **T2.5 注册 RPC router**
  - Files: `src/rpc/router.rs`。
  - Acceptance Criteria:
    - `METHODS` 增加 `evidence.insert`、`job.has_evidence`、`job.mark_requires_evidence`。
    - dispatch match 增加对应 handler。
    - router unknown-method / missing-field 测试不回归。

- [ ] **T2.6 Phase 2 验证**
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook evidence_insert_rpc_records_read job_has_evidence_is_job_and_path_scoped job_mark_requires_evidence_sets_physical_gate -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --lib rpc::router::tests -- --test-threads=1`

### Phase 3: CCB_JOB_ID / CCB_SOCKET Hook 上下文注入

- [ ] **T3.1 CCB_SOCKET passthrough 回归固定**
  - Files: `src/provider/manifest.rs` tests。
  - Acceptance Criteria:
    - `ENV_PASSTHROUGH` 包含 `CCB_SOCKET`。
    - `collect_spawn_env` 在宿主 env 设置 `CCB_SOCKET=/tmp/ccbd.sock` 时透传该键。

- [ ] **T3.2 增加 CCB_JOB_ID env passthrough**
  - Files: `src/provider/manifest.rs`。
  - Acceptance Criteria:
    - `ENV_PASSTHROUGH` 增加 `CCB_JOB_ID`。
    - `collect_spawn_env` 测试覆盖 `CCB_JOB_ID=job_x` 透传。

- [ ] **T3.3 在 job dispatch/send 路径注入当前 CCB_JOB_ID**
  - Files: `src/db/jobs.rs`，`src/orchestrator.rs` 或实际 dispatch/send 相关模块，必要时 `src/rpc/handlers.rs`。
  - Acceptance Criteria:
    - dispatch 到 agent 的命令/输入环境携带当前 `job_id`。
    - 不把 `CCB_JOB_ID` 固定在 agent spawn env，避免后续 job 复用旧 id。
    - 测试证明两个连续 job 的 hook env 使用各自 job_id。

- [ ] **T3.4 Phase 3 验证**
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook dispatched_job_env_contains_ccb_job_id -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --lib provider::manifest::tests -- --test-threads=1`

### Phase 4: Python-based evidence-hook.sh

- [ ] **T4.1 新增 hook 脚本骨架**
  - Files: `assets/hooks/evidence-hook.sh`。
  - Acceptance Criteria:
    - `set -euo pipefail`。
    - 使用 `python3 -c` 做 JSON 解析与输出构造。
    - 不依赖 jq；测试扫描脚本内容不得出现 `jq` 调用。

- [ ] **T4.2 Claude tool 解析**
  - Files: `assets/hooks/evidence-hook.sh`，`tests/pr1b_readfirst_hook.rs`。
  - Acceptance Criteria:
    - 识别 `tool_name == "Read"`，提取 `tool_input.file_path` 或等价字段。
    - 识别 `tool_name in ["Edit", "Write", "MultiEdit"]`，提取目标 path。
    - `NotebookEdit` 可按 design 扩展，但 PR-1b 最小验收固定 Read/Edit/Write/MultiEdit。

- [ ] **T4.3 Gemini tool 解析**
  - Files: `assets/hooks/evidence-hook.sh`，`tests/pr1b_readfirst_hook.rs`。
  - Acceptance Criteria:
    - 识别 `tool_name == "read_file"`。
    - 识别 `tool_name in ["replace", "write_file"]`。
    - 未识别工具默认 allow，不调用 arming RPC。

- [ ] **T4.4 Read evidence 写入**
  - Files: `assets/hooks/evidence-hook.sh`。
  - Acceptance Criteria:
    - 使用 `CCB_JOB_ID` 作为 `job_id` 调 `evidence.insert`。
    - 使用 `CCB_SOCKET` 连接 ccbd RPC。
    - `subject_path` 统一写项目相对路径；无法解析 path 时 allow 且输出 reason。

- [ ] **T4.5 Write/Edit 查询 + 动态武装**
  - Files: `assets/hooks/evidence-hook.sh`。
  - Acceptance Criteria:
    - 写类工具先调用 `job.has_evidence(job_id=CCB_JOB_ID, evidence_type="read", subject_path=path)`。
    - 无论 has_evidence 结果如何，写类工具都调用 `job.mark_requires_evidence(job_id=CCB_JOB_ID)`。
    - 无 read 时 deny；有 read 时 allow。

- [ ] **T4.6 Phase 4 验证**
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook evidence_hook_protocol_python_read_and_write evidence_hook_deny_allow_outputs_match_provider_protocols -- --test-threads=1`

### Phase 5: 显式 ah.toml 集成 + Completion Gate 回归

- [ ] **T5.1 显式 hooks 配置示例**
  - Files: 示例配置或测试 fixture。
  - Acceptance Criteria:
    - Claude 使用 `PreToolUse` 显式声明 `assets/hooks/evidence-hook.sh`。
    - Gemini 使用 `BeforeTool` 显式声明同一脚本或 provider 参数化 wrapper。
    - 没有声明 hooks 时不自动物化，不恢复 PR4c H1 隐式发现逻辑。

- [ ] **T5.2 mark_idle gate 保持 PR-1a 契约**
  - Files: `src/db/state_machine.rs` tests 或 `tests/pr1a_evidence_statemachine.rs`。
  - Acceptance Criteria:
    - `job.mark_requires_evidence` 只设置 `requires_physical_evidence`。
    - completion gate 仍要求 `mtime_changed` / `diff_generated` 等物理修改证据；`read` evidence 只用于 read-before-edit 允许，不单独满足 completion。
    - deny 时保持 DISPATCHED + BUSY，并注入 SYSTEM DENY 提示，不转 FAILED。

- [ ] **T5.3 绕过 hook 的动态武装回归**
  - Files: `tests/pr1b_readfirst_hook.rs`。
  - 场景：hook 曾看到写类工具并调用 `job.mark_requires_evidence`，随后 agent 试图完成但没有 diff/mtime evidence。
  - Acceptance Criteria:
    - `requires_physical_evidence == true`。
    - `mark_idle` 被 evidence gate 拦住。
    - 状态保持 DISPATCHED + BUSY。

- [ ] **T5.4 Phase 5 验证**
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook mark_requires_evidence_blocks_completion_without_physical_evidence -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1a_evidence_statemachine -- --test-threads=1`

### Phase 6: 全局回归 + ship

- [ ] **T6.1 PR1b 目标测试**
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1b_readfirst_hook -- --test-threads=1`
  - Acceptance Criteria:
    - PR1b 新增测试全部通过。

- [ ] **T6.2 PR1a/PR4c 回归**
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr1a_evidence_statemachine -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins -- --test-threads=1`
  - Acceptance Criteria:
    - evidence state machine 与 hooks 物化不回归。

- [ ] **T6.3 lib 回归**
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`
  - Acceptance Criteria:
    - 当前 lib baseline 不退化。

- [ ] **T6.4 重点 acceptance 回归**
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test mvp12_home_layout -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test mvp2_acceptance -- --test-threads=1`

- [ ] **T6.5 ship**
  - Files: 按实际修改文件逐一 `git add <files-by-name>`。
  - Acceptance Criteria:
    - commit message 真实描述 evidence RPC、mark_requires_evidence、CCB_JOB_ID env、Python hook 四块。
    - 不 merge，不 force-push，不 amend。
