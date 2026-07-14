# ah-config & ah-runtime-state 自我知识内建 Skill 设计 (a3)

本项目设计旨在为 `ah` 系统提供关于配置面（`ah-config`）与运行时状态面（`ah-runtime-state`）两个「自我知识」内建 Skill 的架构设计与触发策略。

两只 Skill 均声明为 **MasterOnly**，排他性提供给 Master 级编排 Agent（Worker Agent 不予装载以防 Context 污染）。主受众是运行在开发者本机的外部集成 Agent（如 Claude Desktop, Cursor/Codex, Studio-agent），次受众是 `ah` 托管沙箱内的 Master。

---

## 一、 结构决策：分两个还是合一个？

对于将这两个自我知识内建 Skill 表现为「拆分独立」还是「合并为 `ah-internals`」，在渐进式披露（Progressive Disclosure）视角下的权衡如下：

### 1. 方案对比与权衡

| 维度 | 分为两只独立 Skill (`ah-config` & `ah-runtime-state`) | 合为一只 Skill (`ah-internals`) |
| :--- | :--- | :--- |
| **触发意图精准度** | **高**。配置变更与运行时状态查询属于性质完全不同的行为，分开后能独立触发，互不干扰。 | **低**。只要提到配置或状态之一，就会把另一大块无关的知识强行载入 Context。 |
| **Context 资源占用** | **按需加载**。通常状态查询行为（读 snapshot/events）非常频繁，而修改配置（改写 `ah.toml`）非常低频。分开可避免频繁的状态查询行为携带庞大的配置长文本。 | **开销大**。每次状态轮询或故障诊断，都必须塞入完整的配置模型与文件布局说明，导致上下文无意义膨胀。 |
| **开发与维护复杂度** | **中等**。需要在 `assets/builtin/skills/` 下维护两个子文件夹，注册表包含两项。 | **低**。只需维护一个 Skill 描述与文件。 |

### 2. 明确推荐：分两个独立 Skill
**推荐采用「拆分方案」**。
主要基于**渐进式披露（Progressive Disclosure）的最少必要知识原则**。`ah-config` 的信息大多为静态布局、字段 Schema 与组合模型；而 `ah-runtime-state` 则是关于 `RuntimeSnapshot` 的 JSON 格式与 `ah events` 的流形状。两者的意图在 Agent 的心智模型中属于完全不同的决策树分支。强行合并不仅会大幅稀释 Agent 的注意力，还会因频繁的状态查询导致上下文的 Token 持续被静态配置文档占用。

---

## 二、 触发词策略 (Trigger Word Strategy)

为了实现低噪、高灵敏度的启发式触发，以下针对两只 Skill 分别设计 `description`，均使用与 `ah-commands` 一致的 `Use when...` 格式。

### 1. `ah-config` Skill 触发设计
```yaml
description: Use when configuring the 'ah' environment, modifying 'ah.toml' fields, defining new custom worker agents or custom rules under '.ah/rules/', registering project-specific skills, or resolving how configuration settings compile into provider-specific documentation files (like CLAUDE.md or AGENTS.md). Do NOT activate this for general application configuration or web/framework settings unrelated to the 'ah' daemon setup.
```
* **覆盖真实意图**：覆盖了“配置怎么写”、“规则路径放哪”、“toml 格式”以及“最终落盘机制”。
* **防误触发论证**：由于 `config` 和 `toml` 是高度通用的词汇，描述中强绑定了核心专有名词：`'ah' environment`、`'ah.toml'`、`'.ah/rules/'` 以及 `CLAUDE.md or AGENTS.md`。同时，显式增加了否定指示（Do NOT...），从而在 Agent 修改非 `ah` 配置文件（如 `Cargo.toml` 或 `package.json`）时实现物理防爆，防止误触发。

### 2. `ah-runtime-state` Skill 触发设计
```yaml
description: Use when inspecting or querying the active 'ah' daemon runtime state, parsing the 'RuntimeSnapshot' JSON schema, understanding agent lifecycle status, streaming run-time events via 'ah events', or checking if the ahd daemon and its tmux panes are active. Do NOT activate this for general database status, system health checks, or application-level state queries.
```
* **覆盖真实意图**：覆盖了“读运行态”、“RuntimeSnapshot 的结构”、“生命周期状态”、“通过 events 获取权威状态”。
* **防误触发论证**：通过显式锁定专有结构名词 `'RuntimeSnapshot'`、`'ah events'` 与 `'ahd'`，与一般的应用状态查询（如 `systemctl status` 或一般的 API status）划清界限。末尾的否定说明进一步确保 Agent 在开发普通 Web 或数据库应用程序时不会由于“status / state”泛词导致此 Skill 被无故唤醒。

