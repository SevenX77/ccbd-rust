# Design: ah PR-7 OOM Self-Restart + Codex/Agy Resume Completion

## §0.5 继承字段表

PR-7 继承 PR-6 已落地的 CRASHED recovery 基础，不推翻 claude `--continue`。

| 继承项 | 当前事实 | PR-7 处理 |
|---|---|---|
| `ProviderManifest.resume_args` | `src/provider/manifest.rs:5-11` 定义静态 `resume_args`; codex 为空 `:159-175`, claude 为 `["--continue"]` `:205-211`, antigravity 为空 `:223-228` | 保留，作为 claude 与未知 provider 的静态兜底；新增动态 hook |
| recovery argv 拼接 | `src/sandbox/systemd.rs:116-135` 在 `is_recovery` 时追加 `manifest.resume_args` | [BREAKING] 扩展为动态 `RecoverySpawn` args 优先，空时回退静态 `resume_args` |
| CRASHED realign 分支 | `src/rpc/handlers/realign.rs:156-158` 对 `running.state == "CRASHED"` 删除旧 row 后 `spawn_realign_agent(..., is_recovery=true)` | 保持 CRASHED 才恢复；在 spawn 时根据 provider home 计算动态 resume args |
| spawn 入口 | `src/rpc/handlers/agent.rs:68-89` 解析 sandbox dir 并 materialize home, `:91-99` 调 `wrap_command` | 在构造 command 前计算 recovery args；普通 `agent.spawn` 仍为非恢复 |
| startup reconcile | `src/bin/ahd.rs:56-60` 启动时调用 `reconcile_startup_with_tmux_socket` | ahd OOM 重启后继续补标 CRASHED，但必须保留可恢复 provider home |
| `session.realign` | `src/rpc/router.rs:79-83` 暴露 RPC；`src/cli/up.rs:29-52` 是唯一现有 CLI realign 入口 | PR-7 明确恢复触发者为操作者运行 `ah up` |

## §1 目标与非目标

目标：

1. ahd 被 OOM/SIGKILL 等非自愿退出后，由 systemd user service 有界重启。
2. ahd 重启后通过 startup reconcile 把失心跳 agent 标为 `CRASHED`，并保留 recoverable provider 的 sandbox home。
3. 操作者运行 `ah up` 后，现有 `session.realign` CRASHED 分支触发 provider 原生 resume，恢复 codex/antigravity/claude worker。
4. `ah start` 对同一 project 的既有 ACTIVE/recoverable session 做复用/接管，避免再次 mint 新 session 导致保留 home 孤儿化、`ah up` 多 session 选择死锁。
5. auth materialization 增加诊断清晰的 fallback ladder，区分宿主未登录和沙箱凭据挂载失败。

非目标：

- daemon 内部全自动恢复、ahd 重生 master、daemon 用 DB-only 配置自动 realign：显式切到另案。理由是 daemon config-blind。当前 agents schema 只有 `id/session_id/provider/state/state_version/pid/exit_code/error_code/created_at/sub_state/config_hash/updated_at`，见 `src/db/schema.rs:18-30`; 旧迁移样本同样没有 `env/hooks/plugins`，见 `src/db/mod.rs:379-390` 与 `:493-504`。realign 的 env/hooks/plugins 来自 ah.toml，经 `src/cli/up.rs:36-49` 发送。ahd 若自行 respawn，只能空 env/hooks/plugins，属于静默坏恢复；全自动必须先持久化完整 spawn spec，是更大的 [BREAKING] PR。
- 否决方案 (a): ahd 内部重生 master / daemon 自动 realign。除 config-blind 外，master pane 由 `session.spawn_master_pane` 根据 CLI 传入的 cmd/hooks/plugins 创建，见 `src/rpc/handlers/sessions.rs:185-226`; daemon 没有这些完整输入。
- 否决方案 (b) 原话的 “`ah start` 触发恢复”：命令错。现有 `ah start` 在 `src/cli/start.rs:54-116` 走 `session.create`、`session.spawn_master_pane`、`agent.spawn`，而不是 `session.realign`；正确操作者触发是 `ah up`。PR-7 会修正 `ah start` 的 session 可达性，但不把 `ah start` 定义为恢复触发命令。
- gemini provider 显式 defer：ah 中 gemini provider 已弃用中，PR-7 不实现 gemini resume。
- 不改变 `KILLED` 语义：用户显式 kill / graceful shutdown 不自动恢复。
- 不猜测 agy conversation-id 落盘格式；格式未确认前 antigravity 只用 `--continue` 兜底。

