请用中文回答。

# 任务 Round 4：7 候选项目深度对比 + Build-vs-Fork 终判（A 类 #2 补料）

延续之前的工作（你已经在 ccbd-rust 项目里熟悉 DESIGN.md / synthesis 痛点 / by-gemini.md 等）。

Round 2 你给 verdict "缺 A 类不能推进"，A 类有两项：
- A 类 #1 = by-gemini.md 重写 → Round 3 已写 151 行，但**引用 line number 多处 hallucinate**（凭印象编而非 grep 真命中）—— 留待你单独修，本轮不做
- A 类 #2 = 7 候选项目深度对比 → **本轮要做**

## 任务范围（专注一件事）

对 `research/candidates/` 下 7 个项目（agent-orchestrator / batty / ccswarm / cli-agent-orchestrator / metaswarm / overstory / tamux）做**深度源码对比**，输出"Build-vs-Fork-vs-借用 决策矩阵"。

## 严格工作流（避免 Round 3 偷工减料）

**铁律：每条结论引用 = 先 grep / Read 命中 → 粘贴 grep 原始输出 line number → 再写 finding。不允许"凭印象估计 line number"。**

具体做法：
1. 对每个项目先 `Glob 'research/candidates/<name>/**/*.rs'` 或 `**/*.{ts,js,py}` 找代码文件
2. 对每个核心决策点（见下面 8 个），用 `grep -rn` 在该项目里搜对应关键词
3. 把 grep 输出**完整粘贴**进 finding（带 file:line + 命中字符串）
4. 然后才写"这意味着..."的判断

## 8 个核心决策点（横向对比维度）

对每个项目分别评估这 8 个维度：

1. **SoT 持久化**：是否用 SQLite / 内存 / 文件系统？table schema 抽象层级？是否解决跨进程 race？
2. **IPC 协议**：UDS / TCP / stdin-pipe / mqueue？JSON-RPC / gRPC / 自定义 frame？支持订阅推送还是只 polling？
3. **PTY / Terminal 接管**：是否用 tmux 接管？还是直接 portable-pty / nix::pty？怎么解析 ANSI escape？
4. **Lifecycle 管理**：spawn / kill / 心跳 / orphan 接管的 state machine 在哪一层？是否有 reconciliation loop？
5. **Sandbox / 隔离**：cgroup / namespace / bwrap / chroot / 仅依赖 cwd？怎么实现"per-master 不串台"？
6. **Auth 共享**：怎么处理 OAuth credential / API key 在多 agent 间复用？symlink 还是 copy 还是 env 透传？
7. **Completion 检测**：怎么判断 agent 任务完成？anchor / sentinel / pane 静默 / output 解析 / hook 信号？多少层 fallback？
8. **Stuck / Health 监控**：怎么发现 agent 卡死？阈值 / multi-signal / deadline？

## 输出格式

### Step A: Per-Project Inventory
对每个项目输出（务必逐项列）：
- 主语言 / 主要依赖 crates 或 packages
- 项目大小（LOC 用 `wc -l`）
- 核心入口文件（main / lib / index）
- 是否还在维护（看 latest commit / CHANGELOG）

### Step B: 横向矩阵（核心交付物）

输出一个大矩阵：行 = 8 决策点，列 = 7 项目，单元格 = 该项目对该决策的实现方式 + grep 证据片段 + 一句话评价。

例：
```
| 决策点 | overstory | ccswarm | tamux | ... |
| --- | --- | --- | --- | --- |
| SoT 持久化 | SQLite via `bun:sqlite`（[ref] candidates/overstory/src/db.ts:12 `import { Database } from 'bun:sqlite'`）→ 强类型但 bun-only | 不持久化，全内存（grep 在 src/state.rs 找不到 sqlx/rusqlite）→ 不适用本项目需求 | ... | ... |
```

### Step C: Build-vs-Fork-vs-借用 决策（per 决策点）

对每个决策点说：
- 哪些项目的实现可以**直接 fork / 集成**进 ccbd-rust（连同 git history / license check）
- 哪些项目的实现可以**借鉴思路但 rewrite**（设计良好但 stack 不匹配）
- 哪些项目**没有可借的**（要全自研）

每个决策都要有 3 选 1 的明确表态 + 引用证据。

### Step D: 最终 Verdict

综合 Round 2 提到的剩余 A/B 类缺口（包括 by-gemini.md 引用 hallucinate，你自己决定要不要算 blocker），给最终判决：

1. **"能推进顶层设计定稿"** — 还需补啥（如有 minor）一起列
2. **"缺 A 类 X 不能推进"** — 说明缺什么
3. **"可推进但 B 类风险需 acknowledge"** — 列风险

### Step E: Open Asks

下一步主控应该做什么。

## 协作铁律（不变 + 新增）

1. **每条结论必须有 grep / Read / Glob 真命中的证据**——不接受"我相信"、"看起来"、"通常"
2. **line number 必须从工具输出粘贴**（grep -n / cat -n / Read 工具显示的）—— 不允许凭印象估计
3. **不恭维**——客观挑刺各项目的局限 + DESIGN.md v1 的盲点
4. **读不到就说**——不绕，不 hallucinate
5. **专注**：本轮只做 candidates 深度对比，**不重写 by-gemini.md**（那是另一轮的事）
6. 中文，markdown 章节结构

## 时间预算

允许耗时——不要为了快而省略 grep 步骤。如果 7 个项目实在读不完，**优先深读 overstory + ccswarm + tamux**（这三个 Round 2 你提到最相关），其他四个可以稍浅。
