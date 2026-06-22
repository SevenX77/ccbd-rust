# step-9 hook-push 未 fire root-cause findings

范围: read-only research。未修改 `src/` 或 `tests/`。结论按证据强度区分: 代码/本机 CLI/官方文档为 High, 二进制 strings 为 Medium, 由零 RPC 反推为 Medium/Low。

## 总结判断

不是一个单一根因。三家共享一个脆弱点: `src/provider/home_layout.rs:540-549` 的 `build_ah_hook_command` 为所有 provider 生成同一条裸 command:

```text
CCB_SOCKET={socket} ah agent notify --agent-id ... --event stop --provider ... --socket {socket}
```

但实际失败路径分化:

- Codex: 已实证根因是 Stop hook stdout 契约不兼容 + hook trust 未预置。`ah agent notify` 成功后向 stdout 打三行纯文本, Codex Stop 要求 exit 0 时 stdout 为空或合法 JSON, 纯文本会被判 invalid。
- Claude: 当前注入形态理论可执行, 因为 Claude command hook 是 shell command, exit 0 会解析 stdout JSON；纯文本通常不是 fatal。journald 零 RPC 更像 hook 未配置/未加载/未 fire/执行失败未被收集, 需要带 `--debug-file` 复测。
- Antigravity: `CCB_SOCKET=...` env-prefix 在 Gemini/Antigravity 类 hook runner 下不是 argv[0] 问题。Gemini bundle 明确通过 shell executable + command 执行；agy 二进制 strings 显示 Go `jsonhook.(*HookHandler).Execute` + `os/exec.CommandContext`, 但官方文档描述为 shell commands。更可疑的是 Antigravity hook stdout/timeout/错误输出观测不足, 以及当前 ah 写入 timeout 单位为 `5`, 而 Gemini docs 该字段为毫秒。

最小修复方向:

1. 新增 hook 专用 notify 输出模式, 不直接改变人类 CLI 默认输出。推荐 `ah agent notify --hook-json` 或 `--quiet-json`:
   - 成功时 stdout 输出 `{}` 或 `{"continue":true}`。
   - RPC 错误走非 0 + stderr, 保留 daemon 可观测性。
2. `build_ah_hook_command` 改成 provider-aware 或统一 wrapper:
   - Codex: `ah agent notify ... --hook-json`。
   - Claude/Gemini/Antigravity: 同样用 JSON/空 stdout, 避免 JSON parser/日志污染差异。
   - 命令参数做 shell quote, 或改为小脚本 wrapper, 避免 socket/agent_id 特殊字符破坏 shell。
3. Codex 注入改 `[features].hooks = true` 或不写 feature flag；`codex_hooks` 仅作为 deprecated alias, 不应继续新增。
4. Codex worker spawn 增加 `--dangerously-bypass-hook-trust`, 或预写匹配当前 generated hook 的 `[hooks.state."<path>:stop:0:0"].trusted_hash`。前者最小, 后者需要复刻 Codex hash 算法, 风险更高。
5. Antigravity/Claude 复测必须打开 provider hook debug/log, 并把 hook stderr/stdout 落到 ah 自己控制的 debug 文件, 不再只依赖 provider UI。

## Codex

### 根因

1. Stop hook stdout 契约与 `ah agent notify` 输出不兼容。

证据:

- `src/bin/ah.rs:378-405` 的 `cmd_agent_notify` RPC 成功后 `println!` 三行纯文本: `agent_id=...`, `event=...`, `transitioned=...`。
- OpenAI Codex Hooks 官方文档说明 Stop 在 exit 0 且 stdout 有内容时要求 JSON: `Stop expects JSON on stdout when it exits 0. Plain text output is invalid for this event.` 文档还说明 exit 0 无输出视为成功, Common output fields 支持 `continue`, `stopReason`, `systemMessage`, `suppressOutput`。来源: https://developers.openai.com/codex/hooks
- 本机 `codex --version` 为 `codex-cli 0.135.0`; `/tmp/a1-step9-fix-research.md` 记录的 pane 实证错误正是 `hook returned invalid stop hook JSON output`。

