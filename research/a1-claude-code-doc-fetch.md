# 任务：抓 Claude Code CLI 官方文档全集（任务 2 续）

## 背景

之前主控派的 claude-code-guide subagent **撒谎了**——它 WebFetch 了内容但没 WriteFile 落盘，INVENTORY.md 标 ✅ 11 个 .md 但实际只有 3 个文件落地（cli-reference.md / INVENTORY.md / SETUP.md）。

你（Codex / a1）已经证明能真落盘——codex 抓取你已经写了 42 个 .md。这次接手 Claude Code 文档抓取。

## 任务

抓 Claude Code 官方文档全集到 `docs/agent-cli-knowledge-base/claude-code/`。

**保留** subagent 已经写好的 3 个文件（cli-reference.md / INVENTORY.md / SETUP.md），**重写** INVENTORY.md 反映真实状态，**新增**所有缺的 .md。

## 必须抓 + 真 WriteFile 落盘的（缺一不算完成）

1. **commands.md** — 所有 slash commands（/clear /resume /compact /memory /config /model /help /batch 等）每条精确语义
2. **hooks.md** — Hooks 系统（SessionStart / SessionEnd / Stop / PreToolUse / PostToolUse / UserPromptSubmit / PermissionRequest 等）触发时机 + 输入 JSON 格式 + 输出协议
3. **settings.md** — settings.json 全字段 schema（user / project / local 三层优先级）
4. **memory.md** — CLAUDE.md + auto-memory 系统 + context compaction
5. **checkpointing.md** — session 恢复 + 文件追踪 + --resume / --continue 协议
6. **skills.md** — Skill 系统 + frontmatter 参考 + bundled skills
7. **subagents.md** — Agent tool / subagent_type / isolation
8. **mcp.md** — Model Context Protocol 集成
9. **vs-code.md** — VS Code 扩展
10. **changelog.md** — 完整 release notes（2.1.78 - 2.1.119 + 任何更新版本）
11. **policies.md** — Anthropic Usage Policy（subagent 标"不在范围"是错的，文档明确要求抓）：
    - https://www.anthropic.com/legal/aup（Usage Policy）
    - https://www.anthropic.com/legal/commercial-terms
    - https://www.anthropic.com/legal/consumer-terms

## 工作流

**Step 1**：列 doc tree
- `curl -L https://docs.claude.com/en/docs/claude-code/ -o /tmp/cc-index.html` 或 `WebFetch`
- 提取所有 sub-page URL

**Step 2**：每个 sub-page 都做以下三步：
1. WebFetch 拿到 markdown 内容
2. **立刻 Write 到 `docs/agent-cli-knowledge-base/claude-code/<topic>.md`**（这步 subagent 跳过了，导致空文件）
3. 文件顶部加 front matter:
   ```
   ---
   source_url: <原 URL>
   fetched_at: 2026-04-26
   fetched_by: codex (a1 via ccb)
   ---
   ```

**Step 3**：抓 Anthropic policy 三个 page（usage policy / commercial terms / consumer terms），单独写 `policies.md`（合并三个），每段标 source_url。

**Step 4**：重写 `INVENTORY.md`：
- 真实列出每个 .md（用 `ls -la docs/agent-cli-knowledge-base/claude-code/*.md` 验证）
- 每条带文件大小（用 `wc -c`）
- 未抓 + 原因（404 / 沙盒 / 其他）

**Step 5**：完成后输出（人话简洁）：
```
=== Claude Code 文档真实抓完 ===
- 已落盘 N 个 .md，总大小 K 字节
- 全部用 ls + wc 验证过
- INVENTORY.md 路径：docs/agent-cli-knowledge-base/claude-code/INVENTORY.md
- 未抓的（明确列）：<list>
```

## 铁律

1. **每个 sub-page 必须 WriteFile**，不能 WebFetch 完就以为"抓完了"
2. **WriteFile 后立刻用 `ls -la` 验证文件真在** —— subagent 撒谎不能再发生
3. **每文件加 source_url front matter**
4. **Anthropic Policy 必抓** —— 这关系到 ccbd-rust 不能违反什么红线
5. **Changelog 必抓全** —— 行为变更对 ccbd-rust 设计关键
6. **完成 summary 必须包含 ls + wc 输出作为证据**，不接受"我已抓"这种 claim

## 不要做

- 不要修改 docs/agent-cli-knowledge-base/codex/ 或 gemini-cli/（codex 你已抓完，gemini-cli 是下一份任务）
- 不要碰 docs/DESIGN.md
- 不要做评估 / 设计建议
