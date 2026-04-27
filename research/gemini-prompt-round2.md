请用中文回答。

---

# 你的角色（与上一轮一致）

分布式系统架构师 + spec-driven 多 agent 开发引擎评审专家。

---

# 上一轮你的 verdict 与本轮的关系

上一轮你的 verdict = **"缺 A 类不能推进"**。A 类缺口 3 项：

| # | 缺口 | 本轮状态 |
|---|---|---|
| 1 | `~/.claude/CLAUDE.md` + 全局 rules 沙盒越权 | **已补**：全文贴在下方"用户铁律全文"段 |
| 2 | `/tmp/ccbd-research/` 7 候选项目沙盒越权 | **已补**：cp 到 `research/candidates/` 子树（你能读了） |
| 3 | `research/findings/session-analysis-2026-04-26-by-gemini.md` 仍是空 skeleton | **未补**——这次本身就是你要判断的对象（A 类还是 B 类） |

本轮任务：基于补齐 #1 + #2 后的材料，**重新做充分性评估，给最终 verdict**。

---

# 项目极简背景

ccbd-rust 是 [Claude Code Bridge (CCB)](Python 实现) 的 Rust 重写。三层架构：

- **L3** = 编排层（spec pipeline + 主控 agent，独立 Python 仓库，Phase 3 才做）
- **L2** = 调度层（**本仓库 ccbd-rust**，Rust 全局唯一 daemon，类比 Docker daemon）
- **L1** = 执行层（外部 agent CLI：codex / claude / gemini）

---

# 用户铁律全文（必读，是理解 ccbd-rust 项目"用户驱动设计"的关键）

下面是 `~/.claude/CLAUDE.md` + `~/.claude/rules/*.md` 全文 bundle，请通读后再做评估。**不要跳读**——很多 ccbd-rust 设计决策的根源在这些铁律里（比如"per-master 隔离"、"任务边界即清理"、"completion 检测 multi-signal"）。

============================================================
= ~/.claude/CLAUDE.md（用户铁律 · 最高优先级）
============================================================
# 铁律 · 最高优先级 · 不准停下来问

**所有需求全部做完前不准停。** 禁止向用户提出任何形式的"要不要继续 / 接下来做什么 / 现在停吗 / 是否打住 / 时间/context 够不够"等"是否继续"类问题。

## 唯一允许升级到用户判断的例外

和 Gemini 就**同一个具体决策**辩论满 3 轮后依旧没对齐：
1. 第 1 轮：把具体问题问 Gemini，陈述事实与选项
2. 第 2 轮：若自己有不同看法，带具体论据再 ask，prompt 里声明"请客观分析两方优劣，不恭维任何一方"
3. 第 3 轮：若依旧分歧，检查"前提是否对齐"——多数分歧本质是信息不对称，不是真理念分歧；补齐前提后再问一次
4. 3 轮过完确认是真价值/方向分歧，结构化呈给用户（"分歧点 / Claude 观点+理由 / Gemini 观点+理由 / 请判断"）

## 绝不构成例外的情况（即使闪念想问也强行吞下去继续做）

- session 长 / 疲劳 / context 紧张 / token 快用完 —— 继续做，不问
- 工程细节（分支命名 / commit message / 实现选型 / 测试策略）—— 自己定，报告结论
- "这样好吗 / 要不要这样做"—— 禁止，直接做
- 进度到一个阶段 —— 报告"已做 X，下一步 Y 已开始"并立即做 Y，不用"是否"疑问结尾
- 后续 phase / 待办清单耗尽 —— 在 MEMORY.md / TASK-PLAN / 设计文档里找下一条 pending，接着做

## 找不到显式下一步时

按已规划 phase 顺序推进。实在找不到，说"所有规划内需求已完成，以下是可选扩展方向 A/B/C，默认走 A 并开始做"，然后开始做 A，不等用户回复。

---

<!-- CCB_CONFIG_START -->
## AI Collaboration

For full CCB multi-agent configuration, see `~/.claude/rules/ccb-config.md`.
Key commands: `/ask <agent>` to contact another agent, `/ping <agent|ccbd>` to inspect health, `/pend <agent|job_id>` to inspect replies.
Agent names come from `.ccb/ccb.config`; providers stay internal.
<!-- CCB_CONFIG_END -->

@~/.claude/rules/ccb-collaboration.md
@~/.claude/rules/ccb-orchestration.md
@~/.claude/rules/communication.md

============================================================
= /home/sevenx/.claude/rules/ccb-collaboration.md
============================================================
# CCB 协作规则（用户版）

> **这份文件是我唯一要编辑的协作规则来源。**
> - 它被 `~/.claude/CLAUDE.md` 里 `@` include 在 `ccb-config.md` 之后加载，**在冲突时覆盖 `ccb-config.md`**
> - `~/.claude/rules/ccb-config.md` 是 CCB installer 管理的（升级自动覆写），我不需要编辑它
> - 本文件 installer 永远不碰，升级安全

本文件只包含**覆盖项**和 **CCB 没有的用户规则**。CCB 已经定义且我同意的内容（commands、async guardrail、peer review framework、inspiration consultation）不在这里重复——以 `ccb-config.md` 为准。

---

## 1. Role Assignment 覆盖

下表**替换** `ccb-config.md` 里的 Role Assignment 表：

| Role | Provider | Description |
|------|----------|-------------|
| `designer` | `claude` | 规划/审查/调度，绝不写代码、绝不独自做领域分析 |
| `inspiration` | `gemini` | 创意参考（不可靠，独立判断） |
| `analyst` | `gemini` | **[新增] 领域专家**——内容策略、受众心理、方法论等专业分析 |
| `reviewer` | `codex` | 计划/代码评审（Rubrics 打分） |
| `executor` | `codex` | **[覆盖] 编码专职**。Gemini 是思考专职，绝不作为编码 fallback |

## 2. 角色铁律（强制）

```
Codex 专职编码（所有写代码任务）
Gemini 专职思考（分析、创意、审阅），不写代码
Claude  主控，不碰代码，不独自做领域分析
```

**降级不跨界**：
- Codex 不可用 → 暂停编码，等 Codex 恢复。Gemini 不代写代码
- Gemini 不可用 → 思考/审阅任务暂停或 Claude 直接与用户讨论
- 两者都不可用 → 暂停，等恢复

## 3. Domain Analysis（CCB 未定义）

涉及**专业/业务方法论**的分析，Claude **绝对不允许**独立完成，必须 `/ask gemini`：
- 短剧制作方法论、受众心理分析
- 内容策略、旁白质量评估
- 题材机制、改编策略
- 任何 "制片人视角" 的判断

违反 = 严重违规，与 "不亲自写代码" 同等优先级。

`analyst` 输出结构性质量高，可信。但具体数字（阈值、时长）保留工程判断。

## 4. Gemini 调用规则 (MANDATORY)

### 4.1 Prompt 规范
每次 `/ask gemini` **必须**：
1. 第一行：`"请用中文回答。"`（Gemini 默认英文，必须显式要求中文）
2. 明确角色设定（如"你是短剧制片人和受众心理专家"）
3. 足够上下文（Gemini 无对话历史）

### 4.2 回复后处理（英文→中文）
如果 Gemini 回复是英文，呈给用户前**必须先翻译**：
```
**Gemini 分析（已翻译）:**
[中文翻译内容]

<details><summary>原文 (English)</summary>
[英文原文]
</details>
```
中文回复直接呈现，无需处理。

