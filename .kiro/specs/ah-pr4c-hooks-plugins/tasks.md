# ah PR4c Tasks: Hooks 与 Plugins 双侧物化

## §0. 数据校验声明

本任务清单只覆盖 PR4c：把 `ah.toml` 中声明的 hooks/plugins 从 CLI 配置、RPC 参数一路透传到 provider HOME 物化层，并按 Provider 官方配置形状写入 `settings.json` / `config.toml`。不实现外部 Git 下载；PR4d 再做 auto-provisioning。

已读：
- `.kiro/specs/ah-pr4c-hooks-plugins/design.md:1-120`，设计范围为 hooks/plugins 自动部署与激活。
- `.kiro/specs/ah-pr4c-hooks-plugins/design.md:27-33`，Provider 落点表：Claude/Gemini hooks、Claude/Codex plugins。
- `.kiro/specs/ah-pr4c-hooks-plugins/design.md:39-47`，PR4c 嵌入 `prepare_home_layout_with_role` 物化流水线。
- `.kiro/specs/ah-pr4c-hooks-plugins/design.md:55-60`，新增 `HookGroup` / `HookItem` 配置形状。
- `.kiro/specs/ah-pr4c-hooks-plugins/design.md:68-76`，3 个 tests-first 验收场景。
- `.kiro/specs/ah-pr4c-hooks-plugins/design.md:86-108`，Gemini scope、外部资产 provisioning、symlink/copy 风险判定。

已 grep / verify：
- `src/cli/config.rs:23-31` 现有 `MasterConfig` 只有 `cmd` / `enabled`。
- `src/cli/config.rs:56-61` 现有 `AgentConfig` 只有 `provider` / `env`。
- `src/cli/start.rs:90-111` `ah start` 只把 `extra_env_vars` 和 `sandbox_overrides` 传给 `agent.spawn`。
- `src/rpc/handlers.rs:209-236` `session.spawn_master_pane` 当前固定调用 `prepare_home_layout_with_role("claude", ...)`，没有扩展参数。
- `src/rpc/handlers.rs:312-350` `agent.spawn` 当前只解析 `extra_env_vars`，再调用 `prepare_home_layout(...)`。
- `src/provider/home_layout.rs:45-68` `prepare_home_layout_with_role` 是 Provider HOME 物化入口。
- `src/provider/home_layout.rs:263-287` `materialize_gemini_settings` 当前只补 auth。
- `src/provider/home_layout.rs:311-329` `materialize_claude_settings` 当前只补 bypass/permissions。
- `src/provider/home_layout.rs:332-353` `prepare_managed_codex_home` 当前只确保 `config.toml` 与 workspace trust。
- `src/provider/home_layout.rs:790-802` 已有 Codex home layout 单测锚点。
- `tests/mvp12_home_layout.rs:69-150` 已覆盖 Provider HOME 物化主路径，可扩展 PR4c 集成验收。
- `src/cli/rpc_client.rs:102-112` ccbd RPC socket 通过 `CCB_SOCKET` / state layout 解析，不是 `CCB_TMUX_SOCKET`。

## §1. Tasks 全景图

| Phase | 内容 | 核心 T 数 | 估算 | 依赖前置 |
| :--- | :--- | :--- | :--- | :--- |
| Phase 0 | 准备与 spec 锚定 | 2 | 0.5h | main 最新 |
| Phase 1 | tests-first 红灯 | 4 | 1.5h | Phase 0 |
| Phase 2 | HookGroup 类型定义 + ah.toml 解析扩展 | 4 | 1.5h | Phase 1 |
| Phase 3 | RPC `agent.spawn` / `session.spawn_master_pane` 参数透传 | 4 | 1.5h | Phase 2 |
| Phase 4 | `prepare_home_layout` 接口重构 + Claude/Gemini hooks 注入 | 5 | 2h | Phase 3 |
| Phase 5 | Codex/Claude plugins 激活 + 物化目录 | 4 | 1.5h | Phase 4 |
| Phase 6 | 全局回归 + ship | 5 | 1h | Phase 5 |

