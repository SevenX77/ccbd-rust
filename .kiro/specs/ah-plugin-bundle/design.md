# ah Plugin Bundle 设计（v2）

| 项 | 值 |
| :--- | :--- |
| 状态 | 设计定案（design only，Q1-Q11 已拍板） |
| 作者 | a4 |
| 日期 | 2026-07-02 |
| 范围 | ah 自有 "plugin bundle"：把 skills + hooks + rules + MCP 打成一个包，ah.toml 一个配置项引用，ah 按 provider 各自翻译并注入 |
| 前置 | 复用 v1 skills 注入（分支 `feat/skills-injection`）、`src/provider/home_layout.rs` 现有 `materialize_*`、`src/provider/extensions.rs` 的 `ExtensionConfig`、`src/provider/fingerprint.rs` 的漂移机制 |

**本阶段只出设计。不写实现，不跑 cargo。** 结构与详尽度对齐 `.kiro/specs/ah-macos-port/design.md`。

---

## 0. 立足现状（先钉事实，再谈设计）

设计必须踩在现有基建上。下面是从代码读出的事实（带 file:line）与已实测闭合的 provider 落点。

### 0.1 skills v1 的真实形态（三 provider 通用，已实测闭合）

skills v1 已确认可按 provider 分发到各自可消费目录，bundle 设计直接复用该能力。三 provider 的真实落点如下：

- claude：`$CLAUDE_CONFIG_DIR/skills/<name>`
- codex：`$CODEX_HOME/skills/<name>`
- antigravity：`<沙箱 home>/.gemini/config/skills/<name>`（支持 symlink）

源目录 `.ah/skills/<name>`（须含 `SKILL.md`）由 `resolve_project_skills` 解析，带 canonicalize + symlink-escape 防逃逸。bundle skills 继续沿用相同目录结构与防逃逸规则，按 provider 翻译到上述落点。skills **未进 fingerprint**（PR4c/PR4e 已把 rules/skills 移出 fingerprint scope，见 `.kiro/specs/ah-pr4e-up-fingerprint/design.md`），但 bundle 内容会通过整包 `BundleDigest` 纳入 fingerprint（§4.4）。

### 0.2 hooks / plugins / rules 现状（`src/provider/home_layout.rs`）

物化统一入口：`prepare_home_layout_with_extensions_for_slot`（`home_layout.rs:132`），按 provider 分派到 `prepare_claude_overrides` / `prepare_codex_overrides`(→`prepare_managed_codex_home`) / `prepare_antigravity_overrides`；未知 provider 静默 no-op（`home_layout.rs:177-180`）。`ExtensionConfig`（`src/provider/extensions.rs:5`）当前只有 `hooks` + `plugins`（分支上 +`skills`）。

| 内容 | claude | codex | antigravity |
| :--- | :--- | :--- | :--- |
| hooks | 脚本 symlink 到 `.claude/hooks/`，声明注入 `.claude/settings.json` 的 `hooks`（event→matcher→[{type,command,timeout}]）。`materialize_claude_hooks`(`:611`)+`inject_claude_hooks`(`:655`) | 拷 `.codex/hooks.json` + `config.toml [features] hooks=true` + `merge_codex_hook_push`(`:796-836`) | `.gemini/config/hooks.json`（顶层命名对象，非 event-keyed）+ `enableJsonHooks=true` 门（`:279-331`）；仅 `active_hook_push_ctx` 命中才物化 |
| plugins | 两条 symlink `.claude/plugins/<name>`+`.claude/plugins/cache/<name>` + `enabledPlugins` 入 settings（`:715-739`）。spec 支持 `id` 或 `name@git@url#ref`（`plugins.rs`），git 落 `$XDG_CACHE_HOME/ah/cache/git/...` | 一条 symlink `.codex/plugins/cache/<name>` + `config.toml [plugins.<name>] enabled=true`（`:913-953`） | **不支持**：`prepare_antigravity_overrides` 根本不接收 extensions，`plugins` 被静默忽略 |
| rules | 组合 markdown 写入 `.claude/CLAUDE.md`（master+worker 都写）。`materialize_builtin_rules`(`:392`)：`compose_rules(kernel, body)`，body = `.ah/rules/<slot_id>.md` 或内建默认 | worker 写 `.codex/AGENTS.md`；**master 早退不写**（`:402-404`） | worker 写 `.gemini/AGENTS.md`；**master 早退不写** |

