请用中文回答。

# 任务：抓 gemini-cli 官方文档全集（任务 2）

## 背景

ccbd-rust 项目（Rust 重写 CCB daemon）要 100% 了解 gemini-cli 作为知识库。靠主观判断不够——**必须以官方文档为准**。

## 你（Gemini / a2）的优势

你**就是 gemini-cli 自己**——抓自己最权威。

## 任务范围

把所有 gemini-cli 官方文档 verbatim copy 落盘到 `docs/agent-cli-knowledge-base/gemini-cli/`。

要 100% 覆盖：

1. **CLI 命令行参数全表**（`gemini --help` 全部分支）
2. **slash commands 完整清单**（`/clear` / `/new` / `/compress` / `/auth` / `/help` 等，每条的精确语义）
3. **`@<file>` 引用语法**（怎么解析 / 沙盒边界 / 多文件 @ref 行为）
4. **内置工具集**（Read / Glob / Grep / Shell / WebFetch / WriteFile 等，每个的参数 / 返回 / 错误模式）
5. **Sandbox / workspace 边界规则**（哪些路径能读 / `.gitignore` 是否被尊重 / 怎么 bypass）
6. **`GEMINI.md` 文件协议**（项目根的 GEMINI.md 怎么被读 / 字段定义 / 优先级）
7. **conversation 管理**（`/clear` 真正做什么 / context window / 自动 compress 触发条件 / `/compress` 手动触发）
8. **Auth & Quota**（OAuth Google / API key / Plan 等级 / Free tier vs Code Assist / 限额）
9. **Google AI / Generative AI usage policy**（硬限红线）
10. **changelog / release notes**（特别注意 0.39.x 版本的行为变更）
11. **shell mode**（pane 显示 `!` 触发的 shell 模式怎么工作 / 怎么进入 / 怎么退出）
12. **paste vs keystroke 行为**（这跟 ccb 的"slash command via paste 不识别" bug 直接相关）

## 工作流

**Step 1：克隆官方仓库**

```
git clone --depth=1 https://github.com/google-gemini/gemini-cli docs/agent-cli-knowledge-base/gemini-cli/_source/
```

**Step 2：列 inventory**

读 `_source/README.md` + `_source/docs/`（如有）+ `_source/packages/`，列所有官方 .md 到 `docs/agent-cli-knowledge-base/gemini-cli/INVENTORY.md`。

**Step 3：抓网络源**

WebFetch 以下（如可访问）：
- https://ai.google.dev/gemini-api/terms（Google AI Terms）
- https://ai.google.dev/gemini-api/docs（如有 gemini-cli 相关）
- https://policies.google.com/terms（Google 通用 ToS，相关部分）
- 任何 `_source/README.md` 里链接到的 doc page

**Step 4：组织目录**

```
docs/agent-cli-knowledge-base/gemini-cli/
├── INVENTORY.md
├── overview.md
├── installation.md
├── cli-reference.md
├── slash-commands.md           ← 重点：/clear /new /compress /auth 各自语义
├── at-file-syntax.md
├── tools.md
├── sandboxing.md
├── gemini-md-protocol.md
├── conversation-management.md  ← /clear 与 /compress 区别
├── auth-and-quotas.md
├── policies-and-limits.md
├── shell-mode.md
├── paste-vs-keystroke.md       ← 关键，与 ccb bug 直接相关
├── changelog.md
└── _source/                    ← clone 的官方仓库
```

每个文件顶部 front matter：
```
---
source: <URL 或仓库内路径>
fetched_at: 2026-04-26
fetched_by: gemini (a2 via ccb)
---
```

**Step 5：完成报告**

输出一段（简洁）：
```
=== gemini-cli 文档抓取完成 ===
- 已抓 N 份 .md
- _source/ 大小 K MB
- INVENTORY.md：docs/agent-cli-knowledge-base/gemini-cli/INVENTORY.md
- 未抓 + 原因：<list>
- 缺口风险：<list>
```

## 铁律

1. **verbatim copy** —— 不二次概括
2. **不能漏 paste-vs-keystroke 这部分** —— 这跟 ccb 投递 bug 直接相关
3. **不能漏 changelog** —— 0.39.x 行为变更对 ccbd-rust 关键
4. **不能漏 policy** —— Google AI Terms 决定 ccbd-rust 红线
5. 网络/仓库抓不到的明确写在 INVENTORY.md，不静默跳过

## 不要做

- 不修改其他 agent 的文档目录
- 不写设计建议 / verdict
- 这次纯 information retrieval，不思考、不分析
