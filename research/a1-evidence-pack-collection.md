# 任务：基于完整知识库收集 ccbd-rust 设计需要的真实证据（纯机械整理）

## 这是什么任务

**纯严谨操作 / 机械整理 / 真证据收集**。

**不要做"分析"、"判断"、"独立见解"、"推荐"、"verdict"** —— 这些是 Gemini 的活。下一轮我会拿你的产出派 Gemini 做分析。

你这一轮**只做**：grep + Read + Glob 收集真证据，每条带 file:line + 原文片段，整理成结构化对照表。

## 输出文件

`research/findings/codex-evidence-pack.md`（新文件，不覆盖任何已有）

## 三大类机械产出

### 类 1：CCB 实际行为 vs 三家 agent 文档预期 对照表

工作流：
1. 通读 `docs/agent-cli-knowledge-base/` 下三家文档（codex / claude-code / gemini-cli）
2. 找出文档明确规定的 CLI 行为（slash commands 协议 / paste vs keystroke / hooks 触发 / session resume / sandbox 边界 / auth 流 / completion / 等）
3. 对每条规定，去以下三处 grep CCB 实际怎么做：
   - `~/.local/share/codex-dual/lib/`（CCB Python 源码）
   - `~/coding/claude_code_bridge/lib/`（fork 仓库源码，可能更全）
   - `research/sessions/`（master Claude 18 天用 CCB 的实际行为记录）
4. 输出对照表，每行：
   ```
   | 维度 | 文档说 X | CCB 实际做 Y | 偏差 |
   | --- | --- | --- | --- |
   | <CLI 行为> | <docs 文件路径:line> + 原文片段 | <ccb 源码或 sessions 文件路径:line> + 原文片段 | <偏离哪一条标准> |
   ```

只做"找偏差 + 列证据"，不做"为什么偏 / 应该怎么改"——那是 Gemini 的活。

### 类 2：18 天 corpus 痛点 finding 清单（每条真 grep 命中）

工作流：
1. 通读 `research/sessions/home-sevenx/markdown/`（虽然被 .gitignore 但物理可 grep / cat / Read，用绝对路径）
2. 通读 `research/sessions/agent-harness/markdown/`
3. 用 grep -rn 抽取每个独立 bug / 用户痛点 / 设计缺陷 / 用户纠正
4. 每条 finding 写：
   ```
   ### F-XX <一句话描述>
   - **类别**: A1 数据一致性 / A2 并发竞争 / A3 沙盒边界 / A4 协议不对齐 / A5 lifecycle / A6 观测性
   - **引用**: `<file:line>`
   - **原文**: > <grep 输出粘贴原文片段，不要概括>
   ```

只做"抽 + 分类 + 引用"，不做"为什么这是问题 / 怎么修"——Gemini 的活。

### 类 3：7 候选项目可借用代码清单（每条真源码 file:line）

工作流：
对 `research/candidates/` 下每个项目（tamux / overstory / batty / ccswarm / cli-agent-orchestrator / metaswarm / agent-orchestrator）：
1. Glob 关键源码文件（*.rs / *.ts / *.py 等）
2. 找出三类代码段：
   - **可直接 fork**：项目内某个 module / function 接近 ccbd-rust 需求，可直接搬（带源码 file:line + 一句"它做什么"）
   - **借鉴思路重写**：设计良好但 stack 不匹配（file:line + 一句"它的什么思路有用"）
   - **应避免反例**：用了 anti-pattern（file:line + 一句"哪里不该学"）

只做"列代码 + 一句描述功能"，不做"应不应该用 / 评价它的优劣 / 推荐"——Gemini 的活。

## 工作流（严格）

1. 每条 finding / 对照行 / 代码段 必须 grep / Read 真命中后才写入文件
2. 每条带原文片段（不只是 line number，要 paste 真原文）
3. 每写一条立刻用 Edit 工具加到 codex-evidence-pack.md（不要积压到最后）
4. 完成后用 `ls -la` + `wc -l/wc -c` 实证 + 输出 self-summary

## sessions 目录读取注意

`/home/sevenx/coding/ccbd-rust/research/sessions/` 被 .gitignore 排除，**ReadFolder 工具会过滤为空**。绕过方法：用 Shell `find` / `grep -rn` / `cat <绝对路径>` 直接读，文件物理可读。例：
```
grep -rn "delivering" research/sessions/home-sevenx/markdown/
```
可以正常工作。

## 完成后回复（人话简洁，一段）

```
=== Evidence pack 收集完成 ===
- 类 1 对照表: N 行
- 类 2 痛点 finding: K 条（按类别分布: A1=a, A2=b, A3=c, A4=d, A5=e, A6=f）
- 类 3 候选项目可借用代码:
  - tamux: x 段 fork / y 段借鉴 / z 段反例
  - overstory: ...
  - ...
- 总文件: <wc -l> 行 / <wc -c> 字节
- ls 验证: <ls -la 输出>
```

## 铁律（违任一项作废）

1. **只做事实整理**——不写"我认为 / 这意味着 / 应该 / 推论 / verdict / 建议"
2. **每条带真原文片段**（grep 输出原文，不只 line number）
3. **类 1 / 类 2 / 类 3 必须各自完整**——不能只做一两类就报完成
4. **不要修 by-gemini.md / DESIGN.md**——这次只新增 codex-evidence-pack.md
5. **不要做分析**——你这轮的产出是 Gemini 下一轮的输入，越机械越好

## 不要做

- 不要写 verdict / 设计建议 / 推荐
- 不要做"复杂归纳"
- 不要"为了让产出更有价值"加分析评论——硬性禁止
