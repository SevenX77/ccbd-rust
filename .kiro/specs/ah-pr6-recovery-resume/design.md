# Design: ah PR-6 ERROR Recovery + Claude Worker Resume

## §1 Goal + Scope

PR-6 的目标是修复 `session.realign` 对 `CRASHED` agent 的观测盲区，并在恢复 Claude Agent Worker 时真实追加 `--continue`，让 provider 在同一 sandbox home 中续接已有会话。范围限定为 Claude worker recovery：不实现 Codex/Gemini resume，不处理 Master CLI cmd drift，不做 session-level aggregate status，不做仓库命名重构。

`KILLED` 不进入恢复范围。`KILLED` 表示用户或 daemon 显式终止，默认不自动 resume；PR-6 只处理异常崩溃语义，即 `running.state == "CRASHED"`。

## §2 Schema 改动

### a. ProviderManifest 新增 `resume_args` [NEW]

在 `src/provider/manifest.rs:5-20` 的 `ProviderManifest` 增加字段：

```rust
pub resume_args: &'static [&'static str],
```

字段使用静态 slice，避免 `Vec<String>` 的运行时分配。所有 manifest 静态实例都必须填值：`bash` (`manifest.rs:138-150`) 为 `&[]`，`codex` (`:153-177`) 为 `&[]`，`gemini` (`:180-194`) 为 `&[]`，`claude` (`:197-211`) 为 `&["--continue"]`。`get_manifest` 的 unknown fallback (`:216-230`) 也填 `&[]`。

### b. `running_agent_hashes` 排除集变更 [BEHAVIOR]

当前 `running_agent_hashes` 在 `src/rpc/handlers.rs:629-654` 查询：

```sql
state NOT IN ('CRASHED', 'KILLED')
```

PR-6 改为只排除 `KILLED`，让 `CRASHED` 行进入 realign 比对：

```sql
state != 'KILLED'
```

该变更不改返回类型，但改变 realign 可见 agent 集合。`KILLED` 仍被排除，避免主动终止节点被自动恢复。

### c. `wrap_command` 签名加 `is_recovery: bool` [BREAKING]

当前签名在 `src/sandbox/systemd.rs:8-16`：

```rust
pub fn wrap_command(..., manifest: &ProviderManifest, extra_env_vars: &HashMap<String, String>) -> Vec<String>
```

PR-6 新签名追加 `is_recovery: bool`。现有调用默认传 `false`；只有 `handle_session_realign` 的 `CRASHED` 恢复分支传 `true`。`wrap_command` 需要把该信号继续传给 `command_with_env_prefix` (`systemd.rs:108-123`)。

grep 当前影响面：生产调用 1 处 `src/rpc/handlers.rs:717`，`src/sandbox/systemd.rs` 单测调用 12 处 (`:177`, `:205`, `:220`, `:239`, `:331`, `:348`, `:369`, `:386`, `:465`, `:490`, `:506`, `:526`)。这些调用都要显式传 `false`，并新增至少一个 `true` 的单测覆盖 Claude `--continue`。

### d. `spawn_realign_agent` 签名加 `is_recovery: bool` [BREAKING]

当前签名在 `src/rpc/handlers.rs:582-588`：

```rust
async fn spawn_realign_agent(ctx, session_id, agent, expected_hash, killed_before_spawn)
```

PR-6 增加 `is_recovery: bool`，调用点在 `handlers.rs:458` 的 NEW 路径传 `false`，`handlers.rs:505` 的普通 drift/force realign 传 `false`，新增 `CRASHED` recovery 分支传 `true`。

### e. `handle_agent_spawn` 内部信号入口 [BEHAVIOR]

`handle_agent_spawn` 当前公开 RPC 函数签名位于 `src/rpc/handlers.rs:670`，并在 `:684-685` 保留 `agent_exists` 重复保护。PR-6 不放宽该检查，不让普通 `agent.spawn` 覆盖已有 row。

实现上应新增内部 helper 或给内部 spawn 路径增加显式信号，使 `spawn_realign_agent(..., is_recovery)` 能调用到带 `is_recovery` 的 spawn 实现；router 的 `agent.spawn` (`src/rpc/router.rs:79`) 继续走 `is_recovery=false`。CRASHED recovery 必须先 `delete_agent`，再 spawn，避免触发 `AGENT_ALREADY_EXISTS`。

## §3 控制流改动

`handle_session_realign` 当前从 `handlers.rs:439` 读取 `running_agents`，在 `:454-466` 无 row 时走 NEW，在 `:467-473` hash 一致时直接 `NO_CHANGE`，在 `:475-520` drift/force 时 delete + spawn + `REALIGNED`。

PR-6 增加一条优先级高于 `NO_CHANGE` 的分支：

| 条件 | 旧行为 | 新行为 |
|---|---|---|
| DB 无 row | NEW spawn | 不变，`is_recovery=false` |
| `state == IDLE` 且 hash 一致 | NO_CHANGE | 不变，不 spawn |
| `state == BUSY` 且非 force | SKIPPED_BUSY | 不变，不 spawn |
| hash 异且可 realign | REALIGNED | 不变，`is_recovery=false` |
| `state == CRASHED` | 旧 SQL 看不见，误走 NEW 后撞 `AGENT_ALREADY_EXISTS` | REALIGNED-RECOVERY，不论 hash 是否一致，`is_recovery=true` |
| `state == KILLED` | 看不见 | 仍看不见，不恢复 |