## §2 Grounding

### B1 / C3: OOM -> CRASHED -> `ah up` resume 桥

ahd 启动后在 `src/bin/ahd.rs:56-60` 调 `reconcile_startup_with_tmux_socket`。该函数在 `src/db/system.rs:894-905` 进入同步 reconcile。

startup reconcile 关键 phase：

| Phase | 代码 | 语义 |
|---|---|---|
| prompt pending preserve | `src/db/system.rs:521`, `:541-561` | 记录但不重启/不 crash `PROMPT_PENDING` |
| A select | `src/db/system.rs:522`, `:564-594` | 只选 `SPAWNING/WAITING_FOR_ACK/BUSY/IDLE` agent |
| B pid probe | `src/db/system.rs:523`, `:617-646` | `pid` 缺失或 `kill(pid,0)` 失败视为 dead |
| B2 alive IO | `src/db/system.rs:524-526`, `:648-694` | alive 但 fifo 缺失/打不开也转 dead |
| C crash dead | `src/db/system.rs:527`, `:711-752` | `UPDATE agents SET state='CRASHED'...` 并 fail dispatched jobs |
| D reregister alive | `src/db/system.rs:537`, `:755-780` | alive agent 重新注册 pidfd watcher |

实时 worker 崩溃检测靠 pidfd task：`src/monitor/agent_watch.rs:68-76` 确认进程死后调用 `mark_agent_crashed_with_exit`。生命周期层会 `wake_up()`，见 `src/db/agents_lifecycle.rs:164-169`，但 orchestrator 只查询 `IDLE` agent 派 job，见 `src/orchestrator/mod.rs:45-53`，不会碰 `CRASHED`。因此 Case A 单 worker OOM 的触发者同样是操作者运行 `ah up`。

`ah up` 是现有唯一 CLI realign 入口：`src/cli/up.rs:29-52` 解析 session 后调用 `session.realign` 并传入 master/agents 配置。`session.realign` 的 CRASHED 恢复分支在 `src/rpc/handlers/realign.rs:156-158`。`ah attach` 只 attach tmux session，见 `src/bin/ah.rs:179` 和 `:324-333`，不触发恢复。

master 是否应发现 peer CRASHED 后自动 `ah up`：不在本 repo 代码内。repo 内没有 master 周期 realign actor；这属于 PM rules / master 行为，PR-7 不实现。

### B2: ahd systemd service 现状与 BindsTo 张力

`scripts/install_ah.sh:13-50` 只安装 `ah`/`ahd` wrapper，没有 user service unit，也没有 `Restart=`。`ensure_daemon_running` 当前 socket 可连接即短路，见 `src/bin/ah.rs:238-245`; 否则直接 `Command::new(ahd_bin).spawn()`，见 `:273-281`。

现有代码能探测自身 service unit：`src/systemd_unit.rs:1-18` 从 `/proc/self/cgroup` 找 `ahd.service` 或 `ah-*.service`。agent/tmux scope 在有 daemon unit 时绑定：

- agent scope: `src/sandbox/systemd.rs:77-82` 追加 `BindsTo={unit}` 和 `PartOf={unit}`。
- tmux scope: `src/tmux/scope.rs:39-42` 追加同样属性。

