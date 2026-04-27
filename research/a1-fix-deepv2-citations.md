# 任务：修 by-gemini-deep-v2.md 的"类 1 第 N 行"引用错位（mechanical 收尾）

## 问题

`research/findings/by-gemini-deep-v2.md` 里 Gemini 写了几处"基于类 1 第 N 行 codex --json / 类 1 第 8-9 行 PreToolUse Hook"等引用，但 codex-evidence-pack.md 类 1 表格里这些条目**位置对不上**——Gemini 凭印象编了行号。

但 evidence pack 类 1 表格里**确实有**这些条目（codex --json / PreToolUse Hook / stream-json 等）。

## 任务

1. Read `research/findings/by-gemini-deep-v2.md` 找出所有"类 1 第 N 行"引用
2. 对每条引用：
   - 提取关键词（如 "codex --json" / "PreToolUse Hook" / "stream-json" 等）
   - 在 `research/findings/codex-evidence-pack.md` 类 1 表格段（## 类 1 ... ## 类 2 之间）grep 真实命中行号
   - 用 Edit 工具把 by-gemini-deep-v2.md 的错行号改成真行号
3. 如果某关键词在类 1 找不到，把引用改成"见 codex-evidence-pack.md 类 1（位置：<手动定位>）"，或删除该引用
4. 完成后 ls + wc + 简洁 summary

## 工作流（grep-then-cite）

每条修改必须先 grep 真命中：
```bash
grep -n "codex --json\|PreToolUse" research/findings/codex-evidence-pack.md | head -5
```
然后 Edit by-gemini-deep-v2.md。

## 铁律

- **不改 by-gemini-deep-v2.md 的分析内容**（只动行号引用）
- 每条修改有 grep 输出作为证据
- 完成回复一行：`修了 N 处类 1 引用，文件大小变化 X→Y bytes`