## §2. 详细 Tasks

### Phase 0: 准备

- [ ] **T0.1 切实施分支**
  - Files: 无。
  - 从最新 `main` 切 `feat/ah-pr4c-hooks-plugins`。
  - Acceptance Criteria:
    - `git status -sb` 显示当前分支为 `feat/ah-pr4c-hooks-plugins`。
    - 记录 main HEAD；不要基于未提交的 PR-1a 工作树实施。

- [ ] **T0.2 复核 design baseline 与源码锚点**
  - Files: `.kiro/specs/ah-pr4c-hooks-plugins/design.md`，`src/cli/config.rs`，`src/cli/start.rs`，`src/rpc/handlers.rs`，`src/provider/home_layout.rs`。
  - grep:
    - `rg -n "materialize_claude_settings|materialize_gemini_settings|prepare_home_layout_with_role|prepare_managed_codex_home" src/provider/home_layout.rs`
    - `rg -n "extra_env_vars|handle_session_spawn_master_pane|handle_agent_spawn" src/rpc/handlers.rs src/cli/start.rs`
  - Acceptance Criteria:
    - 实施记录使用实测 file:line，不沿用漂移行号；当前 `materialize_gemini_settings` 锚点是 `src/provider/home_layout.rs:263`。

### Phase 1: tests-first 红灯

- [ ] **T1.1 写 Claude Hook 自动化 failing test**
  - Files: `tests/pr4c_hooks_plugins.rs` 或扩展 `tests/mvp12_home_layout.rs`。
  - 场景：构造 `HookGroup { matcher: "*", hooks: [{ type: "command", command: <host script>, timeout: None }] }`，调用重构后的 Claude home materialization。
  - Acceptance Criteria:
    - sandbox `~/.claude/settings.json` 写入官方嵌套结构：`hooks.PreToolUse[0].matcher == "*"`，`hooks.PreToolUse[0].hooks[0].type == "command"`，`command` 指向 sandbox 内 `.claude/hooks/<name>.sh`。
    - `.claude/hooks/<name>.sh` 是指向宿主脚本的 symlink，脚本路径存在。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins claude_hook_materializes_settings_and_symlink -- --test-threads=1`

- [ ] **T1.2 写 Codex Plugin 激活 failing test**
  - Files: `tests/pr4c_hooks_plugins.rs` 或扩展 `tests/mvp12_home_layout.rs`。
  - 场景：插件完整 ID 为 `github@openai-curated`，宿主缓存存在 `.codex/plugins/cache/github@openai-curated/`。
  - Acceptance Criteria:
    - sandbox `~/.codex/config.toml` 含 `[plugins."github@openai-curated"] enabled = true`。
    - sandbox `.codex/plugins/cache/github@openai-curated` 是 symlink 或目录，指向/来自宿主缓存。
    - 使用完整 ID，不接受裸 `github` 自动猜测 marketplace。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins codex_plugin_materializes_config_and_cache -- --test-threads=1`

