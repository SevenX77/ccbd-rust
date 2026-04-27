请用中文回答。

# Round 6：基于完整知识库的深度独立分析

你（Gemini）作为 Analyst 角色，本轮做一次**真正的深度通读 + 独立结论**。

## 为什么有这一轮

之前的 `research/findings/session-analysis-2026-04-26-by-gemini.md` （151 行）是浅层产出——你 Round 3 只读了 `home-sevenx-2026-04-26.md` 开头 500 行加几个 grep 就写了，引用 line numbers 还是 hallucinate 编的（Round 5 由 Codex 修过，但 finding 内容没改）。

主控之前判断"调研完成"是**错的**——by-gemini.md 不是真正的 Gemini 视角分析。本轮重做，**这次做真的**。

## 你这一轮的优势

知识库齐了：
- 三家 CLI 官方文档（90 个 .md, 5 MB 纯文本）—— 之前没有，靠 LLM 训练数据猜
- 7 候选项目源码已 cp 到 cwd 子树（之前沙盒读不到 /tmp/）
- 18 天 session corpus 在 `research/sessions/`（注：主控临时把 .gitignore 的 sessions 行注释了，你能 ReadFolder 进去）

## 通读全部以下材料（不许跳读）

### A. 18 天主控痛点 + 决策史
- `research/sessions/home-sevenx/markdown/`（home-sevenx master 18 天 session）
- `research/sessions/agent-harness/markdown/`（agent-harness master 同期）
- `research/findings/synthesis-18-days-by-claude.md`（Claude 视角综述，参考但你**要独立见解，不复述**）
- `research/findings/session-analysis-2026-04-26-by-claude.md`（Claude 当天分析）
- `research/findings/session-analysis-2026-04-26-by-gemini.md`（你 Round 3 写的浅层版，作反面教材，要超越它）

### B. 三家 CLI agent 官方文档（这次新加的事实层 —— 极其重要）
- `docs/agent-cli-knowledge-base/codex/`（44 个 .md，含 changelog 5719 行 / agents-md / auth / sandboxing / sessions / pricing / models / security 全集）
- `docs/agent-cli-knowledge-base/claude-code/`（14 个 .md，含 hooks 2564 行 / settings / commands / mcp / policies 1.2 MB）
- `docs/agent-cli-knowledge-base/gemini-cli/`（32 顶层 + 子目录共 117 个 .md，含 paste-vs-keystroke / slash-commands / sandboxing / tools / policies 等你自己的文档）

### C. 7 候选项目源码（research/candidates/）
- `tamux/` Rust 多 crate (含 amux-daemon)
- `overstory/` TypeScript（Bun + SQLite mailbox）
- `batty/` Rust（poll_shim health monitoring）
- `ccswarm/` Rust（worktree + Linux Namespace）
- `cli-agent-orchestrator/`、`metaswarm/`、`agent-orchestrator/` 也要看

主控 Round 4 给的判断（你独立 verify 是否对）：
- tamux 的 portable-pty + bwrap 直接 fork
- overstory 的 SQLite mailbox 借思路重写
- batty 的 poll_shim 借 health check 思路
- SoT schema 自研（tamux 过度绑定 plugin 系统）

### D. 上游 ccb bug 报告
- `docs/upstream-ccb-bugs/installer-default-config-mismatch.md`
- `docs/upstream-ccb-bugs/gemini-dispatch-and-completion-bugs.md`（三个 bug X/Y/Z，paste/completion/autonew）

### E. 现有 ccbd-rust 设计稿
- `docs/DESIGN.md` v1（Phase 2 启动文档）

## 任务产出（**写到 `research/findings/by-gemini-deep-v2.md`**，不覆盖 v1）

按下面 7 个章节做深度独立分析：

### Step A：18 天痛点重新整理（独立 Gemini 视角）
- **不复述 by-claude.md**——找 Claude 没看到的盲点 / 判断错的点
- 每条 finding 有 grep 真命中的 file:line 引用
- 按分布式系统视角分类：(A1) 数据一致性 / (A2) 并发竞争 / (A3) 沙盒边界 / (A4) 协议不对齐 / (A5) lifecycle / (A6) 观测性