`BindsTo/PartOf` 让 daemon unit 停止/失败时可能级联停止 agent/tmux scope。这与“旧 pane 原地接管”冲突，但与 provider resume 不冲突；resume 依赖 provider home 持久化。因此 PR-7 必须保留 recoverable home。

### B3: session 可达性缺口

现有 `handle_session_create` 每次 mint 新 session：`src/rpc/handlers/sessions.rs:43` 使用 `format!("sess_{}", Uuid::new_v4())`。`ah start` 每次调用 `session.create`，见 `src/cli/start.rs:54-62`。同 cwd 多个 running session 时，`ah up` 会报 `multiple running sessions match {cwd}; cannot choose one`，见 `src/cli/up.rs:82-89`。

`session.list` 当前来自 `list_session_summaries_sync`，它 LEFT JOIN agents 并统计 active_agents，见 `src/db/sessions.rs:187-205`，因此全 CRASHED 的 ACTIVE session 仍应可列出。另一个 `query_active_sessions_sync` 会排除全 CRASHED session，见 `src/db/sessions.rs:103-112`，不能作为 `ah up` 选择依据。

### B4: auth ladder 地基

现有 auth materialization 已有分流：

- provider auth 白名单在 `src/provider/home_layout.rs:14-22`。
- `materialize_sandbox_home_links` 在 `src/provider/home_layout.rs:294-300` 遍历白名单。
- `link_auth_file_into_sandbox` 在 `src/provider/home_layout.rs:338-349` 分流动态 OAuth 文件与普通 auth 文件。
- 动态 OAuth 文件枚举在 `src/provider/home_layout.rs:351-358`。
- 普通 auth symlink 在 `src/provider/home_layout.rs:950-972`; 动态 OAuth copy 在 `:975-990`。

PR-7 的 auth ladder 建在这套分流上。

### B5: graceful vs crash 语义

`master_watch` 对 master exit 做 session cascade kill，见 `src/monitor/master_watch.rs:36-57`。显式 session kill 会清理 agent/master sandbox home，见 `src/rpc/handlers/sessions.rs:85-108`。这些是自愿/管理型退出，不进入 resume。PR-7 的 recovery 只对 `CRASHED`，即 pidfd unexpected exit、startup reconcile dead pid、OOM/SIGKILL 等非自愿退出。

### C3 / SPAWNING_INTERVENTION 风险核验

`src/db/state_machine.rs:59-61` 的 `is_active_state` 只包含 `SPAWNING/WAITING_FOR_ACK/BUSY`，测试在 `:1634-1640` 明确断言 `SPAWNING_INTERVENTION` 非 active。marker idle 和 unknown timeout 依赖 active 判断：`src/db/state_machine.rs:315-321`, `:772-777`。另有 intervention 失败路径只接受 `SPAWNING_INTERVENTION`：`src/db/state_machine.rs:856-870`。

结论：这是 intervention 专用语义，不直接影响 PR-7 的 CRASHED recovery 路径。若 OOM 后 agent 停在 `SPAWNING_INTERVENTION`，startup reconcile 当前 phase A 不会选它，因为 `src/db/system.rs:574` SQL 不含该状态；标 out of scope 另案。

## §3 设计

### §3.1 Provider recovery hook [NEW]

新增 provider 层动态 hook：

```rust
pub fn compute_recovery_args(provider: &str, sandbox_home: &Path) -> Vec<String>
```

规则：

| Provider | 返回 |
|---|---|
| claude | `["--continue"]`; 可直接复用静态 `resume_args` |
| codex | 优先读取 `sandbox_home/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` 最新文件首行 `session_meta.payload.id`, 返回 `["resume", "<uuid>"]`; 失败退 `["resume", "--last"]` |
| antigravity | 当前返回 `["--continue"]`; 后续 spike 确认 `.gemini/antigravity-cli/conversations` 格式后升级为 `["--conversation", "<id>"]` |
| gemini | defer，返回空 |
| bash/unknown | 返回空 |