- [ ] **T1.3 写 Hook 协议连通性 failing test**
  - Files: `tests/pr4c_hooks_plugins.rs`。
  - 场景：生成一个 `PreToolUse` hook 脚本，脚本读取 `CCB_SOCKET`，输出符合 Claude hooks `hookSpecificOutput.permissionDecision` 协议的 JSON。
  - Acceptance Criteria:
    - 测试不启动真实 Claude；直接执行物化后的脚本，注入临时 `CCB_SOCKET` env，断言 stdout 是 JSON。
    - JSON 包含 `hookSpecificOutput.permissionDecision` 与 `hookSpecificOutput.permissionDecisionReason`，不使用 deprecated `decision/reason` 作为主协议。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins hook_script_emits_permission_decision_protocol -- --test-threads=1`

- [ ] **T1.4 写配置透传 failing test**
  - Files: `src/cli/config.rs` tests，`src/cli/start.rs` tests，`src/rpc/handlers.rs` tests。
  - Acceptance Criteria:
    - `ah.toml` 解析 `[agents.a1.hooks] PreToolUse = [...]`、`plugins = [...]`。
    - `start_project` 的 mock RPC payload 在 `agent.spawn` 中包含 `hooks` / `plugins`。
    - `session.spawn_master_pane` payload 包含 master 侧 `hooks` / `plugins`。
  - 红灯命令：
    - `CARGO_BUILD_JOBS=1 cargo test --lib pr4c -- --test-threads=1`

### Phase 2: HookGroup 类型定义 + ah.toml 解析扩展

- [ ] **T2.1 定义 HookGroup / HookItem / ExtensionConfig**
  - Files: `src/cli/config.rs`，必要时新增 `src/provider/extensions.rs` 并在 `src/provider/mod.rs` 导出。
  - Acceptance Criteria:
    - 类型支持 design `HookGroup { matcher, hooks: Vec<HookItem> }` 与简写 TOML `PreToolUse = ["./scripts/audit.sh"]`。
    - `HookItem` 支持 `type = "command"`、`command`、`timeout: Option<u64>`。
    - 默认 matcher 为 `"*"`；默认 type 为 `"command"`。

- [ ] **T2.2 扩展 MasterConfig / AgentConfig**
  - Files: `src/cli/config.rs`。
  - Acceptance Criteria:
    - `MasterConfig` 增加 `rules: Vec<String>`、`skills: Vec<String>`、`hooks: HashMap<String, Vec<HookGroup>>`、`plugins: Vec<String>`，全部 `#[serde(default)]`。
    - `AgentConfig` 增加同名字段，全部 `#[serde(default)]`。
    - 既有配置文件不写新字段时继续通过。

- [ ] **T2.3 增加配置解析与校验单测**
  - Files: `src/cli/config.rs` tests。
  - Acceptance Criteria:
    - 支持 `[agents.a1.hooks] PreToolUse = ["./scripts/audit.sh"]` 简写。
    - 支持完整对象写法：`PreToolUse = [{ matcher = "Edit|Write", hooks = [{ type = "command", command = "./scripts/read-first.sh", timeout = 5 }] }]`。
    - 插件 ID 校验拒绝空字符串；Codex plugin 推荐完整 ID `name@marketplace`，裸 ID 至少 warning 或在 tasks 实施时明确策略。

- [ ] **T2.4 Phase 2 验证**
  - Files: 无。
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --lib config -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --lib pr4c -- --test-threads=1`
  - Acceptance Criteria:
    - Phase 2 新增配置测试转绿，旧 config 测试不回归。

### Phase 3: RPC `agent.spawn` / `session.spawn_master_pane` 扩 hooks/plugins 参数

- [ ] **T3.1 CLI start 透传 agent 扩展字段**
  - Files: `src/cli/start.rs`。
  - Acceptance Criteria:
    - `agent.spawn` payload 在 `src/cli/start.rs:102-111` 附近加入 `rules_layers`、`skills`、`hooks`、`plugins`。
    - 保持 `extra_env_vars` 合并语义不变。

- [ ] **T3.2 CLI start 透传 master 扩展字段**
  - Files: `src/cli/start.rs`。
  - Acceptance Criteria:
    - `session.spawn_master_pane` payload 包含 `hooks`、`plugins`、`rules_layers`、`skills`。
    - master disabled 时不发送 master 扩展字段。

- [ ] **T3.3 RPC handlers 解析扩展参数**
  - Files: `src/rpc/handlers.rs`。
  - Acceptance Criteria:
    - `handle_agent_spawn` 在 `src/rpc/handlers.rs:312-350` 附近解析 `hooks` / `plugins`，解析失败返回 `IpcInvalidRequest`。
    - `handle_session_spawn_master_pane` 在 `src/rpc/handlers.rs:209-236` 附近解析 master 扩展参数。
    - 未传字段时默认空，不影响现有 RPC 调用方。

- [ ] **T3.4 Phase 3 验证**
  - Files: `src/cli/start.rs` tests，`src/rpc/handlers.rs` tests。
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --lib start::tests -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --lib rpc::handlers::tests -- --test-threads=1`
  - Acceptance Criteria:
    - mock RPC payload 测试转绿。
    - `handle_agent_spawn` / `handle_session_spawn_master_pane` 旧测试不需要改业务断言即可通过。