copy vs symlink 规律：**脚本/插件目录 = symlink**（`force_symlink`，`:1310`，先删后建）；**rules markdown / 各类 settings/config = 拷贝或写入后就地编辑**。

### 0.3 MCP 现状：greenfield

ah **当前完全不处理 MCP**。全仓 `src/` 只有 4 处 `mcp` 命中，都是 `.claude.json` trust 对象里的**空占位**（`home_layout.rs:1082-1093`：`mcpServers`/`mcpContextUris`/`enabledMcpjsonServers`/`disabledMcpjsonServers` 初始化为空、从不填充）。`ah.toml`、`.kiro/specs/` 无任何 MCP 字段/设计。各 provider 的 MCP 原生落点（据仓内 `docs/agent-cli-knowledge-base/`）：

| provider | MCP 落点 | 格式 |
| :--- | :--- | :--- |
| claude | `.claude.json` 的 `mcpServers`（ah 已建空占位）/ workspace 的 `.mcp.json` | JSON `{command,args,env}` 或 `{url,headers}` |
| codex | `.codex/config.toml` 的 `[mcp_servers.<name>]` | TOML `command/args/env` |
| antigravity | `.gemini/config/mcp_config.json` 的 `mcpServers` | JSON；stdio 用 `command`/`args`/`env`（`${VAR}` 展开）；远程用 `serverUrl` |

antigravity MCP 已确认落点为 sandbox home 下的 `.gemini/config/mcp_config.json`，不是 settings.json。antigravity 不支持 legacy `url`/`httpUrl` 形态；bundle 若声明 provider 不支持的 MCP 形态，按 Q8 的默认硬错/manifest optional 规则处理。

### 0.4 fingerprint / 漂移（`src/provider/fingerprint.rs`）

`ConfigFingerprintInput{ role, hooks, plugins }`（`fingerprint.rs:17`），`role` = `Master{cmd}` | `Agent{provider,env}`。`compute_config_hash` 做 sorted-key deterministic JSON → SHA256 hex。存 `sessions.config_hash` / `agents.config_hash` / `agent_spawn_specs.config_hash`。比较在 `realign.rs`：agent 不一致→kill+respawn；master 不一致→审计（DRIFT），除非 `--force`。**skills/rules 不在 fingerprint 内**。四个 `compute_config_hash` 调用点：`sessions.rs:353`、`agent.rs:269`、`sessions.rs:837`、`realign.rs:87/101/160`。恢复走 `AgentSpawnSpec`（`db/recovery.rs`）+ `spawn_realign_agent`（`realign.rs:317`），master 复活重供 `master_watch.rs:1934`。

**红线**：任何新增、可离开 ah.toml 独立变化的物化输入（bundle 内容会变而 `ah.toml` 不变），**必须**进 fingerprint，否则会"内容漂移但不重建"。

---

## 1. Bundle 磁盘结构与 manifest

### 1.1 核心心智模型

> **Bundle 是 `ExtensionConfig` 的一个"来源"，不是新的物化通道。**

Bundle 在**解析层**被展开成一份 `ExtensionConfig` 贡献（skills/hooks/rules/mcp），与 `ah.toml` 里散配的字段**合并**成"有效 ExtensionConfig"，再交给**现有** `materialize_*` 物化。这样：

- 物化层几乎不改（复用 `materialize_claude_skills/hooks`、codex/antigravity 的 hooks 写入、`materialize_builtin_rules`）；
- 新增的只有两类物化器：**rules 叠加层**（在既有 `compose_rules` 中插一层）和 **MCP 写入器**（greenfield，三 provider 各一）；
- fingerprint 只需覆盖"bundle 内容摘要"，不必逐字段散进。

