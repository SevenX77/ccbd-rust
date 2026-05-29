# ah PR4d Tasks: Git-based Plugins Auto-provisioning

## 0. 数据校验声明

本任务清单基于 `.kiro/specs/ah-pr4d-auto-provisioning/design.md` round 3 lock（commit `1def58d`），只覆盖 plugins Git URL 自动补齐与物化，不扩展 skills/rules。

现有代码锚点：

- `src/cli/config.rs:35`：`MasterConfig.plugins: Vec<String>`。
- `src/cli/config.rs:71`：`AgentConfig.plugins: Vec<String>`。
- `src/provider/extensions.rs:4-10`：`ExtensionConfig` 当前保持 `plugins: Vec<String>`。
- `src/provider/home_layout.rs:33`：`prepare_home_layout` 入口。
- `src/provider/home_layout.rs:109`：Claude 当前直接用 `extensions.plugins` 物化插件。
- `src/provider/home_layout.rs:495-564`：`materialize_claude_plugins` / `materialize_codex_plugins` 当前从宿主 provider-native cache 目录查找 `plugin` 字符串。
- `src/provider/manifest.rs:101`：`XDG_CACHE_HOME` 已在 provider env passthrough 中。

设计锚点：

- `design.md:37-49`：`<name>@git@<url>[#<ref>]` grammar，首个 `@git@` 分隔，支持 SSH URL 内部 `git@host`。
- `design.md:50-53`：XDG cache 目录为 `$XDG_CACHE_HOME/ah/cache/git/<host>/<owner>/<repo>/<ref>/`，fallback `$HOME/.cache/ah/cache/...`。
- `design.md:55-63`：provisioning pipeline 与 Claude/Codex sandbox 目标。
- `design.md:65-73`：git clone 继承宿主 env、幂等、`.tmp -> mv` 原子写入。
- `design.md:75-107`：保留 `ExtensionConfig.plugins: Vec<String>`，Materialize 初期解析为 `ResolvedPlugin`，物化接口接收 `&[ResolvedPlugin]`。
- `design.md:131-151`：5 个 tests-first 验收场景。

实测 grep：

- `rg -n "plugins" src/cli/config.rs src/provider/extensions.rs src/provider/home_layout.rs src/provider/manifest.rs`
- `rg -n "materialize_claude_plugins|materialize_codex_plugins|prepare_home_layout|XDG_CACHE_HOME" src/provider`

## 1. 全景表

| Phase | 名称 | 目标 | 预计任务数 | 依赖 |
| :--- | :--- | :--- | :--- | :--- |
| Phase 0 | Branch + Spec 锚定 | 确认从 design lock 开工，不漂移 scope | 2 | 无 |
| Phase 1 | Tests-first 红灯 | 覆盖 design §6 5 个验收场景 | 5 | Phase 0 |
| Phase 2 | 类型 + 解析 | `PluginSpec` / `GitUrlSpec` / `ResolvedPlugin` 与 `@git@` parser | 4 | Phase 1 |
| Phase 3 | Provisioner | XDG cache helper、git clone subprocess、原子 `.tmp -> mv` | 5 | Phase 2 |
| Phase 4 | Home Layout 接入 | `Vec<String> -> Vec<ResolvedPlugin>`，失败即 barrier | 4 | Phase 3 |
| Phase 5 | Claude/Codex 物化改造 | 两侧使用 `ResolvedPlugin.name` 与 `cache_path` | 4 | Phase 4 |
| Phase 6 | 回归 + Ship | PR4d 专项、PR4c、lib、diff hygiene、commit 准备 | 4 | Phase 5 |

## 2. 详细 Tasks

### Phase 0: Branch + Spec 锚定

- [ ] **T0.1 确认分支与基线**
  - Files: `.kiro/specs/ah-pr4d-auto-provisioning/design.md`，`src/provider/extensions.rs`，`src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - `git status -sb` 无未理解的 staged 改动。
    - `git log --oneline -- .kiro/specs/ah-pr4d-auto-provisioning/design.md -5` 可看到 design lock commit `1def58d` 或其后继。
    - 不修改 skills/rules 相关代码。

- [ ] **T0.2 建立 PR4d 专项测试文件**
  - Files: `tests/pr4d_auto_provisioning.rs` (NEW)，`src/provider/extensions.rs`，`src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - 新增测试文件只写 PR4d 场景，不改 `tests/pr4c_hooks_plugins.rs` 的既有断言。
    - 测试 helper 可创建临时 HOME / `XDG_CACHE_HOME` / sandbox / workspace，并在 drop 时恢复 env。