---

## 三、 外部交付机制 (External Delivery Mechanism)

### 1. 核心矛盾
现有 `materialize_builtin_skills` 仅将内建技能下发到 `ah` 自身托管并拉起的沙箱中。然而，本轮自我知识 Skill 的主受众是**外部集成 Agent**（例如开发者本地运行的 Claude Desktop, Cursor, 或 Studio），它们没有运行在 `ah` 的沙箱中，导致其无法自动加载该 Skill 资产。

### 2. 方案选项与权衡比较

我们假设 `ah` 目前没有任何「向沙箱外部全局目录写 Skill」的非安全写穿机制（此假设依赖 Codex 的核实结果；若已存在，则应复用其安全通路）。

| 方案 | (a) 新增 CLI 导出命令<br/>（如 `ah skills install/export`） | (b) 随包与文档分发<br/>（手动导引） | (c) 推荐方案：**项目级本地编译同步 (Workspace-level Project Sync)** |
| :--- | :--- | :--- | :--- |
| **工作原理** | 通过 CLI 子命令将内建 Skill 写死复制到外部 Agent 的全局配置目录（如 `~/.claude/skills`）。 | 在 `README.md` 中指引开发者手动下载并导入 Skill 文件到 IDE 或全局目录。 | 在项目初始化（`ah init` 或每次运行）时，自动或通过命令将内建 Skill 实体化导出到项目目录下的 `.ah/skills/` 中。 |
| **目标安装路径** | 外部 Agent 的全局系统级配置目录（如 `~/.claude/` ）。 | 开发者手工指定的任意路径。 | 宿主机当前项目的工作空间目录 `.ah/skills/` 内。 |
| **更新与版本漂移** | 差。如果升级了 `ah` 二进制，需用户手动再次运行 `install` 刷新全局目录，容易产生版本不同步。 | 极差。极易遗忘更新，导致文档陈旧引发幻觉。 | **极佳**。因为 Skill 存储在二进制内部，每次 `ah` 命令行运行或构建时均可自动同步覆盖项目本地，确保与当前二进制版本绝对锁死。 |
| **侵入度** | **高**。需硬编码识别各家外部 IDE/扩展的私有路径，且面临宿主机写权限与系统安全拦截风险。 | **低**。纯文档性说明。 | **低**。只在当前项目工作空间内读写，无需超出项目沙箱，完全安全。 |

### 3. 推荐交付方案
**推荐采用 方案 (c)「项目级本地编译同步」方案**。
具体设计如下：
1. **统一编译导出**：当用户在项目目录运行 `ah` 相关命令（或显式执行新命令 `ah skills sync`）时，CLI 从二进制的 `include_str` 中提取最匹配当前版本的 `ah-config` 和 `ah-runtime-state` 两个 `SKILL.md`，直接将其释放至 `.ah/skills/ah-config/` 和 `.ah/skills/ah-runtime-state/` 中。
2. **外部 Agent 自然发现机制**：外部 Agent（如 Claude, Cursor）通常会扫描并吞入工作空间中的 `.ah/` 结构。更重要的是，`ah` 现有的组合模型编译器在将配置编译成 `.claude/CLAUDE.md`、`.codex/AGENTS.md` 时，会**自动将 `.ah/skills/` 下的内建 Skill 编译拼接到对应的 Provider 说明文件中**。
3. **优势**：该方案实现了“零外部目录侵入 + 二进制版本强锁定 + 外部 Agent 自动无感加载”，彻底解决了版本漂移与手动配置的痛点。

---

## 四、 关联标注 (Not In-Scope Association)

> [!IMPORTANT]
> 经与项目规划确认，本轮设计**仅文档化现有的状态面**（如 `RuntimeSnapshot` 的 JSON 定义与 `ah events` 流形状）。
> 另一条规划中的产品线「新增 `ah status --json` 命令及其生命周期有限状态机（FSM）」为**未立项轨道**，其具体实现与命令设计不包含在本方案的设计范围内，未来立项时本 Skill 应同步更新扩展。