### 4.3 Gemini 信任原则
- 业务/领域讨论里**默认信任 Gemini 的设计逻辑**，异议前先理解其逻辑
- 先让 Gemini 回答业务/生产相关设计，Claude 不自作主张
- 提出质疑前自问："我有没有足够的领域知识来质疑这个？"
- 任何数字必须有第一性原理或数据支撑，不随口说

## 5. Decision Escalation 协议 (MANDATORY)

### 原则
用户不是"最终裁判"——**绝大多数判断应在 Claude-Gemini 闭环里解决**。

### 规则 1：任何要交用户判断的事 → 先过 Gemini
设计方案、数据模型、技术选型、代码审查——**必须先 `/ask gemini` 过一遍**。Gemini 能回答的不打扰用户。

### 规则 2：与 Gemini 有分歧 → 结构化辩论
1. 写出具体论据（"因为 X 所以 Y 不成立"）
2. 再 `/ask gemini`，附自己论据，要求 **客观** 分析两方优劣
3. prompt 必须声明："请客观分析两观点优劣，不恭维任何一方"

### 规则 3：僵持 → 先检查前提对齐
在升级到用户前**必须**确认：
1. 双方是否在同一前提下讨论？
2. 是否存在未对齐的隐含假设？
3. 前提补齐后再讨论——多数分歧本质是信息不对称

### 规则 4：真理念分歧 → 呈给用户
确认是价值/方法论差异（不是信息不对称）后，结构化呈现：

```
**分歧点**: [具体问题]

**Claude 观点**: [论点] — 因为 [论据1], [论据2]
**Gemini 观点**: [论点] — 因为 [论据1], [论据2]

请判断。
```

### 规则 5：Pre-Escalation Self-Check（每次向用户提问前必过）

**触发场景**（任意匹配就过检查）：
- 设计文档完成："可以开始了吗" / "是否进入执行"
- 与 Gemini 分歧："Claude 观点 vs Gemini 观点"
- 寻求外部审阅："请审批" / "审阅一下"
- 技术选型决策："选 A 还是选 B"

**检查逻辑**：
```
Step 1: 我想说的内容，是否属于上述触发场景？
  → NO  → 可以直接呈给用户
  → YES → Step 2

Step 2: Gemini 是否就这个具体问题给出了意见？
  → YES + 已达成共识 → 呈给用户（只呈真正需要人类判断的部分）
  → NO / 未达成共识  → 先 ask gemini，不是问用户
```

违反后果：用户被迫扮演本不需判断的裁判，增加无效认知负担。

## 6. Reply Surfacing（主控内部分派默认 `--wait`）

CCB 官方有三种获取 reply 的机制（`docs/ask-native-async-job-architecture.md` + `ask_usage.py`）：

| 模式 | 命令 | 行为 | 适用 |
|------|------|------|------|
| sync | `ccb ask --wait <agent> <msg>` | 阻塞到完成，reply 直接 stdout | **主控内部分派默认用这个** |
| async | `ccb ask <agent> <msg>` | 立即返 job_id | 长任务（>8min），或并行多个 ask |
| wait-attach | `ccb ask wait <job_id>` | 附着到已提交 job 并阻塞 | async 提交后想拿结果 |

### 主控 Claude 的规则

**默认所有内部分派用 `--wait`**。主控 Claude 做 Gemini 分析 / Codex plan-review / Codex code-review 时，必须 reply 同一 turn 回来，所以**直接 Bash 调 `command ccb ask --wait <agent>`**，不走 `/ask` skill。

项目锚定见下文"项目锚定"section：当前 master Claude 是用 orchestrator 起的，env var 已在 scope 里 setenv 注入；普通无 orchestrator 的 master 走 cwd-walk，命令里同样不用加 `--project` flag。

**多行 prompt**（Gemini 长设计审阅、Codex review 带 git diff 等）用 heredoc：

```bash
command ccb ask --wait --timeout 300 a2 <<'EOF'
<多行 prompt>
EOF
```

**单行短 prompt**（如 `ping`、一句话追问）可以直接参数式调用，不用 heredoc：

```bash
command ccb ask --wait --timeout 60 a2 "ping"
```

（TD-007 修复了原本 stdin 是 Unix socket 会 hang 的问题，现在不带 heredoc / 不带 `< /dev/null` 也不会挂；见 PR #188）

- `--timeout` 默认 3600s（1h），主控内部分派建议设 300-600s（5-10min）防 Bash tool 超时（Bash tool 上限 10min）
- reply 从 stdout 直接回来，在同一 turn 就能呈给用户

### 只在以下场景才走 async

1. **用户显式说"不等回复"/"后台跑就行"**
2. **预计 >8 分钟的长任务**（超过 Bash tool 10min 超时）
3. **并行多个 ask**（同时问 a1 a2 a3）

走 async 时：
1. 必须挂 Monitor 监听 `ccb ask get <job_id>` 直到 `status: completed`
2. Monitor 触发时主控 Claude 拉 reply，呈给用户
3. 禁止"Gemini processing..."后什么都不做 — 必须挂监听

### 禁止

- `/ask` skill 的 default async 模式 + end turn + 不监听 = **消息孤儿**（用户成中转站）
- 遇到 reply 依赖但懒得 `--wait`，就让用户手动 `ccb pend` 捞

### `pend` 的正确定位

`pend` 是**观测接口**，不是主流程。用于调试 / 排查历史 job，不参与 reply 采集。主流程用 `--wait` / `wait <job_id>` / `get <job_id>`。

### 项目锚定（per-session 注入，2026-04-25 重设计）

#### 设计原理（Gemini holistic redesign 2026-04-25）

`CCB_PROJECT_DIR` 是 **per-session（per-master Claude）的会话锚**，**不是**全局工作区指针。一个 master Claude → 一个 ccbd → N 个 agent，三者的生命周期通过 systemd `BindsTo=` 绑死。两个不同 repo 里的 master Claude 必须有各自的 ccbd 和 agent，**绝不能共用**。

