请用中文回答。

# 任务 Round 5：修 by-gemini.md 的 line number 引用（A 类 #1 闭环）

Round 4 的 verdict 是"可推进顶层设计定稿，但需 A 类 #1 闭环"。A 类 #1 = `research/findings/session-analysis-2026-04-26-by-gemini.md` 的引用 line numbers 是 Round 3 hallucinate 的（已 spot-check 验证：12040 / 376 / 1748 都对不上原文）。

本轮任务：**逐条修复或删除** by-gemini.md 中所有引用，确保每条 finding 的 file:line 都是真 grep 命中的。

## 工作流（严格遵守）

1. 读 `research/findings/session-analysis-2026-04-26-by-gemini.md`
2. 找出文件里所有形如 `research/sessions/<path>.md:<line>` 的引用（用 grep 自己的 markdown content 找）
3. **对每条引用**：
   - 提取该 finding 的关键词（如 "job 卡 delivering"、"completion detector 旧探针"、"Janitor 单调时钟"、"Shotgun Surgery"、"35/100 评分" 等）
   - 用 `grep -nrE "<关键词>" research/sessions/` 真搜
   - 如果有命中 → 用命中的 file:line 替换原引用（保持 finding 原文，只改 line number）
   - 如果**完全找不到** → **删除该 finding**（因为 hallucinate 的内容不能保留）
4. 最后用 Write/Edit 工具更新 by-gemini.md

## 铁律

- 所有 line number 必须从 `grep -n` 实际输出粘贴
- **不允许凭印象估计 line number**（这是 Round 3 翻车的原因）
- **找不到真实证据的 finding 必须删除**（不要改成模糊引用蒙混过关）
- 完成后**回复一行**：`by-gemini.md 引用修复完成，<改动数> 处修改，<删除数> 处删除，最终行数 <N>`

## 不要做

- 不重写章节结构（B-01 / B-02 等编号保留，只动引用）
- 不添加新 finding
- 不做评估 / verdict