### Phase 1: Tests-first 红灯

- [ ] **T1.1 本地 ID 插件 regression 红灯**
  - Files: `tests/pr4d_auto_provisioning.rs`，`src/provider/home_layout.rs:495-564`。
  - Scenario: `plugins = ["github@openai-curated"]`。
  - Acceptance Criteria:
    - 宿主 `.codex/plugins/cache/github@openai-curated` 存在时，Codex sandbox `.codex/plugins/cache/github@openai-curated` symlink 成功。
    - 不触发 git clone helper。
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4d_auto_provisioning id_only_plugin_keeps_pr4c_behavior -- --test-threads=1` 初始红灯，实施后转绿。

- [ ] **T1.2 Git 插件初次安装红灯**
  - Files: `tests/pr4d_auto_provisioning.rs`，`src/provider/extensions.rs:4-10`，`src/provider/home_layout.rs:109`。
  - Scenario: `plugins = ["my-plugin@git@github.com:foo/bar.git"]`。
  - Acceptance Criteria:
    - 测试使用本地 bare repo 或 fixture repo，避免真实网络。
    - `$XDG_CACHE_HOME/ah/cache/git/github.com/foo/bar/main/` 被创建。
    - Claude/Codex sandbox 目标以逻辑 ID `my-plugin` 建 symlink。

- [ ] **T1.3 缓存命中幂等红灯**
  - Files: `tests/pr4d_auto_provisioning.rs`，`src/provider/provisioner.rs` (NEW)。
  - Scenario: 第二次同一 plugin spec 启动。
  - Acceptance Criteria:
    - 第二次不调用 git clone subprocess。
    - 已有 cache 目录非空则视为 hit。
    - 可通过 fake git runner 计数或 PATH shim 计数验证。

- [ ] **T1.4 Clone 失败 barrier 红灯**
  - Files: `tests/pr4d_auto_provisioning.rs`，`src/provider/home_layout.rs:61-90`。
  - Scenario: 非法 Git URL 或 fake git 返回非 0。
  - Acceptance Criteria:
    - `prepare_home_layout_with_extensions` 返回 `Err(CcbdError::EnvironmentNotSupported { .. })` 或等价 typed error。
    - sandbox `.claude/plugins/cache/<name>` / `.codex/plugins/cache/<name>` 不存在。
    - `.tmp` 残留被清理或不会被当作 cache hit。

- [ ] **T1.5 私有库 / SSH URL parse 红灯**
  - Files: `tests/pr4d_auto_provisioning.rs`，`src/provider/extensions.rs`。
  - Scenario: `legacy-mod@git@git@internal.com:ops/mod.git#v1`。
  - Acceptance Criteria:
    - `name = "legacy-mod"`。
    - `url = "git@internal.com:ops/mod.git"`。
    - `reference = "v1"`。
    - 测试验证 git subprocess env 继承 `SSH_AUTH_SOCK`，不把 SSH 凭证复制到 sandbox。

### Phase 2: GitUrlSpec + ResolvedPlugin 类型 + @git@ parse 模块

- [ ] **T2.1 定义类型**
  - Files: `src/provider/extensions.rs:4-10`。
  - Acceptance Criteria:
    - 新增 `PluginSpec`, `GitUrlSpec`, `ResolvedPlugin`。
    - `ExtensionConfig.plugins: Vec<String>` 保持不变，保证 ah.toml / RPC 旧 schema 不破坏。
    - `ResolvedPlugin { name, cache_path }` 使用 `PathBuf` 承载物理路径。

- [ ] **T2.2 实现 `PluginSpec::parse`**
  - Files: `src/provider/extensions.rs:4-10`。
  - Acceptance Criteria:
    - 不含 `@git@` -> `PluginSpec::IdOnly(raw)`。
    - 使用首个 `@git@` 分隔。
    - `#ref` 缺省为 `main`。
    - 空 `name` / 空 `url` / 空 `ref` 返回 typed error，不 panic。

- [ ] **T2.3 实现 cache path key 解析辅助**
  - Files: `src/provider/extensions.rs`，`src/provider/provisioner.rs` (NEW)。
  - Acceptance Criteria:
    - HTTPS URL `https://github.com/org/repo.git#v2` -> `github.com/org/repo/v2`。
    - SCP-like URL `git@github.com:org/repo.git` -> `github.com/org/repo/main`。
    - 去除 `.git` 后缀，拒绝 `..` / 绝对路径 / 空 segment。

