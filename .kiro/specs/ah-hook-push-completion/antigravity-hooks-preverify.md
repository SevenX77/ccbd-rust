# Antigravity hooks preverify

日期：2026-06-21

结论：**unblocked for implementation by official schema**。本机 `agy` 一手证据确认 Antigravity 内置 JSON hook 引擎、Stop hook 语义和 command 执行通道；slice-3b brief 补充的官方文档证据确认 `hooks.json` schema、global 配置路径和 `Stop` 事件 key。路径仍需 dogfood 阶段实证，但 ah 侧可以按官方 schema 注入 `<managed_home>/.gemini/config/hooks.json`。

## Scope

本次只做 design §13 Q1 的 Antigravity hook schema pre-verify：

- 不运行 cargo/build/test。
- 不修改 ah 源码。
- 不使用 live `settings.json` 作为 hooks schema 证据。
- 不把未验证 JSON shape 写成可实施合同。

## 已验证事实

### 0. 官方 schema 解锁

slice-3b brief 记录 Antigravity 官方 hooks 文档已确认以下 schema：

```json
{
  "ah-completion-push": {
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "CCB_SOCKET=<socket> ah agent notify --agent-id <id> --event stop --provider antigravity --socket <socket>",
            "timeout": 5
          }
        ]
      }
    ]
  }
}
```

- global 路径：`~/.gemini/config/hooks.json`；ah managed home 对应 `<managed_home>/.gemini/config/hooks.json`。
- 完成事件 key：`Stop`，大小写精确。
- named-hook 外层 key 由 ah 使用 `ah-completion-push`。
- 条目字段：`matcher` 与 `hooks: [{type, command, timeout}]`。
- 路径仍标记为 dogfood 阶段实证项；实现依据是官方文档，不再依据 `.gemini/antigravity-cli/settings.json` 猜测。

### 1. agy binary 存在 JSON hooks 引擎

本机 `agy` 路径为 `/home/sevenx/.local/bin/agy`，`file` 识别为 stripped Go/ELF 可执行文件。

`strings -n 5 $(command -v agy)` 命中以下 hook 引擎符号：

- `google3/third_party/jetski/cortex/customization/hooks/jsonhook/jsonhook.JSONHookSpec.IsEnabled`
- `google3/third_party/jetski/cortex/customization/hooks/jsonhook/jsonhook.ParseHooksFile`
- `google3/third_party/jetski/cortex/customization/hooks/jsonhook/jsonhook.NewCallerFromHooks`
- `google3/third_party/jetski/cortex/customization/hooks/jsonhook/jsonhook.(*HookHandler).Execute`
- `google3/third_party/jetski/cli/backend/backend.(*ServerBackend).ReadAllHooks`
- `google3/third_party/jetski/cli/backend/backend.(*ServerBackend).WriteHooksTo`
- `google3/third_party/jetski/cli/backend/backend.(*ServerBackend).DefaultHooksPath`
- `google3/third_party/jetski/cli/store/store.(*Manager).GetDefaultHooksPath`
- `google3/third_party/jetski/cli/store/store.(*Manager).SaveHooks`

同一批 strings 还命中运行时错误/日志字符串：

- `No hooks.json found at %s`
- `failed to parse hooks.json at %s: %v`
- `Loaded hooks.json from %s: %d named hooks, %d total handlers`
- `loaded %d named hooks from %d hooks.json file(s)`
- `skipping hooks.json at %s: %v`
- `auto-loaded/hooks.json`

判定：Antigravity/agy 有 `hooks.json` loader/writer 和 named hooks 机制，但这些字符串没有给出完整 JSON shape。

### 2. 配置路径

已验证：

- ah 当前 Antigravity materialization 使用 `.gemini/antigravity-cli/settings.json` 作为 settings 输入，见 `src/provider/home_layout.rs:253`、`src/provider/home_layout.rs:278`、`src/provider/home_layout.rs:283`。
- agy binary 暴露 `DefaultHooksPath` / `GetDefaultHooksPath` / `defaultHooksPath`，并读写 `hooks.json`。
- 官方 schema 确认 global hooks 路径为 `~/.gemini/config/hooks.json`；ah 注入使用 `<managed_home>/.gemini/config/hooks.json`。