之前 fork commit `45d42fd` + `12e07a2`（upstream PR [#190](https://github.com/bfly123/claude_code_bridge/pull/190)）把 `export CCB_PROJECT_DIR="$HOME"` 钉进 `~/.claude/shell/ccb.sh` 和 `/usr/local/bin/claude-sandbox`，让所有 master 都打到同一个 `/home/sevenx/.ccb/`。结果两个不同 repo 的 master 共用一个 ccbd、一个 Codex 进程、一个 Codex 对话历史 —— **per-repo 隔离被这条硬绑定彻底毁掉**。2026-04-25 用户发现这个问题后做的全局重设计就是要把 env var 改回它该有的角色：**会话锚，由 orchestrator 在起 master 时按需注入**。

#### 锚定来源优先级（discovery.py 行为）

1. **`CCB_PROJECT_DIR` env var**（如果有）：直接锚定到该路径
2. **cwd-upward walk**：从 cwd 往上走找最近的 `.ccb/`
3. **未找到**：报错（不再 fallback 到 `$HOME`）

#### 谁会注入 env var

- **orchestrator 起的 task scope**（`claude-ccb-orchestrator start-task-scope`）：通过 `systemd-run --setenv=CCB_PROJECT_DIR=<task_dir>` 注入，整个 task scope 里所有进程都能看到
- **普通 `claude-sandbox` 起的日常 master**：**不再注入**任何 env var，全靠 cwd-walk 找最近的 `.ccb/`；如果 cwd 不在 CCB 项目里，先 `cd` 到目标 repo 或显式 `ccb --project <path>`

#### 日常使用规则

- 在 CCB 项目目录里：`ccb ask / pend / ping / ps / watch` 直接用，cwd-walk 自动找 `.ccb/`
- 在 orchestrator scope 里：env var 已注入，无视 cwd
- **Fallback**：env var 没生效 + cwd 也不在 CCB 项目里 → 用 global flag `ccb --project /home/sevenx <subcommand>` 临时显式指定

**`--project` flag 位置**：global flag，必须在 subcommand **前面**，`ccb --project <path> ask ...`；写成 `ccb ask --project ...` 会报 `unknown ask option`。

**claude_code_bridge 源码仓的特殊处理**：仓里自带的 `.ccb/ccb.config` 历史上 upstream 用的是 `agent1..agent5` 命名，跟我们日常的 `a1/a2/a3` 不一致。本地已 `git update-index --skip-worktree .ccb/ccb.config` + 改为 `a1/a2/a3`，不会进 git 也不会被 upstream 覆盖。未来 upstream issue [#191](https://github.com/bfly123/claude_code_bridge/issues/191) 讨论是否改成 `.ccb.config.example` 清根儿。

**旧的 cwd 纪律（"每次 pwd + cd 回项目根"）部分回归**：因为 daily master 不再有 env var 兜底，cwd 飘到非 CCB 项目时 `ccb` 命令会报 "no .ccb found"。但这不是 bug 是 feature——它强制 master 只能在自己 repo 里操作 ccb，per-repo 隔离从此真正生效。Bash tool 命令前可以加 `(cd /home/sevenx/<repo> && ccb ...)` 显式锚定。

### 多 Claude 并发警告

同项目目录下开 2+ Claude Code，都会连到**同一个 ccbd**（singleton），然后：
- 消息**串行化**（官方 `serial-per-agent` queue policy），不会并发冲突
- **Context 会污染**：a2 是长 session 的 Gemini，两方消息进同一个对话历史
- 官方没有"多 caller 隔离"机制

**规则**：
- 避免同项目并发 2+ 主控 Claude 对同一 agent 发消息
- 如果必须，每次切主控前先 `ccb ask <agent> /new` 或用 `autonew` skill reset agent session
- 最好的做法：一项目一主控 Claude

## 7. Tool Commands

CCB 命令（`/ask`/`/ping`/`/pend` 等）以 `ccb-config.md` 为准，随 CCB 版本自动更新。本文件不重复。

============================================================
= /home/sevenx/.claude/rules/ccb-config.md
============================================================
<!-- CCB_CONFIG_START -->
## AI Collaboration
Use `/ask <agent>` to contact another CCB agent by name.
Use `/ping <agent|ccbd>` to inspect project control-plane health.
Use `/pend <agent|job_id>` to inspect mailbox/job replies.

Agent names come from `.ccb/ccb.config`. Providers are implementation details.

## Async Guardrail (MANDATORY)

When you run `ask` (via `/ask` skill OR direct `Bash(ask ...)`) and the output contains `[CCB_ASYNC_SUBMITTED`:
1. Reply with exactly one line: `<Provider> processing...` (use actual provider name, e.g. `Codex processing...`)
2. **END YOUR TURN IMMEDIATELY** — do not call any more tools
3. Do NOT poll, sleep, call `pend`, check logs, or add follow-up text
4. Wait for the user or completion hook to deliver results in a later turn

This rule applies unconditionally. Violating it causes duplicate requests and wasted resources.

<!-- CCB_ROLES_START -->
## Role Assignment

Abstract roles map to concrete AI providers. Skills reference roles, not providers directly.

| Role | Provider | Description |
|------|----------|-------------|
| `designer` | `claude` | Primary planner and architect — owns plans and designs |
| `inspiration` | `gemini` | Creative brainstorming — provides ideas as reference only (unreliable, never blindly follow) |
| `reviewer` | `codex` | Scored quality gate — evaluates plans/code using Rubrics |
| `executor` | `claude` | Code implementation — writes and modifies code |

To change a role assignment, edit the Provider column above.
When a skill references a role (e.g. `reviewer`), resolve it to the configured agent that owns that role.
<!-- CCB_ROLES_END -->

<!-- CODEX_REVIEW_START -->
## Peer Review Framework

The `designer` MUST send to `reviewer` (via `/ask`) at two checkpoints:
1. **Plan Review** — after finalizing a plan, BEFORE writing code. Tag: `[PLAN REVIEW REQUEST]`.
2. **Code Review** — after completing code changes, BEFORE reporting done. Tag: `[CODE REVIEW REQUEST]`.

Include the full plan or `git diff` between `--- PLAN START/END ---` or `--- CHANGES START/END ---` delimiters.
The `reviewer` scores using Rubrics defined in `AGENTS.md` and returns JSON.

**Pass criteria**: overall >= 7.0 AND no single dimension <= 3.
**On fail**: fix issues from response, re-submit (max 3 rounds). After 3 failures, present results to user.
**On pass**: display final scores as a summary table.
<!-- CODEX_REVIEW_END -->

<!-- GEMINI_INSPIRATION_START -->
## Inspiration Consultation

For creative tasks (UI/UX design, copywriting, naming, brainstorming), the `designer` SHOULD consult `inspiration` (via `/ask`) for reference ideas.
The `inspiration` provider is often unreliable — never blindly follow. Exercise independent judgment and present suggestions to the user for decision.
<!-- GEMINI_INSPIRATION_END -->

<!-- CCB_CONFIG_END -->

============================================================
= /home/sevenx/.claude/rules/ccb-orchestration.md
============================================================
# CCB Orchestration 规则（master Claude 用）

> 配套工具：`~/.local/bin/claude-ccb-orchestrator`（含 start-task-scope / stop-task-scope / list-my-scopes / cleanup-orphans 四个子命令）。
> 设计文档：`/home/sevenx/coding/claude_code_bridge/docs/claude-ccb-scope-orchestration-plan.md`。
> Phase 1 MVP 已完成；本规则是 Phase 2 P2-1（task-boundary 启发式）的具体化。

## 什么时候 **必须** 开独立 orchestrator scope

master Claude 要给一个任务开独立 sibling scope（用 `claude-ccb-orchestrator start-task-scope <name> --agents <list>`），触发条件任一成立即必须开：

- **T1 重负载 agent 调用**：预计要派 codex / claude / gemini 做 > 30 秒的工作（典型：跑测试套、实施大功能、长 code review、带全量 diff 的审阅）
- **T2 高并发 fan-out**：预计同时触发 ≥ 2 个 agent 并行工作
- **T3 spawn 大量 subprocess**：预计 agent 工作里会 spawn ≥ 20 个线程或子进程（典型：pytest test/ 全量、`gh pr create` 大量 check、node/npm 树状安装）
- **T4 可能污染状态**：任务逻辑上需要干净的 agent session（比如测一个 provider 的 cold start 行为、验证新安装的 CCB 的行为）

## 什么时候 **可以** 不开独立 scope，直接用日常 `/home/sevenx/.ccb/`

- **L1 单问**：一次性向 Gemini / Codex 提问，预估 < 30 秒（典型：design review、rubrics scoring、ask plan opinion）
- **L2 文档任务**：写文档、读文件、整理 markdown，不 spawn 子进程
- **L3 git 元数据操作**：push / PR / issue / status — 这些本身不占 CCB 预算

## 任务开始的语义信号（topic shift 触发自动开 scope）

除了上面的硬阈值，下面的对话信号也触发开新 scope（每条都是独立触发）：

- 用户显式说 **"新任务"** / **"开新项目"** / **"开新 task"** / **"这次换个主题"**
- 连续 3 轮用户消息的核心名词完全不同（heuristic；有歧义就谨慎，倾向于开新 scope）
- 上一个 task 已经走完"PR opened / deployed / done"等完成信号，用户继续给新指令

## 任务结束的语义信号（触发 stop-task-scope）

- 任务逻辑闭合（PR 提了 / 改完测了 / 部署成功）且用户未派生新子任务
- 用户显式说 **"做完了"** / **"告一段落"** / **"先打住"** 等结束信号
- **T 时长**: 同一个 task 持续 ≥ 2 小时且没新动作——**force-flush**，stop 并把下次调用视为新 task
- **S scope 利用率**: `claude-ccb-orchestrator list-my-scopes` 显示该 task scope `tasks_current / tasks_max > 0.8` 持续超过 60s——**force-flush**，stop 并开新 scope（避免撞上限）

## 命名约定

- task-name 必须 `[a-z0-9-]+` ≤ 40 字符
- 推荐 slug 模板：`<action>-<object>[-<qualifier>]`
  - `heavy-pytest-ccb`
  - `impl-td-008-ccbd-rewrite`
  - `review-agent-harness-pr`
- 避免时间戳式命名（`test-mvp-20260424`）——人读不易记忆，也没额外信息

## 默认参数（无特殊需求就走默认）

- `--tasks-max 500`：足够覆盖一个 codex/claude agent 起来 + 几十个子进程
- 重任务（codex 跑 pytest 全量、并发 3 agent）：`--tasks-max 800`
- `--memory-max 512M`：默认不设；如果在内存紧张机器（比如这台 VPS 7.7G）且已知 task 会吃 GB 级内存，显式设 `512M` 甚至 `256M`

## --force 的使用时机

**默认不用**。用 `--force` 仅当：
- 前一个 task 异常退出但 tracking 还有残留
- 确认要放弃前一个 task 的所有状态（session、.ccb/cbbd、pane logs）

## --reset 的使用时机（stop-task-scope）

**默认不用**。`--reset` 会 rmtree 掉 project_dir 下所有东西（包括 session files）。用 `--reset` 仅当：
- 这个 task-name 以后不再用了（完全抛弃）
- 确认要从 clean slate 重启（比如 provider 版本升级后要清缓存）

**不要 --reset 的典型场景**：同一 task-name 中途停下、过会儿继续——此时想要 session 延续（--continue 语义），保留 project_dir。

## 调用 ccb ask 时的 CCB_PROJECT_DIR 切换

起了独立 scope 后，要显式用新 project 跑 ask：

```bash
RESULT=$(claude-ccb-orchestrator start-task-scope heavy-pytest --agents a1:codex --tasks-max 800)
PROJECT_DIR=$(echo "$RESULT" | jq -r .project_dir)

CCB_PROJECT_DIR="$PROJECT_DIR" command ccb ask --wait --timeout 900 a1 "<brief>"
# ... 继续在这个 project 下的 ask ...

claude-ccb-orchestrator stop-task-scope heavy-pytest
```

记得 **每次** 加 `CCB_PROJECT_DIR=<path>` 前缀。原因：master Claude 自己**在 task scope 之外**跑（scope 只关 task 的 ccbd + agent），所以 master 不继承 scope 的 `--setenv CCB_PROJECT_DIR`；不显式带这个前缀的话，master 的 `ccb ask` 会按 cwd-walk 找最近的 `.ccb/`，错命中当前 cwd 所在 repo 的 ccbd（或没有 `.ccb/` 时直接报错）。

（2026-04-25 redesign 注：以前 `~/.claude/shell/ccb.sh` 和 `claude-sandbox` 把 `CCB_PROJECT_DIR=$HOME` 钉死在 master 环境里，相当于"任何不带前缀的 `ccb ask` 默认打到 `/home/sevenx/.ccb/`"。这条钉死被撤了——它会让两个不同 repo 的 master 共用同一个 ccbd。现在的规则更严：master 不锚定到任何全局位置，所有 cross-task 通信必须显式 `CCB_PROJECT_DIR=<path>`。）

## Janitor 自动兜底（不是 master Claude 的责任）

`claude-ccb-janitor.timer` 已 enable（systemd user timer，每小时 + boot 后 5 分钟触发），自动清理：
- 孤儿 systemd units（tracking 没认领的 `claude-ccb-*.service`）
- 脱钩的 tracking 条目（tracking 有但 systemd 已无 unit 的）
- 默认路径下孤儿 project_dir

所以 master Claude 不用担心 session 崩了忘 stop——janitor 会兜。但**不要依赖它**：主动 stop 仍是正确纪律。

## Scope 归属纪律（只信 tracking，不 ps 猜）

**master Claude 只动自己起的 scope，不动任何不在自己 tracking 里的东西。**

### 规则

1. **起 scope 必经 orchestrator**：任何新 scope 都走 `claude-ccb-orchestrator start-task-scope <name>`，不手动拉 tmux / ccbd。这样 tracking 自动登记，起归属信息。
2. **任务做完立即 stop**：任务结束信号一触发（PR 合、测试过、用户说"告一段落"等，见"任务结束的语义信号"一节），**同一个 turn 里** 调 `stop-task-scope <name>`，不拖到"稍后统一清"。拖延会在对话切换后忘记，变成无主残留。
3. **查状态只用 `list-my-scopes`**：要知道自己起了哪些 scope，**只**跑 `claude-ccb-orchestrator list-my-scopes`，看 tracking 文件。
   - **禁止** 用 `ps` / `ls ~/.local/state/claude-ccb-projects/` / 扒 tmux socket 路径来判断归属。
   - **禁止** 看到进程名/路径像自己起过的，就当成"八成是我的"去清理。
4. **tracking 里没有 = 不是我的 = 不要动**：哪怕 `ps` 里看见一个叫 `test-mvp` 的 tmux、哪怕路径是 `~/.local/state/claude-ccb-projects/xxx/`——只要 `list-my-scopes` 里没有它，就是别人的 scope、上一个 master 的残留、或者无主孤儿，**全部不归主控 Claude 管**。
5. **孤儿清理是 janitor 或用户的职责**：不在 tracking 里的孤儿 scope / project_dir 交给 `claude-ccb-janitor.timer` 自动兜底，或让用户手动决定清不清。主控不要越权。

### 为什么这条这么严

动错别人的 scope = 删掉对方正在用的 session。上一次差点犯错：看到 `ps` 里有个 3h 的 `test-mvp` tmux、`list-my-scopes` 空、systemd scope 空，**路径证据链完整**——但这个证据链恰恰说明"它和我的 orchestrator 机制没任何关系"，不是"它是我的孤儿"。两者表象一样，语义完全不同。只看 tracking 可以一步避开这类误判。

"横查竖比对、扒进程猜归属"本质上是想扮演 janitor 的角色，但主控 Claude 不是 janitor——主控只对自己 tracking 里的 scope 负责。

## 不允许的做法

- **不许**直接从 master Claude scope 里 spawn `pytest test/` 全量——那会占 master 自己的 TasksMax 预算。应该开独立 scope 或交给 agent 在新 scope 内跑
- **不许**在没 stop 上一个 task 的情况下，用不同 task-name 反复起 scope——scope 不断叠加会堆满 systemd user session 预算
- **不许**用 `--force` 绕过同名 task 而不理解前一个 task 为什么还在——先 `list-my-scopes` 看清楚，再决定 stop 或 force
- **不许**用 `ps` / 扒文件路径 / 看 tmux socket 等方式判断某个 scope 是不是自己的。scope 归属的唯一真相是 `list-my-scopes`（tracking 文件）。tracking 里没有的 scope，哪怕表象再像自己起的，都不要动——见 "Scope 归属纪律" 一节

## 快速参考

| 操作 | 命令 |
|---|---|
| 开 scope | `claude-ccb-orchestrator start-task-scope <name> --agents a1:codex` |
| 停 scope 保 state | `claude-ccb-orchestrator stop-task-scope <name>` |
| 停 scope 清状态 | `claude-ccb-orchestrator stop-task-scope <name> --reset` |
| 列现状 | `claude-ccb-orchestrator list-my-scopes` |
| 对账清理 | `claude-ccb-orchestrator cleanup-orphans` |
| 看 janitor 状态 | `systemctl --user status claude-ccb-janitor.timer` |
| 强制接管已存在 | `claude-ccb-orchestrator start-task-scope <name> --force ...` |

============================================================
= /home/sevenx/.claude/rules/code-review.md
============================================================
# Code Review Rules

## Workflow-Driven Code Review

### Principle
Code review is not "is this code syntactically/logically correct" but "is this code correct in the context of the entire workflow."

### Review Method: Trace the Flow
After implementation, review using the following approach (not just running tests):

1. **Data Flow Tracing**: From data entry to exit, verify step by step:
   - What format does the upstream module output? Does this module's input parsing match?
   - What format does this module output? Can downstream modules consume it correctly?
   - Are field names, types, and nesting structures consistent?
   - After adding/removing/renaming fields, are all references synchronized?

2. **Dead Code Detection**: Check in this modification:
   - Are there functions/methods written but never called?
   - Are there variables/parameters defined but never used?
   - Are there imported modules that aren't used?
   - Are there branch conditions that can never trigger?

3. **Interface Contract Verification**: Check that function/tool callers and implementations match:
   - Do parameter names, types, required/optional match?
   - Does the return value format match caller expectations?
   - Does error handling match the contract between caller and callee?

4. **End-to-End Scenario Walkthrough**: Using a concrete data example, mentally trace the full flow:
   - Input X → step A → output Y → step B → ...
   - At each node: is the data format correct? Are edge cases handled?

### Review Scope
- Don't only look at modified code — also check its **callers** and **callees**
- Modified data format → trace all downstream code consuming that data
- Modified interface signature → trace all upstream code calling that interface
- Added new field → confirm all serialization/deserialization points handle it

### Anti-Patterns
- Only looking at the modified code block, not its callers and callees
- Assuming tests passing means no problems (tests may not cover integration scenarios)
- "This code itself has no bugs" — but it uses the wrong context in the workflow
- Adding new fields without checking all downstream consumers
- Only doing local syntax/logic checks, not end-to-end data flow traces

============================================================
= /home/sevenx/.claude/rules/code-style.md
============================================================
# Code Style Rules

## Python
- Python 3.11+, strict mypy enabled
- `from __future__ import annotations` at the top of every file
- No `Any` type. Use `object`, `Unknown` patterns, or explicit interfaces
- Prefer `dataclasses` for value objects, `Pydantic` for validated external data
- Use `TypeAlias` for complex type expressions
- Use `Literal` types and discriminated unions over stringly-typed fields

## Naming
- Variables/functions: `snake_case`
- Classes: `PascalCase`
- Constants: `UPPER_SNAKE_CASE` for true constants, `snake_case` for derived values
- Boolean variables: prefix with `is_`, `has_`, `should_`, `can_` (e.g., `is_loading`, `has_permission`)
- Private members: single underscore prefix `_internal_method()`

## Functions
- Max function length: 40 lines. Extract if longer.
- Max parameters: 3. Use a dataclass or TypedDict for more.
- Return early for guard clauses; avoid deep nesting.
- NEVER use nested ternaries. Use early returns or `if/else` blocks.
- Use `*` to force keyword-only arguments where clarity matters.

## File Organization
- One primary export per file. Supporting types/utils can coexist if small.
- Import order: (1) `__future__` (2) stdlib (3) third-party (4) local, each group separated by blank line
- No circular imports. If detected, refactor to break the cycle.
- Max file length: 300 lines. Split into focused modules if larger.

## String Formatting
- f-strings for simple interpolation
- `.format()` or `%` for log messages (lazy evaluation)
- Triple-quoted strings for multiline content

## Error Handling
- Use typed exception classes for domain errors (inherit from project base)
- Errors at system boundaries only (API calls, user input, file I/O)
- Avoid try/except for control flow
- Let unexpected errors propagate to boundary error handlers
- Always specify explicit exception types in `except` clauses

## Async
- Use `async/await` for I/O-bound operations (LLM calls, file I/O)
- Never mix sync and async in the same call chain without explicit bridge
- Use `asyncio.gather()` for concurrent independent operations

## Modularity & Cohesion

### Single Responsibility
- Each file has one clear primary responsibility, file name should summarize its content
- A file exceeding 300 lines → must consider splitting
- "This function is convenient to put here" ≠ correct reason. Correct reason = responsibility ownership

### No Convenience Dumping
- Do not add unrelated functions to a file just because it already imports needed modules
- Do not violate responsibility boundaries just because "modifying this file is faster than creating a new one"
- Scattered utility functions → consolidate to appropriate module

### Patches Must Be Cohesive
When fixing bugs or adding features:
- First evaluate: which module does this change logically belong to?
- If the change touches 3+ unrelated files with scattered modifications → stop and consider extracting an independent module
- "Adding if-checks everywhere" to accommodate a new requirement = design problem signal

### New Feature Module Placement
Before adding a new feature, answer:
1. Does this belong to an existing module? → add it there
2. Doesn't belong anywhere existing? → is it worth a new module? Ask the user
3. Too small for a new module? → put it in the most related module, ensure responsibility consistency

## Zero Tolerance for Silent Failures

### Rules
1. **No bare except**: `except Exception: pass` is always a bug
2. **No catch-all to debug**: `except Exception as e: logger.debug(...)` = invisible in production = silent failure
3. **Interface calls must verify**: when calling external module APIs, confirm parameter signatures match (don't rely on try/except as a safety net)
4. **Degradation must be explicit and observable**:
   - Must log at WARNING or ERROR level
   - Must explain the reason and impact of degradation
   - Degradation behavior must be documented

### Review Requirement
For every try/except block during code review:
- What specific exception is this catching? (cannot be Exception/BaseException)
- What does it do after catching? (cannot be pass/ignore)
- How will operators know an exception occurred? (must have observable output)

============================================================
= /home/sevenx/.claude/rules/communication.md
============================================================
# 表达清晰度规则（全局）

> 2026-04-23 由用户明确要求并写入全局规则。违反这条规则会被视为严重问题，优先级高于"回复简洁"。

## 铁律

**用人类自然语言把话说清楚说完整。不要为了简洁而偷懒省略必要的字词和短语，不要用程序员黑话或省略语造成歧义。**

## 为什么有这条规则

之前在讨论 graph_agent 的 subgraph 机制时，我写了："subgraph phase 的 tools 列表被强制跳过解析，保持空"、"被丢弃的只是父 phase 的 tools 字段"。这些表述在我看来是"准确"的，但对用户来说有严重歧义——"父 phase 的 tools" 到底指什么？是父 skill 的所有 tools？还是只是父 skill 在 subgraph 声明处额外写的 tools？还是子 skill 的 tools？用户因此误以为"子 skill 的 tools 被丢弃，那子 skill 怎么跑"，被严重误导。

根因是我**为了回复简洁而省略了必要的限定词和前因后果**，导致指代不清、语境缺失。这种表达方式对熟悉代码的人可能够用，对正在理解设计的用户完全不够用。

## 具体要求

### 1. 不要用省略代词或省略短语指代复杂概念

**不好**：
> 父 phase 的 tools 字段被丢弃

**好**：
> 当父 skill 的某个 phase 使用 `subgraph:` 字段委派给子 skill 时，**这个父 phase 自己在 phase_config 里额外写的 tools 列表**会被 loader 静默忽略。子 skill 内部每个 phase 各自声明的 tools 不受影响，保持正常加载和使用。

区别：前者省略了"父 skill 的某个 phase"、"在 phase_config 里额外写的"、"子 skill 内部的 tools 不受影响"等关键限定，给用户留下猜测空间。

### 2. 不要默认用户能自动补全技术术语

**不好**：
> 这是有意的正交性设计，静默丢弃是 bug。

**好**：
> 这个 subgraph 跟 tools 互相排斥的约束，是框架**有意设计的**，不是 bug。设计的目的是让每个 phase 的职责只有一种——要么委派给子 skill（subgraph 模式），要么让 LLM 在 agent loop 里用 tools（LLM 模式），要么纯代码执行（code 模式）——不要混搭。
> 但是，**当 PM 误写了 subgraph + tools 混搭时，框架目前是静默丢弃的（既不报错也不警告）**，这部分确实是 bug，应该在 compiler 层加规则检查，在 compile 时就报错提醒 PM。

区别：把"正交性设计"拆解成具体意思；把"静默丢弃是 bug"拆解成"什么情况下会发生+框架现在怎么表现+为什么是 bug"。

### 3. 说完整的句子，不要用 "即"、"即可"、"故"、"其" 这类省略连接词

**不好**：
> 即 subgraph 会递归加载子 skill 实例，故丢弃无意义。

**好**：
> 当 loader 看到 subgraph 字段时，会把子 skill 的 SKILL.md 作为一个独立的完整 harness 实例加载进来。运行到这个 phase 时，框架会调用这个子 harness 的 run 方法，让子 skill 按自己内部的拓扑和 tools 跑完整个流程。在这种情况下，父 phase 自己声明的 tools 根本没有地方被调用（因为父 phase 本身不跑 LLM、不跑 agent loop），所以 loader 把父 phase 的 tools 忽略掉。

### 4. 引用代码或设计时必须给具体定位

**不好**：
> loader 跳过了 tools 解析。

**好**：
> 在 `src/core/graph_agent/core/loader.py` 的第 578 行，代码写着 `if subgraph_harness is None:`，这意味着**只有在当前 phase 没有 subgraph 字段时，tools 列表才会被正常解析**；一旦 subgraph 字段存在，整个 tools 解析的 for 循环都会被跳过。

### 5. 在用户可能困惑的地方主动插入澄清

**场景**：用户刚问了一个问题，我回答前应该预判"我这句话会不会让用户产生某种误解"，如果会，提前用一句话澄清。

**示范**：
> 先澄清一下："subgraph phase 的 tools 被清空" 这个说法容易让人误解成"子 skill 的所有 tools 都被清空了"，实际上**被清空的只是父 skill 在声明 subgraph 的那个 phase 里额外写的 tools 字段**。子 skill 内部所有 phase 自己的 tools 是完整保留的，正常工作。下面我从这个澄清出发继续讲……

### 6. 用对比和具体例子代替抽象归纳

**不好**：
> subgraph 是静态组合，sub_skills 是动态决策。

**好**：
> 两种机制的差别在于**"谁来决定调用这个子 skill"**：
>
> - **subgraph 模式**：在 SKILL.md 里写死 `subgraph: subskills/render/SKILL.md`，运行时框架 100% 按照这个声明去跑子 skill，**没人能动态选择"不跑"或"换一个"**。就像代码里写死的函数调用 `render()`。
> - **sub_skills 模式**：在 SKILL.md 里声明 `sub_skills: [render, refine, enhance]`，把这三个子 skill 包装成三个工具暴露给 LLM。**LLM 在 agent loop 里根据当前 context 自己判断**，可能调 render，可能调 refine，可能都不调。就像给 LLM 一个工具箱，让它自己决定什么时候用哪个。

### 7. 表格要列完整的上下文，不要只列关键字

**不好**：

| 模式 | tools | prompts |
|------|-------|---------|
| LLM | ✅ | ✅ |
| Subgraph | ❌ | ❌ |
| Code | ✅ | ❌ |

**好**：

| Phase 模式 | 父 phase 能不能自己声明 tools | 父 phase 能不能自己写 system_prompt | 谁来真正执行 |
|-----------|------------------------------|--------------------------------------|-------------|
| **LLM phase**（有 system_prompt，没 subgraph） | 可以，LLM 会在 agent loop 里按需调用这些 tools | 必须写，没有 prompt 没办法驱动 LLM | DeerFlow agent loop（LLM + tools） |
| **Subgraph phase**（有 subgraph，没 system_prompt） | 不可以（就算写了 loader 也会忽略），因为没人会调用 | 不可以（就算写了也会被忽略），因为这个 phase 不直接跑 LLM | 子 skill 自己的 harness（完整跑子 skill 的所有 phase）|
| **Code-only phase**（都没有 system_prompt 也没有 subgraph） | 可以，这些 tools 被当作纯函数顺序调用 | 没必要写，这个 phase 本来就不调 LLM | 框架直接按顺序调用 tools 里的函数 |

## 优先级

这条"说清楚"规则**优先于**"回复简洁"的默认指引。即使因此让回复变长，也必须说清楚。

但"说清楚"不等于"重复废话"。原则是：
- 每个关键概念第一次出现时给完整定义
- 指代不明的代词必须替换成具体名词
- 跨句子的因果关系要用完整连接词（"因为...所以..."，不要只用"故"或"即"）
- 涉及代码的部分必须带文件路径 + 行号

## 自检

每次回复前自问三个问题：
1. **如果一个完全没读过代码的人看这段话，会不会产生歧义？**
2. **我用的每个限定语（"这个"、"这类"、"上面的"、"刚才提到的"）指代是否在同一段内清楚？**
3. **我省略的字词（为了简洁）里，有没有任何一个是用户理解意思必需的？**

如果任一答案是"有可能"，就必须补齐。

============================================================
= /home/sevenx/.claude/rules/git-workflow.md
============================================================
# Git Workflow Rules

## Branch Strategy
- `main` -- production-ready, protected, requires PR review
- `feat/<short-desc>` -- new features
- `fix/<short-desc>` -- bug fixes
- `refactor/<short-desc>` -- code improvements
- `chore/<short-desc>` -- tooling, deps, config

## Commit Message Format
```
<type>: <concise description of WHY, not WHAT>

[optional body: explain motivation, tradeoffs, or link to issue]

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>
```

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`, `ci`

## Commit Discipline
- One logical change per commit. Avoid lumping unrelated changes.
- Every commit should leave the project in a buildable, testable state.
- Commit frequently: small, atomic commits over large monolithic ones.
- Stage specific files by name. NEVER use `git add .` or `git add -A`.
- NEVER amend published commits. Create new commits to fix issues.
- NEVER force push to main/master.
- NEVER skip pre-commit hooks (--no-verify).

## Pull Request Standards
- Title: under 70 characters, descriptive of the change
- Body: include Summary (bullet points), Test Plan (checklist), and link to issue
- One feature per PR. If a PR grows beyond ~400 lines, split it.
- All CI checks must pass before merge.
- Squash merge for feature branches to keep main history clean.

## Pre-Commit Checks
Before any commit:
1. Linter passes
2. Type checker passes
3. Affected tests pass
4. No secrets or credentials in staged files

## Tagging & Releases
- Semantic versioning: `v<major>.<minor>.<patch>`
- Tag after merging to main, not on feature branches
- Changelog generated from conventional commit messages

============================================================
= /home/sevenx/.claude/rules/logging.md
============================================================
# Logging Rules

## Logging Discipline (Iron Rule)

### Every action must have a log
This rule has been repeatedly requested but not consistently followed, hence elevated to an iron rule.

1. **Any side-effect action** (file read/write, API call, state change, tool call, model call)
   must have a log before and after:
   - Before: `logger.info("action description, input param summary")`
   - After: `logger.info("result summary")` or `logger.error("failure reason")`

2. **Control flow decision points** (if/else branches, retry, fallback, skip)
   must record which path was taken and why:
   - `logger.info("choosing path X, reason: Y")`
   - `logger.warning("skipping step X, reason: Y")`

3. **Exception handlers** must not silently swallow exceptions:
   - `except Exception: pass` — FORBIDDEN
   - `except Exception as e: logger.debug(str(e))` — FORBIDDEN (debug level = invisible in production = no record)
   - `except SpecificError as e: logger.warning("XXX failed: %s, degrading to YYY", e)` — CORRECT

### Log Level Standards
- `DEBUG`: per-item processing details within loops (can be verbose)
- `INFO`: start/end of each step, key decisions (MUST have)
- `WARNING`: degradation, skipping, unexpected but continuable situations (MUST have)
- `ERROR`: failure, non-continuable situations (MUST have)

### Callback / Tracing Completeness
Framework-level callbacks (TracingCallback, MetricsCallback, etc.) must cover all event types:
- When adding new event types, all callbacks must have corresponding handlers
- Review checklist: list all framework events → compare callback implementations → missing = bug

### Verification
After implementing logging, run through one real scenario and check:
- Can the full execution flow be reconstructed from logs alone?
- Can the specific step and cause be identified in error scenarios?
- If not → logging is insufficient, continue adding

### Log Format Suggestion
```python
logger = logging.getLogger(__name__)

# Step start
logger.info("phase=%s action=start tool=%s input=%s", phase_name, tool_name, input_summary)

# Step end
logger.info("phase=%s action=end tool=%s duration=%.2fs output=%s", phase_name, tool_name, elapsed, output_summary)

# Decision point
logger.info("phase=%s decision=%s reason=%s", phase_name, choice, reason)

# Degradation
logger.warning("phase=%s fallback from=%s to=%s reason=%s", phase_name, primary, secondary, reason)
```

============================================================
= /home/sevenx/.claude/rules/prompt-engineering.md
============================================================
# Prompt Engineering Rules

## Creative Prompt Principle: Intent Over Technique
When writing LLM prompts for creative tasks, describe the **audience psychology + desired effect**, not **specific techniques**.

Prescribing techniques (e.g., "use time anchors") is like giving the answer — it won't transfer to other projects. Describe "what state is the audience in, what effect do you need to achieve" for universal applicability.

When writing SKILL.md or any prompt involving creative decisions, ask yourself: am I describing an effect or prescribing a technique? If you find yourself writing "use XX method" or "for example XX", be alert — switch to describing audience psychology and desired viewing experience.

## Five Iron Rules

### 1. Define Mechanisms, Don't Stack Rules
Find the role's "first principle" (e.g., "the narrative instinct of a million-view narrator"), use one core mechanism to drive all behavior. Don't list 20 conflicting do/don'ts.
- Bad: "transition segments 40-80 words, satisfaction segments 80-150 words"
- Good: define the role's professional identity and thinking mode, let the LLM judge what to expand or tighten

### 2. Design for All Inputs, Not Test Cases
We build general-purpose tools. Prompts must not contain words that only apply to current test data. Summarize the role's universal professional skills.
- Bad: "when encountering enemies/approaching threats, must break down action chains"
- Good: describe this role's professional instinct when facing any high-tension scenario

### 3. Don't Prescribe Specific Numbers
Don't hardcode word counts, sentence counts, or paragraph lengths. Let the LLM exercise professional judgment driven by the role identity.
- Bad: "EP1 total word count 2800-3500 words", "2-4 sentences per paragraph"
- Good: let role identity determine density

### 4. Don't Write Negative Corrections
The LLM has no awareness of the previous prompt version. Writing "don't do absolute separation" only draws attention to "absolute separation" and causes drift. Write only positive mechanisms.
- Bad: "the two should work together, don't do absolute separation"
- Good: directly describe the correct collaboration approach

### 5. Use Semantic Anchors Instead of Verbose Explanations
Use high-information-density metaphors like "bar-room bragging", "Hemingway editing", "million-view narrator" to activate semantic clusters from LLM pre-training. More effective than 10 lines of rules.

## Anti-Patterns
- Seeing suboptimal output and immediately patching the prompt (prescribing word counts, listing specific scenarios, writing negation constraints)
- Using 10 do/don'ts instead of one clear role mechanism
- Writing rules based on test case characteristics (will fail on different data)
- Prescribing specific numbers as "quality standards" (constrains LLM's professional judgment)

============================================================
= /home/sevenx/.claude/rules/security.md
============================================================
# Security Rules

## Secrets Management
- NEVER commit secrets, API keys, tokens, or credentials to git
- Use `.env` for development secrets (gitignored)
- Use environment variables or secret managers in production
- Rotate secrets regularly. Revoke immediately if leaked.
- Scan staged files for secrets before committing

## Files That Must Be Gitignored
```
.env
.env.local
.env.*.local
*.pem
*.key
credentials.json
serviceAccountKey.json
```

## Input Validation
- Validate at system boundaries: CLI arguments, file inputs, API responses
- Use Pydantic models for structured external input validation
- Reject invalid input early. Don't sanitize and hope for the best.
- File inputs: validate encoding, size, and content type.

## Command Injection Prevention
- NEVER use `os.system()` or `subprocess.shell=True` with user input
- Use `subprocess.run()` with list arguments for external commands
- Sanitize all file paths derived from user input (prevent path traversal)

## LLM-Specific Security
- Never pass raw user secrets to LLM prompts
- Sanitize LLM outputs before using in file operations or commands
- Rate limit LLM calls to prevent cost overruns
- Log LLM interactions for audit (excluding secrets)

## Dependencies
- Run security audits regularly
- Keep dependencies updated. Pin exact versions in lockfile.
- Review changelogs before upgrading major versions
- Avoid dependencies with known vulnerabilities or abandoned maintenance

## Data Handling
- Project data stays in gitignored directories
- Never log full content — use truncated excerpts in logs
- Respect copyright: process user-provided content only

============================================================
= /home/sevenx/.claude/rules/testing.md
============================================================
# Testing Rules

## Philosophy
- Tests are first-class citizens: every feature PR must include tests.
- Write failing tests FIRST, then implement (TDD when practical).
- Tests document behavior. A well-named test is better than a code comment.
- Test behavior, not implementation. Mock sparingly; prefer real instances.

## Structure
- Test files mirror source: `src/app/core/config.py` → `tests/core/test_config.py`
- Use `class TestX` for logical grouping; `def test_y` for specific behaviors.
- Test names follow pattern: `def test_should_behavior_when_condition()`
- Arrange-Act-Assert (AAA) pattern within each test.
- One assertion concept per test (multiple `assert` calls are fine if testing one thing).

## What to Test
- **Always test**: business logic, data transformations, edge cases, error paths
- **Test selectively**: LLM integration (mock the client), file I/O operations
- **Skip testing**: type definitions, constants, simple pass-through functions, third-party library behavior

## Running Tests
- Run the SINGLE relevant test file: `uv run pytest tests/core/test_config.py -x`
- Run the full suite only before committing: `uv run pytest tests/ -x`
- Verbose mode for debugging: `uv run pytest tests/core/test_config.py -xvs`

## Mocking Guidelines
- Prefer dependency injection over `unittest.mock.patch`
- Mock external services and I/O (LLM calls, filesystem, network)
- NEVER mock the module under test
- Use factories for test data; avoid hardcoded fixtures
- Reset all mocks between tests (use `pytest` fixtures with appropriate scope)

## Coverage
- No coverage percentage targets. Coverage is a tool for finding blind spots, not a goal.
- Focus on critical path coverage. Untested utility functions are acceptable if trivial.

## Test Data
- Use realistic data that mirrors production patterns
- Use factory functions to create test data: `create_test_config()`, `create_test_entity()`
- No shared mutable state between tests
- Each test must be independently runnable and order-independent
- Shared fixtures go in `tests/fixtures/` or `conftest.py`

## Pytest Conventions
- Use `@pytest.fixture` for setup/teardown, not `setUp`/`tearDown` methods
- Use `@pytest.mark.parametrize` for testing multiple inputs
- Use `tmp_path` fixture for filesystem tests (auto-cleanup)
- Use `monkeypatch` for environment variable tests


---

# 你这轮可访问的关键路径（cwd = /home/sevenx/coding/ccbd-rust）

| 路径 | 内容 |
|---|---|
| `docs/DESIGN.md` | Phase 2 启动文档 v1（草拟设计） |
| `docs/upstream-ccb-bugs/installer-default-config-mismatch.md` | 上游 CCB bug 文档（参考） |
| `research/findings/synthesis-18-days-by-claude.md` | 18 天痛点综述（claude 视角） |
| `research/findings/session-analysis-2026-04-26-by-claude.md` | claude 当天分析（268 行实质内容） |
| `research/findings/session-analysis-2026-04-26-by-gemini.md` | **空 skeleton**（12 行只有标题）—— 这是你要判断 A/B 类的对象 |
| `research/findings/per-day/` | 18 个 daily .md 详情 |
| `research/sessions/home-sevenx/markdown/` | 主控 master 原始 session（按天 .md） |
| `research/sessions/agent-harness/markdown/` | agent-harness master 原始 session |
| `research/candidates/agent-orchestrator/` | ComposioHQ 多 agent 调度器（核心参考） |
| `research/candidates/batty/` | Rust，38MB，含 src/ 子目录可深度看 |
| `research/candidates/ccswarm/` | Rust + tmux + worktree 隔离，4.3MB 紧凑 |
| `research/candidates/cli-agent-orchestrator/` | awslabs，13MB |
| `research/candidates/metaswarm/` | dsifry，多 provider |
| `research/candidates/overstory/` | jayminwest，自定义 SQLite mailbox + tmux + git worktree |
| `research/candidates/tamux/` | Rust，69MB，含 crates/ 多包结构 |

---

# 任务（按 Step A → F 严格顺序）

## Step A: Access Inventory
对上面表格里**所有路径**都用 Read/Glob 实际试一次。标 ✅ / ❌ / ⚠️。失败的列错误。

## Step B: Decision Inventory
基于已读材料，**重新列**做这个系统顶层设计必须回答的设计决策点（每条一句话）。
建议覆盖：架构边界 / SoT 持久化 / IPC / 生命周期 / 隔离 / sandbox / completion 检测 / stuck 检测 / reconciliation / build-vs-fork-vs-自研。

## Step C: Sufficiency Assessment
对每条决策点：
- 充分性 **YES / NO / PARTIAL**
- 论据**必须引用文件路径 + 行号或片段**
- 不充分的部分明确缺什么

**特别关注 7 候选项目的深度评估**——你这次能读源码了。对 SoT 持久化、completion 检测、orphan 接管、IPC 等核心决策点，给出**对照各候选项目实际实现的判断**。例如：overstory 的 SQLite mailbox 是怎么写的，能否直接借鉴；ccswarm 的 git worktree 隔离是 Rust 实现还是调 git 命令；tamux/batty 的 PTY 处理用什么 crate；等等。

## Step D: Gap List
- A 类必补：缺它就不能推进
- B 类应补：可推进但带显著风险
- C 类可不补

**对 `by-gemini.md` 空 skeleton 单独定级**：是 A 类还是 B 类？
- 如果 A 类：本轮 verdict 仍是"缺 A 类不能推进"，下一步主控会单独派一次任务请你重写它
- 如果 B 类：本轮可以推进顶层设计定稿，by-gemini.md 在写更下层 contract / schema 时再补也行

## Step E: Verdict（最终判决）
明确给出三选一：
1. **"能推进顶层设计定稿"**
2. **"缺 A 类不能推进"**（列具体缺什么 + 怎么补）
3. **"可推进但 B 类风险需 acknowledge"**（列风险 + 缓解建议）

## Step F: Open Asks
还需要主控做什么（贴文件 / 派 Codex 写代码 / 调研某 crate / 等等）。

---

# 协作铁律（不变 · 违反任一项 = 这次回复作废）

1. **不接受 "我相信" / "看起来" / "通常" / "一般而言"** —— 所有判断必须有**文件路径或代码片段引用**作为证据
2. **不恭维** —— 客观挑刺 DESIGN.md v1 里凭印象 / 想当然 / 没经验证的部分
3. **不绕过权限失败** —— 读不到就在 Step A 直接说，不靠猜
4. **每个 finding 至少回到原始证据**核对一次
5. 输出**中文**，**严格按 A → B → C → D → E → F 顺序**，每段 markdown 章节标题分隔