codex 文件选择：只扫描 `sandbox_home/.codex/sessions` 下 `rollout-*.jsonl`; 取 mtime 最新文件，同 mtime 按路径稳定排序取最后一个；只读首行 JSON，要求 `type == "session_meta"` 且 `payload.id` 为非空 UUID 字符串；失败 warn 并退 `resume --last`。

多会话歧义缓解：每-agent sandbox home 隔离是主保障。实现期可把 agent crash 前 `updated_at` 作为软锚点传给 hook；文件 mtime 与 DB `updated_at` 偏差过大时 warn，不阻断恢复。

### §3.2 Recovery args 传递 [BREAKING]

`resume_args` 保留为静态兜底；新增每次 spawn 的动态 `recovery_args`。

建议签名：

```rust
pub struct RecoverySpawn {
    pub is_recovery: bool,
    pub args: Vec<String>,
}
```

`handle_agent_spawn_with_recovery` 在 `src/rpc/handlers/agent.rs:68-89` 已能拿到 sandbox dir 并 materialize home。PR-7 应在 materialize 后得到 provider home path，调用 `compute_recovery_args`，再把结果传给 `systemd::wrap_command`。`wrap_command` 在 `src/sandbox/systemd.rs:116-135` 中改为：

1. 追加 `manifest.command`。
2. recovery 时追加动态 `recovery_args`。
3. 动态为空时追加 `manifest.resume_args`，保证 claude 兼容。

迁移范围：

- 生产调用 1 处：`src/rpc/handlers/agent.rs:91`。
- test 调用 16 处：`src/sandbox/systemd.rs:198`, `:228`, `:244`, `:264`, `:358`, `:376`, `:398`, `:416`, `:496`, `:522`, `:539`, `:556`, `:570`, `:583`, `:606` 加上新增动态 args test。现有 grep 结果包含 15 个旧测试调用，PR-7 新增 1 个动态优先级测试后为 16。
- router 不变；e2e payload 不变；persistence 不改 schema。

### §3.3 ahd systemd 自举 [NEW]

`ah start` / default action 的 `ensure_daemon_running` 改为幂等 systemd 自举：

1. socket 可连接时保持 `src/bin/ah.rs:238-241` 的短路。
2. socket stale 时删除旧 socket 后，若 user systemd 可用，用 `systemd-run --user --unit=ahd.service --property=Restart=on-failure --property=RestartSec=1s --property=StartLimitIntervalSec=60 --property=StartLimitBurst=5 ... ahd` 启动。
3. 启动前对 `ahd.service` 执行 best-effort `systemctl --user reset-failed ahd.service`，避免 OOM crash-loop 后 start-limit 残留永久拉黑。
4. 已在 `ahd.service` 或 `ah-*.service` 内时不递归自举，沿用直接启动/连接逻辑。

N1: `StartLimitIntervalSec` / `StartLimitBurst` 必须随 runtime unit 一起设置，OOM crash-loop 进入有界失败，而不是永久不可恢复。

### §3.4 startup reconcile 保留 recoverable home [NEW]

当前有两条删除 sandbox home 的路径都必须纳入 recovery-eligible 守卫。

路径一是 startup reconcile：`src/db/system.rs:528-535` 删除 dead agent sandbox home。PR-7 改为：

- 对 provider 属于 `codex/antigravity/claude` 的 dead candidate，不删除 sandbox home。
- 对 `bash/gemini/unknown` 可保持现有清理。
- `src/db/system.rs:406-442` 的 active refs 当前排除 `CRASHED/KILLED`，orphan scope reconcile 会把 CRASHED scope 视为 orphan。PR-7 需要给 recovery-eligible CRASHED 记录加入回收白名单，至少保护其 unit/home 元数据直到操作者 `ah up` realign 成功或恢复超时/失败。

