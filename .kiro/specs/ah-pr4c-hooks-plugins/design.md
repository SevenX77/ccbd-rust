# PR4c 设计提案：Hooks 与 Plugins 的双侧物化

| 状态 | 草案 (Draft) |
| :--- | :--- |
| **日期** | 2026-05-28 |
| **范围** | ah 环境中自定义扩展能力（钩子与插件）的物化与激活 |

## 1. 设计初衷

PR4c 旨在实现 `ah.toml` 中声明的 `hooks` 和 `plugins` 在沙箱环境中的自动部署。与规则（Rules）不同，Hooks 和 Plugins 的生效不仅依赖于文件存在，还必须在 Provider 的配置文件（如 `settings.json`）中显式注册。本设计通过“双侧物化”机制确保扩展能力“开箱即用”。

---

## 2. 核心机制：双侧物化 (Dual-Side Materialization)

物化过程分为两个并发阶段：

1.  **文件侧 (File Side)**：将脚本文件或插件目录从宿主机缓存（或项目目录）链接（Symlink）到沙箱内部的特定约定路径。
2.  **注册侧 (Registration Side)**：在物化沙箱配置文件（如 `settings.json`）时，动态注入注册信息，完成能力激活。

---

## 3. 对照真实约定的落点表

基于对宿主机 `~/.claude` 和 `~/.codex` 的实证调研，确定的物化落点如下：

| 资源类别 | 物理文件落点 (Relative to Sandbox HOME) | 配置激活方式 |
| :--- | :--- | :--- |
| **Claude Hooks** | `.claude/hooks/<name>.sh` | `settings.json` -> `hooks.<EventName>[]` 嵌套数组注入。必须支持 `hookSpecificOutput.permissionDecision` 协议。 |
| **Claude Plugins** | `.claude/plugins/cache/<id>/` | `settings.json` -> `enabledPlugins.<id>` 设为 `true`。 |
| **Codex Plugins** | `.codex/plugins/cache/<id>/` | `config.toml` -> `[plugins."github@openai-curated"] enabled = true` (需完整 ID)。 |
| **Gemini Hooks** | `.gemini/settings.json` | `hooks.<EventName>[]` 数组对象，含 `type/command/matcher/timeout`。 |
| **Gemini Plugins** | 暂无原生协议 | 调研未发现 `~/.gemini/plugins` 目录或相关配置。 |

---

## 4. 与 PR4b 物化流水线的衔接

PR4c 的逻辑将嵌入 `src/provider/home_layout.rs` 中的 `prepare_home_layout_with_role` (L45) 流水线：

1.  **数据透传**：`ah.toml` 中的 `hooks` 和 `plugins` 声明经 RPC 传递至 `ccbd`。**注意**：需先重构 `prepare_home_layout` 接口以接收扩展参数，此为 6.1 验收的前提 API Seam。
2.  **配置修补 (Patching)**：
    -   在调用 `materialize_claude_settings` (L311) / `materialize_gemini_settings` (L231) / `prepare_managed_codex_home` (L331) 等函数时，增加修补逻辑。
    -   使用 `serde_json` 或 `toml` 加载模板，根据声明动态注入嵌套的 `hooks` 组（含 `matcher: "*"` 默认包装）或 `enabledPlugins`。
3.  **文件物化**：
    -   在沙箱内创建 `.claude/hooks/`、`.claude/plugins/cache/` 或 `.codex/plugins/cache/` 目录。
    -   执行 `symlink` 操作，将本地资源挂载至上述落点。

---

## 5. 继承字段表 (Inherited Fields Audit)

| 类别 | 字段 / 接口 | 现状 [file:line] | PR4c 变更 [NEW/BREAKING] |
| :--- | :--- | :--- | :--- |
| **ah.toml** | `[master]` | `cmd: String` (src/cli/config.rs:28), `enabled: bool` (src/cli/config.rs:30) | `[NEW]` 增加 `rules: Vec<String>`, `skills: Vec<String>`, `hooks: HashMap<String, Vec<HookGroup>>`, `plugins: Vec<String>`。 |
| **ah.toml** | `[agents.<id>]` | `provider: String` (src/cli/config.rs:58), `env: HashMap<String, String>` (src/cli/config.rs:60) | `[NEW]` 增加 `rules: Vec<String>`, `skills: Vec<String>`, `hooks: HashMap<String, Vec<HookGroup>>`, `plugins: Vec<String>`。 |
| **RPC** | `agent.spawn` | `extra_env_vars: HashMap<String, String>` (src/rpc/handlers.rs:317) | `[NEW]` 增加 `rules_layers: Vec<String>`, `skills: Vec<String>`, `hooks: HashMap<String, Vec<HookGroup>>`, `plugins: Vec<String>`。 |
| **物化配置文件** | `settings.json` / `config.toml` | 基础结构注入 | `[NEW]` 动态注入嵌套 `hooks` (Claude/Gemini) 和插件完整 ID 激活字段。 |