### 1.2 目录布局

```text
<project_root>/.ah/bundles/<bundle-name>/
  bundle.toml              # 清单（必需，manifest）
  skills/
    <skill-name>/
      SKILL.md             # 复用 v1 skill 格式，整目录 symlink
      ...                  # 附带资源
  hooks/
    <script-file>          # hook 可执行脚本；由 bundle.toml [hooks] 引用
  rules/
    master.md              # 可选：master 角色规则片段
    worker.md              # 可选：worker 角色规则片段
  mcp/                     # 可选：MCP 辅助文件（如 .env 模板、CA 证书），预留
```

约束（沿用 v1 skills 的防逃逸做法）：`<bundle-name>` 与内部路径经 canonicalize，**必须落在 `.ah/bundles/<name>/` 内**；拒绝绝对路径、`..`、symlink 逃逸、跨 bundle 引用。

### 1.3 `bundle.toml`（manifest，必需）

manifest 承载**结构化声明**（hooks 的 event→matcher 映射、rules 的角色路由、MCP servers），而 skills 走目录约定（`skills/` 下每个子目录一个 skill）。

```toml
# .ah/bundles/domain-x/bundle.toml
name = "domain-x"           # 必须与目录名一致
version = "1"               # bundle schema 版本
description = "领域 X 的能力包：文档技能 + 提交前钩子 + worker 规则 + context7 MCP"

# --- skills：默认取 skills/ 下所有子目录；可用 include 显式白名单 ---
[skills]
include = ["doc-writer", "api-linter"]   # 省略则 = skills/ 下全部

# --- hooks：沿用 ExtensionConfig 的 HookGroup 形状；command 相对 bundle 根 ---
[hooks]
PreToolUse = [
  { matcher = "Bash", hooks = [{ type = "command", command = "hooks/guard.sh", timeout = 5 }] },
]
Stop = [
  { command = "hooks/notify.sh" },        # 简写：等价 matcher="*"、type="command"
]

# --- rules：按角色路由到 rules/ 下的文件 ---
[rules]
master = "rules/master.md"   # 可选
worker = "rules/worker.md"   # 可选

# --- mcp：provider-neutral 声明，ah 翻译到各 provider ---
[[mcp.servers]]
name = "context7"
transport = "stdio"          # stdio | http | sse
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
env = { CONTEXT7_TOKEN = "${CONTEXT7_TOKEN}" }   # 只声明变量名；值来自宿主进程 env

[[mcp.servers]]
name = "acme-remote"
transport = "http"
url = "https://mcp.acme.dev/sse"
headers = { Authorization = "Bearer ${ACME_KEY}" }

# 对当前 provider 不支持的能力，默认硬错；可逐块标 optional 降级为警告+跳过。
# optional = true
```

设计取舍：
- **manifest 必需**：单一事实源、便于校验与版本化、便于 fingerprint（对 manifest + 被引用文件做内容摘要）。纯目录约定会让"哪些是 skill、hooks 绑哪个 event"变得隐式。
- **hooks 复用 `HookGroup`/`HookItem` 反序列化**（`extensions.rs:12-56`），零新解析器。`command` 相对 bundle 根解析（对齐 v1 `resolve_extension_source` 的相对/绝对逻辑）。
- **skills 走目录约定**：与 v1 `.ah/skills/<name>` 结构一致，`SKILL.md` 必需，整目录 symlink，无格式翻译。
- **rules 按角色路由**：master/worker 各一片段，缺省则该角色不叠加 bundle 规则。
- **MCP provider-neutral schema**：`name` + `transport` + (`command`/`args`/`env` | `url`/`headers`)。bundle.toml **只放需要的环境变量名，绝不放值**；值只来自宿主进程 env，在物化 provider MCP 配置文件那一刻写入每个 agent 沙箱；绝不落 ahd.sqlite，绝不进入 git 追踪文件。spawn 时变量缺失硬错报人话（如 `bundle requires XXX_API_KEY, not set in current environment`），不静默、不留空。
- **plugins 不在 bundle scope**：已定 bundle = skills+hooks+rules+MCP。git/id plugins 继续走散配 `plugins` 字段。

