# Design: ah PR-1b Evidence Collection Hook (Read-First)

| 状态 | 草案 (Draft) |
| :--- | :--- |
| **日期** | 2026-05-28 |
| **范围** | 基于 Hooks 的物理证据采集与拦截 (Read-before-Edit) |

## 1. 目标 + 痛点对齐

PR-1b 旨在通过物理拦截手段闭环 [PR-1a](../../ah-evidence-statemachine/design.md) 定义的证据状态机。它通过在 Agent 执行敏感操作（如修改文件）前强制要求前置动作（如读取文件），解决以下核心痛点：

- **痛点 2: Read-before-Edit 一意识到就跳过**。Agent 经常在不了解文件内容的情况下直接尝试修改，导致生成低质量或错误的补丁。
- **痛点 9: Done 自欺欺人**。Agent 声明完成任务但实际未进行必要的验证。PR-1b 通过物理证据（Read/Test）的强制要求，让 "COMPLETED" 状态具备物理真实性。

---

## 2. 继承字段表 (Inherited Fields Audit)

| 类别 | 字段 / 接口 | 现状 [file:line] | PR-1b 变更 [NEW/BREAKING] |
| :--- | :--- | :--- | :--- |
| **Evidence DB** | `evidence` table | `job_id`, `evidence_type`, `subject_path` (src/db/schema.rs:53-54) | `[NEW]` `evidence_type` 增加 `read` 常量。 |
| **Jobs DB** | `jobs` table | `requires_physical_evidence`, `requires_test_evidence` (src/db/schema.rs:76) | `[NEW]` 激活 `requires_physical_evidence = 1` 的生产路径。 |
| **Hooks 基建** | `ExtensionConfig` | `hooks` 字段 (src/provider/extensions.rs:6-7) | 无。 |
| **RPC** | `evidence.insert` | **不存在** | `[NEW]` 暴露 `handle_evidence_insert` 接口。 |
| **RPC** | `job.mark_requires_evidence` | **不存在** | `[NEW]` 供 Hook 动态武装 Job 关卡。 |

---

## 3. 核心机制

### 3.1 环境变量注入 (a1-G1)
在 `ccbd` 调度 Job 时，通过环境变量注入上下文，确保 Hook 脚本具备可识别性：
- `CCB_JOB_ID`: 当前活跃的 Job ID。
- `CCB_SOCKET`: RPC 通讯路径。

### 3.2 证据写入 (Detection)
在 Claude/Gemini 沙箱中，通过 `ah.toml` 显式声明注入 `PreToolUse` (Claude) 或 `BeforeTool` (Gemini) Hook。
- **Claude 工具匹配**: `Read`
- **Gemini 工具匹配**: `read_file`
- **动作**: Hook 拦截到上述工具调用后，通过 Python (`python3 -c "import json, sys; ..."`) 解析 STDIN，并向 `ccbd` 发起 `evidence.insert` RPC。

### 3.3 Read-first 拦截与动态武装 (F1 推荐方案)
在执行写类工具前：
- **Claude 匹配**: `Edit` | `Write` | `MultiEdit` | `NotebookEdit`
- **Gemini 匹配**: `replace` | `write_file`
- **逻辑流程**:
  1. **查询 (Query)**: Hook 查询 `ccbd`：“当前 Job 对目标文件是否有 `read` 证据？”。
  2. **决策 (Decision)**: 若无证据，返回 `deny` 结构（见 §3.4）。
  3. **动态武装 (Arming)**: 无论是否 deny，Hook 检测到写意图时，立即调用 `job.mark_requires_evidence(CCB_JOB_ID)`。这确保了后续 `mark_idle` 阶段必须看到 `mtime/diff` 物理证据，闭环防绕过逻辑，且无需扫描 prompt 文本。

### 3.4 跨 Provider Hook 契约 (F2/F5)
由于 `jq` 非沙箱标配，Hook 统一使用 **Python -c** 处理 I/O。

**Claude Deny Output (STDOUT)**:
```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "Evidence Required: You must read the file before editing it."
  }
}
```

**Gemini Deny Output (STDOUT)**:
```json
{
  "decision": "block",
  "reason": "Evidence Required: You must read the file before editing it.",
  "systemMessage": "🛡️ Read-First Gate Blocked Action"
}
```

---

## 4. 现有代码兼容性与基建

- **显式声明原则 (a1-G2)**: Hook 脚本不默认物化。仅在 `ah.toml` 中通过 `hooks` 字段显式启用时才进入沙箱。
- **RPC 扩展**: `src/rpc/handlers.rs` 需补齐 `handle_evidence_insert` 和 `handle_job_mark_requires_evidence`。

---

## 5. PR 范围 + 实施切片

1. **M1 (RPC & Dispatch)**: 实现 `CCB_JOB_ID` 注入及 `evidence.insert` / `mark_requires_evidence` 接口。
2. **M2 (Hook 脚本)**: 编写基于 Python 解析的 `evidence-hook.sh`。
3. **M3 (Integration)**: 更新 `ah.toml` 示例，验证 Claude/Gemini 拦截闭环。

---

## 6. 验收场景 (Tests-First)

### 场景 1: 裸写拦截 (Claude)
- **输入**: Agent 直接调用 `Edit(file="src/lib.rs", ...)`。
- **预期**: Hook 返回 `deny`，Agent 界面提示 "Evidence Required"。

### 场景 2: 顺次执行 (Gemini)
- **步骤**: 先 `read_file(path="src/lib.rs")` 再 `replace(path="src/lib.rs", ...)`。
- **预期**: `read_file` 产生证据；`replace` 校验通过，允许执行。

### 场景 3: 绕过 Hook 路径拦截 (Shell Bypass)
- **场景**: Agent 通过 `run_shell_command` 等非 Hook 覆盖手段修改代码。
- **流程**: 此时 Hook 未被触发，无法通过动态标记锁定物理证据要求。
- **结果**: 若该 Job 预先由 L3/RPC 标记了 `requires_physical_evidence`，则 `mark_idle` 仍会因缺失 Read/Diff 证据而拦截；若未预标记，则此类“逃逸”行为依赖后期审计。
- **预期状态**: 保持 **DISPATCHED + BUSY**，注入 SYSTEM DENY 提示。

---

## 7. 风险 + 待 PM 拍板

| 议题 | 描述 | 影响 | 推荐 |
| :--- | :--- | :--- | :--- |
| **议题 7.1: Python 依赖** | 假设沙箱必有 Python3。 | Medium (M) | **OK**。Provider (Claude/Gemini) 自身依赖 Python。 |
| **议题 7.2: 写类工具漏检** | 随着 Provider 更新，新工具可能逃逸。 | Medium (M) | **通配符匹配**。对 `*Edit*` 等模式进行前置覆盖。 |
| **议题 7.3: 证据隔离度** | 跨 Sandbox 的 Read 证据是否共享？ | Low (L) | **Job 级隔离**。Read 必须发生在当前 Job 生命周期内才算有效。 |
| **议题 7.4: Shell Bypass** | `run_shell_command` 绕过 Hook 的处理。 | Low (L) | **Evidence Gate 兜底**。shell 绕过无法完全防住，主要靠 `mark_idle` 时的物理证据（mtime/diff）强制检查作为最后防线。 |

