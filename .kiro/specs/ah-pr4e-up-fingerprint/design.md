# Design: ah PR4e Fingerprint Audit & Alignment (`ah up`)

| 状态 | 1e 正式设计 |
| :--- | :--- |
| **日期** | 2026-05-29 |
| **范围** | 基于 `ah.toml` 配置指纹的漂移检测与声明式对齐 |

## 1. 目标 + 痛点对齐

PR4e 解决 PR4a-d 之后仍存在的运行时漂移：`ah.toml` 可以变更，但已启动 Master/Agent 的 sandbox HOME、hooks、plugins、env 仍停留在旧物化状态。research 已确认当前 DB 无 `config_hash`（`src/db/schema.rs:8-29`），CLI 无 `ah up`（`src/bin/ah.rs:32-97`），spawn 路径已统一经过 `prepare_home_layout_with_extensions`（`src/rpc/handlers.rs:208-245,317-365`）。

目标：

- 为 Master 与 Agent 分别计算 deterministic `config_hash`。
- 在物化成功后持久化运行指纹。
- 新增 `ah up`，对比本地期望指纹与 DB 运行指纹，输出 NO_CHANGE / DRIFT / ORPHAN / NEW / SKIPPED_BUSY。
- 对 Agent DRIFT 执行安全 Stop-then-Spawn 对齐；Master DRIFT / ORPHAN / BUSY 默认只审计或跳过，`--force` 才执行破坏性操作。

非目标：

- 不实现完整 RFC 8785 JCS；PR4e 只实现 sorted-key `serde_json` deterministic serialization。
- 不把 PR4d 的 resolved cache path 纳入指纹；指纹只包含 raw spec。
- 不重新引入 rules/skills；PR4c 已将它们移出当前 scope。

## 2. 继承字段表

| 类别 | 字段 / 接口 | 现状 [file:line] | PR4e 变更 |
| :--- | :--- | :--- | :--- |
| **DB sessions** | `sessions` table | `id`, `project_id`, `master_pid`, `master_pane_id`, `status`, `created_at`（`src/db/schema.rs:8-15`） | `[NEW]` 增加 `config_hash TEXT`，同步扩展 `Session`（`src/db/schema.rs:107-115`）与 session query/insert 路径。 |
| **DB agents** | `agents` table | `id`, `session_id`, `provider`, `state`, `state_version`, `pid`, `exit_code`, `error_code`, `created_at`, `sub_state`, `updated_at`（`src/db/schema.rs:17-29`） | `[NEW]` 增加 `config_hash TEXT`，同步扩展 `Agent`（`src/db/schema.rs:117-130`）与 agent query/insert 路径。 |
| **DB session writes** | `insert_session_sync` / `insert_session` | `src/db/sessions.rs:17-35,202-212` | `[NEW]` 保持创建兼容，新增 `update_session_config_hash` / `query active hash` helper。 |
| **DB agent writes** | `insert_agent_sync` / `insert_agent` | `src/db/agents.rs:7-28,132-145` | `[NEW]` 保持创建兼容，新增 `update_agent_config_hash` / `query active hash` helper。 |
| **ah.toml master** | `MasterConfig` | `cmd`, `enabled`, `hooks`, `plugins`（`src/cli/config.rs:23-36`） | 无 schema 变化。 |
| **ah.toml agent** | `AgentConfig` | `provider`, `env`, `hooks`, `plugins`（`src/cli/config.rs:63-72`） | 无 schema 变化。 |
| **extension config** | `ExtensionConfig` | `hooks: HashMap<String, Vec<HookGroup>>`, `plugins: Vec<String>`（`src/provider/extensions.rs:4-9`） | 无反序列化变化；hash 层做排序归一化。 |
| **CLI Cmd** | `Cmd` enum | 无 `Up`，现有 variants 在 `src/bin/ah.rs:32-97` | `[NEW]` 增加 `Up { force: bool }`，调用 `cli::up::run_up`。 |
| **RPC router** | method registry | `METHODS` 无 realign（`src/rpc/router.rs:13-34`），dispatch match 在 `src/rpc/router.rs:71-85` | `[NEW]` 注册 `session.realign` 与 `agent.realign`。 |
| **RPC master spawn** | `handle_session_spawn_master_pane` | `src/rpc/handlers.rs:208-296`，物化调用在 `src/rpc/handlers.rs:233-241` | `[NEW]` 物化成功且 spawn/registration 成功后写 `sessions.config_hash`。 |
| **RPC agent spawn** | `handle_agent_spawn` | `src/rpc/handlers.rs:317-475`，物化调用在 `src/rpc/handlers.rs:352-362`，agent insert 在 `src/rpc/handlers.rs:463-475` | `[NEW]` 物化成功且 agent insert 成功后写 `agents.config_hash`。 |
| **Lifecycle** | kill / event helpers | `mark_agent_killed`（`src/db/agents_lifecycle.rs:128-145`），`insert_event`（`src/db/events.rs:100-109`） | `[NEW]` realign 复用 kill / event 语义，新增 `drift_realigned` / `drift_skipped` 事件。 |