> **HookGroup 定义**：`{ matcher: String, hooks: Vec<HookItem> }`，其中 `HookItem` 为 `{ type: "command", command: String, timeout: Option<u64> }`。

---

## 6. 用户视角与验收场景

### 6.1 验收场景 (Tests-First)

按 SOP-08 准则设计的红灯验收方案（需在 `prepare_home_layout` 接口重构完成后执行）：
1.  **Claude Hook 自动化验证**：
    -   **输入**：`ah.toml` 中 `[agents.a1.hooks] PreToolUse = ["./scripts/audit.sh"]`。
    -   **预期**：Sandbox 内 `~/.claude/settings.json` 的 `hooks.PreToolUse` 呈现官方嵌套结构 `[{"matcher":"*", "hooks":[{"type":"command", "command":"..."}]}]`；`~/.claude/hooks/audit.sh` 是有效 symlink。
2.  **Codex Plugin 激活验证**：
    -   **输入**：`ah.toml` 中 `[agents.a2] \n plugins = ["github@openai-curated"]`。
    -   **预期**：Sandbox 内 `~/.codex/config.toml` 含 `[plugins."github@openai-curated"] enabled = true`；物理插件目录存在。
3.  **Hook 协议连通性**：
    -   **场景**：执行 `PreToolUse` hook 脚本时，验证脚本能通过 `CCB_SOCKET` 向 `ccbd` 发起查询，并能输出符合 `hookSpecificOutput.permissionDecision` 协议的 JSON。

### 6.2 为什么要这么设计？
-   **真正的“自动驾驶”**：你不再需要手动在每个项目的沙箱里打 `claude config set plugins...`。只要在 `ah.toml` 里写一次，ah 启动时会自动帮你链接好插件并改好配置文件。
-   **环境确定性**：由于 `settings.json` 是在启动时根据 `ah.toml` 动态生成的，你可以确信每个 Agent 都只拥有它被授权的钩子和插件，没有任何残留的陈旧配置。

---

## 7. 风险与决策议题 (Decision Log)

### 议题 7.1: Gemini 扩展能力 Scope 判定
-   **现状**：Gemini 已具备 Hook 协议（AfterAgent/BeforeAgent），但尚无标准 Plugin 目录。
-   **推荐**：PR4c 仅实现 Gemini 的 **Hooks 物化**。Plugins 标记为 "Unsupported in current Gemini CLI version"，待上游更新后补齐。
-   **三轴判定**：
    -   **证据**：实测 `~/.gemini/` 无 plugins 目录。
    -   **影响**：Low (L)。当前 Gemini 主要作为辅助 Analyst 使用。
    -   **置信度**：High (A)。

### 议题 7.2: 外部资产（Git URL）的 Provisioning 归属
-   **争议**：是否在 PR4c 中实现 `git clone` 逻辑？
-   **推荐**：**不归属 PR4c**。PR4c 专注于“物化（Materialization）”——即本地已存在资产的部署。Git 下载逻辑归属 PR4d (Auto-provisioning)。
-   **三轴判定**：
    -   **证据**：SOP-06 最小化变更原则；PR4d 已在 pending 列表中。
    -   **影响**：Medium (M)。明确了模块边界，降低 PR4c 复杂度。
    -   **置信度**：Medium (B)。

### 议题 7.3: Hook 脚本的物化策略：Symlink vs Copy
-   **争议**：Symlink 可能导致 Sandbox 泄露宿主机路径信息。
-   **推荐**：**默认使用 Symlink**。原因：Hook 脚本通常在项目开发过程中频繁调整，Symlink 可实现“零延迟生效”。若有极高隔离需求，后续可在 `ah.toml` 中增加 `strategy = "copy"`。
-   **三轴判定**：
    -   **证据**：现有 auth 文件已使用 symlink，且 sandbox 本身有 bind-mount 隔离。
    -   **影响**：High (H)。显著提升开发效率。
    -   **置信度**：High (A)。

---

## 8. 审计实证记录 (Evidence Trail)


为了确保设计不落空，我执行了以下实证操作：
1.  `cat ~/.claude/settings.json | jq '.hooks'`: 验证了 Claude hooks 的嵌套数组结构。
2.  `ls -la ~/.claude/plugins/`: 验证插件目录约定。*注：在未安装任何三方插件的环境下，该目录可能为空或不存在，但其 `cache/` 布局已由 `codex` 影子环境对齐证实。*
3.  `cat ~/.gemini/settings.json | jq '.hooks'`: 确认了 Gemini hooks 的嵌套配置结构。
4.  `cat ~/.codex/config.toml`: 确认了 Codex 插件在 `[plugins."id"]` 段落下的 `enabled = true` 激活方式。
5.  `prepare_home_layout` 源码阅读: 确认了现有的 `materialize_claude_settings` 是嵌入逻辑的最佳切入点。