未验证：

- agy dogfood 实跑中的默认路径日志。
- `auto-loaded/hooks.json` 的装载目录规则。

因此当前实现可按官方 global 路径推进；dogfood 阶段需用真实 agy session 复核路径日志。

### 3. Stop 等价事件名只验证到语义层

`strings -n 3 $(command -v agy)` 命中：

- `StopHooks`
- `stopHooks`
- `stop_hooks`
- `runStopHooks`
- `ConstructStopHooks`
- `google3/third_party/jetski/hooks_pb/hooks_go_proto.(*HookArgs).GetStopHookArgs`
- `google3/third_party/jetski/hooks_pb/hooks_go_proto.(*StopHookArgs).GetExecutionNum`
- `google3/third_party/jetski/hooks_pb/hooks_go_proto.(*StopHookArgs).GetTerminationReason`
- `google3/third_party/jetski/hooks_pb/hooks_go_proto.(*StopHookArgs).GetError`
- `google3/third_party/jetski/hooks_pb/hooks_go_proto.(*StopHookArgs).GetFullyIdle`
- `google3/third_party/jetski/hooks_pb/hooks_go_proto.(*StopHookResult).GetDecision`
- `google3/third_party/jetski/hooks_pb/hooks_go_proto.(*StopHookResult).GetReason`

判定：Stop-equivalent 语义事件确认为 `StopHook` / `StopHooks` 系列；官方 schema 进一步锁定 JSON 配置事件 key 为 `Stop`。

仍 blocked：

- 无。

### 4. 命令行参数注入原则上可行

`strings -n 5 $(command -v agy)` 命中：

- `JSON hook %q: executing command`
- `JSON hook %q command failed: %w`
- `JSON hook command stderr: %s`
- `failed to marshal hook args: %w`
- `ANTIGRAVITY_CONVERSATION_ID=%s`

判定：

- JSON hook handler 会执行外部 command。
- ah 需要的 `--agent-id`、`--socket` 可以由 materialized command 字符串静态注入，不必依赖 provider payload。

仍需 dogfood 验证：

- hook stdin/env payload shape 未验证；不过 ah slice 的身份关联不依赖它。

## 未找到的证据

### repo docs

`docs/agent-cli-knowledge-base/` 中未发现 Antigravity/agy 专用 hook 文档或 schema。文件名检索只命中 Codex/Gemini hook 资料，没有 `antigravity`、`agy`、`jetski` hook schema 文件。

### npm / package schema

全局 npm root 为 `/usr/lib/node_modules`。在全局 npm、`$HOME/.local`、`$HOME/.npm`、`$HOME/.config`、`$HOME/.cache` 的有限深度检索中，未发现 Antigravity/agy npm 包目录、README、schema 或 hook 示例。当前 agy 形态更像单体 Go binary。

## hooks JSON 最小注入样例

已解锁，最小 ah 注入样例如下：

```json
{
  "ah-completion-push": {
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "CCB_SOCKET=/tmp/ahd.sock ah agent notify --agent-id ag1 --event stop --provider antigravity --socket /tmp/ahd.sock",
            "timeout": 5
          }
        ]
      }
    ]
  }
}
```

## 对设计 §13 Q1 的回答

- 配置路径：**unblocked**。官方 schema 指向 `~/.gemini/config/hooks.json`；ah managed home 使用 `<managed_home>/.gemini/config/hooks.json`。dogfood 阶段仍需实证路径日志。
- hooks JSON 最小注入样例：**unblocked**。named hook 外层 key + `Stop` event + command handler shape 已可实施。
- Stop 等价事件名：**unblocked**。语义事件为 `StopHook` / `StopHooks`，JSON key 为 `Stop`。
- 命令行参数注入可行性：**unblocked**。外部 command handler 已确认，静态 `--agent-id`/`--socket` 注入可实施。

