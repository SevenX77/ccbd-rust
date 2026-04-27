# 任务：抓 codex CLI 官方文档全集（任务 2）

## 背景

ccbd-rust 项目（Rust 重写 CCB daemon）需要 100% 了解 codex CLI 的官方信息作为知识库。这份信息会决定 ccbd-rust 怎么 spawn / 投递 / 探测 codex 这个 L1 agent。靠主观判断或 LLM 训练知识不够——**必须以官方文档为准**。

## 你（codex / a1）的优势

你**就是 codex CLI 自己**——抓自己的官方文档最权威。你比谁都清楚：
- codex 的 release / 分发渠道
- AGENTS.md 协议的官方定义
- session resume 协议
- 内置工具集
- usage policy

## 任务范围

把所有官方文档抓下来，verbatim copy 落盘到 `docs/agent-cli-knowledge-base/codex/`。

要 100% 覆盖以下知识点（缺一项都不算完成）：

1. **CLI 命令行参数全表**（`codex --help` 全部分支）
2. **AGENTS.md 协议**（每个项目根放的 AGENTS.md 文件 codex 怎么读 / 哪些字段有意义 / 优先级）
3. **session resume 协议**（`--resume <id>` 字节级行为：找哪个文件 / 找不到怎么办 / session 文件位置 / 格式）
4. **config 文件**（位置 / 字段 / 默认值 / 优先级链）
5. **slash commands 完整清单**（如 `/new` / `/clear` / `/resume` 等，各自语义）
6. **内置工具集**（codex 提供的 Read/Edit/Bash 等工具的边界 / 沙盒规则）
7. **usage policy**（OpenAI 的硬限：能做什么 / 不能做什么 / 触发限流的行为）
8. **quota / rate limit**（free tier vs Plus / Pro / 自定义模型的限额）
9. **changelog / release notes**（行为变更——这是 ccbd-rust 设计要 anti-fragile 抗的关键）
10. **auth 流程**（OAuth / API key / 凭证文件位置）
11. **模型选择**（gpt-5.5 / 5.5-codex / 模型名变动）
12. **stdin/stdout 协议**（如有 JSON streaming / event format）

## 工作流（严格）

**Step 1：发现官方源**

```bash
which codex
codex --version
codex --help
codex --help | grep -iE "doc|reference|policy|http"  # 找官方 URL 提示
```

如果 codex 自带 internal docs，列出来。如果有 GitHub repo，用 `git clone --depth=1 <url>` 浅拷到 `docs/agent-cli-knowledge-base/codex/_source/`。

**Step 2：列文档 inventory**

把找到的所有官方文档 URL / 路径列在 `docs/agent-cli-knowledge-base/codex/INVENTORY.md`：
```
- [ ] https://platform.openai.com/docs/...
- [ ] https://github.com/openai/codex/...
- [ ] /home/sevenx/.local/.../codex-help-output.txt
- [ ] ...
```

**Step 3：逐个抓**

- 网络源用 `curl -L <url> -o tmp.html`，用 `pandoc tmp.html -o output.md` 或类似工具转 markdown
- GitHub 仓库内已是 markdown 直接 `cp`
- 命令输出（如 `codex --help`）直接写文件
- 每个文件顶部加 front matter：
  ```
  ---
  source: <URL or path>
  fetched_at: 2026-04-26
  fetched_by: codex (a1 via ccb)
  ---
  ```

**Step 4：组织目录**

```
docs/agent-cli-knowledge-base/codex/
├── INVENTORY.md              ← 列已抓 / 未抓 / 缺口
├── overview.md
├── installation.md
├── cli-reference.md           ← codex --help 全文
├── slash-commands.md
├── agents-md-protocol.md
├── session-resume.md
├── config.md
├── auth.md
├── policies-and-limits.md
├── changelog.md
├── tools.md
└── _source/                   ← git clone 的原仓库（如有）
```

**Step 5：完成报告**

完成后输出（一段，简洁）：

```
=== codex 文档抓取完成 ===
- 已抓 N 份 .md，总 K 字
- _source/ 大小 M MB（如适用）
- INVENTORY.md 路径：docs/agent-cli-knowledge-base/codex/INVENTORY.md
- 未抓的（沙盒拒/paywall/其他）：
  - <list>
- 已知缺口或风险：
  - <list>
```

## 铁律

1. **verbatim copy**——不要二次概括 / 总结 / 改写
2. **每文件加 front matter** source + fetched_at
3. **不能漏 changelog**——行为变更是 ccbd-rust 设计要预防的关键
4. **不能漏 policy**——usage policy 决定 ccbd-rust 不能做什么
5. **网络抓不到的**（沙盒 block / 网络故障）必须在 INVENTORY.md 明确写"未抓 + 原因"，不要静默跳过
6. **不要拉跟 codex 无关的文档**（比如 OpenAI 的 ChatGPT / GPT API 文档跟 codex CLI 没关系，不抓）

## 不要做

- 不要修改 docs/agent-cli-knowledge-base/claude-code/ 或 gemini-cli/（那是别人的活）
- 不要碰 docs/DESIGN.md
- 不要做评估 / verdict / 设计建议（这次只是 information retrieval）
