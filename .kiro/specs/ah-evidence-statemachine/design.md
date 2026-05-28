# ah PR-1 设计提案：核心状态机与证据链 (Evidence Statemachine)

| 状态 | 完稿 (Round 2 修订) |
| :--- | :--- |
| **日期** | 2026-05-28 |
| **范围** | ah 核心状态机的物理实证拦截与 TDD 流程强制 |

## §0. 数据校验声明

在完成本设计补全前，我已执行以下数据校验：
1. **读取文件**：查阅了 `/tmp/a2-pr4b-redesign-proposal.md`, `/tmp/a3-master-pain-distill.md`, `src/rpc/handlers.rs`, `src/db/state_machine.rs`, `src/monitor/agent_watch.rs`, `assets/builtin/master_rules.md`, `src/bin/ah.rs`。
2. **Grep 实证**：
   - 抓取 `src/rpc/handlers.rs` 中的 `fn handle_` 签名，确认了现有的 RPC 入口形态。
   - 抓取 `src/db/jobs.rs` 中的 job 状态，确认当前真实存在的状态仅为 `QUEUED`, `DISPATCHED`, `COMPLETED`, `FAILED`, `CANCELLED`。
   - 读取 `src/db/schema.rs:45-57` 确认已有 `evidence` 表基建，及 `src/db/evidence.rs` 的 CRUD API 现状。
3. **a1 + a3 双审反馈**：吸收并修正了 Read-first hook 的 Claude 官方协议、`evidence` 表复用扩展、真实状态名统一及 PR 切分重估等 5 项意见。

---

## §1. 任务目标 + 解决的痛点

**目标**：通过在 Rust 底层代码中引入强校验状态机，将 Prompt 层的软约束升级为物理层面的不可逾越的硬屏障。
**解决的痛点**：
- **痛点 2 (不物理实证)**：Agent 习惯凭幻觉或记忆声称任务完成。本设计在 Agent 报 Done 转 `IDLE` 前强制校验其 `Read/ls/diff` 行为。
- **痛点 9 (TDD 流程不守)**：Agent 习惯跳过测试直接 Merge。本设计要求任务进入 `COMPLETED` 前必须在数据库中关联 `TEST_PASS`（如 `cargo test` 绿灯）的证据。

---

## §2. 继承字段表

基于对现有代码和 Schema 的实证梳理：

| 类别 | 字段 / 状态 / 接口 | 现状 | 变更 [NEW/BREAKING] |
| :--- | :--- | :--- | :--- |
| **Agent 状态** | `IDLE`, `BUSY` 等 | 已存在 | 不动 |
| **Job 状态** | `DISPATCHED`, `COMPLETED` 等 | 已存在 | `[NEW]` 将 `EVIDENCE_PENDING` 概念作为 `DISPATCHED` 的内部子状态或检查标识（Job 仍为 `DISPATCHED`，但拦截转 `COMPLETED`）。 |
| **数据库** | `jobs.requires_physical_evidence`, `jobs.requires_test_evidence` | 不存在 | `[NEW]` PR-1a wire-only staging signal：默认 `0`，仅测试 fixture / 后续 arming 路径显式打开；生产 `job.submit` 在 PR-1a 不设置这两个标记，避免 evidence 写入来源尚未接入时阻塞真实 dispatch。 |
| **数据库** | `evidence` 表 | 现存 l3 人工复核语义 | `[BREAKING]` 扩体现有 `evidence` 表，新增字段：`job_id` (NULLABLE), `evidence_type` (TEXT), `subject_path` (TEXT), `payload` (TEXT)。由于 Read-first evidence 可能在 job 创建前发生，故 `job_id` 必须允许 NULL，而 `agent_id` 必填。 |
| **RPC** | `job.submit`, `job.wait` | 基础派单等待 | 不动 (底层被拦截时 `job.wait` 会持续阻塞)。 |
| **RPC** | `agent.mark_idle_matched` | 直接转 `IDLE` | `[BREAKING]` 修改逻辑：转 `IDLE` 前先校验当前 Job 的证据链。 |

> **`evidence` 表 Schema 迁移路径 (Migration Path)**：
> 1. **Persistence**：在 db 迁移脚本中 `ALTER TABLE evidence ADD COLUMN ...`。
> 2. **Router test**：补充 `job_id NULLABLE` 测试用例，验证 hook 记录和 job 创建时序。
> 3. **Integration test**：测试 `evidence.rs` CRUD API 兼容新增字段。
> 4. **E2E test**：验证 Agent 完成流中的证据关联全链路。

---

## §3. PR-1 机制设计 (核心)

### 3.1 物理验证状态机扩展 (Evidence-Required Node)
- **触发条件**：当 PTY 解析器捕获到 Agent 的完成标记 (Marker matched)，调用 `mark_agent_idle_matched_sync` 时触发。
- **状态流转机制**：
  1. 拦截该次流转，查询 `evidence` 表中属于当前 Agent 且关联对应 `job_id` 的有效证据（如 `mtime_changed`, `diff_generated`, `test_passed`）。
  2. 若证据齐全，则按原逻辑流转至 `IDLE`，Job 设为 `COMPLETED`。
  3. **若没证据**，状态机拒绝转 `IDLE`，将 Agent 状态维持在 `BUSY`（或设为特定的阻塞态），并通过 PTY 向 Agent 强行注入系统提示：“*SYSTEM DENY: Missing physical evidence. You must output a git diff or test result before finishing.*”

### 3.2 Edit/Write Read-first hook
强制在修改文件前必须有过读取行为。
- **拦截层**：走 **`settings.json` hooks 注入**（依赖 PR4c，作为 Claude `PreToolUse` Hook 脚本）。
- **检测机制**：
  - Hook 脚本被 Claude 调用时，向 `ccbd` 的本地 Socket 查询当前 `agent_id` 针对目标文件路径是否有过 `Read` 类型的 `evidence` 记录（此时 `job_id` 可能为 NULL）。