`is_recovery` 明确定义为 `running.state == "CRASHED"`，不是“DB 有 row”。这能防止 IDLE/BUSY drift realign 误注入 `--continue`，保持 PR-3 case_06-09 的行为边界。

spawn event reason 采用现有 `DRIFT_REALIGN`，不新增 `RECOVERY` reason。case_11 可通过 `status=REALIGNED`、CRASHED 前置状态、PID 变化、marker 文件含 `--continue` 来区分恢复路径；如需更细分，可在 event payload 增加 `is_recovery: true`，但 `reason` 保持 `DRIFT_REALIGN` 以减少 schema 扩散。

## §4 case_11 测试改造

`tests/ah_full_e2e_realign_extra.rs:971-1001` 的 `case_11_error_recovery_known_gap` 目前断言 JSON-RPC error `AGENT_ALREADY_EXISTS`。PR-6 将其翻转为成功恢复断言，并保留 grand tour ignored 测试入口 `grand_tour_realign_extra_matrix`。

fake Claude 脚本位于 `tests/ah_full_e2e_realign_extra.rs:495-515`。新增 marker 行为：启动时若存在 `GRAND_TOUR_RESUME_ARG_MARKER`，把 `"$@"` 写入该文件。测试在 case_10 让 `a_crash` 进入 `CRASHED`，case_11 再 realign 同一 agent，断：

- `session.realign` 返回 `statuses[a_crash].status == "REALIGNED"`。
- `a_crash` 回到 `IDLE`，PID 与 crash 前 PID 不同。
- marker 文件存在且内容包含 `--continue`。
- `agent_spawned` 事件出现，reason 仍为 `DRIFT_REALIGN`。
- case_06-09 不变：IDLE/BUSY drift 不应带 `--continue`，因为 `is_recovery` 只对 CRASHED 为 true。

## §5 影响面 + 迁移路径

`ProviderManifest` 结构新增字段会影响所有 struct literal：4 个 manifest 静态实例和 `get_manifest` fallback，另有 `src/rpc/handlers.rs` 单测里的手写 `ProviderManifest` 构造点需要补字段。

`wrap_command` 签名 fan-out：`src/rpc/handlers.rs:717` 生产调用传入 real recovery signal；`src/sandbox/systemd.rs` 12 个现有单测调用传 `false`。新增单测覆盖 `wrap_command(..., is_recovery=true)` 时 Claude command 最终包含 `--continue`，而 Codex/Gemini/Bash 不追加参数。

`command_with_env_prefix` 在 `src/sandbox/systemd.rs:108-123` 负责最终 provider argv 拼接。它应在 `manifest.command` 后、返回前，按 `is_recovery` 条件追加 `manifest.resume_args`，确保 systemd 和 unsafe-no-sandbox 两条路径行为一致。

`spawn_realign_agent` 当前只有两个调用点：NEW (`handlers.rs:458`) 和 drift/force (`:505`)。PR-6 新增 CRASHED branch 后有第三条调用路径；`handle_agent_spawn` 的公开 RPC router (`src/rpc/router.rs:79`) 和现有单测调用都继续是非恢复。

`agents_lifecycle` 不需要 schema 修改。CRASHED 的写入仍由 `mark_agent_crashed_with_exit_sync` (`src/db/agents_lifecycle.rs:57-63`) 进入 `mark_agent_crashed_sync`，其 SQL 在 `:85` 把 active agent 更新为 `CRASHED`；`STATE_CRASHED` 常量在 `src/db/state_machine.rs:24`。

## §6 LOC 估算

- B1 SQL + `handle_session_realign` CRASHED 分支：约 50 LOC，含 unit tests。
- B2 信号透传：约 80 LOC，覆盖 `spawn_realign_agent`、内部 spawn helper、`wrap_command`、`command_with_env_prefix`、调用点和现有单测签名。
- B3 `ProviderManifest.resume_args` 字段 + 4 个 manifest + fallback 填值：约 30 LOC。
- B4 case_11 改造 + fake Claude marker 验证：约 120 LOC。

合计约 280 LOC，落在 research §8 的 250-350 LOC 区间内。

## §7 风险 + 缓解

- 风险：`wrap_command` / `ProviderManifest` 签名变更漏改调用点。缓解：这是编译期错误，`cargo test` 会直接捕获。
- 风险：CRASHED hash 一致时仍走 `NO_CHANGE`。缓解：在 `handle_session_realign` 中把 `running.state == "CRASHED"` 分支放在 hash 一致判断之前。
- 风险：IDLE/BUSY drift 被误加 `--continue`。缓解：`is_recovery = running.state == "CRASHED"`，普通 REALIGNED 传 `false`。
- 风险：放宽 `agent_exists` 破坏普通 `agent.spawn` 重复保护。缓解：不改 `agent_exists` 语义；recovery 走 `delete_agent -> handle_agent_spawn(is_recovery=true)`。
- 风险：KILLED 被自动恢复。缓解：`running_agent_hashes` 仍排除 `KILLED`，不进入 recovery 分支。