- [ ] **T2.4 类型单测**
  - Files: `src/provider/extensions.rs`。
  - Acceptance Criteria:
    - 覆盖 ID-only、HTTPS、SSH、带 ref、非法输入。
    - `cargo test provider::extensions --lib -- --test-threads=1` 通过。

### Phase 3: cache 目录 helper + git clone subprocess + 原子操作

- [ ] **T3.1 新增 provisioner 模块**
  - Files: `src/provider/provisioner.rs` (NEW)，`src/provider/mod.rs`。
  - Acceptance Criteria:
    - `src/provider/mod.rs` 导出 `provisioner`。
    - provisioner 不依赖 CLI 层，不读 ah.toml 文件，只消费 `PluginSpec` / `GitUrlSpec`。

- [ ] **T3.2 实现 XDG cache root helper**
  - Files: `src/provider/provisioner.rs`，`src/provider/home_layout.rs:623-627`。
  - Acceptance Criteria:
    - 有 `XDG_CACHE_HOME` -> `$XDG_CACHE_HOME/ah/cache/git/...`。
    - 无 `XDG_CACHE_HOME` -> `$HOME/.cache/ah/cache/git/...`。
    - 与 `src/provider/manifest.rs:101` 的 passthrough 兼容。

- [ ] **T3.3 实现 git clone subprocess 封装**
  - Files: `src/provider/provisioner.rs`。
  - Acceptance Criteria:
    - 使用 `Command::new("git")`。
    - clone 命令包含 `-c core.hooksPath=/dev/null`。
    - 子进程继承宿主 env，包括 `SSH_AUTH_SOCK`。
    - 支持超时（例如 60s）并返回 typed error。

- [ ] **T3.4 实现 checkout/ref 处理**
  - Files: `src/provider/provisioner.rs`。
  - Acceptance Criteria:
    - `main` 可直接 clone 默认分支或 clone 后 checkout `main`。
    - 非 main ref clone 后 checkout branch/tag/sha。
    - checkout 失败时删除 `.tmp` 并返回 barrier error。

- [ ] **T3.5 实现原子 `.tmp -> mv` 与 cache hit**
  - Files: `src/provider/provisioner.rs`。
  - Acceptance Criteria:
    - 最终目录存在且非空 -> hit，不 clone。
    - miss 时 clone 到 sibling tmp 目录，成功后 rename 到最终路径。
    - tmp 名称包含 process/thread 或随机 token，避免并发冲突。
    - 失败不会留下可被误判为 hit 的最终目录。

### Phase 4: prepare_home_layout 接入

- [ ] **T4.1 在 materialize 初期解析 plugins**
  - Files: `src/provider/home_layout.rs:61-90`，`src/provider/extensions.rs:4-10`。
  - Acceptance Criteria:
    - `prepare_home_layout_with_extensions` 或 provider-specific prepare 函数开头执行 `Vec<String> -> Vec<PluginSpec> -> Vec<ResolvedPlugin>`。
    - hooks 路径不受影响。
    - 没有 plugins 时不做 provisioning。

- [ ] **T4.2 ID-only bridge**
  - Files: `src/provider/home_layout.rs:495-564`，`src/provider/provisioner.rs`。
  - Acceptance Criteria:
    - `IdOnly("github@openai-curated")` 的 `ResolvedPlugin.cache_path` 指向既有 provider-native cache 路径。
    - Claude 使用宿主 `.claude/plugins/cache/<id>`。
    - Codex 使用宿主 `.codex/plugins/cache/<id>`。
    - 保持 PR4c regression 不变。

- [ ] **T4.3 Git provisioning barrier**
  - Files: `src/provider/home_layout.rs:61-90`，`src/provider/provisioner.rs`。
  - Acceptance Criteria:
    - Git spec provisioning 失败时，`prepare_home_layout_with_extensions` 直接返回 Err。
    - 不继续写 provider settings。
    - 错误 message 包含 plugin name、URL/ref、git stderr 或 timeout reason。

- [ ] **T4.4 Provider 分流**
  - Files: `src/provider/home_layout.rs:93-160`。
  - Acceptance Criteria:
    - Claude/Codex 都调用同一 provisioning 结果。
    - Gemini plugins 仍 unsupported / 不新增 Gemini plugin 物化。
    - 未知 provider 不触发 plugin provisioning。