---

## 2. ah.toml 配置面

### 2.1 一个配置项：`bundle`

在 `MasterConfig`（`config.rs:41`）与 `AgentConfig`（`config.rs:82`）各加**一个**字段 `bundle`，与现有 `skills`/`hooks`/`plugins` 并列。为兼顾"一个配置项"与"可组合多个"，`bundle` 接受 **string 或 string 数组**（serde untagged），归一化为 `Vec<String>`：

```toml
version = "1"

[master]
bundle = "master-ops"            # 单个：master 领域包

[agents.a1]
provider = "claude"
bundle = ["domain-x", "team-conventions"]   # 多个：按序合并
skills = ["extra-local-skill"]   # 散配仍可与 bundle 共存（见 §2.2）

[agents.a2]
provider = "codex"
bundle = "domain-x"              # 同一 bundle 跨 provider 复用，ah 各自翻译
```

字段语义：`bundle` 是"引用名"，实体在 `.ah/bundles/<name>/`。per-agent 与 master 各自独立引用（沿用现有 per-role 作用域，无全局 fan-out）。

### 2.2 与现有 skills/hooks/plugins 的关系：叠加，非取代

- **默认零影响**：不配 `bundle` → 行为与今天完全一致。
- **叠加合并**：bundle 展开的 ExtensionConfig 贡献与散配字段**取并集**。多个 bundle 按数组顺序合并。
- **冲突策略（已定 Q2）**：**真冲突即硬错**（fail-fast，贴合仓库既有"能支持就通用、不支持就明确报错而非静默"的哲学）。真冲突定义：
  - 两个来源声明**同名 skill** 但指向不同源目录；
  - 同名 MCP server 但配置不同；
  - 同一 (event, matcher, command) 重复 → 幂等去重（不算冲突，参 `remove_ah_owned_hook_groups` 的幂等思路）；
  - 同名 skill/MCP 且**完全相同** → 幂等去重。
- **rules 是叠加不是冲突**：多来源 rules 片段按"kernel → 各 bundle（数组序）→ 项目 `.ah/rules/<slot>.md`"顺序拼接（§4.2）。

### 2.3 校验（`validate_project_config`，`config.rs:130`）

配置加载期新增：
- bundle 名合法性（同 agent-id 字符集）；
- `.ah/bundles/<name>/bundle.toml` 存在且可解析；`name` 与目录一致；`version` 已知；
- 引用的 skills 子目录、hooks 脚本、rules 文件存在（早失败，不拖到 home-layout 期）；
- **provider 适配预检**：若某 agent 的 provider 无法消费 bundle 中的某"必需"内容类型，默认硬错；manifest 对该块标 `optional=true` 时警告+跳过（Q8）。

---

## 3. per-provider 翻译注入矩阵

Bundle 的每类内容，在三 provider 的落点/格式/翻译，以及"不支持时的语义"。**总原则：能支持 → provider 通用；不支持 → 默认硬错，不静默。** 单个内容类型可在 manifest 标 `optional` 以把"硬错"降级为"警告+跳过"（Q8）。

### 3.1 矩阵

