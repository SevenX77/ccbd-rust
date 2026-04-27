# 任务：修复 by-gemini.md 中所有 hallucinate 的 line number 引用

文件：`research/findings/session-analysis-2026-04-26-by-gemini.md`

## 背景（仅 1 段，不长）

这份文件是 Gemini 上一轮（Round 3）写的，里面每条 finding 都标了 `**证据引用**：research/sessions/<file>.md:<line>` 格式的引用，但 spot-check 显示**这些 line numbers 是 hallucinate 编的**：

- B-01 引 `2026-04-22-session.md:12040`，实际 line 12040 是 settings.json schema 字段，跟"job 卡 delivering"无关
- B-02 引 `2026-04-26-session.md:376`，实际是 cleanup-orphans 调试输出
- B-03 引 `2026-04-25-session.md:1748`，实际是 BridgeRuntimeState struct
- 其他没 spot-check 但很可能同样问题

## 你的任务

逐条修复或删除引用，确保每条 finding 的 file:line 是**真实 grep 命中**的。

## 工作流（严格按此走）

**步骤 1：读 by-gemini.md**

```bash
cat research/findings/session-analysis-2026-04-26-by-gemini.md
```

**步骤 2：找出所有引用**

`grep -nE '\*\*证据引用\*\*' research/findings/session-analysis-2026-04-26-by-gemini.md`

**步骤 3：对每条引用执行**

a. 从 finding 描述提取关键词（如 "delivering"、"探针"、"READY"、"单调时钟"、"shotgun"、"35/100" 等）

b. grep 真命中：

```bash
grep -nrE '<关键词>' research/sessions/ | head -10
```

c. **判定**：
   - **命中** → 用 Edit 工具把原 finding 里的错 line number 替换成 grep 输出的真 line number。引用格式必须明确：`research/sessions/home-sevenx/markdown/2026-04-22-session.md:6789` （冒号 + 真实数字）
   - **没命中**（grep 完全找不到对应原文） → **删除该 finding 整段**（包括标题 + 技术细节 + 证据引用 + Gemini 深度分析），不要保留 hallucinate 内容

d. 修改后立刻 grep 验证：`grep -n '<新 line number>' <session.md 文件>` 应该返回该行

**步骤 4：每改一条都用 Edit / Write，不要 batch 攒到最后**

（小步前进比一次写大改稳，避免 Edit 工具拒绝）

**步骤 5：完成后输出**

输出**严格按此格式**：

```
=== Round 5 完成 ===
修改：N 处
删除：K 处
最终行数：M 行

每处修改的 grep 证据（每条一行）：
- B-01: 旧 2026-04-22-session.md:12040 → 新 <file>:<真行号>
  grep 命中：<grep -n 输出片段>
- B-02: ...
- ...
```

## 铁律

1. **每个 line number 必须从 grep -n 实际输出粘贴**——不能凭印象、不能估算
2. **找不到证据的 finding 必须删除**——不要改成"模糊引用"蒙混
3. **不重写章节结构**（B-01 / B-02 编号保留），只动引用 line number
4. **不添加新 finding**
5. **不做评估 / verdict**——这次只做 mechanical fix

## 不要做

- 不要联网 fetch
- 不要修 by-claude.md / synthesis-18-days.md（只动 by-gemini.md）
- 不要碰 docs/DESIGN.md
- 不要在最终输出里说"我相信"、"应该是"、"大概"——所有判断都基于 grep 输出