## 3. 核心机制

### 3.1 `config_hash` 计算

新增 `src/provider/fingerprint.rs`：

```rust
pub enum ConfigRole<'a> {
    Master { cmd: &'a str },
    Agent { provider: &'a str, env: &'a HashMap<String, String> },
}

pub struct ConfigFingerprintInput<'a> {
    pub role: ConfigRole<'a>,
    pub hooks: &'a HashMap<String, Vec<HookGroup>>,
    pub plugins: &'a [String],
}

pub fn compute_config_hash(input: &ConfigFingerprintInput<'_>) -> Result<String, CcbdError>;
pub fn deterministic_json(value: serde_json::Value) -> Result<String, CcbdError>;
```

输入字段：

- Master：`cmd`（`src/cli/config.rs:29`）、`hooks`（`src/cli/config.rs:33`）、`plugins`（`src/cli/config.rs:35`）。
- Agent：`provider`（`src/cli/config.rs:65`）、`env`（`src/cli/config.rs:67`）、`hooks`（`src/cli/config.rs:69`）、`plugins`（`src/cli/config.rs:71`）。
- 不包含 `ResolvedPlugin.cache_dir`（`src/provider/plugins.rs:23-27`），只包含 raw `plugins: Vec<String>`。
- 不包含 rules/skills，因为当前 config schema 不存在这些字段（`src/cli/config.rs:23-72`）。

序列化策略：

- 使用 `serde_json::Value` 构造逻辑对象。
- 将所有 `Object` 转换为按 key 排序的 `serde_json::Map`；`hooks` / `env` 的 `HashMap` 必须排序。
- `plugins` 列表按字母序排序，避免 TOML list 顺序造成同义漂移。
- `Vec<HookGroup>` 内部保持用户声明顺序；这代表 hook 执行顺序，属于语义。
- 最后用 `serde_json::to_string` 输出排序后的 JSON，再用 `sha2::Sha256` 计算十六进制 hash。项目已有依赖：`serde_json`（`Cargo.toml:13`）、`sha2`（`Cargo.toml:38`）。
- 明确不实现完整 RFC 8785：不做 number formatting、Unicode normalization、IEEE edge cases 等完整 JCS 细节。

### 3.2 指纹存储

DB schema 变更：

- `sessions.config_hash TEXT`：Master 当前运行配置指纹。
- `agents.config_hash TEXT`：Agent 当前运行配置指纹。

推荐 helper：

```rust
pub(crate) fn update_session_config_hash_sync(
    conn: &Connection,
    session_id: &str,
    config_hash: &str,
) -> Result<(), CcbdError>;

pub(crate) fn update_agent_config_hash_sync(
    conn: &Connection,
    agent_id: &str,
    config_hash: &str,
) -> Result<(), CcbdError>;
```

写入时机：