| 内容 | claude 落点/格式 | codex 落点/格式 | antigravity 落点/格式 | 不支持语义 |
| :--- | :--- | :--- | :--- | :--- |
| **skills** | symlink `bundle/skills/<n>` → `$CLAUDE_CONFIG_DIR/skills/<n>`（无翻译） | symlink `bundle/skills/<n>` → `$CODEX_HOME/skills/<n>` | symlink `bundle/skills/<n>` → `<沙箱 home>/.gemini/config/skills/<n>` | 三 provider 已确认支持；若未来 provider 不支持则走 Q8：默认硬错，`optional` 警告+跳过 |
| **hooks** | 脚本 symlink→`.claude/hooks/`，声明注入 `.claude/settings.json` `hooks`（event-keyed）。复用 `materialize_claude_hooks`+`inject_claude_hooks` | 写 `.codex/hooks.json` + `config.toml [features] hooks=true`。复用 codex hooks 写入（`enable_codex_hooks`/`merge_codex_hook_push`），把 bundle hooks 一并 merge | 写 `.gemini/config/hooks.json` + `enableJsonHooks=true`。复用 antigravity hooks 写入；注意其顶层是**命名对象**、非 event-keyed，需做形状翻译 | 三 provider 都支持 → 通用；仅格式翻译不同 |
| **rules** | 叠加进 `.claude/CLAUDE.md`（master+worker）。扩展 `compose_rules`（§4.2） | 叠加进 `.codex/AGENTS.md`（**仅 worker**；v1 master 恒 claude，非 claude master 角色目前不存在） | 叠加进 `.gemini/AGENTS.md`（**仅 worker**；v1 master 恒 claude，非 claude master 角色目前不存在） | codex/antigravity 的 **master rules** 当前不支持 → 若 master 引用了含 master-rules 的 bundle 且 provider 非 claude，走 Q8：默认硬错，`optional` 警告+跳过 |
| **MCP** | 填充 `.claude.json` 的 `mcpServers`（已有空占位 `home_layout.rs:1086`）；stdio→`{command,args,env}`，http/sse→`{url,headers}` | 写 `.codex/config.toml` `[mcp_servers.<n>]`（ah 已在编辑此 TOML）；TOML `command/args/env`，远程按 codex 能力检查 | 写 `.gemini/config/mcp_config.json` 的 `mcpServers`；stdio→`command`/`args`/`env`（`${VAR}` 展开），远程→`serverUrl`；不支持 legacy `url`/`httpUrl` | transport 按声明（stdio/http/sse）对每 provider 做能力检查；不支持走 Q8：默认硬错，`optional` 警告+跳过 |

### 3.2 翻译要点

- **skills**：无翻译（整目录 symlink）。唯一变量是"目标 skills 目录路径"随 provider 变：claude=`$CLAUDE_CONFIG_DIR/skills/<name>`，codex=`$CODEX_HOME/skills/<name>`，antigravity=`<沙箱 home>/.gemini/config/skills/<name>`。
- **hooks**：三 provider 语义都是"事件触发命令"，但磁盘形状不同（claude=settings.json event-keyed / codex=hooks.json / antigravity=hooks.json 命名对象）。翻译 = 把统一的 `HashMap<event, Vec<HookGroup>>` 铺进各自形状。timeout 单位差异已在 `hook_timeout_for_provider` 处理（antigravity 5000ms vs 其它 5）。
- **rules**：翻译 = 选目标文件（CLAUDE.md/AGENTS.md）+ 拼接顺序。master 角色 v1 恒 claude；非 claude master-rules 不提前开洞，按 Q8 处理。
- **MCP**：翻译 = provider-neutral schema → 各自 JSON/TOML 键。transport 按声明（stdio/http/sse）逐 provider 做能力检查；不支持走 Q8。bundle.toml 只声明所需 env var 名，值只来自宿主进程 env，并只在物化 provider MCP 配置时写入 agent 沙箱；变量缺失则 spawn 硬错报人话，绝不静默空值。

---

## 4. 物化流程

### 4.1 解析 → 合并 → 物化（三段）

```text
[解析] resolve_bundles(project_root, &[bundle_names], role)
          -> Vec<BundleContribution{ skills, hooks, rules_fragment, mcp_servers, digest_parts }>
[合并] merge into 有效 ExtensionConfig（散配字段 ∪ 各 bundle，按 §2.2 冲突策略）
          -> EffectiveExtensions{ skills, hooks, plugins, rules_layers, mcp_servers }
[物化] prepare_<provider>_overrides 内按序物化（复用 + 两个新物化器）
```