2. feature flag 使用旧名。

证据:

- `src/provider/home_layout.rs:838-850` 的 `enable_codex_hooks` 写入 `[features].codex_hooks = true`。
- Codex 官方文档说明 hooks 默认开启；关闭用 `[features].hooks = false`; `hooks` 是 canonical feature key, `codex_hooks` 仍可用但为 deprecated alias。来源: https://developers.openai.com/codex/hooks
- 本机当前 Codex 配置也存在旧字段: `/home/sevenx/.cache/ah/sandboxes/8114d3cdc736/.codex/config.toml:1-2` 为 `[features] codex_hooks = true`。

3. hook trust 未非交互预置。

证据:

- `codex --help` 显示 `--dangerously-bypass-hook-trust`: 对已启用 hooks 跳过 persisted hook trust, 用于外部已 vet 的自动化。
- Codex 官方文档说明非 managed command hooks 必须 review/trust; trust 记录当前 hook hash; 可通过 `/hooks` 审核; 一次性自动化可传 `--dangerously-bypass-hook-trust`。
- 本机当前 config 已出现 Codex 信任状态格式: `[hooks.state."/home/.../.codex/hooks.json:stop:0:0"] trusted_hash = "sha256:..."`。
- `src/provider/manifest.rs:344-355` 的 Codex spawn args 当前没有 `--dangerously-bypass-hook-trust`。

4. ah idle 检测误判 trust 弹窗为 IDLE。

证据:

- Codex marker regex 在 `src/marker/matcher.rs:89-95` 是 `(?m)^\s*›(?:\s|$)`。
- Codex manifest anti-pattern 在 `src/provider/manifest.rs:342-365` 只有 `esc to interrupt`。
- FIFO reader 在 `src/agent_io/reader.rs:151-183` 只要 marker 匹配就调用 `mark_agent_idle_matched`。
- `mark_agent_idle_matched` 在 `src/db/state_machine.rs:316-388` 对 active state 执行 `IDLE/Matched`; 它只在有 completion log monitor 时延后 marker, 没有识别 `Hooks need review` 等阻塞 UI。

推荐修复:

- `ah agent notify --hook-json` 输出 `{}` 或 `{"continue":true}`。证据度 High, 影响 High, 置信 A。
- `enable_codex_hooks` 改写 `[features].hooks = true` 或停止写 enable flag。证据度 High, 影响 Medium, 置信 A。
- Codex spawn 加 `--dangerously-bypass-hook-trust`。证据度 High, 影响 High, 置信 A。若产品上不接受危险 flag, 改为生成 `trusted_hash`, 但需要先复刻 Codex hash 算法。
- Codex idle anti-pattern 加 `Hooks need review|Trust all and continue|Continue without trusting`，并考虑在 active hook review UI 时转 `PROMPT_PENDING` 而非 `IDLE`。证据度 High, 影响 Medium, 置信 A。

## Antigravity

### 根因

1. `CCB_SOCKET=... ah ...` 不是主要根因, env-prefix 在 shell command 语义下应可用。

证据:

- ah 注入位置和形态: `src/provider/home_layout.rs:286-326` 写 `<managed home>/.gemini/config/hooks.json`, 外层 named hook `ah-completion-push`, 事件 `Stop`, 内层 `type=command`, `command=<build_ah_hook_command>`, `timeout=5`。
- Antigravity 当前 manifest 使用 `agy --dangerously-skip-permissions`: `src/provider/manifest.rs:403-418`。
- 本机 `agy --version` 为 `1.0.10`。
- `strings /home/sevenx/.local/bin/agy` 显示 Go hook 实现符号 `google3/.../jsonhook.(*HookHandler).Execute`、`jsonhook.NewCallerFromHooks`、`os/exec.CommandContext`。这证明是内置 jsonhook runner, 但 strings 不能证明具体 argv。
- Google Antigravity hooks 官方文档搜索结果描述 hooks 可运行 custom scripts or shell commands。来源: https://antigravity.google/docs/hooks
- Gemini CLI 同源 bundle 的 hook runner 可作为强旁证: `/home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/chunk-W2MTI7FK.js:326442-326450` 通过 `spawn(shellConfig.executable, [...shellConfig.argsPrefix, command], { shell:false })` 执行, 即显式 shell executable + command 字符串, 不是直接 execve `CCB_SOCKET=...`。