- `handle_session_spawn_master_pane`：在 `prepare_home_layout_with_extensions` 成功返回（`src/rpc/handlers.rs:233-241`）且 pane spawn / `set_session_master_pane_id` 成功（`src/rpc/handlers.rs:254-265`）后，更新 `sessions.config_hash`。
- `handle_agent_spawn`：在 `prepare_home_layout_with_extensions` 成功返回（`src/rpc/handlers.rs:352-362`）且 `insert_agent` 成功（`src/rpc/handlers.rs:463-475`）后，更新 `agents.config_hash`。
- 指纹更新必须晚于 PR4d provisioning barrier；`resolve_plugins_for_provider` 失败会从 `prepare_home_layout_with_extensions` 冒泡（`src/provider/home_layout.rs:62-91`，Claude path `src/provider/home_layout.rs:110-113`，Codex path `src/provider/home_layout.rs:156-162`）。

### 3.3 `ah up` 对比流程

新增 CLI 子命令：

```rust
enum Cmd {
    Up {
        #[arg(long)]
        force: bool,
    },
}
```

新增 `src/cli/up.rs`：

```rust
pub struct UpOptions {
    pub config_path: Option<PathBuf>,
    pub cwd: PathBuf,
    pub force: bool,
}

pub async fn run_up<C: RpcClient>(client: &C, options: UpOptions) -> Result<(), CliError>;
```

流程：

1. 复用 `load_project_config` 解析 `ah.toml`。解析失败时直接返回错误，不调用任何 realign RPC。
2. 计算 Master 与每个 Agent 的 expected hash。
3. 通过新 RPC `session.realign` 传入 expected roles、raw config slices 与 `force`。
4. RPC 从 DB 读取 active sessions/agents 的 running hash。
5. 输出：
   - `NO_CHANGE`：hash 相同。
   - `DRIFT`：hash 不同，报告字段级原因（初版只需 `plugins changed` / `hooks changed` / `env changed` / `provider changed` / `cmd changed`）。
   - `ORPHAN`：DB 中运行角色不存在于新 config。
   - `NEW`：新 config block，DB 无对应 row。
   - `SKIPPED_BUSY`：Agent BUSY 且未 `--force`。
6. Master DRIFT 默认只审计并报告，不自动重启；`--force` 时才触发 Master 全量重启。

### 3.4 五阶段事务实施

新增 RPC：

```rust
pub async fn handle_session_realign(params: Value, ctx: &Ctx) -> Result<Value, CcbdError>;
pub async fn handle_agent_realign(params: Value, ctx: &Ctx) -> Result<Value, CcbdError>;
```

Router 接入：在 `src/rpc/router.rs:13-34` 加 method，在 `src/rpc/router.rs:71-85` dispatch。

Stage 1: Preparation

- `session.realign` 接收 session id、master config、agent configs、expected hashes、`force`。
- 验证 session 存在，读取 DB 状态。
- 计算/校验 expected hash，不能信任 CLI 原样传入的 hash。

Stage 2: Interruption Gate

- Master drift 且 `force=false` 时只报告 DRIFT，不进入 destruction / reconstruction；`force=true` 才允许全量重启 Master pane。
- 对 Agent 读取 `state` / `state_version`。`BUSY` 语义以 state machine 为准，核心路径在 `mark_agent_idle_matched_outcome_sync`（`src/db/state_machine.rs:278-330`）。
- 若 Agent 为 `BUSY` 且 `force=false`，不 kill、不 spawn，写 `drift_skipped` event。

Stage 3: Destruction

- Master：仅在 `force=true` 时复用 `stop_session_anchor`（`src/rpc/handlers.rs:192-205`）或等价 session kill path 停止物理沙箱。
- Agent：复用 `mark_agent_killed`（`src/db/agents_lifecycle.rs:128-145`）与现有 kill 资源清理路径；不能仅用 `tmux respawn-pane`，因为它不能重建 bwrap/env/pidfd/FIFO。
- 失败处理：停止失败或资源未释放时终止该角色对齐，不进入 Stage 4。