- **拦截行为（遵循官方协议）**：
  - 参考 [Claude Hooks 官方文档](https://code.claude.com/docs/en/hooks)：如果未读先写，Hook 必须以 `exit 2` 退出并将拒绝原因输出至 stderr，**或**向 stdout 输出 JSON `{"decision": "deny", "reason": "You must read the file content first"}`。
  - **严禁使用 `exit 1`**（这不符合官方拦截语义）。

### 3.3 Report-done 前强制 ls/diff/cargo test 通过
- **Enforcement 归属**：由 `ccbd` 守护进程强制 Enforce。
- **执行方式**：在 `mark_agent_idle_matched_sync` 时校校验 `evidence` 表中是否有对应的 `test_pass` 记录。
- **失败处理**：同 3.1，拒绝转 `IDLE` 和 `COMPLETED`，通过 PTY 返回拒绝原因。

### 3.4 PR-1a wire-only scaffolding
- **PR-1a 范围**：只落库 schema、查询 API、状态机 gate 和拒绝结果回传能力；生产 `handle_job_submit` 不武装 evidence gate，`jobs.requires_physical_evidence` / `jobs.requires_test_evidence` 默认保持 `0`。
- **原因**：PR-1b / PR4c 前尚未接入可靠的 tool hook evidence 写入来源；若 PR-1a 直接全量武装，会让真实 dispatch 因永远缺 evidence 而卡死。
- **后续 arming**：等 PR-1b/PR4c 接入 Read/diff/test evidence 写入后，再由生产 dispatch 路径按任务类型显式设置 staging signal，并把 §3.1 / §3.3 从 wire-only 变为生产强制。

---

## §4. 现有代码兼容性

| RPC 端点 / 函数 | 兼容性评估 | 影响细节 |
| :--- | :--- | :--- |
| `handle_session_spawn_master_pane` | 独立 | 不受影响，正常创建。 |
| `handle_agent_spawn` | 独立 | 不受影响，正常拉起。 |
| `cmd_ask` | 独立 | 不受影响。无论底层 Agent 是否因为缺少证据被反复打回，CLI 只关注最终 `job.wait` 返回。 |
| `cmd_pend` | 独立 | 不受影响。 |

---

## §5. PR4c 关系再确认

| PR-1 子部分 | 跟 PR4c 关系 |
| :--- | :--- |
| **§3.1 状态机拦截** | **独立** (Rust 实现，可立即实施) |
| **§3.2 Read-first hook** | **依赖 PR4c** (必须等 PR4c 提供 `PreToolUse` hook 注入基建) |
| **§3.3 Done 前强制** | **独立** (Rust 实现，可立即实施) |

- **实施顺序**：由于 §3.1 和 §3.3 是独立于用户态 Hook 的 Rust 层验证，**PR-1a** 可以先于 PR4c 独立实施；PR4c 完成后再实施需要 Hook 注入框架的 **PR-1b**。

---

## §6. 实施切片与测试设计 (Tests-First)

考虑到工作量，PR-1 拆分为两个独立的子 PR：

- **PR-1a: 核心拦截与证据表扩展**（涵盖 §3.1, §3.3，约 150-250 LOC，独立实施）
- **PR-1b: Read-first Hook 注入**（涵盖 §3.2，约 400-700 LOC，5-8 文件，依赖 PR4c）

**Failing Tests (红灯) 验收场景**：
### 6.1 状态机拦截验收 (PR-1a)
- **场景**：派发改代码 Job，Agent 打印完成 Marker 但系统内无 `diff` 证据。
- **断言**：`assert!(state != IDLE)` 且 `assert!(job.status != COMPLETED)`，Agent PTY 末尾出现 `SYSTEM DENY: Missing physical evidence`。

### 6.2 Done 前 TDD 验收 (PR-1a)
- **场景**：派发 TDD 任务，Agent 直接报 Done 且无 `cargo test` 绿灯证据。
- **断言**：Job 不得从 `DISPATCHED` 转 `COMPLETED`，触发拦截打回。

### 6.3 Edit-First Hook 验收 (PR-1b)
- **场景**：Agent 尝试调用 `Edit` 工具修改 `src/main.rs`，但 `evidence` 表无针对该文件的 `Read` 记录。
- **断言**：Hook 脚本返回 `exit 2` 并在 stderr 输出 `deny` 理由，Claude 工具调用失败。

---

## §7. 风险 + 待 PM 拍板 (Open Issues)

- **议题 1**："Read-first hook 拦截层: RPC 拦 (ccbd 端) vs settings.json hooks 注入 (依赖 PR4c)"
  - **我推荐**：**settings.json hooks 注入 (依赖 PR4c) 并作为 PR-1b 延后实施**。
  - **理由**：通过 Provider 原生的 Tool Hook 拦截（`exit 2` 协议）极度准确且能被模型自我消化，而 RPC 层截获文本既脆弱又容易被绕过。遵循 Claude 官方协议是第一性原理的最佳实践。
  - **置信度**：证据: H | 影响: H | 方案置信度: A

- **议题 2**："Evidence 存储: 扩充现有 `evidence` 表 vs 起新表"
  - **我推荐**：**扩充现有 `evidence` 表 (方案 a)**。
  - **理由**：现有 `evidence` 已经包含了 `agent_id`, `event_seq_id` 及状态追踪基建，扩充只需加 nullable 的 `job_id` 等字段，工程代价最小，DB 迁移最平滑，复用了数据层的基础设施。
  - **置信度**：证据: H | 影响: M | 方案置信度: A