路径二是单 worker OOM 的运行时 cleanup：`src/monitor/agent_watch.rs:68-82` 标 CRASHED 后调用 `cleanup(&agent_id).await`; `src/agent_io/mod.rs:28-30` 进入 `shutdown_reader`; `src/agent_io/registry.rs:75-100` 的 `cleanup_agent_runtime_resources` 当前会清 fifo、kill tmux session，并无条件调用 `remove_agent_sandbox_dir_sync` 删除 sandbox home。这条路径覆盖 Case A，必须同步加守卫，否则操作者稍后 `ah up` 时 codex `.codex/sessions/.../rollout-*.jsonl` 已被删除。

运行时 cleanup 守卫方向：

- recoverable provider (`codex/antigravity/claude`) 的 worker 转 `CRASHED` 时，runtime cleanup 仍可 abort reader、清 fifo、kill agent tmux session、取消 marker/log monitor，但必须跳过 `remove_agent_sandbox_dir_sync`，只保留 sandbox home。
- 非 recoverable provider (`bash/gemini/unknown`) 维持现有删除 home。
- 显式 `KILLED` / graceful 管理动作维持现有删除 home。
- 判据必须来自 DB 或显式上下文，不能只看 agent_id。推荐最小实现：给 `cleanup_agent_runtime_resources` 增加可选 `Db`/policy 参数，清理前查询 agent row，只有 `state == CRASHED && provider in {codex, antigravity, claude}` 时保留 home；`agent_watch` 在 mark CRASHED 成功后调用该 recovery-aware cleanup。显式 kill/session kill 路径继续调用删除 home 的 cleanup policy，或直接走现有 `remove_agent_sandbox_dir_sync`。

S1 home 回收契约：

- 同 `session_id + agent_id` 的 sandbox home 是 recovery residency。`ah up` 对 CRASHED agent 复用同 agent_id 路径读取 resume state。
- `session.kill` / explicit KILLED 仍可清理 home。
- realign 成功后，新进程继续使用同 home；失败或超过 recovery TTL 后，GC 可清理。
- orphan scope 可清理已死 scope，但不得清理 recovery-eligible home。

### §3.5 session 可达性修复 [BREAKING]

PR-7 必须改变 `ah start` 的同 project 行为：发现同 absolute_path/project 已有 ACTIVE/recoverable session 时，复用该 session 并引导/执行 `ah up` 语义，而不是再 `session.create` mint 新 uuid。

选择：`ah start` 复用并接管既有 ACTIVE session，然后走与 `ah up` 相同的 `session.realign` payload。

理由：

- 直接报错引导 `ah up` 虽更小，但用户 OOM 后最自然动作是 `ah start`; 报错仍会让恢复体验断裂。
- 继续 mint 新 session 会孤儿化已保留 provider home，并制造 `ah up` 多 session cwd 死锁。
- 复用 session 让 `ah start` 与 `ah up` 在 recoverable session 上收敛，避免同 cwd 多 ACTIVE session。

迁移路径：

- CLI start: `src/cli/start.rs:54-116` 先 `session.list` 按 canonical cwd 查 ACTIVE/recoverable session；若唯一匹配，调用 `session.realign` 而非 `session.create`/`agent.spawn`。
- RPC: 可新增 `session.find_by_path` 或让 `session.list` payload 足够复用；router 需覆盖新方法时新增 tests。
- integration: 更新 start tests 中“总是 session.create”的断言，新增已有 ACTIVE session 时调用 `session.realign` 的测试。
- e2e: 增加 “OOM 后 ah start 复用旧 session，不创建第二个 session，随后 ah up/realign 可恢复 CRASHED agent”。
- 文档/CLI 输出：start summary 标注 `session_id=<old>` 与 `action=reused` / `action=realigned`。

验证要求：

