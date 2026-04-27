# 任务：为 ccbd-rust 项目整理 design reference materials

## 项目背景

ccbd-rust 是用 Rust 重写的多 CLI agent 调度 daemon（取代 Python 版 CCB）。我们需要把已有的参考材料整理成结构化引用文档，方便后续设计阶段查找。

**这是纯整理任务，不做评价 / 推荐 / 判断。**（分析下一轮派 Gemini 做）

## 输出文件

`research/findings/codex-evidence-pack.md`（新建，每条 grep 真命中后立刻 Edit 写入）

## 三类机械整理

### 类 1：三家 CLI agent 文档行为规范摘录

通读 `docs/agent-cli-knowledge-base/` 下三家官方文档（codex / claude-code / gemini-cli），按以下维度抽取规范条文：

- slash commands 各命令的精确语义
- paste / keystroke / submit 的官方行为说明
- hooks 触发机制 + 输入输出协议
- session resume / continue 协议
- workspace / sandbox 边界规则
- auth 流程 + 凭证文件位置
- completion / 输出流协议

输出格式：
```
| 维度 | Provider | 文档原文 | docs file:line |
| --- | --- | --- | --- |
| <slash /clear> | gemini-cli | "..." (粘贴原文片段) | gemini-cli/slash-commands.md:N |
```

### 类 2：18 天 corpus design observations 清单

通读 `research/sessions/home-sevenx/markdown/` + `research/sessions/agent-harness/markdown/` 全部 markdown，抽取以下三类 observation：

- **用户多次强调的设计需求**（用户原话 + file:line）
- **重复出现的运行 pattern**（如 mailbox 状态变化 / 进程生命周期 / 用户工作流）
- **设计决策的演进**（哪一天做了什么决定）

每条：
```
### O-XX <一句话描述>
- **类别**: A1 数据一致性 / A2 并发 / A3 沙盒 / A4 协议 / A5 lifecycle / A6 观测
- **引用**: `<file:line>`
- **原文**: > <grep 输出粘贴原文片段>
```

注意：sessions 目录被 .gitignore 排除，**ReadFolder 工具会过滤为空**。绕过：用 Shell `find` / `grep -rn` / `cat <绝对路径>` 直接读文件，物理可读。例：
```
grep -rn "delivering" research/sessions/home-sevenx/markdown/
```

### 类 3：7 候选项目 code reference 索引

对 `research/candidates/` 下每个项目（tamux / overstory / batty / ccswarm / cli-agent-orchestrator / metaswarm / agent-orchestrator）：

- 列项目内关键的 module / function（带 file:line + 一句功能说明）
- 按主题分类（PTY 处理 / SQLite 存储 / IPC / lifecycle / sandbox / health monitoring 等）

格式：
```
### tamux
**PTY 处理**:
- `crates/amux-daemon/src/pty_session.rs:N` — 用 portable-pty 封装 PTY 创建
- `crates/amux-daemon/src/sandbox.rs:N` — Bubblewrap 封装

**SQLite 存储**:
- `crates/amux-daemon/src/plugin/persistence.rs:N` — 用 rusqlite 持久化

(7 个项目都要这样列)
```

## 工作流（严格）

1. 每条记录 grep / Read 真命中后写入（不能凭印象编 line number）
2. 每条带原文片段（不只 line number，要 paste 真原文）
3. 每写一批立刻 Edit 文件（不积压到最后）
4. 完成后用 `ls -la` + `wc -l/wc -c` 实证 + 输出 self-summary

## 完成后回复（人话简洁）

```
=== Reference materials 整理完成 ===
- 类 1 文档规范摘录: N 行（codex K + claude-code L + gemini-cli M 维度）
- 类 2 corpus observations: K 条
- 类 3 候选项目 code references:
  - tamux: x 条
  - overstory: y 条
  - ...
- 总: <wc -l> 行 / <wc -c> 字节
- ls -la 验证: <输出>
```

## 铁律（这轮纯整理）

1. 只做事实摘录 + 真引用——不写"我觉得 / 这意味着 / 应该 / 推荐 / verdict / 建议"
2. 每条带真原文片段（grep 输出原文）
3. 类 1 / 类 2 / 类 3 必须各自完整
4. **不要修 by-gemini.md / DESIGN.md** —— 只新增 codex-evidence-pack.md
5. **不要做评价 / 判断 / 分析** —— 这是 Gemini 下一轮的活
