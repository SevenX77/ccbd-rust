# a1 antigravity hook-push research

## 结论

选 A: 可修。

根因不是 `Stop` 事件名、PATH、mount、timeout、命令拆分或 stdin/stdout 协议。当前 ah 只写了 Antigravity 的 `hooks.json`，但没有把 Antigravity JSON hooks 的 feature gate 写进 CLI 读取的 config/settings 文件。`agy` 当前版本的二进制里同时存在:

- `CustomizationConfig.GetEnableJsonHooks`
- `config.applyFeatureProviderJSONHooksConfig`
- `json-hooks-enabled`
- `skipping hooks.json`
- `Loaded hooks.json`

这说明 `hooks.json` 解析/调度路径和真正 JSON hook enablement 之间有显式 gate。MassGen 对同一个 `agy` 后端的 live 集成也给出同一结论: Antigravity native hooks 使用 standalone `hooks.json`，并且 `settings.json` 必须设置 `enableJsonHooks: true`; 没有该 gate 时 agy 会跳过 hooks。

ah 现状只创建/复制 `.gemini/antigravity-cli/settings.json`，只写 `.gemini/config/hooks.json`，没有写 `.gemini/config/config.json` 或 `.gemini/config/settings.json` 的 `enableJsonHooks`。这正好解释 live log 的:

```text
config.json not found or unreadable, using CLI settings only: open <HOME>/.gemini/config/config.json: no such file or directory
```

以及后续只看到 `"executing command"`，但没有任何外部进程副作用。我的判断是当前 `agy` 会进入 JSON hook caller/logging 路径，但真正外部 command executor 被 customization config 的 `enableJsonHooks` gate 静默短路，或落到不 spawn 的兼容/stub 分支。

## 二进制证据

`/home/sevenx/.local/bin/agy` 是 stripped Go ELF，但保留 Go pclntab 字符串。解析 pclntab 后拿到关键函数:

- `jsonhook.(*Caller).CallHook`
- `jsonhook.(*HookHandler).Execute`
- `jsonhook.JSONHookSpec.IsEnabled`
- `config.applyFeatureProviderJSONHooksConfig`
- `os/exec.CommandContext`
- `os/exec.(*Cmd).Run`
- `os/exec.(*Cmd).Start`

`HookHandler.Execute` 区域确实有 hook handler 执行逻辑；二进制也确实链接了 `os/exec.CommandContext` / `Cmd.Run` / `Cmd.Start`，所以不是“物理上不支持 spawn”。但 `enable_json_hooks/json=enableJsonHooks`、`GetEnableJsonHooks`、`json-hooks-enabled` 和 `skipping hooks.json` 同时存在，证明 command exec 不是无条件走到底。

## Web 调研证据

- 官方 Antigravity hooks 文档搜索摘要: hooks 可在 Antigravity execution loop 的特定点运行 custom scripts 或 shell commands，配置在 customization directory 的 `hooks.json`。
- Google GitHub issue #49: agy CLI 实际触发路径是 `~/.gemini/config/hooks.json`，不是 `~/.gemini/antigravity-cli/hooks.json`。
- Google DevRel 文章: Antigravity 读取 workspace `.agents/hooks.json` 或全局 `~/.gemini/config/hooks.json`; hook 输入通过 stdin JSON，脚本 stdout 输出 `allow` / `deny` / `ask`。
- MassGen v0.1.89 release: Antigravity hooks emit standalone `hooks.json`; `settings.json` enables hooks through `enableJsonHooks`。
- MassGen source: `_write_workspace_settings_json(has_hooks=True)` 写 `settings["enableJsonHooks"] = True`; 注释写明无此 gate 时 agy log 是 `skipping hooks.json`，有此 gate 才拾取 hooks。

## ah 端修复方案

最小改 `src/provider/home_layout.rs`，仍保持 sandbox HOME 内写配置，不碰 live agent 配置。

目标:

1. `AntigravityHomeLayout` 增加 `config_path: PathBuf`，指向 `home_root/.gemini/config/config.json`。
2. `prepare_antigravity_overrides` 在 hook_push 激活时，除 `materialize_antigravity_hooks` 外，再写 gate。
3. 新增 `materialize_antigravity_json_hooks_gate(layout)`:
   - 读 `.gemini/config/config.json`，不存在则 `{}`。
   - 设置顶层 `"enableJsonHooks": true`。
   - 可选兼容: 同步写 `.gemini/config/settings.json` 的 `"enableJsonHooks": true`，因为 MassGen 用的是 settings.json，而本机 agy log 明确找 config.json。为了覆盖版本差异，建议两者都写。

建议 diff 形状:

```diff
diff --git a/src/provider/home_layout.rs b/src/provider/home_layout.rs
@@
     if let Some(ctx) = active_hook_push_ctx(hook_push_ctx, "antigravity") {
         materialize_antigravity_hooks(source_home, &layout, ctx)?;
+        materialize_antigravity_json_hooks_gate(&layout)?;
     }
@@
+fn materialize_antigravity_json_hooks_gate(layout: &AntigravityHomeLayout) -> Result<(), CcbdError> {
+    for path in [&layout.config_path, &layout.config_settings_path] {
+        let mut root = read_json_object(path).unwrap_or_default();
+        root.insert("enableJsonHooks".to_string(), Value::Bool(true));
+        write_json_object(path, &root)?;
+    }
+    Ok(())
+}
@@
 struct AntigravityHomeLayout {
     antigravity_dir: PathBuf,
     cache_dir: PathBuf,
     settings_path: PathBuf,
+    config_path: PathBuf,
+    config_settings_path: PathBuf,
     hooks_path: PathBuf,
     onboarding_path: PathBuf,
 }
@@
             settings_path: antigravity_dir.join("settings.json"),
+            config_path: config_dir.join("config.json"),
+            config_settings_path: config_dir.join("settings.json"),
             hooks_path: config_dir.join("hooks.json"),
```

测试建议:

- 新增 unit test: antigravity hook_push 激活时，sandbox home 下同时存在 `.gemini/config/hooks.json` 和 `.gemini/config/config.json`，且 `config.json.enableJsonHooks == true`。
- 保留现有 hook JSON shape，不要改事件名。
- `cargo test` 全量跑。

## fallback 可靠性

a3 的 UI-pull fallback 当前仍可靠，可继续作为 Antigravity 的兜底和最终防线: 已有 live 实证显示 MARKER_MATCHED 能把 Antigravity 转 IDLE。修复 hook-push 后也不应移除 fallback，因为 Antigravity hook gate/协议仍可能随 agy 版本变化。

## 读过的文件

- `/tmp/a1-antigravity-hook-research.md`
- `src/provider/home_layout.rs`
- `docs/agent-cli-knowledge-base/gemini-cli/hooks/reference.md`
- `/home/sevenx/.local/bin/agy`

## 跑过的命令/查询

- `cat /tmp/a1-antigravity-hook-research.md`
- `rg -n "materialize_antigravity_hooks|inject_antigravity_hook_push|antigravity|hooks.json|config.json" src docs -S`
- `ls -l /home/sevenx/.local/bin/agy /home/sevenx/coding/ccbd-rust`
- `file /home/sevenx/.local/bin/agy`
- `strings /home/sevenx/.local/bin/agy | rg ...`
- `readelf -S /home/sevenx/.local/bin/agy`
- `readelf -sW /home/sevenx/.local/bin/agy`
- `grep -aob "executing command|json_hook_caller.go|enableJsonHooks|skipping hooks.json|Loaded hooks.json|json-hooks-enabled" /home/sevenx/.local/bin/agy`
- `objdump -d --no-show-raw-insn --start-address=... --stop-address=... /home/sevenx/.local/bin/agy`
- 自写只读 Python pclntab parser，解析 `HookHandler.Execute` / `Caller.CallHook` / `applyFeatureProviderJSONHooksConfig` / `os/exec.*` 地址。
- Web: `Google Antigravity CLI hooks agy hooks.json Stop hook command`
- Web: `Antigravity CLI agy hooks json hook command config.json enableJsonHooks`
- Web: `site:github.com/massgen/MassGen "enableJsonHooks"`

## 关键证据行

- `src/provider/home_layout.rs:276-278`: hook_push 只调用 `materialize_antigravity_hooks`。
- `src/provider/home_layout.rs:291`: source hook 路径是 `.gemini/config/hooks.json`。
- `src/provider/home_layout.rs:301-303`: 只写 `layout.hooks_path`。
- `src/provider/home_layout.rs:329-357`: 只维护 `.gemini/antigravity-cli/settings.json` 的 `trustedWorkspaces`。
- `src/provider/home_layout.rs:1502-1510`: Antigravity layout 只有 `antigravity-cli/settings.json` 和 `config/hooks.json`，没有 `config/config.json` gate。
- `strings agy`: `CustomizationConfig.GetEnableJsonHooks`、`config.applyFeatureProviderJSONHooksConfig`、`json-hooks-enabled`、`skipping hooks.json`、`Loaded hooks.json`。
- GitHub issue #49: agy 实际触发路径为 `~/.gemini/config/hooks.json`。
- MassGen source: `settings["enableJsonHooks"] = True`，注释说明无 gate 会 `skipping hooks.json`。