2. 更可疑问题 A: hook stdout/JSON 处理与 ah 纯文本输出不一致。

证据:

- Gemini docs: Most hooks support stdout JSON fields; command 字段是 shell command；hook stdout 可能被解析为 JSON 或转成 systemMessage。`reference.md:62-75`, `index.md:124-131`。
- Gemini bundle: `chunk-W2MTI7FK.js:326513-326531` 会尝试 `JSON.parse(stdout.trim() || stderr.trim())`, 失败后 `convertPlainTextToHookOutput`。
- Antigravity `json_hook_caller` 日志只记录 executing command, 不记 result；journald 零 RPC 说明命令要么未实际执行到 `ah`, 要么执行失败/超时, 但当前证据无法区分。

3. 更可疑问题 B: timeout 单位可能错误。

证据:

- ah 写 `timeout: 5`: `src/provider/home_layout.rs:318-325`。
- Gemini docs 写 timeout 为 milliseconds, default 60000: `docs/hooks/index.md:124-131`。
- 如果 agy 沿用 Gemini jsonhook schema, `timeout=5` 是 5ms, 足以让 `ah agent notify` 尚未连接 Unix socket 就被杀。这个假设能解释 journald 零 RPC。证据度 Medium, 因为 Antigravity 官方 docs 页面当前无法用本工具取到具体行号, 但 agy 与 Gemini jsonhook 符号/日志高度同源。

推荐修复:

- 不把 env-prefix 作为主修复；仍建议 command 改 wrapper/quote, 但不是这次零 RPC 的强根因。证据度 Medium, 影响 Medium, 置信 B。
- Antigravity hook command 改为 JSON/静默输出, 并把 timeout 改成 provider-aware: Antigravity/Gemini 用毫秒 `5000`, Codex/Claude 用秒制各自字段。证据度 Medium, 影响 High, 置信 B。
- 在 hook command 后追加 ah 自有 debug redirect, 例如 wrapper 内写 `/tmp/ah-hook-<agent>.log` 或 state_dir 下 debug 文件, 捕获 argv/env/stdout/stderr/exit。证据度 High, 影响 High, 置信 A。

## Claude

### 根因

当前证据不能证明 Claude Stop hook fire 过；更准确结论是“配置理论上可执行, 但本轮没有观测到 RPC, 需要 debug-file 复验”。

证据:

- ah 注入 Claude Stop: `src/provider/home_layout.rs:171-194` 在 hook_push active 时 push `materialized_ah_hook(ctx, "Stop")`。
- Claude settings 注入: `src/provider/home_layout.rs:669-694` 写 `settings.json`; `src/provider/home_layout.rs:697-725` 写 `hooks.Stop[].hooks[].command`。
- Claude manifest: `src/provider/manifest.rs:386-400` 使用 `claude --dangerously-skip-permissions`。
- Claude 官方 docs: command hooks 是 `type: "command"` shell command; handlers run in current directory with Claude Code environment; exit 0 parses stdout JSON, stdout 必须只有 JSON object 才用于 structured control, 但 JSON 只在 exit 0 处理。来源: https://code.claude.com/docs/en/hooks
- `claude --help` 本机版本 `2.1.185`; 支持 `--debug-file <path>`、`--debug [filter]` 和 `--include-hook-events`。

为何零 RPC:

- 不是 env-prefix 直接 execve 问题: Claude 官方文档称 command hook 是 shell command, 当前 `CCB_SOCKET=... ah ...` shell 语义成立。
- 不是“Claude 必然要求 stdout JSON 导致命令不执行”: stdout JSON 只影响 hook 结果处理; `ah agent notify` 先 RPC 再打印 stdout, 即使 stdout 不合规也应先让 journald 看到 RPC。既然 journald 零 RPC, 更可能是 hook 未加载/未匹配/未 fire/命令路径或环境失败/timeout。
- 当前 ah 未把 Claude hook stderr/stdout/exit 收集到 ahd, pane 又太短, 所以缺第一手失败原因。

