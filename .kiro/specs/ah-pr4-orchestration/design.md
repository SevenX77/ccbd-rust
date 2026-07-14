# PR4 编排设计：独立产品形态、三层隔离与环境物化

| 状态 | 草案 (Round 8 - 配置来源分层) |
| :--- | :--- |
| **日期** | 2026-05-27 |
| **范围** | L3 编排层：主控与 Worker 的全量环境物化 |

## 1. 核心设计：独立产品入口与三层隔离

`ah` 是一个完全独立的 Agent 协作产品。它**不是**对宿主机原有 `claude` 命令的劫持，而是通过专门的入口（`ah start` / `ah up`）拉起一套独立的“Agent 蜂群”工作环境。

### 1.1 独立的产品入口
- **宿主机日常使用**：用户在终端正常打 `claude` 命令，其行为完全不变，继续使用宿主机原有的 `~/.claude` 配置。`ah` 对此**绝不触碰、绝不污染**。
- **ah 产品环境**：只有当用户运行 `ah start` 或 `ah up` 时，系统才会拉起一套受控的、隔离的协作环境。

### 1.2 三层隔离模型 (Three-Layer Isolation)
在此产品形态下，系统分为三个互不干涉的层级：

1.  **宿主机层 (Host)**：用户原本的开发环境。`ah` 保证其底层设置（如 `~/.claude`）的纯净。
2.  **主控层 (Master)**：**ah 内部的编排主控角色**。当 `ah` 启动时，它会拉起一个专门负责决策和下令的 Master 进程。这个 Master 运行在独立的沙箱中，拥有一整套为其定制的私有配置（Rules / Skills / Hooks / Plugins）。
3.  **执行层 (Worker Agents)**：由 `ah` 管理的各个执行者（a1, a2...）。每个 Worker 拥有各自独立的隔离沙箱与专属配置。

---

## 2. M0 阶段：主控隔离的函数级实现

“主控隔离”是 PR4 确保“蜂群”确定性的核心。它是 `ah` 内部角色的环境准备，而非对用户工具的修改。

### 2.1 `spawn_master_pane` 的物化流程
`src/rpc/handlers.rs` 中的 `handle_session_spawn_master_pane` 在拉起 **ah Master** 时执行：
1.  **沙箱路径解析**：调用 `path::resolve_sandbox_dir` 为该 Master 分配专用路径 `.../sandboxes/<sess_id>/master`。
2.  **触发物化 (Barrier)**：调用 `prepare_home_layout("claude", ...)`。如果配置不齐（如所需的 Skill 缺失），函数返回错误，`ah` 拒绝启动环境。
3.  **环境注入**：将物化产出的 `HOME` 和 `CLAUDE_CONFIG_DIR` 注入 `systemd::master_command`，确保该 Master 只能看到沙箱内的配置。
4.  **凭证共享**：主控默认采用“只读共享宿主机 OAuth 凭证”模式（只读链接 `.credentials.json`），确保能登录但不会修改宿主机的登录态。

---

## 3. M1 阶段：Hooks 与 Plugins 的“双侧物化”

为了让 `ah` 环境内的 Agent 具备特定能力，必须在沙箱配置中完成激活。

### 3.1 激活机制
物化层在生成沙箱 `settings.json` 时执行：
1.  **文件侧 (Payload)**：将脚本/插件链接至沙箱内 `.claude/hooks/` 和 `.claude/plugins/`。
2.  **配置侧 (Registration)**：
    - **Hooks**：在 `settings.json` 的 `hooks` 对象下注册事件（如 `PreToolUse`）。
    - **Plugins**：在 `enabledPlugins` 数组中激活插件 ID。

### 3.2 全 Provider 物化映射表
| Provider | 规则文件 (Rules) | 扩展资产 (Skills/Plugins) | 激活配置 (Settings) |
| :--- | :--- | :--- | :--- |
| **Claude** | `.claude/CLAUDE.md` | `.claude/skills/`, `.claude/plugins/` | `.claude/settings.json` |
| **Gemini** | `.gemini/GEMINI.md` | `.gemini/skills/` | `.gemini/settings.json` |
| **Codex** | `.codex/AGENTS.md` | `.codex/skills/` | `.codex/config.toml` |

---

## 4. 配置来源分层：产品内建层 vs 项目附加层 [NEW]

为了保证 `ah` 环境的“开箱即用”与协议一致性，配置来源被划分为两个核心层级。

### 4.1 ah 自带的「运转必需层」（产品代码级）
这层配置被视为 `ah` 产品代码的一部分，硬编码或打包在安装包内。

- **包含内容**：
  - **基础宪法**：编排规则文件（如 `CLAUDE.md` / `GEMINI.md`）。定义了 Master 的编排逻辑与 Worker 的红线边界。
  - **通信原语**：内建技能（`ask`, `ping`, `pend`）。这些是 Agent 之间打招呼、派任务、等结果的“标准普通话”。