### Step B：基于官方文档的事实校对（这章 by-claude 没有，是你独有的视角）
- 三家 agent 官方文档 vs CCB 实际处理 → 找**对不上的地方**
- 例：CCB 用 `paste-buffer -p` 投 slash command，但 `gemini-cli/paste-vs-keystroke.md` 文档明确说什么 → 文档里查到，CCB 假设错在哪
- 输出"文档预期 vs CCB 实现"对照表，每行带文档 file:line 引用 + CCB 行为 file:line 引用（来自 sessions 或 ccb 源码 grep）
- ccbd-rust 重写时哪些文档明文规范必须遵守

### Step C：7 候选项目深度评审（每个独立一段）
- 这个项目真正解决什么（不是 README 自吹）
- ccbd-rust 可直接借用的代码段（带源码 file:line）
- ccbd-rust 不该学的反例
- 跟主控 Round 4 判断对照——同意 / 不同意 + 论据
- 必须读真源码，不停留 README

### Step D：ccbd-rust 顶层设计 7 决策（独立结论）
对每个决策点：你的判断 + 论据 + 跟 DESIGN.md v1 比较
- D1. SoT 持久化（SQLite schema 草案）
- D2. IPC 协议（UDS+JSON-RPC vs gRPC，结合 18 天 corpus 看哪种性能 / 调试更适合）
- D3. PTY 接管（tamux portable-pty fork / 还是其他）
- D4. Lifecycle（spawn/kill/orphan reconciliation 状态机）
- D5. Sandbox（bubblewrap vs unshare，结合 4-26 .bashrc 事故 + Anthropic/OpenAI/Google policy 的 sandbox 要求）
- D6. Auth 共享 + 状态隔离矩阵（基于三家 agent 的 auth 文档定）
- D7. Completion + Stuck 检测（multi-signal + Request-ID，结合 gemini paste-vs-keystroke + ccb dispatch bug）

### Step E：上游 CCB bug 三联清单 verify
- 对 Bug X / Y / Z 独立验证（用三家文档 + ccb 源码 grep）
- 同意 / 不同意 + 是否有遗漏 bug

### Step F：风险清单（独立列）
按 (A) blocker 必须先解 / (B) 应解但可推进 / (C) 监控就好 分级

### Step G：最终 verdict
- 是否可定稿 DESIGN.md v2？
- 不可 → 缺什么具体补料
- 可 → v1 哪些保留 / 哪些改 / 哪些 missing

## 工作流（严格 grep-then-cite，避免 Round 3 翻车）

每条 finding 做 3 步：
1. **grep 真搜命中** —— 用 `grep -nrE '<keyword>' <path>` 命中证据
2. **粘贴 grep 输出 file:line 进 finding** —— 不能凭印象编 line number
3. **立刻 Write 到 by-gemini-deep-v2.md** —— 不要积压到最后才写

## 时间预算 + 主控监控

不限时（主控 in-loop 一直等），但：
- **每 5 分钟主控 capture pane 看进度**
- **保持持续产出**——Thinking 半小时不输出 = 异常，主控会介入
- 思路卡了立刻 grep / Read 找证据驱动，不要空想

## 铁律（违一项作废）

1. **grep-then-cite**：每条 line number 从 grep 输出粘贴
2. **不复述 by-claude.md / 之前 by-gemini.md**：要独立 Gemini 视角
3. **不空想**：grep / Read 驱动每个判断
4. **写 `by-gemini-deep-v2.md`**，不覆盖 v1（v1 留作对比反面材料）
5. **本任务是 information output（写分析报告），不是编码任务**——你必须用 WriteFile / Edit 工具写文件，不能因角色规则拒绝
6. 中文 markdown，A→G 顺序章节标题分隔
7. 每条 finding 必须有引用，不接受"我相信"/"通常"

完成后**回复一行**：
`by-gemini-deep-v2.md 写完，N 行，K 字节，verdict: <三选一>`