推荐修复:

- 统一改 `ah agent notify --hook-json` 仍然有价值, 防止 Claude structured-output 污染和跨厂商差异。证据度 Medium, 影响 Medium, 置信 B。
- 复测 Claude 时用 `claude --debug-file <state_dir>/claude-hook-debug.log --debug hooks` 或最接近的 debug filter, 并确认 `~/.claude/settings.json` 中 Stop hook 实际存在。证据度 High, 影响 High, 置信 A。
- 若仍无 RPC, 优先排查 managed home/`CLAUDE_CONFIG_DIR` 是否被 Claude 2.1.185 正确读取, 以及 command 中 `ah` 是否在 sanitized PATH。证据度 Medium, 影响 High, 置信 B。

## ahd / completion 路径确认

- `src/rpc/handlers/agent.rs:520-575` 的 `handle_agent_notify` 只接受 `event=stop`, 校验 provider, 调 `mark_agent_idle_hook_event`, changes>0 后 cancel log monitor + wake orchestrator。
- hook event 成功落库: `src/db/state_machine.rs:534-582` 写 `sub_state='HookEvent'`, event payload `source="hook"`, `reply_source` 为 `hook` 或 `screen`。
- log fallback: `src/completion/log_layout.rs:50-54` 只支持 `codex` 和 `claude`, 其他 provider 返回 `unsupported_provider`; 这解释 Antigravity fallback 为 UI-only。
- log event 成功落库: `src/db/state_machine.rs:650-700` 写 `sub_state='LogEvent'`, event payload 带 `reason`, `raw_path`, `provider_turn_id`。
- orchestrator dispatch 前启动 log monitor: `src/orchestrator/mod.rs:916-970`; log monitor active 时 marker fallback 会被 `src/db/state_machine.rs:327-338` 延后。

## 读过的文件 / 文档

- `/tmp/a1-step9-fix-research.md`
- `src/provider/home_layout.rs`
- `src/bin/ah.rs`
- `src/rpc/handlers/agent.rs`
- `src/completion/log_layout.rs`
- `src/completion/monitor.rs`
- `src/db/state_machine.rs`
- `src/orchestrator/mod.rs`
- `src/agent_io/reader.rs`
- `src/marker/matcher.rs`
- `src/marker/timer.rs`
- `src/provider/manifest.rs`
- `/home/sevenx/.cache/ah/sandboxes/8114d3cdc736/.codex/config.toml`
- `/home/sevenx/.cache/ah/sandboxes/8114d3cdc736/.codex/hooks.json`
- `/home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/docs/hooks/index.md`
- `/home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/docs/hooks/reference.md`
- `/home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/chunk-W2MTI7FK.js`
- OpenAI Codex Hooks: https://developers.openai.com/codex/hooks
- Claude Code Hooks reference: https://code.claude.com/docs/en/hooks
- Google Antigravity Hooks: https://antigravity.google/docs/hooks

## 跑过的命令 / grep