- **物化机制**：
  - **二进制嵌入**：使用 Rust `include_str!` 宏将规则文本直接嵌入 `ah` 二进制文件。
  - **强制铺底**：在物化阶段，无论 `ah.toml` 如何配置，这层内容总是最先写入沙箱，作为“System Layer”。
  - **不可篡改**：Worker Agent 无法通过 `ah.toml` 或沙箱内修改来覆盖这层核心逻辑。

### 4.2 项目/用户附加层（Provisioning 层）
用户在 `ah.toml` 中声明的、针对特定项目的扩展。

- **来源约束**：
  - **项目本地**：位于项目根目录的 `skills/` 或 `plugins/`。
  - **Git 地址**：直接在 `ah.toml` 中写的 `git@github.com:...` 链接。
- **获取机制**：
  - **Auto-Provisioning**：若声明的资产在本地缓存 (`~/.ah/cache/`) 中缺失，则自动拉取。
  - **凭证复用**：拉取时强制复用宿主机的 Git SSH/HTTPS 凭证，支持私有库。
- **定位**：不再提供中心化的 Registry 索引，保证系统结构的扁平与透明。

---

## 5. M2 阶段：Auto-Provisioning（配置自动补齐）

为了解决“声明了但本地没有”导致的启动失败，`ah` 引入自动补齐机制。它像 `npm install` 一样，在环境物化前自动拉取所需的资产。

### 5.1 资产来源 (Sources)
`ah` 通过以下渠道自动寻找 Skill 和 Plugin：
1.  **项目本地**：优先查找项目根目录下的 `skills/` 或 `plugins/`。
2.  **直接指向**：支持在 `ah.toml` 中直接写 Git 地址。例如：`skills = ["rust-expert@git@github.com:my-org/rust-skills.git"]`。

### 5.2 Provisioning 工作流
Provisioning 流程紧嵌在“物化屏障”之前：
1.  **扫描声明**：解析 `ah.toml` 识别所有需要的 Skill/Plugin ID。
2.  **检查缓存**：查看全局缓存目录 `~/.ah/cache/` 是否已有对应资产。
3.  **自动补齐 (Provision)**：对于缺失资产，`ccbd` 会自动执行 `git clone`（复用宿主 Git 凭证）。
4.  **物化就绪**：资产补齐后，继续执行 Symlink 挂载与配置注册。
5.  **强校验 (Final Barrier)**：**只有当自动补齐也失败**（如：地址无效、网络断开）时，才会报错并拒绝启动。

---

## 6. `ah up`：基于指纹的对齐审计

`ah up` 负责检查当前运行中的 `ah` 环境是否与最新的 `ah.toml` 定义一致。

- **指纹计算 (Fingerprinting)**：`config_hash = SHA256(provider + rules + skills + hooks + env)`。
- **审计看板**：`ah up` 对比实时指纹与数据库指纹，发现不符（DRIFT）时提示用户。
- **强制对齐**：运行 `ah up` 会重启对应角色的进程，重新触发全套物化流水线（含 Provisioning）。

---

## 7. 继承字段表 (Inherited Fields Audit)

| 类别 | 字段 / 接口 | 现状 (PR1-3) | PR4 变更 [NEW/BREAKING] |
| :--- | :--- | :--- | :--- |
| **ah.toml** | `[master]` | 仅 `cmd`, `enabled` | `[NEW]` 增加 `rules`, `skills`, `hooks`, `plugins`。 |
| **ah.toml** | `[agents.<id>]` | `provider`, `env` | `[NEW]` 增加 `skills`, `rules`, `hooks`。 |
| **RPC** | `session.spawn_master_pane` | 无隔离物化 | `[BREAKING]` 强制执行隔离物化与 Provisioning 屏障。 |
| **RPC** | `agent.spawn` | 仅基础环境注入 | `[NEW]` 增加 `rules_layers`, `skills` 参数。 |

---

## 8. 用户视角：为什么要这么设计？

- **独立且纯净**：你可以放心地在终端用你的 `claude` 干私活，然后在另一个窗口用 `ah start` 拉起一个专门为本项目定制的、带全套规则和技能的“专家集群”，两者互不干扰。
- **开箱即用 (Package Manager)**：你不需要手动去搜集 Skill 文件。只要在 `ah.toml` 里写下 ID 或 Git 地址，`ah` 就像 `npm` 自动装包一样帮你把所有专家技能拉齐、装好。
- **系统内生逻辑**：`ah` 不仅仅是一个搬运工，它自带了支撑多 Agent 协作所需的“基础宪法”和“通信原语”。即使你什么都不配，它也能保证基本的编排框架正常运转。
- **安全隔离**：`ah` 拉起的任何角色（无论是 Master 还是 Worker）都被锁在沙箱里，它们看到的规则、能用的插件都是根据你的 `ah.toml` 临时生成的，绝对不会改动你的宿主机设置。