Stage 4: Reconstruction

- Master：调用与 `handle_session_spawn_master_pane` 等价的全量 spawn 路径，必须重新经过 `prepare_home_layout_with_extensions`（`src/rpc/handlers.rs:233-241`）。
- Agent：调用与 `handle_agent_spawn` 等价的全量 spawn 路径，必须重建 sandbox dir、HOME、systemd command、tmux pane、FIFO、pidfd reader（关键路径 `src/rpc/handlers.rs:341-475`）。
- 若物化或 spawn 失败，不写新 hash，返回 per-role error。

Stage 5: Commitment

- 只有 Stage 4 成功后，更新 `sessions.config_hash` / `agents.config_hash`。
- 通过 `insert_event`（`src/db/events.rs:100-109`）写 `drift_realigned` 审计事件。
- 若 hash 写入失败，返回 CRITICAL error；此时物理状态已更新但 DB state 未提交，需要用户重跑 `ah up` 或人工处理。

### 3.5 容错

- `ah.toml` 解析失败：CLI 直接失败，不发 RPC，不停任何进程。
- DB 读取失败：RPC 返回错误，不进入 destruction。
- PR4d provisioning 失败：`prepare_home_layout_with_extensions` 返回错误，不写新 hash。
- Agent BUSY：默认 skip，`--force` 才 kill。
- 多个 `ah up` 并发：PR4e 初版应在 RPC 层加 per-session realign mutex；不要依赖泛泛 CAS。现有 `SESSION_WINDOW_LOCKS` 是 spawn 窗口锁（`src/rpc/handlers.rs:47-48`），可以复用模式但不能直接复用语义。

## 4. 现有代码兼容

- `HomeOverrides` 当前只含 `home_root` 与 `extra_env`（`src/provider/home_layout.rs:23-26`）。PR4e 不要求 `prepare_home_layout_with_extensions` 返回 hash；hash 由 spawn/realign 层基于 raw config 计算，避免把物化路径写进逻辑指纹。
- PR4d 的 `ResolvedPlugin`（`src/provider/plugins.rs:23-27`）只用于物化；PR4e hash 输入必须使用 raw `plugins: Vec<String>`。
- `handle_session_spawn_master_pane` 与 `handle_agent_spawn` 已解析 `ExtensionConfig`（`src/rpc/handlers.rs:218,322`；helper 在 `src/rpc/handlers.rs:1118-1131`），可在同一 params 中读取 raw hooks/plugins 用于 hash。
- 现有 event 表可承载审计事件（`src/db/schema.rs:33-43`），无需新表。
- `tabled` 已是 CLI 依赖（`Cargo.toml:36`），可用于 `ah up` 报告输出。

## 5. PR 范围 + 实施切片

### 5.1 实施切片

1. **M1 Core + Schema**
   - Files: `src/provider/fingerprint.rs` (NEW), `src/provider/mod.rs`, `src/db/schema.rs`, `src/db/sessions.rs`, `src/db/agents.rs`。
   - 内容：sorted-key deterministic JSON、SHA256、`config_hash` migration、query/update helpers。

2. **M2 Spawn Storage Hook**
   - Files: `src/rpc/handlers.rs`, `src/rpc/router.rs`。
   - 内容：spawn 成功后写 hash；新增 `session.realign` / `agent.realign` stubs 与 router tests。

3. **M3 `ah up` Audit**
   - Files: `src/bin/ah.rs`, `src/cli/mod.rs`, `src/cli/up.rs` (NEW), `src/cli/output.rs`。
   - 内容：`Cmd::Up { force }`，加载 config，计算 expected hash，调用 RPC，输出 NO_CHANGE/DRIFT/ORPHAN/NEW/SKIPPED_BUSY。

4. **M4 Realignment Pipeline**
   - Files: `src/rpc/handlers.rs`, `src/db/agents_lifecycle.rs`, `src/db/events.rs`。
   - 内容：五阶段事务、BUSY skip、force kill、全量 re-spawn、commit hash、审计事件。

