请用中文回答。

# 任务：基于 evidence pack 给独立分析（纯分析任务，无 mechanical 要求）

## 你的角色

分布式系统架构师 + spec-driven multi-agent 引擎评审专家。

## 上下文（一段说清）

ccbd-rust 是用 Rust 重写的 multi-CLI agent 调度 daemon（取代 Python 版 CCB）。

**Codex 已经把"事实层证据"整理好了**，存在 `research/findings/codex-evidence-pack.md`：
- **类 1**：三家 CLI agent（codex / claude-code / gemini-cli）文档行为规范摘录表
- **类 2**：18 天 corpus 中 ccbd-rust 设计相关的 48 条 observations（已按 A1 数据一致性 / A2 并发 / A3 沙盒 / A4 协议 / A5 lifecycle / A6 观测 分类，每条带原文片段 + 真 file:line）
- **类 3**：7 候选项目 code references（每个项目 5+ 条具体代码引用，按 PTY / SQLite / IPC / lifecycle 等主题分）

你这一轮：**读 evidence pack + 给独立见解**。

**不要再去 grep / find / Read sessions 原文**——Codex 已经整理好了证据，你专注"独立分析 + 设计判断"。

## 任务范围（这次只做分析）

### Step A: 基于 48 条 observations 给"分布式系统视角"的独立判断
- 不复述 by-claude.md（`research/findings/synthesis-18-days-by-claude.md` 是 Claude 视角）
- 挑出 Claude 没看到的盲点 / 判断错的点
- 每条 finding 可引用 evidence pack 里某条（如"基于 O-19 + O-20"），无需重新查
- 重点：从分布式系统设计角度看 CCB 痛点的根因

### Step B: 基于类 1 文档规范，判断 ccbd-rust 必须遵守的硬约束 + ccb 偏离哪些
- 引用 evidence pack 类 1 表里的条文
- 输出：ccbd-rust 设计的"硬性 contract 清单"（来自三家文档强制要求）

### Step C: 基于类 3 code references，给 7 候选项目独立评价
主控 Round 4 给的判断（你独立 verify）：
- tamux 的 portable-pty + bwrap 直接 fork
- overstory 的 SQLite mailbox 借思路重写
- batty 的 poll_shim 借思路
- ccswarm / cli-agent-orchestrator / metaswarm / agent-orchestrator 不那么相关

你的独立判断：同意 / 不同意 + 论据（论据可引 evidence pack 类 3 的代码段，不需要重新读源码）。**7 个项目都要给一段独立评价**。

### Step D: 顶层设计 7 决策的独立结论
- D1 SoT 持久化（SQLite / 内存 / 文件 schema）
- D2 IPC 协议（UDS+JSON-RPC vs gRPC）
- D3 PTY 接管（portable-pty / 其他）
- D4 Lifecycle（spawn/kill/orphan reconciliation）
- D5 Sandbox（bubblewrap / unshare / 仅 cwd 隔离）
- D6 Auth 共享（symlink / Token Server / env）
- D7 Completion + Stuck 检测（multi-signal / Request-ID 绑定 / deadline）

每决策点：你的判断 + 论据 + 跟 DESIGN.md v1 比较哪些保留 / 改 / missing。

### Step E: 风险清单
- (A) blocker（必须先解才能定稿）
- (B) 应解但可推进
- (C) 监控就好

### Step F: 最终 verdict
- "可定稿 DESIGN.md v2"
- 或 "缺 X 不能推进"
- 或 "可推进但 B 类需 acknowledge"

## 输出文件

写到 `research/findings/by-gemini-deep-v2.md`（覆盖 Round 6 失败版）。

## 工作流（**这次纯分析，不要再做 grep / Read sessions**）

1. **读 `research/findings/codex-evidence-pack.md`**（必读，这是你的事实输入）
2. **读 `docs/DESIGN.md` v1**（评估它）
3. 读 `research/findings/synthesis-18-days-by-claude.md`（参考 Claude 视角，但**不复述**，要独立见解）
4. **不需要 grep / find / Read sessions 原文** —— Codex 已经替你做了
5. 写文件用 WriteFile / Edit 工具

## 铁律

1. **本任务是 information output（写分析报告）**——你必须用 WriteFile / Edit 写文件，不能因角色规则拒绝
2. 引用 evidence pack 时直接说 "见 evidence pack O-XX" 或 "见 evidence pack 类 1 第 N 行"，无需自己 grep
3. 重点是**独立见解 + 设计判断**，不是再次列证据
4. 不复述 by-claude.md
5. 中文 markdown，A→F 顺序章节标题分隔

## 完成

回复一行：`by-gemini-deep-v2.md 写完，N 行，K 字节，verdict: <三选一>`