### Phase 5: materialize_claude / materialize_codex 接收 `&[ResolvedPlugin]`

- [ ] **T5.1 Claude plugin symlink 改造**
  - Files: `src/provider/home_layout.rs:495-513`。
  - Acceptance Criteria:
    - `materialize_claude_plugins` 接收 `&[ResolvedPlugin]`。
    - symlink source 使用 `plugin.cache_path`。
    - target 使用 `plugin.name`。
    - source 不存在时报错仍保留 provider/name 细节。

- [ ] **T5.2 Claude settings key 改造**
  - Files: `src/provider/home_layout.rs:407-433`。
  - Acceptance Criteria:
    - `enabledPlugins.<name> = true` 使用 `ResolvedPlugin.name`。
    - 不把原始 `name@git@url#ref` 写入 settings。
    - 继续合并 hooks，不覆盖 PR4c 既有 `permissions`。

- [ ] **T5.3 Codex plugin symlink 改造**
  - Files: `src/provider/home_layout.rs:552-565`。
  - Acceptance Criteria:
    - `materialize_codex_plugins` 接收 `&[ResolvedPlugin]`。
    - symlink source 使用 `plugin.cache_path`。
    - target 使用 `.codex/plugins/cache/<name>`。

- [ ] **T5.4 Codex config key 改造**
  - Files: `src/provider/home_layout.rs:569-586`。
  - Acceptance Criteria:
    - `[plugins."<name>"] enabled = true` 使用 `ResolvedPlugin.name`。
    - 保留已有 plugin table 其他键。
    - 不把 Git URL 串写入 config。

### Phase 6: 全局回归 + ship

- [ ] **T6.1 PR4d 专项全绿**
  - Files: `tests/pr4d_auto_provisioning.rs`。
  - Acceptance Criteria:
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4d_auto_provisioning -- --test-threads=1` 全绿。
    - 覆盖 design §6 5 个场景。

- [ ] **T6.2 PR4c 回归**
  - Files: `tests/pr4c_hooks_plugins.rs`，`src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - `CARGO_BUILD_JOBS=1 cargo test --test pr4c_hooks_plugins -- --test-threads=1` 6/6 passed。
    - ID-only plugins 行为不退化。

- [ ] **T6.3 lib 回归**
  - Files: `src/provider/extensions.rs`，`src/provider/provisioner.rs`，`src/provider/home_layout.rs`。
  - Acceptance Criteria:
    - `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1` passed。
    - `cargo fmt` passed。
    - `git diff --check` clean。

- [ ] **T6.4 Ship 准备**
  - Files: `.kiro/specs/ah-pr4d-auto-provisioning/tasks.md`，`src/provider/extensions.rs`，`src/provider/provisioner.rs`，`src/provider/home_layout.rs`，`tests/pr4d_auto_provisioning.rs`。
  - Acceptance Criteria:
    - commit message 建议：`feat(ah-pr4d): auto-provision git plugins`。
    - PR 描述列出 PR4d 专项、PR4c 回归、lib 回归结果。
    - 不包含 skills/rules scope。

## 3. 风险

- **并发 clone 锁**：当前 design 只要求 `.tmp -> mv` 原子性，未定义多进程同时 clone 同一 ref 的锁。PR4d 实施时至少使用唯一 tmp 目录，并保证 rename 前再次检查 final 目录；严格 file lock 可留 follow-up。
- **缓存清理**：PR4d 不做 GC。`$XDG_CACHE_HOME/ah/cache/git/...` 版本目录会长期累积，清理策略留 PR4e 或后续 maintenance。
- **Git URL 规范化**：HTTPS、SCP-like SSH、`.git` 后缀、ref 中特殊字符都要 sanitize；不能让 URL segment 形成路径穿越。
- **网络与凭证**：git clone 在宿主执行并继承 env，但 systemd service 环境可能没有 `SSH_AUTH_SOCK`。测试可验证继承机制，生产配置文档/doctor 检查可后补。
- **依赖关系**：PR4d ship 后解锁 PR4e 指纹/变更检测/强制对齐；PR4d 不实现 `ah up` diff 或自动更新已有 cache。

## 4. 估算

- LOC：约 400-700 LOC。
- 文件：约 8-12 个文件，核心为 `src/provider/extensions.rs`、`src/provider/provisioner.rs`、`src/provider/home_layout.rs`、`tests/pr4d_auto_provisioning.rs`。
- Wall-clock：a1 实施约 8-15 h，含 tests-first、实现、回归与 PR 整理。