### 5.2 估算

- LOC：约 650-950 LOC。
- 文件：约 10-14 个文件。
- 主要测试：新增 `tests/pr4e_up_fingerprint.rs`，必要时补少量 unit tests 于 `src/provider/fingerprint.rs`。

## 6. 验收场景 (Tests-First)

1. **NO_CHANGE 基线**
   - 场景：`ah start` 后立即 `ah up`。
   - 预期：报告 `Everything is up to date`，无 kill/spawn。
   - 命令：`CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint no_change_reports_up_to_date -- --test-threads=1`

2. **DRIFT: plugins changed**
   - 场景：Agent `plugins` 增加一个 ID-only 或 Git spec。
   - 预期：报告 `Agent a1 drifted: plugins changed`，触发 Agent 重建，sandbox 出现新 plugin symlink。
   - 命令：`CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint plugin_drift_realigns_agent -- --test-threads=1`

3. **DRIFT: hooks changed**
   - 场景：Agent `hooks.PreToolUse` 增加/替换脚本。
   - 预期：报告 `hooks changed`，重建后 provider settings 指向新 hook。
   - 命令：`CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint hook_drift_realigns_agent -- --test-threads=1`

4. **ORPHAN: config 删除 Agent**
   - 场景：DB 中有 `a2`，新 `ah.toml` 删除 `agents.a2`。
   - 预期：默认报告 `Agent a2 is no longer in config`，不 kill、不 spawn；带 `--force` 时调用 kill/cleanup，不 spawn 新进程。
   - 命令：`CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint orphan_agent_is_reported -- --test-threads=1`

5. **BUSY skip + --force**
   - 场景：`a1` 为 `BUSY` 且 hash drift。
   - 预期：无 `--force` 时报告 `SKIPPED_BUSY` 且不 kill；带 `--force` 时 kill 并重建。
   - 命令：`CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint busy_agent_skip_and_force_realign -- --test-threads=1`

6. **DRIFT: Master changed**
   - 场景：Master `cmd` / hooks / plugins 改变。
   - 预期：默认报告 Master DRIFT 且不重启；带 `--force` 时全量重启 Master pane，并写入新的 `sessions.config_hash`。
   - 命令：`CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint master_drift_audit_only_by_default master_drift_force_triggers_realign -- --test-threads=1`

7. **NEW: New agent in config**
   - 场景：`ah.toml` 加新 agent block，DB 无对应 row。
   - 预期：`ah up` 标 NEW + spawn 新 agent（不走 realign/kill），DB 写入新 agent + `config_hash`。
   - 命令：`CARGO_BUILD_JOBS=1 cargo test --test pr4e_up_fingerprint new_agent_is_spawned -- --test-threads=1`

## 7. 风险 + 已锁定决策

已锁定决策：Master 对齐默认只审计 drift，不自动重启；`--force` 才触发全量重启 Master pane。

| 议题 | 描述 | 影响 | 置信 | 推荐方向 |
| :--- | :--- | :--- | :--- | :--- |
| **7.2 Hash 差异报告粒度** | 深度 diff hooks/env 复杂，初版过细会拖慢实现。 | M | A | 初版字段级 diff，后续再做结构化彩色 diff。 |
| **7.3 并发 `ah up`** | 多个 realign 同时运行会竞争 kill/spawn/hash commit。 | H | B | 新增 per-session realign mutex；不要只依赖 DB CAS。 |
| **7.4 Commitment 失败** | 物理重建成功但 DB hash 写入失败会造成 false drift。 | H | B | 返回 CRITICAL，并写事件；用户可重跑 `ah up` 修复 hash。 |
| **7.5 完整 JCS 取舍** | 完整 RFC 8785 增加实现/依赖复杂度。 | L | A | PR4e 只做 sorted-key serde_json；明确测试 HashMap 顺序稳定即可。 |
