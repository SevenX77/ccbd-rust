# Design: ah PR2 - 隔离核心 (Isolation Core) - 定稿

本文档定义了 `ah` (Agent Hypervisor) 的 PR2 阶段最终设计方案。PR2 旨在通过环境变量重定向和物理路径软链接，实现 Agent 与宿主配置的深度隔离，同时彻底移除 `bwrap` 内核沙盒依赖。

## 0. 设计定位与愿景

根据 User 最终指令，`ah` 定位于高可靠、易用的 Agent 隔离编排产品。
*   **隔离逻辑**：放弃强制 `bwrap` 沙盒，全面转向“环境变量配置重定向”。
*   **配置定向不允许出错**：这是 PR2 的核心红线。配置必须 100% 准确落入沙盒虚拟目录，严禁向宿主真实配置目录泄漏。
*   **权限模型**：维持现有 `ccb` 权限级别，不做跨沙盒防读等强力权限管控，以换取更好的易用性。

---

## 1. 继承与现状审计 (Inherited Fields)

以下字段和变量经主控实证（含 binary 内核字符串与 bundle 源码分析）后的定稿策略。

| 类别 | 标识符 | 现状 (ccbd-rust) | PR2 策略 | 理由 |
|---|---|---|---|---|
| **Env Var** | `CLAUDE_CONFIG_DIR` | 尚未引入 | **[NEW] 唯一重定向变量** | Claude 2.1+ 官方支持的配置目录重定向。 |
| **Env Var** | `CODEX_HOME` | `home_layout.rs:109` 已使用 | **保持使用** | 已实证在 Codex 隔离中表现稳定。 |
| **Env Var** | `GEMINI_CLI_HOME` | 尚未引入 | **[NEW] 唯一重定向变量** | 经 bundle 源码实证为 Gemini 官方唯一配置重定向变量。 |
| **Env Var** | `CLAUDE_PROJECTS_ROOT`| `home_layout.rs:68` | **[BREAKING] 移除** | **实证为空操作**。迁移路径：影响 `mvp12_home_layout.rs:109` 断言。 |
| **Env Var** | `CLAUDE_PROJECT_ROOT` | `home_layout.rs:72` | **[BREAKING] 移除** | **实证为空操作**。单一变量原则下删除。 |
| **Env Var** | `GEMINI_ROOT` | `home_layout.rs:96` | **[BREAKING] 移除** | **实证为空操作**。Gemini 不读此变量，现有隔离全靠 bwrap 改 HOME。 |
| **Env Var** | `CODEX_SESSION_ROOT` | `home_layout.rs:111` | **[BREAKING] 移除** | **实证为空操作**。迁移路径：影响 `mvp12_home_layout.rs:145` 断言。 |
| **Const** | `PROVIDER_AUTH_WHITELIST` | `home_layout.rs:14` 包含 5 项 | **保持使用** | 包含 `.claude.json`, `.claude/.credentials.json` 等关键凭据。 |
| **Function** | `copy_credentials` | `home_layout.rs:173` 执行 COPY | **[BREAKING] 改名并重写** | 改为 `link_credentials`，使用 `symlink` 以支持 Token 自动双向同步。 |

---

## 2. 核心改动设计

### 2.1 彻底删除 bwrap 依赖 [BREAKING]

根据 User 决策，`ah` 追求轻量化，不再提供内核级沙盒选项。

*   **迁移路径**：
    *   **删除文件**：彻底删除 `src/sandbox/bwrap.rs`。
    *   **重构 `src/sandbox/mod.rs`**：移除 `bwrap_available` 字段，`check_environment` 不再探测 `bwrap` 二进制。
    *   **重构 `src/sandbox/systemd.rs`**：`wrap_command` 函数删除 `bwrap_args` 参数，不再拼接 `bwrap` 指令。
    *   **单元测试清理**：删除所有依赖 `bwrap` 行为的测试用例。

### 2.2 配置重定向：精准定向 [NEW]

在 `src/provider/home_layout.rs` 的各个 Provider `overrides` 函数中，注入以下定稿的环境变量，确保存量/新装 CLI 均能精准命中沙盒：

1.  **Claude**: `CLAUDE_CONFIG_DIR` -> `sandbox_path(".claude")`
2.  **Codex**: `CODEX_HOME` -> `sandbox_path(".codex")`
3.  **Gemini**: `GEMINI_CLI_HOME` -> `sandbox_path(".gemini")`

### 2.3 OAuth 供给：Symlink 机制 [BREAKING]

将凭据同步逻辑从物理复制改为符号链接，实现“宿主登录，沙盒透明可用”。

*   **逻辑说明**：在沙盒初始化时，针对 `PROVIDER_AUTH_WHITELIST` 列表中的每个文件，建立从 `host_path -> sandbox_path` 的软链接。
*   **迁移路径**：修改 `home_layout.rs` 及其单测，将断言从 `fs::read_to_string` 校验内容改为 `fs::read_link` 校验软链目标。

---

## 3. 验证方案 (Verification Plan)

对应 User 指令：“配置定向不允许出错”。必须通过以下自动化验证方案方可验收。

### 3.1 路径隔离集成测试 (E2E Config Drift Check)
设计专门的测试脚本，模拟三个 CLI 的真实调用：
1.  **执行逻辑**：
    *   在 `ah up` 后，向隔离的 Agent 发送 `ls -a ~` 和 `env` 指令。
    *   **预期 1**：Agent 反馈的环境变量中，`CLAUDE_CONFIG_DIR`/`CODEX_HOME`/`GEMINI_CLI_HOME` 必须指向 `/home/sevenx/.cache/ah/sandboxes/...` 下的路径。
    *   **预期 2**：在 Agent 内执行一个简单的配置修改指令（如 `claude config set theme light`），检查宿主 `~/.claude/settings.json` 的 `mtime` 是否变化。若变化，则判定为“定向出错/泄漏”。
2.  **文件残留检测**：
    *   运行测试后，核实宿主真实 HOME 下不应产生任何带项目 ID 特征的新文件或新配置项。

---

## 4. 冲突处理与兼容性说明

*   **单一变量原则**：不再注入 `GEMINI_ROOT` 或 `CLAUDE_PROJECT_ROOT` 等辅助变量，仅使用经实证的唯一正确变量。
*   **PR2 与 PR3 的关系**：
    *   PR2 删除了 `bwrap` 但**保留并简化**了 `src/sandbox/systemd.rs` 中的 `systemd-run --scope` 包装能力。
    *   这为 PR3 注入 `--property=BindsTo=ah.service` 提供了干净的插槽。

---

## 5. OPEN QUESTIONS (已全部解决)
1.  **bwrap 源码**：User 拍板彻底删除。
2.  **权限管控**：User 拍板不做特殊加固，沿用现有关联权限。
3.  **变量名**：已实证并确定唯一正确名为 `GEMINI_CLI_HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`。
