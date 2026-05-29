# Research: ah PR4e Fingerprint Audit & Alignment (ah up)

本文档通过源码实证（Commit `a18fdd4`），系统梳理了实现 `ah up` 指纹审计与强制对齐机制的物理根基与现有架构限制。

## 1. 现有架构实证 (PR4a-d 落地状态)

### 1.1 DB Schema 现状 (`src/db/schema.rs`)
- **Sessions 表** (`L8-15`): 包含 `id`, `project_id`, `master_pid`, `master_pane_id`, `status`, `created_at`。**无 `config_hash` 字段**。
- **Agents 表** (`L17-29`): 包含 `id`, `session_id`, `provider`, `state`, `state_version`, `pid`, `exit_code`, `error_code`, `created_at`, `sub_state`, `updated_at`。**无 `config_hash` 字段**。
- **Evidence/Jobs 表** (`L45-78`): 已合入 PR-1a/b 的物理拦截字段（`job_id`, `evidence_type`, `subject_path`, `requires_physical_evidence` 等）。
- **结论**: 实现 PR4e 必须执行 DB Migration，在 `sessions` 和 `agents` 表中新增 `config_hash: TEXT` 字段。

### 1.2 ah.toml Schema 现状 (`src/cli/config.rs`)
- **MasterConfig** (`L24-36`): 包含 `cmd`, `enabled`, `hooks`, `plugins`。**无 `rules` 或 `skills` 字段**。
- **AgentConfig** (`L64-72`): 包含 `provider`, `env`, `hooks`, `plugins`。**无 `rules` 或 `skills` 字段**。
- **Hooks/Plugins 类型**: 
    - `hooks`: `HashMap<String, Vec<HookGroup>>` (见 `src/provider/extensions.rs:8`)。
    - `plugins`: `Vec<String>` (见 `src/cli/config.rs:35/71`)。

### 1.3 物化流水线 (`src/provider/home_layout.rs`)
- **核心入口**: `prepare_home_layout_with_extensions` (`L62-91`)。
- **签名**: `(provider, sandbox_dir, workspace_path, role, extensions: &ExtensionConfig) -> Result<HomeOverrides, CcbdError>`。
- **物化结束点**: 函数末尾返回 `HomeOverrides` (`L91`)，此前已完成 `resolve_plugins` 和 `materialize_*_settings`。
- **Provisioning Barrier**: PR4d 拦截点位于 `resolve_plugins_for_provider` 内部，若失败则直接通过 `?` 向上抛出错误。

### 1.4 RPC Handlers (`src/rpc/handlers.rs`)
- **Spawn Flow**: 
    - `handle_session_spawn_master_pane` (`L208`): 内部调用 `prepare_home_layout_with_extensions`。
    - `handle_agent_spawn` (`L317`): 同样调用该物化流。
- **对齐能力**: 
    - 现有 `stop_session_anchor` (`L192`) 实现了 `systemctl --user stop` 的初步封装。
    - **无** `realign` 或 `restart` 相关 RPC 接口。

### 1.5 CLI 命令现状 (`src/bin/ah.rs`)
- **命令定义** (`L32-97`): `Cmd` 枚举包含 `Ping`, `Version`, `Ps`, `Start`, `Ask`, `Pend`, `Cancel`, `Kill`, `Watch`, `Logs`, `Attach`, `Stop`, `Doctor`, `Config`, `Prompt`。
- **二级命令**: 
    - `ConfigCmd`: `Validate`, `Migrate` (`L99-107`)。
    - `PromptCmd`: `Resolve` (`L109-120`)。
- **结论**: **不存在 `ah up` 命令**。

### 1.6 数据库写入路径 (DB Write Paths)
- **Session 插入**: `src/db/sessions.rs:17-35` (`insert_session_sync`)。需在此处或后续更新中增加 `config_hash` 写入。
- **Agent 插入**: `src/db/agents.rs:7-28` (`insert_agent_sync`)。同上，需在初始化时捕获并存储指纹。

### 1.7 状态机与 BUSY 语义
- **BUSY 状态**: 见 `src/db/state_machine.rs:164-674`。
- **拦截逻辑**: `mark_agent_idle_matched_outcome_sync` (`L281`) 中包含了 `dispatched_job` 存在时的 `evidence_denial_for_job` 检查。
- **事件记录**: `insert_event_sync` (`src/db/events.rs:30-109`) 用于记录 `state_change`。

### 1.8 Respawn 原语调研
- **Tmux 侧**: `src/tmux/session.rs:190` 实现了 `respawn_initial_window_sync`，封装了 `tmux respawn-pane`。这为 Master/Agent 的热重启提供了原子化的底层支持。
- **原子性**: 现有的 `save_kb_atomic` (`src/prompt_handler/kb.rs:33`) 展示了通过临时文件 + rename 实现原子写入的模式。

---

## 2. 序列化与哈希现状

- **序列化**: 全仓统一使用 `serde_json` 库。
- **Canonical 行为**: `serde_json::to_string` 的默认实现**不保证** Key 排序。在 `ExtensionConfig` 的 `HashMap` 序列化时会产生非确定性。需参考 **RFC 8785 (JCS)** 进行规范化处理。
- **哈希库**: 已在 PR4d 中引入 `sha2` crate (由 `sha2::{Digest, Sha256}` 支持)，可直接复用于指纹计算。

---

## 3. 外部类比与第一性原理

- **Terraform plan/apply**: 通过比对期望状态 (ah.toml) 与实际状态 (DB Config Hash) 进行 drift 检测，输出结构化 diff 报告。
- **Nix store path hashing**: 采用规范化输入 (Raw Spec) 计算 derivation hash，确保环境可重现。
- **Kubernetes Reconcile Loop**: 声明式 API + 自动调谐 (ah up 手动触发)。
- **RFC 8785 (JCS)**: JSON Canonicalization Scheme，为 JSON 数据提供确定性的序列化输出，是保证哈希稳定的国际标准。

---

## 4. PR4d 落地状态对 PR4e 的约束

- **ResolvedPlugin** (`src/provider/plugins.rs:24-27`): 结构为 `{ name, cache_dir }`。
- **指纹源选择**: 指纹应包含 **原始 Spec** (如 `my-plugin@git@github.com...`) 而非 `cache_dir`。物理路径受 `XDG_CACHE_HOME` 环境影响，不应计入逻辑一致性指纹。
- **Barrier 行为**: PR4d 实施了“物化失败则阻断”，PR4e 的对齐流程应继承此 Barrier，确保只有物化全成功的 Sandbox 才能更新 `config_hash`。

---

## 5. 已知 Gap (Research 待续点)

1. **Deterministic 序列化**: 需确认是否需要引入 `serde_json::to_value` 手动排序，或使用支持有序 Map 的 crate（如 `indexmap`）来保证 `hooks` 哈希稳定。
2. **Master 状态查询**: 当前 `sessions` 表记录的是 Master 进程 PID，但 `ah up` 需要在不重启整个 daemon 的情况下，通过 RPC 触发 Master Pane 的热重载。
3. **漂移报告格式**: CLI 层如何优雅地对比并展示 `HashMap<String, Vec<HookGroup>>` 的深度差异。