集成点：`prepare_home_layout_with_extensions_for_slot`（`home_layout.rs:132`）在分派前把 `bundle` 引用解析并合并进传入的 `ExtensionConfig`；各 `prepare_*_overrides` 基本不动，只在 rules 组合处插层、并新增 MCP 写入调用。

### 4.2 claude 物化顺序（含 bundle 后）

沿用现有顺序（`prepare_claude_overrides`），把 bundle 贡献并入既有步骤，新增 MCP 一步：

1. **rules**：`compose_rules` 扩展为 `kernel + Σ(bundle.rules[role], 数组序) + 项目 .ah/rules/<slot>.md 或默认` → `.claude/CLAUDE.md`（copy/write）
2. trust（不变）
3. **skills**：`materialize_claude_skills`（bundle skills ∪ 散配 skills）→ symlink 到 `.claude/skills/`
4. plugins（散配，不变）
5. **hooks**：`materialize_claude_hooks`（bundle hooks ∪ 散配 hooks）→ 脚本 symlink + 注入 settings
6. **MCP（新）**：把合并后的 `mcp_servers` 写进 `.claude.json` 的 `mcpServers`（填充既有空占位）
7. settings / credentials（不变）

codex/antigravity 类比：在各自 `prepare_*_overrides` 中，rules 组合插 bundle 层、hooks merge bundle hooks、新增 MCP 写入（codex→config.toml、antigravity→`.gemini/config/mcp_config.json`）；skills 按 §3.1 的三 provider 落点物化。

### 4.3 symlink vs copy（对齐现状）

| 内容 | 方式 | 依据 |
| :--- | :--- | :--- |
| bundle skills 目录 | **symlink**（`force_symlink`） | 复用 v1；bundle 文件改动即时可见，无需重拷 |
| bundle hook 脚本 | **symlink** | 复用 `materialize_hooks` |
| bundle rules 片段 | **copy/compose**（写入 CLAUDE.md/AGENTS.md） | rules 是拼接产物，非独立文件 |
| MCP 声明 | **写入/merge**（JSON/TOML 就地编辑） | 落进 provider 既有配置文件 |

### 4.4 fingerprint：必须覆盖 bundle 内容摘要

**这是本设计的正确性关键。** bundle 内容会独立于 `ah.toml` 变化（改 `SKILL.md`、改 hook 脚本、改 rules、改 MCP 声明），若只把"bundle 名"进 fingerprint，会"内容漂移但不 realign"。

方案：为每次解析出的有效 bundle 计算 **`BundleDigest`** —— 对 `bundle.toml` + 所有被引用文件（skills 目录树、hook 脚本、rules 片段）的**内容**做稳定摘要（复用 `deterministic_json` + SHA256 思路；文件内容按路径排序后逐个 hash）。然后：

- `ConfigFingerprintInput` 增加 `bundle: Option<BundleDigest>`（或 `bundles: &[BundleDigest]`，排序后折入）；
- 线穿**全部四个** `compute_config_hash` 调用点（`sessions.rs:353`、`agent.rs:269`、`sessions.rs:837`、`realign.rs:87/101/160`）；
- 存入 `AgentSpawnSpec`（`db/recovery.rs`）随快照持久化；
- `drift_reason`（`realign.rs:514`）增加 `"bundle changed"` 分类。

副产品：bundle 内的 skills/rules 借"整包内容摘要"天然进入 fingerprint，绕过了 v1 把 skills/rules 移出 fingerprint 的历史决定（那是因为散配 skills/rules 当时不进 hash；bundle 作为一个内容单元统一 hash，语义更干净）。MVP 摘要粒度已定为**整包一个 `BundleDigest`**；改任一 bundle 文件会触发引用该 bundle 的角色 realign。

### 4.5 recovery / realign 传播

现有恢复链已从 `AgentSpawnSpec` 重建配置（`spawn_realign_agent` `realign.rs:317`、crash 恢复 `orchestrator/mod.rs:486`、master 复活重供 `master_watch.rs:1934`）。只要：