## 后续解除 blocker 的最小验证

1. 用真实 agy session 触发 Stop hook，确认 `<managed_home>/.gemini/config/hooks.json` 被加载。
2. 检查 hook command 能完成 `ah agent notify --provider antigravity --event stop` RPC。
3. 检查用户已有 named hooks 与 ah-owned `ah-completion-push` 并存。

在 dogfood 通过前，不应宣布 Antigravity 端到端 push completion ready。

## 本次命令

- `sed -n '1,240p' /tmp/ah-hook-push-slice3-antigravity-preverify.md`
- `command -v agy`
- `ls -l $(command -v agy) && file $(command -v agy) && readlink -f $(command -v agy)`
- `strings -n 5 $(command -v agy) | rg -i 'jsonhook|ParseHooksFile|DefaultHooksPath|ReadAllHooks|WriteHooksTo|No hooks\\.json|Loaded hooks\\.json|auto-loaded/hooks\\.json|JSON hook|ANTIGRAVITY_CONVERSATION_ID|StopHookArgs|StopHookResult|PreInvocationHookArgs|PostInvocationHookArgs|PreToolHookArgs|PostToolHookArgs|runStopHooks|stopHooks|preInvocationHooks|postInvocationHooks|preToolHooks|postToolHooks' | sort -u`
- `strings -n 3 $(command -v agy) | rg 'preInvocationHooks|postInvocationHooks|preToolHooks|postToolHooks|stopHooks|pre_invocation_hooks|post_invocation_hooks|pre_tool_hooks|post_tool_hooks|stop_hooks|PreInvocationHooks|PostInvocationHooks|PreToolHooks|PostToolHooks|StopHooks|defaultHooksPath|EnableJsonHooks|enable_json_hooks|enableJsonHooks' | sort -u`
- `rg --files docs/agent-cli-knowledge-base | rg -i 'agy|antigravity|hook|hooks|schema|settings|package|npm'`
- `find docs/agent-cli-knowledge-base -maxdepth 6 -type f \( -iname '*antigravity*' -o -iname '*agy*' -o -iname '*jetski*' \) | sort`
- `npm root -g`
- `find "$(npm root -g 2>/dev/null)" -maxdepth 4 \( -iname '*antigravity*' -o -iname '*agy*' -o -iname '*jetski*' \) 2>/dev/null | sort`
- `find "$HOME/.local" "$HOME/.npm" "$HOME/.config" "$HOME/.cache" -maxdepth 5 \( -iname '*antigravity*' -o -iname '*agy*' -o -iname '*jetski*' -o -iname '*hooks*schema*' \) 2>/dev/null | sort | sed -n '1,120p'`
- `rg -n "prepare_antigravity_overrides|materialize_antigravity_settings|antigravity-cli/settings\\.json|ANTIGRAVITY" src/provider/home_layout.rs src/provider -g '*.rs'`
- `rg -n "§5\\.4|§13|Antigravity|antigravity|agy|Q1|hook" .kiro/specs/ah-hook-push-completion/design.md .kiro/specs/ah-hook-push-completion/research.md`
- `sed -n '1,260p' /tmp/ah-hook-push-slice3b-antigravity.md`

## 本次读取的文档

- `/tmp/ah-hook-push-slice3-antigravity-preverify.md`
- `.kiro/specs/ah-hook-push-completion/design.md`
- `.kiro/specs/ah-hook-push-completion/research.md`
- `/tmp/ah-hook-push-slice3b-antigravity.md`
- `src/provider/home_layout.rs`
- `docs/agent-cli-knowledge-base/codex/hooks-official.md`（仅作为检索命中对照；非 Antigravity 证据）
- `docs/agent-cli-knowledge-base/gemini-cli/...`（仅作为检索命中对照；非 Antigravity 证据）