### Phase 4: `prepare_home_layout` 接口重构 + Claude/Gemini hooks 注入

- [ ] **T4.1 引入 HomeExtensionSpec 并重构接口**
  - Files: `src/provider/home_layout.rs`，可能新增 `src/provider/extensions.rs`。
  - Acceptance Criteria:
    - `prepare_home_layout(provider, sandbox_dir, workspace_path)` 保留兼容 wrapper，内部调用新函数并传空扩展。
    - 新增 `prepare_home_layout_with_extensions(provider, sandbox_dir, workspace_path, role, extensions)` 或等价接口。
    - `prepare_home_layout_with_role` 支持扩展参数或保留 wrapper，避免大面积改旧测试。

- [ ] **T4.2 Claude hooks 文件侧物化**
  - Files: `src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - 创建 sandbox `.claude/hooks/`。
    - 将宿主脚本 symlink 到 `.claude/hooks/<basename>`；相对路径按项目根或 config 文件所在目录解析，禁止静默链接不存在文件。
    - 对同名脚本冲突给出 deterministic 错误或稳定命名策略。

- [ ] **T4.3 Claude settings.json 注册侧注入**
  - Files: `src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - 在 `materialize_claude_settings` 当前逻辑 `src/provider/home_layout.rs:311-329` 基础上合并 `hooks`，不覆盖既有 `skipDangerousModePermissionPrompt` / `permissions.defaultMode`。
    - 输出官方嵌套形状：`hooks.<EventName>[] = { matcher, hooks: [{ type, command, timeout? }] }`。
    - `command` 使用 sandbox 内路径，不泄露宿主绝对路径给 provider 配置。

- [ ] **T4.4 Gemini settings.json 注册侧注入**
  - Files: `src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - 在 `materialize_gemini_settings` 当前逻辑 `src/provider/home_layout.rs:263-287` 基础上合并 `hooks`。
    - 保留 `security.auth.selectedType` 与宿主 settings 中其他字段。
    - Gemini hook 对象含 `type/command/matcher/timeout`；不实现 Gemini plugins。

- [ ] **T4.5 Phase 4 验证**
  - Files: `tests/pr4c_hooks_plugins.rs`，`tests/mvp12_home_layout.rs`。
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins claude_hook_materializes_settings_and_symlink -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins gemini_hook_materializes_settings -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test mvp12_home_layout -- --test-threads=1`
  - Acceptance Criteria:
    - Claude/Gemini hook 红灯转绿。
    - mvp12 既有 4 个 home layout 测试不回归。

### Phase 5: Codex/Claude plugins 激活 + 物化目录

- [ ] **T5.1 Codex plugin cache 物化**
  - Files: `src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - 创建 sandbox `.codex/plugins/cache/`。
    - 将宿主 `.codex/plugins/cache/<full-id>/` symlink 到 sandbox 对应目录。
    - 不执行 `git clone`，缺失本地资产时报明确错误，留给 PR4d。

- [ ] **T5.2 Codex config.toml 激活字段写入**
  - Files: `src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - 在 `prepare_managed_codex_home` 当前逻辑 `src/provider/home_layout.rs:332-353` 后合并 `[plugins."<full-id>"] enabled = true`。
    - 保留既有 workspace trust 配置。
    - 对已有 `[plugins."<full-id>"]` 只设置/覆盖 `enabled = true`，不删除其他键。