- `AgentSpawnSpec` 携带 bundle 引用名 **和** `BundleDigest`；
- 重供路径把 bundle 引用重新解析物化（而非缓存展开结果）——保证复活时读的是**当前**磁盘 bundle 内容；digest 用于 drift 判定。

则 recovery/realign **无需新增专门逻辑**，只需字段线穿。`ah up` 触发的 realign：bundle 内容变 → digest 变 → agent kill+respawn / master 审计（同现有 drift 语义）。

---

## 5. 零回归 & 迁移

### 5.1 零回归

- **未配 `bundle`**：`bundle` 字段 `#[serde(default)]` 为空 → `resolve_bundles` 返回空 → 有效 ExtensionConfig 与今天逐字节一致 → 所有现有物化路径、fingerprint、快照、realign 不变。
- **fingerprint 稳定性**：`ConfigFingerprintInput.bundle` 为 `None`/空 时，序列化必须与"无该字段"产生**相同 hash**，否则会一次性把所有现存 session 判成 drift。需在 `compute_config_hash` 里对空 bundle 做"省略键"处理（`skip_serializing_if`）并加回归测试锁死 hash。
- **provider 未知/未适配**：延续 `home_layout.rs:177-180` 的既有语义，不改。

### 5.2 与散配 skills/hooks 共存

- bundle 与散配字段**可同时存在**，按 §2.2 合并（并集 + 真冲突硬错 / 幂等去重）。
- 迁移是**可选**的：用户可把现有 `.ah/skills/*` 与散配 hooks 手动收进一个 bundle，或原样保留。ah 不强制转换。
- 可提供 `ah bundle validate <name>` / `ah bundle list` 辅助（PR-5），以及可选的 `ah bundle init` 从现有散配生成骨架（manifest 必需之外的 nice-to-have）。

---

## 6. PR 切法建议（仿 macos-port，可审的小 PR）

**依赖**：PR-1 建在 skills v1 之上；v2 复用 v1 的 skills 解析、防逃逸与 provider 目录物化能力。

### PR-1：Bundle 脊柱（含 fingerprint）
- `bundle` 配置字段（string|list）+ 校验；`bundle.toml` schema + 解析器 + 防逃逸（复用 v1 skills 校验）；`BundleContribution` + 合并到 `ExtensionConfig`（含冲突策略）。
- `ExtensionConfig` 增 `rules`/`mcp` 承载位（承载但 PR-1 只物化 rules，不物化 MCP）。
- provider 物化：bundle 的 skills（复用 v1 provider 落点）、hooks（复用）、rules 叠加（扩展 `compose_rules`）。
- **`BundleDigest` 全链路**：`ConfigFingerprintInput` + 四个 hash 调用点 + `AgentSpawnSpec` + `drift_reason`。
- MCP：PR-1 承载 schema 与 digest，实际 writer 可在 PR-2 落地；未物化的 MCP 必需项按 Q8 默认硬错，`optional` 可警告+跳过。
- 门槛：`cargo test` 全绿；零回归 hash 锁死测试；不配 bundle 行为不变。

### PR-2：MCP 翻译（跨 provider，greenfield）
- provider-neutral MCP schema → 三 writer：claude `.claude.json mcpServers`、codex `config.toml [mcp_servers]`、antigravity `.gemini/config/mcp_config.json mcpServers`。
- `${VAR}` env 解析；bundle.toml 只放变量名，值只来自宿主进程 env；物化时写入 agent 沙箱 provider MCP 配置；密钥不落 ahd.sqlite、不入 git；变量缺失硬错报人话；不支持的传输按 Q8 默认硬错/optional 警告。
- 门槛：三 provider MCP 写入的纯函数渲染测试；MCP 进 `BundleDigest`。

### PR-3：codex bundle 全适配
- codex hooks 从 bundle merge（复用 codex hooks 写入）；worker rules 叠加进 `.codex/AGENTS.md`；skills 物化到 `$CODEX_HOME/skills/<name>`。
- 门槛：codex 物化 e2e / 渲染测试。