- ahd 重启后旧 session 仍 `ACTIVE` 且 `session.list` 可列出，即使 active_agents 为 0；`src/db/sessions.rs:187-205` 当前支持这一点，测试要锁住。
- 同 cwd 不再出现两个 running/recoverable session；`src/cli/up.rs:82-89` 的多 session 死锁不应被新 `ah start` 制造。
- 并发 `ah start` 的 TOCTOU 要容忍：后到者发现唯一 existing session 后 adopt/realign；若短暂出现竞争冲突，返回 deterministic error，不再创建额外 session。

### §3.6 Auth fallback ladder [NEW]

新增错误类别：

```rust
AUTH_PROVIDER_TOKEN_MISSING
AUTH_SANDBOX_MOUNT_FAIL
```

语义：

- `AUTH_PROVIDER_TOKEN_MISSING`: 宿主 `HOME` 中 provider 必需 auth 文件不存在或不是 file。
- `AUTH_SANDBOX_MOUNT_FAIL`: 宿主文件存在，但 symlink/copy 到 sandbox 失败，或 materialize 后目标不可读。

实现约束：

1. 不绕过 `PROVIDER_AUTH_WHITELIST` 和 `is_dynamic_oauth_auth_file`。
2. 动态 OAuth 文件继续 copy。
3. 非动态文件优先 symlink，失败时 copy 降级；copy 也失败才报 `AUTH_SANDBOX_MOUNT_FAIL`。
4. 所有操作幂等。

## §4 控制流改动表

| 场景 | 触发者 | is_recovery | 行为 |
|---|---|---:|---|
| 普通首次 `ah start` 且无 ACTIVE session | CLI start | false | `session.create` + master/agent spawn |
| `ah start` 发现同 project ACTIVE/recoverable session | CLI start | true for CRASHED agents | 复用旧 session，走 `session.realign` |
| 操作者 `ah up` | CLI up | true for CRASHED agents | 走 `session.realign`; CRASHED agent 追加动态 resume args |
| 单 worker OOM, ahd/master alive | pidfd 标 CRASHED；操作者 `ah up` | true | orchestrator 不自动恢复；`ah up` 恢复 |
| ahd/session OOM 后 systemd 重启 | startup reconcile 标 CRASHED；操作者 `ah up`/`ah start` | true | 保留 home，恢复 provider session |
| IDLE/BUSY drift realign | `ah up --force` 或 drift | false | 可重建，但不 resume |
| KILLED / graceful shutdown | 用户/管理动作 | false | 不恢复，可清理 home |

CRASHED 分支必须在 `delete_agent` 前保留足够上下文：`running.id/session_id/provider/updated_at/sandbox_home`。`src/db/agents.rs:140-144` 的 `delete_agent_sync` 只删 DB row，不删磁盘；真正危险点是 startup reconcile 的 sandbox 删除。

## §5 测试策略

test-first 红灯顺序：

1. provider recovery args unit：
   - codex 精确 uuid: 构造 sandbox `.codex/sessions/.../rollout-*.jsonl`，断 `["resume", uuid]`。
   - codex fallback: 无文件/坏 JSON，断 `["resume", "--last"]`。
   - antigravity: 断 `["--continue"]`。
   - gemini: 断空并在测试名体现 defer。
2. `sandbox::systemd` unit：
   - recovery 时动态 args 优先于静态 `resume_args`。
   - claude 无动态 args 时仍追加 `--continue`。
   - 非 recovery 不追加任何 resume args。
3. `db::system` startup reconcile unit：
   - dead codex/antigravity/claude agent 标 `CRASHED` 后 sandbox home 仍存在。
   - dead bash/非 recoverable provider 维持既有清理策略。
   - recovery-eligible CRASHED home 不被 orphan reconcile/GC 清掉，直到 TTL/失败。
