# 任务：抓 gemini-cli 官方文档全集（任务 2 收尾）

## 背景

a2 (Gemini) 引用 GEMINI.md 里的 role 规则拒绝执行 git clone / WriteFile（视为编码任务），所以 gemini-cli 文档抓取改派给你 (Codex)。你不受这条约束。

## 任务

抓 gemini-cli 全部官方文档 verbatim copy 落盘到 `docs/agent-cli-knowledge-base/gemini-cli/`。

## 工作流

**Step 1：克隆官方仓库**
```bash
git clone --depth=1 https://github.com/google-gemini/gemini-cli docs/agent-cli-knowledge-base/gemini-cli/_source/
```

**Step 2：从仓库提取 + 落盘**
- README.md → `gemini-cli/overview.md`
- 仓库内 `docs/`（如有）→ 全部 cp 到 `gemini-cli/`
- `packages/cli/` 里的 README → `gemini-cli/cli-reference.md`
- 任何 SLASH-COMMANDS / HOOKS / SETTINGS 类文档 → 对应 .md

**Step 3：抓网络源**
- WebFetch / curl `https://ai.google.dev/gemini-api/terms` → `gemini-cli/policies.md`
- WebFetch `https://policies.google.com/terms` 相关部分 → 合并到 policies.md
- WebFetch 任何 README 链到的 doc page

**Step 4：必须 100% 覆盖（缺一不算完成）**

1. **cli-reference.md** — `gemini --help` 全部 + 所有 flags
2. **slash-commands.md** — `/clear` / `/new` / `/compress` / `/auth` / `/help` 等所有 slash 命令的精确语义
3. **at-file-syntax.md** — `@<file>` 引用怎么解析 + 沙盒边界 + 多文件 @ref
4. **tools.md** — 内置工具集（Read / Glob / Grep / Shell / WebFetch / WriteFile）每个的参数 + 错误模式
5. **sandboxing.md** — workspace 边界规则 + .gitignore 是否被尊重 + 怎么 bypass
6. **gemini-md-protocol.md** — GEMINI.md 文件协议
7. **conversation-management.md** — `/clear` 真正做什么 + auto compress 触发 + `/compress`
8. **paste-vs-keystroke.md** — 这跟 ccb 的"slash via paste 不识别" bug 直接相关。文档怎么说 paste 行为？
9. **shell-mode.md** — `!` 触发 shell 模式
10. **auth-and-quotas.md** — OAuth / API key / Plan 等级 / 限额
11. **policies.md** — Google AI / Generative AI usage policy
12. **changelog.md** — 0.39.x 版本行为变更（ccbd-rust 关键）

**Step 5：每文件 front matter**
```
---
source: <URL 或仓库内路径>
fetched_at: 2026-04-26
fetched_by: codex (a1 via ccb)
---
```

**Step 6：写 INVENTORY.md** 真实列出 + 每条带 wc 验证

**Step 7：完成 summary（带 ls + wc 证据）**

## 铁律

1. 每个文件必须真 WriteFile + ls 验证（subagent 撒谎不能重演）
2. paste-vs-keystroke / shell-mode / changelog 三项必抓 —— ccbd-rust 设计直接依赖
3. 仓库 clone 完后留 `_source/` 不删除（作为追溯源）

## 不要做

- 不修 codex / claude-code 目录
- 不评估 / 设计建议
- 这次纯 information retrieval