### PR-4：antigravity bundle 全适配
- antigravity hooks（形状翻译）；worker rules 进 `.gemini/AGENTS.md`；skills 物化到 `.gemini/config/skills/<name>`；MCP 写 `.gemini/config/mcp_config.json`。
- 门槛：antigravity 物化测试。

### PR-5：recovery/realign 加固 + CLI + 文档 + 迁移
- master 复活重供、crash 恢复、`ah up` realign 在 bundle 下的端到端测试（含"bundle 内容变→realign"）。
- `ah bundle validate/list`；文档；迁移指南（散配↔bundle 共存）。

---

## 7. 已定决策（Q1-Q11）

1. **Q1 `bundle` 字段多重性：已定**。`bundle` 接受 string 或 string list，一个配置项同时覆盖单包与多包组合，解析后归一化为 `Vec<String>`。
2. **Q2 冲突策略：已定**。真冲突硬错；完全相同的 skill/MCP 与同一 `(event, matcher, command)` hook 幂等去重。
3. **Q3 skills provider 支持：已闭合**。skills 是三 provider 通用能力：claude→`$CLAUDE_CONFIG_DIR/skills/<name>`，codex→`$CODEX_HOME/skills/<name>`，antigravity→`<沙箱 home>/.gemini/config/skills/<name>`（symlink 支持）。
4. **Q4 MCP 跨 provider 与密钥：已定**。transport 按声明（stdio/http/sse）对每 provider 做能力检查；不支持走 Q8。bundle.toml 只放需要的环境变量名、绝不放值；值只来自宿主进程 env，在物化那刻写进每个 agent 沙箱的 provider MCP 配置文件；绝不落 ahd.sqlite（只记变量名/占位）、绝不进 git 追踪文件；spawn 时变量缺失硬错报人话（如 `bundle requires XXX_API_KEY, not set in current environment`），不静默、不留空。
5. **Q5 antigravity MCP 落点：已确认**。文件是 `<沙箱 home>/.gemini/config/mcp_config.json`，键为 `mcpServers`；stdio 用 `command`/`args`/`env`（`${VAR}` 展开），远程用 `serverUrl`，不支持 legacy `url`/`httpUrl`。
6. **Q6 codex/antigravity master rules：已定**。复用 Q8：bundle 声明 master-rules 但当前 provider 不支持（v1 master 恒 claude，非 claude master 角色目前不存在）时，默认硬错，manifest 可标 optional；不为未来 master provider 提前开洞。
7. **Q7 plugins 是否纳入 bundle：已定**。plugins 不入 bundle；bundle scope 固定为 skills + hooks + rules + MCP，plugins 继续走散配字段。
8. **Q8 不支持能力降级语义：已定**。默认硬错；manifest 可对具体块标 optional，将硬错降级为警告+跳过。
9. **Q9 manifest 强制性：已定**。`bundle.toml` 必需，是 bundle 的结构化单一事实源。
10. **Q10 fingerprint 摘要粒度：已定**。MVP 使用整包一个 `BundleDigest`；改任一 bundle 文件会触发引用该 bundle 的角色 realign。
11. **Q11 PR 依赖/顺序：已定**。PR-1 建在 skills v1 之上，复用 skills v1 的解析、防逃逸和 provider 目录物化能力。

---

## 8. 完成边界

设计到此为止。实现在 PM 批准前不启动。实现必须守住：

- 不配 bundle → 零行为变化、fingerprint hash 不变（锁死测试）。
- bundle 内容进 `BundleDigest` → 内容漂移能触发 realign（不出现"漂移不重建"）。
- 沿用"能支持即通用、不支持即明确报错（非静默）"语义。
- recovery/realign/master-revive 通过 `AgentSpawnSpec` 字段线穿即可复用，不新造恢复逻辑。
- Q1-Q11 已定；实现按本文定案推进，后续只在实现发现新事实与本文冲突时回报 PM。