4. `agent_io` runtime cleanup Case A unit:
   - 单 worker 经 `agent_watch`/`cleanup_agent_runtime_resources` 路径转 `CRASHED` 后，codex/antigravity/claude sandbox home 仍存在。
   - 同一路径下 fifo、tmux session、marker/log monitor 等 runtime 资源仍被清理。
   - bash/gemini/unknown 或显式 KILLED/graceful cleanup 仍删除 sandbox home。
5. `cli::up` / realign integration：
   - CRASHED codex realign command 包含 `resume <uuid>`。
   - codex 无 uuid 时包含 `resume --last`。
   - antigravity CRASHED realign 包含 `--continue`。
   - IDLE/BUSY drift 不包含 resume args。
6. `cli::start` [BREAKING] cutover:
   - 无 existing session 时仍创建新 session。
   - 同 cwd 存在唯一 ACTIVE/recoverable session 时，不调用 `session.create`，改调 `session.realign`。
   - 同 cwd 多 session 时给 deterministic error，不再创建第三个 session。
   - session.list 对全 CRASHED ACTIVE session 仍返回，active_agents=0。
7. ahd systemd 自举 unit：
   - socket healthy 时短路。
   - stale socket + systemd 可用时生成 `systemd-run --user --unit=ahd.service --property=Restart=on-failure`，含 `StartLimitIntervalSec` / `StartLimitBurst`。
   - 已在 `ahd.service` / `ah-*.service` 时不递归。
   - reset-failed best-effort 调用失败不阻断普通启动。
8. auth ladder unit：
   - 动态 OAuth 文件走 copy。
   - 普通 auth symlink 失败后 copy 降级。
   - 源文件缺失返回 `AUTH_PROVIDER_TOKEN_MISSING`。
   - 源存在但目标不可写返回 `AUTH_SANDBOX_MOUNT_FAIL`。

## §6 风险与取舍

- 恢复不是零人工：PR-7 的触发者是操作者 `ah up` 或修订后的 `ah start` 复用 session 后 realign。daemon 全自动切另案，因为当前 schema config-blind。
- `ah start` 行为变更是 A 类 [BREAKING]，但属于“OOM resume 续断点”的必然组成；否则 provider home 虽保留但不可达。
- codex 多会话歧义：每-agent sandbox home 隔离是主要保障；mtime 最新是当前最小可靠策略。DB updated_at 双锚定可先 warn 不 fail。
- `BindsTo` 杀旧 scope：与“进程级原地接管”冲突，但与 provider resume 不冲突；关键是保留 provider home。
- antigravity 精确 id 未确认：PR-7 只承诺 `--continue`; `--conversation <id>` 作为 spike 后续补。
- auth ladder 从静默 best-effort 改为可诊断错误，可能暴露过去被吞掉的宿主登录问题；这是期望行为。
- `SPAWNING_INTERVENTION` 不在 active/startup reconcile 候选集中：本 PR 标 out of scope，另案处理。

## §7 实施边界

本设计允许 PR-7 修改：

- provider recovery args hook。
- `RecoverySpawn` / `wrap_command` / spawn recovery 参数传递。
- startup reconcile sandbox home 清理策略与 recovery-eligible GC 白名单。
- `agent_io` 运行时清理路径的 recovery-eligibility 守卫，包括 `registry.rs::cleanup_agent_runtime_resources` 及 `agent_watch` 触发的 CRASHED cleanup。
- ahd runtime systemd self-start、reset-failed、start-limit 配置。
- `ah start` 同 project ACTIVE/recoverable session 复用，并对该 session 走 realign。
- home_layout auth fallback ladder。
- 对应 unit/integration/e2e/CLI 文档测试。

本设计不允许 PR-7 修改：

- daemon 内部全自动 realign / ahd 重生 master。
- gemini resume。
- `KILLED` / graceful shutdown 自动恢复。
- agy conversation-id 未确认格式的猜测解析。
- `SPAWNING_INTERVENTION` 状态机语义。