- [ ] **T5.3 Claude plugin cache 与 enabledPlugins**
  - Files: `src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - 创建 sandbox `.claude/plugins/cache/<id>/` symlink。
    - 在 `.claude/settings.json` 写入或合并 `enabledPlugins.<id> = true`。
    - 不假设 `.claude/plugins/installed_plugins.json` 必然存在。

- [ ] **T5.4 Phase 5 验证**
  - Files: `tests/pr4c_hooks_plugins.rs`。
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins codex_plugin_materializes_config_and_cache -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins claude_plugin_materializes_enabled_plugins -- --test-threads=1`
  - Acceptance Criteria:
    - Codex/Claude plugin 红灯转绿。
    - 缺失本地 plugin 资产测试返回可读错误，不 panic。

### Phase 6: 全局回归 + ship

- [ ] **T6.1 PR4c 专项测试**
  - Files: 无。
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins -- --test-threads=1`
  - Acceptance Criteria:
    - PR4c 3 个 design §6.1 验收场景全部通过。

- [ ] **T6.2 lib 全量回归**
  - Files: 无。
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`
  - Acceptance Criteria:
    - 不低于当前 baseline：`391 passed / 1 ignored`，或按实际 main baseline 记录差异。

- [ ] **T6.3 mvp12 home layout 回归**
  - Files: 无。
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test mvp12_home_layout -- --test-threads=1`
  - Acceptance Criteria:
    - `4 passed`，并确认新增 PR4c tests 没破坏 provider HOME 物化。

- [ ] **T6.4 mvp2 acceptance 回归**
  - Files: 无。
  - 命令：
    - `CARGO_BUILD_JOBS=1 cargo test --test mvp2_acceptance -- --test-threads=1`
  - Acceptance Criteria:
    - 当前 baseline `7 passed / 2 ignored` 不回归。

- [ ] **T6.5 提交与 PR**
  - Files: 按实际改动逐文件 `git add`，禁止 `git add -A` / `git add .`。
  - Commit:
    - `feat(ah-pr4c): materialize hooks and plugins`
  - PR:
    - base: `main`
    - title: `feat(ah-pr4c): hooks/plugins 双侧物化`
  - Acceptance Criteria:
    - 不 merge，等待 review。

## §3. 风险 + 注意点

- **Gemini scope**：PR4c 只实现 Gemini hooks，不实现 Gemini plugins；`.kiro/specs/ah-pr4c-hooks-plugins/design.md:86-88` 已声明 plugin unsupported。
- **Claude/Gemini Hook 协议漂移**：必须按当前官方嵌套对象写 `hooks.<EventName>[]`，不要退化成 `HashMap<String, Vec<String>>` 直接写脚本路径。
- **CCB_SOCKET vs CCB_TMUX_SOCKET**：hook 脚本访问 ccbd RPC 必须使用 `CCB_SOCKET`，`src/cli/rpc_client.rs:102-112` 是 socket 解析依据；`CCB_TMUX_SOCKET` 仅是 tmux 环境透传。
- **路径解析**：相对 hook/plugin 路径必须有明确 base，推荐 config 文件所在目录或项目根；测试要覆盖相对路径。
- **Symlink 安全**：默认 symlink 是 design 决策；实现时必须拒绝不存在源、目录/文件类型不匹配、重复 basename 冲突。
- **外部资产下载**：PR4c 不做 `git clone` / marketplace install；缺本地 cache 时报错，PR4d 再补 provisioning。
- **PR-1b 依赖**：PR4c ship 后，PR-1b 才能用 `PreToolUse` hook 注入 Read-first evidence writer。

## §4. 估算

- LOC：约 400-700 LOC。
- 文件数：约 8-12 文件。
- Wall-clock：a1 实施约 6-10 h。
- 主要改动文件预估：
  - `src/cli/config.rs`
  - `src/cli/start.rs`
  - `src/rpc/handlers.rs`
  - `src/provider/home_layout.rs`
  - `src/provider/mod.rs`
  - `src/provider/extensions.rs`（可选新增）
  - `tests/pr4c_hooks_plugins.rs`（新增）
  - `tests/mvp12_home_layout.rs`
  - `.kiro/specs/ah-pr4c-hooks-plugins/tasks.md`