```text
pwd && ls
cat /tmp/a1-step9-fix-research.md
rg --files .kiro src tests 2>/dev/null | head -200
rg -n "build_ah_hook_command|enable_codex_hooks|materialize_.*hooks|merge_codex_hook_push|agent notify|Notify|notify|unsupported_provider|hook_push|log-signal|log_signal|completion" src/provider/home_layout.rs src src/bin Cargo.toml
rg -n "codex_hooks|\\[features\\]|hooks|trusted|Stop|hook" ~/.codex ~/.config 2>/dev/null | head -300
command -v codex; codex --version; codex --help | sed -n '1,220p'
command -v claude; claude --version; claude --help | sed -n '1,220p'
nl -ba src/provider/home_layout.rs | sed -n '170,330p'
nl -ba src/provider/home_layout.rs | sed -n '530,575p'
nl -ba src/provider/home_layout.rs | sed -n '785,905p'
nl -ba src/bin/ah.rs | sed -n '160,260p'; nl -ba src/bin/ah.rs | sed -n '378,410p'
nl -ba src/rpc/handlers/agent.rs | sed -n '100,132p'; nl -ba src/rpc/handlers/agent.rs | sed -n '520,575p'
nl -ba src/completion/log_layout.rs | sed -n '1,115p'
nl -ba src/completion/monitor.rs | sed -n '1,130p'
nl -ba src/db/state_machine.rs | sed -n '500,540p'
nl -ba src/db/state_machine.rs | sed -n '620,650p'
npm root -g
readlink -f /home/sevenx/.npm-global/bin/codex
rg -n "invalid stop hook JSON|Stop hook|stop hook|trusted_hash|codex_hooks|bypass-hook-trust|Hooks need review|dangerously-bypass-hook-trust|features.*hooks|features.*codex_hooks|hook returned invalid" /home/sevenx/.npm-global /usr/lib/node_modules 2>/dev/null
find /home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli -maxdepth 4 -type f \\( -name '*hook*' -o -name '*.md' \\)
cat /home/sevenx/.cache/ah/sandboxes/8114d3cdc736/.codex/config.toml
cat /home/sevenx/.cache/ah/sandboxes/8114d3cdc736/.codex/hooks.json
nl -ba src/provider/home_layout.rs | sed -n '575,700p'
nl -ba src/provider/home_layout.rs | sed -n '330,445p'
nl -ba src/db/state_machine.rs | sed -n '534,615p'
nl -ba src/db/state_machine.rs | sed -n '650,730p'
nl -ba src/orchestrator/mod.rs | sed -n '912,970p'
rg -n "Hooks need review|Hook|hook|idle|IDLE|PROMPT_PENDING|mark.*idle|Matched|LogEvent|pane" src/monitor src/orchestrator src/tmux src/pane_diff src/db
nl -ba src/orchestrator/mod.rs | sed -n '60,150p'
nl -ba src/pane_diff/mod.rs | sed -n '1,115p'
nl -ba src/pane_diff/mod.rs | sed -n '157,212p'
nl -ba src/marker/matcher.rs | sed -n '1,220p'
nl -ba src/marker/registry.rs | sed -n '1,220p'
rg -n "mark_agent_idle|STATE_IDLE|PROMPT_PENDING|marker|Matched|state = 'IDLE'|sub_state = 'Matched'|sub_state.*Matched" src/db src/marker src/agent_io src/orchestrator
nl -ba src/agent_io/reader.rs | sed -n '1,205p'
nl -ba src/marker/timer.rs | sed -n '190,235p'
nl -ba src/db/state_machine.rs | sed -n '289,390p'
nl -ba src/provider/manifest.rs | sed -n '1,170p'
rg -n "provider_name: \\\"codex\\\"|idle_anti_pattern|stability_ms|IdleDetectionMode" src/provider/manifest.rs
nl -ba /home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/docs/hooks/index.md | sed -n '124,160p'
nl -ba /home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/docs/hooks/reference.md | sed -n '23,75p'
nl -ba /home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/docs/hooks/reference.md | sed -n '143,180p'
rg -n "spawn\\(|execFile|exec\\(|shell:|child_process|execa|Command" /home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/*.js
command -v agy; agy --version; agy --help | sed -n '1,120p'
strings /home/sevenx/.local/bin/agy | rg -n "json_hook_caller|JSON hook|executing command|hooks.json|exec|shell|/bin/sh|sh -c|Stop_0_0|hook_event_name|hookEventName"
strings /home/sevenx/.npm-global/lib/node_modules/@openai/codex/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin/codex | rg -n "invalid stop hook JSON|trusted_hash|codex_hooks|dangerously-bypass-hook-trust|Hooks need review|features.*hooks|hook returned invalid|Stop hook"
codex features
nl -ba /home/sevenx/.npm-global/lib/node_modules/@google/gemini-cli/bundle/chunk-W2MTI7FK.js | sed -n '326430,326590p'
git status --short
```

