# Design: Step-4 宽版 master 自换 ccb -> ah

日期: 2026-06-15  
状态: 正式设计草稿，可进入 tests-first + 实施  
范围: 真 Master PM 进程从 ccb/外部终端切到 ah-managed master pane，并复用已合 master-revive 自救能力。

## 0. 目标与非目标

目标:

- 保留 1d 收敛后的 CORE: 蓝绿自举、flat-peer scope、resume=provider 对话记忆级。
- 当前旧 Master 自己触发 cutover: 起 ahd、创建/复用 session、拉起绿 Master、完成 handoff、旧 Master 自我放逐。
- 绿 Master 在 ahd 管理下运行: tmux master session + systemd scope + pidfd watcher + master-revive。
- master OOM/崩溃时沿用 corrected master-death: 先无条件 reap 本 session workers；若死亡前有 ActiveWork，则 revive master；复活后由 Master 基于 handoff/recovery 状态重新派发丢失任务。

非目标:

- 不反转已合 worker reap 语义。`src/db/system.rs:224-330` 已实现 master-death worker cleanup；本设计必须在这个语义内做 re-dispatch。
- 不恢复 master 死亡时发了一半的 CLI 命令或阻塞中的 `ah ask --wait` 进程。
- 不追求 ccb CLI 完全兼容。Master PM 后续使用 ah 原生命令，必要兼容另行设计。
- 不用外部 helper 脚本作为主切换机制；helper ccb 只作为短窗口 rollback。

## 0.5 继承字段表 (SOP-06)

| 项 | 现状实证 | 处置 |
| --- | --- | --- |
| `sessions.id/project_id/status/master_pid/master_pane_id/master_generation/master_retry_count/master_next_retry_at/master_last_exit_reason` | `src/db/schema.rs:8-20`; revive CAS 查 `master_pid/master_generation`: `src/master_revival.rs:61-118`; spawn 记录 `master_pane_id`: `src/rpc/handlers/sessions.rs:247-288` | 继承不动。cutover 使用同一 session 行记录绿 Master runtime。 |
| `agents/jobs/events/evidence` | `src/db/schema.rs:22-100`; master-death snapshot 查 active workers 和 queued/dispatched jobs: `src/db/system.rs:166-221` | 继承不动。OOM 后旧 worker 被 KILLED，丢失任务由新 Master 重新创建新 job。 |
| `session.spawn_master_pane` RPC | 注册: `src/rpc/router.rs:14-18,76-80`; handler: `src/rpc/handlers/sessions.rs:191-312`; `ah start` 内部调用: `src/cli/start.rs:152-165` | 继承为底层原语，不暴露裸 RPC 给用户。 |
| `ah start` / 默认入口 | 默认入口检查 nesting 后 start: `src/bin/ah.rs:223-238`;显式 `ah start` 不做 nesting guard: `src/bin/ah.rs:443-460` | 继承不动。新增 cutover CLI 调用 start/session 原语，不要求绿 Master 再裸跑 `ah` 默认入口。 |
| `ah ask/ps/pend/cancel/kill/logs/watch` | CLI 列表: `src/bin/ah.rs:36-106`; ask/pend/cancel/kill: `src/bin/ah.rs:463-531` | 继承不动。Master PM 规则改用 ah 命令，不在本设计改命令语义。 |
| `ah attach <agent_id>` | 只映射 agent session: `src/bin/ah.rs:87-90,374-386`; master/agent tmux 命名分离: `src/tmux/mod.rs:13-32` | [NEW] 扩展 attach 支持 master target。CLI 兼容原 agent 用法。 |
| `CCB_SOCKET` / `AH_STATE_DIR` state 解析 | socket 优先 `CCB_SOCKET`: `src/cli/rpc_client.rs:102-113`; state 优先 `AH_STATE_DIR/CCBD_STATE_DIR`: `src/state_layout.rs:16-47` | 继承解析顺序。[NEW] cutover spawn 绿 Master 时显式注入当前 ahd socket/state。 |
| master home layout | claude master home 注入 HOME/CLAUDE_CONFIG_DIR: `src/provider/home_layout.rs:142-167`; spawn master 只注入 home overrides: `src/rpc/handlers/sessions.rs:205-230` | 继承 auth/rules materialization。[NEW] 增加对话 handover seed，不依赖空沙箱 `--continue`。 |
| master systemd scope | `systemd-run --user --scope --collect`, workspace slice, daemon unit dependency: `src/sandbox/systemd.rs:88-123` | 继承不动。符合 flat-peer。 |
| master-revive 流水线 | lock/snapshot/reap/backoff/CAS/spawn: `src/monitor/master_watch.rs:99-286`; CAS/backoff/fuse: `src/master_revival.rs:95-185` | 继承不动。新增 cutover 后的 recovery prompt/state，供复活 Master re-dispatch。 |
| cutover fencing 状态 | 现无 DB 表；现有锁是进程内 `master_spawn_lock`: `src/master_revival.rs:377-385` 和 `session_window_lock`: `src/rpc/handlers/sessions.rs:31-42` | [NEW] DB 表 `master_cutovers` + CAS，禁止用工作区 `.ah_cutover_active` 作为唯一 fencing。 |
| cutover CLI/RPC | 现无 `ah master cutover` 或类似子命令: `src/bin/ah.rs:36-106` | [NEW] 新增产品化 cutover CLI 和 RPC/helper。 |

新增 schema 需走迁移并测试:

```sql
CREATE TABLE IF NOT EXISTS master_cutovers (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    old_master_pid INTEGER,
    new_master_pid INTEGER,
    new_master_generation INTEGER,
    new_master_pane_id TEXT,
    ah_state_dir TEXT NOT NULL,
    ah_socket_path TEXT NOT NULL,
    handoff_path TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    completed_at INTEGER
) STRICT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_master_cutovers_active
ON master_cutovers(session_id)
WHERE state IN ('PREPARING', 'SPAWNING', 'VERIFYING', 'ACTIVE');
```

状态枚举: `PREPARING`, `SPAWNING`, `VERIFYING`, `ACTIVE`, `ROLLED_BACK`, `FAILED`, `RELEASED`。

## 1. Bootstrap / Cutover 时序

新增 CLI:

```text
ah master cutover [--config <ah.toml>] [--wait] [--print-attach]
ah attach master [--session <session_id>]
ah attach agent <agent_id>
```

兼容要求: 现有 `ah attach <agent_id>` 保持可用，内部等价为 `ah attach agent <agent_id>`。

cutover 时序:

1. 旧 Master 在当前 ccb/外部终端执行 `ah master cutover --wait --print-attach`。
2. CLI 解析 socket/state，并调用 `ensure_daemon_running`。daemon 启动仍沿用 `src/bin/ah.rs:241-339`。
3. CLI 读取 ah config，创建或复用长期 session。底层仍走 `session.create` / `session.realign` / `session.spawn_master_pane`；`session.spawn_master_pane` 已存在于 `src/rpc/router.rs:14-18,76-80` 和 `src/rpc/handlers/sessions.rs:191-312`。
4. 创建 `master_cutovers` 行，状态 `PREPARING`，写入旧 master pid、ah state dir、ah socket path、handoff path。
5. 准备 handoff bundle 和对话 store seed，状态 CAS 到 `SPAWNING`。
6. 调用增强后的 master spawn helper 拉起绿 Master。spawn env 必须包含:
   - `AH_STATE_DIR=<当前 state_dir>`
   - `CCB_SOCKET=<当前 ahd.sock>`
   - `AH_CUTOVER_ID=<cutover id>`
   - `AH_MASTER_HANDOFF=<handoff bundle path>`
   - `AH_MASTER_ROLE=managed`
7. spawn 成功后记录 `new_master_pid/new_master_generation/new_master_pane_id`，状态 CAS 到 `VERIFYING`。
8. cutover CLI 等待绿 Master 发出 ready/ack 事件或检测 pane 输出中的 ready marker。超时则状态 `FAILED`，保留旧 Master 和 helper ccb，可 rollback。
9. 成功后状态 CAS 到 `ACTIVE`，打印:
   - `session_id`
   - `pane_id`
   - `tmux -S <socket> attach -t master_<project>`
   - `ah attach master --session <session_id>`
10. 旧 Master 只做自我放逐: 停止派 ccb/ah 任务，提示用户 attach 绿 Master，随后退出当前 ccb 会话或进入不可交互 wait。严禁执行 `ah kill --session`，因为 `session.kill` 会杀 agents、master pane 和 sandbox: `src/rpc/handlers/sessions.rs:74-140`。

### 1.11 收敛架构决策 — cutover 状态机收进 daemon 单 handler (a2 设计 + a1 工程实证, 2026-06-15)

a3 audit Batch B 抓到命门: §1 step 4 原写「CLI 创建 master_cutovers 行」, 但 **CLI/旧 master 进程没有 DB 句柄** (DB 在 ahd daemon 进程里), 且对话 seed 依赖的 `master_home` 是 daemon spawn 那一刻才在服务端算出来的 (`src/rpc/handlers/sessions.rs:214,224` → `src/provider/home_layout.rs:24,164`)。因此 CLI 侧编排既调不到 fencing CAS (Batch A 表沦为死代码), 也落不了 seed (MF2 命门在生产路径失效)。

派 a2 (设计第一性原理) + a1 (工程可行性勘查) 独立评估, **双方收敛到同一方案 (b)**:

**决策: cutover 状态机整体收进 daemon 侧单一 handler `session.master_cutover`。** CLI 变薄 (哑终端化): 只采集旧 master 上下文参数 + 单次 RPC 调用 + 打印 attach。

- **为什么 (b) 而非 (a) 加 claim/update RPC 让 CLI 逐步 CAS**: 旧 master 本质在自我替换, 对「逐步盯着自己被换」的可见性需求极弱; fencing 要 race-safe 必须在持 DB 的单进程里一次 CAS (跨进程非原子防不住竞态); seed 依赖服务端 `master_home`, CLI 越过 RPC 写服务端硬盘 = 抽象泄露; 失败回滚在单 handler 栈内 (Drop guard / 显式 `release(FAILED)`) 最干净, 无网络中断半状态残留。
- **时序语义不变**: §1 step 4-9 的状态流转 (PREPARING→SPAWNING→VERIFYING→ACTIVE / 失败 FAILED+保留旧 master)、§2 双轨 handover、严禁 `session.kill`、fencing 状态枚举全部保留; 仅「执行主体」从 CLI 进程移到 daemon handler。

**实施注意 (a1 工程勘查 file:line 实证)**:
- daemon handler 拿 `ctx: &Ctx` 含 `pub db: Db` (`src/rpc/mod.rs:15`); 现有 handler 已大量用 `ctx.db` (`sessions.rs:44,85`); CAS API 都是同步 `&Db` (`src/db/master_cutovers.rs:62`), handler 内可直接调。
- **坑1**: `update_master_cutover_state` 当前只更新 state, 不写 `new_master_pid/new_master_generation/new_master_pane_id` (`src/db/master_cutovers.rs:117`) — 需扩 DB API 写 new-master metadata。
- **坑2**: `handle_session_spawn_master_pane` 把 param 解析 + home materialize + tmux spawn + DB runtime + watch 全耦合在 JSON RPC 入口 (`sessions.rs:191`); cutover 要「seed 在 spawn 前」+「CAS 写新 pid/generation」更干净, 应抽一个 **typed 内部 helper** (先例: `session.realign` 已在 handler 内直接调 `handle_session_spawn_master_pane(json!(...), ctx)` `src/rpc/handlers/realign.rs:121`)。
- **坑3**: `seed_claude_project_conversation` 需 `old_home/master_home/cwd/handoff_path` (`src/master_cutover.rs:88`); `master_home` daemon 服务端算, 但 **`old_home` 必须由旧 master CLI 从自身环境传入** request params — daemon 的 `materialization_source_home()` 读的是 daemon 环境 HOME (`src/provider/home_layout.rs:790`), 不是旧 master sandbox HOME。
- 新增 method 样板成本低: router 白名单+match (`src/rpc/router.rs:14,76`) + handler re-export (`src/rpc/handlers.rs:30`), 模式与现有 handler 一致。
- **对 Batch B 的影响**: Batch B (commit c8d78ce) 的 CLI 侧 `run_master_cutover` + fake-client 测试随之重做为「薄 CLI + daemon handler」。feature 分支保留每个 commit, 不回退; Batch B2 重塑落点。

## 2. 对话 handover 机制

问题: `--continue` 不是跨沙箱魔法。旧 Master 的 Claude 本地会话位于旧 HOME/CLAUDE_CONFIG_DIR；ah master 使用独立 sandbox home，`src/provider/home_layout.rs:142-167` 会为 claude master 注入新的 HOME 和 `CLAUDE_CONFIG_DIR=.claude`，`src/rpc/handlers/sessions.rs:205-230` 只传 home overrides。因此空 ah master sandbox 内 `--continue` 可能开新会话。

设计采用“双轨 handover”:

1. Handoff bundle: 写到 state_dir，不写工作区根目录，路径形如 `<state_dir>/cutovers/<cutover_id>/handoff.md`。内容包括:
   - 当前目标: “你是接管后的 ah-managed Master PM”
   - 本轮 cutover id/session id/attach 命令/socket/state dir
   - 旧 Master 最近计划、已派任务、未完成决策、回滚说明
   - 重要约束: 使用 `ah ask/ps/logs/pend/cancel/kill`，不要使用 ccb；master OOM 后旧 worker 会被 reap，需要重新派发丢失任务
2. Claude conversation seed: 在 spawn 绿 Master 前，把旧 Master 当前 Claude project conversation store 复制或链接到 ah master sandbox 的真实 Claude 读取路径: `<master-home>/.claude/projects/<dash-escaped-abs-cwd>/`。
   - `dash-escaped-abs-cwd` 的命名契约: 取 workspace canonical absolute cwd，把路径分隔符 `/` 替换成 `-`。例: `/home/sevenx/coding/ccbd-rust` -> `-home-sevenx-coding-ccbd-rust`。本机 `~/.claude/projects/` 已实证存在该目录名。
   - 不能用 `workspace_trust_key` 当 projects key。`workspace_trust_key` 返回 raw canonical path 字符串，用于 `.claude.json` trust: `src/provider/home_layout.rs:775-781`；Claude projects root 只是 `claude_dir.join("projects")`: `src/provider/home_layout.rs:1189-1195`，代码里没有现成 dash-escape helper。
   - 实施需新增一个 `claude_project_dir_key_for_cwd(cwd)` 之类 helper，唯一职责是实现上述 slash -> dash 命名；seed 和测试都必须调用同一契约。
   - 首选: 只复制当前 workspace 最近会话 JSONL/metadata 到 `<master-home>/.claude/projects/<dash-escaped-abs-cwd>/`，不链接整个 `.claude`，避免污染 ah master rules/auth。
   - 若旧会话路径不可可靠定位，或只能定位到非 dash-escaped 目标，则 fallback 为 handoff bundle + first prompt，并把 cutover 标为“无 conversation seed”；不宣称 `--continue` 保留旧完整对话。
3. 启动命令: 保留默认 `claude --dangerously-skip-permissions --continue /remote-control`，但第一屏由 handoff bundle 明确要求新 Master 读取 `AH_MASTER_HANDOFF` 并回显接管确认。
4. revive 时: master_watch 复用同一 sandbox home 和原 `master_cmd`: `src/monitor/master_watch.rs:204-286`。因此 seed 只需在首次 cutover 做；后续 revive 复用已 seed 的 ah master sandbox conversation store。

测试必须覆盖: 不 seed 旧会话时 `--continue` 不被当作成功 handover；seed 后 conversation 文件必须落在 `<master-home>/.claude/projects/<dash-escaped-abs-cwd>/`，且这个 key 与 Claude 真实命名规则一致；handoff bundle 被写入并注入 env。只断言“sandbox 里有文件”不够，会把文件落到 raw `workspace_trust_key` 目录的错误实现假绿。

## 3. TTY 可见性与盲区控制

新增 attach 行为:

- `ah attach master --session <session_id>`: 查询 session 的 `project_id/master_pane_id`，attach 到 `master_session_name(project_id)`，必要时可精确 target pane。
- `ah attach agent <agent_id>`: 走现有 `agent_session_name(agent_id)`。
- `ah attach <agent_id>`: legacy path 保留。

盲区处理:

1. cutover CLI 在旧终端保持前台，直到绿 Master ready 或 timeout。
2. ready 前旧 Master 不退出；ready 后 CLI 打印 attach 命令并要求旧 Master 在最终回复中只输出 attach 指令和“我将停止派单”。
3. 绿 Master 第一条 handoff prompt 必须要求它:
   - 读取 `AH_MASTER_HANDOFF`
   - 运行 `ah ps`
   - 明确回复 “cutover accepted: <cutover_id>”
4. cutover CLI 只在看到 ready/accepted 后把 fencing 状态推进到 `ACTIVE`。
5. 如果用户未 attach，绿 Master 仍在 tmux pane 内可继续，但验收必须要求人工 attach 后能看到它的确认。

## 4. Cutover Fencing

不要用 `.ah_cutover_active` 工作区文件作为主 fencing。它不能跨进程强约束，也无法防止旧终端误派。

使用 DB/state_dir 强 fencing:

- `master_cutovers` 唯一 active index 防止同一 session 双 cutover。
- 状态变更全部用 CAS: `WHERE id=? AND state=?`。
- `ACTIVE` 后，ah CLI 在 dispatch 类命令前可读取 cutover 状态并输出当前 managed master 信息。此项只做提示，不阻止绿 Master 使用 ah。
- 旧 Master 的 handoff prompt 明确禁止再用 ccb 或 ah 派新任务。旧 Master 若要 rollback，必须执行显式 `ah master cutover rollback <id>`，把状态 CAS 到 `ROLLED_BACK`。
- rollback 只释放 fencing，不杀绿 Master；是否 kill 由人工或后续专门命令决定，避免误杀正在接管的 master。

最低实现顺序:

1. DB table + helper: create/claim/update/release active cutover。
2. cutover CLI 在 spawn 前写 `PREPARING`，spawn 成功后 `VERIFYING`，ready 后 `ACTIVE`。
3. `ah ask` 等 dispatch 命令在检测到非当前 managed master 环境且 active cutover 存在时打印 warning。强 hard-fail ccb 不在 ah 代码内可完全实现，需通过 handoff rules/alias 作为辅助。

## 5. Socket / Env 自洽

绿 Master 必须连接同一 ahd:

- `src/cli/rpc_client.rs:102-113` 说明 `CCB_SOCKET` 优先于 layout 推导。
- `src/state_layout.rs:16-47` 说明 `AH_STATE_DIR` 可覆盖默认 state。
- 当前 master spawn 只注入 home env: `src/rpc/handlers/sessions.rs:205-230`；因此 cutover 必须新增 env 注入。

设计:

- cutover CLI 解析当前 socket path 和 state_dir 后，将它们写入 `master_cutovers`。
- `session.spawn_master_pane` 增加可选 `extra_env` 参数，或新增内部 helper `spawn_master_pane_with_cutover_env`。默认旧调用不变。
- `extra_env` 经 `systemd::master_command_with_env` 的 `extra_env_vars` 进入 `env KEY=VALUE sh -lc ...`: `src/sandbox/systemd.rs:88-114,200-221`。
- 绿 Master 运行 `ah ask/ps/logs` 时优先使用 `CCB_SOCKET` 连接当前 ahd，不依赖 cwd/config 推导。

测试必须覆盖: cutover spawn command 中包含 `CCB_SOCKET` 和 `AH_STATE_DIR`；普通 `ah start` 不额外注入 cutover env。

## 6. PM Handoff 状态与 Re-dispatch 模型

master OOM 后的正确语义:

- 死亡检测后，`src/monitor/master_watch.rs:99-111` 先拿 lock、snapshot、clean workers。
- `src/db/system.rs:166-221` 的 snapshot 判断 ActiveWork/IdleNoWork；`src/db/system.rs:224-330` 负责真 cleanup。
- ActiveWork 才 revive；IdleNoWork 只 reap 不 revive: `src/monitor/master_watch.rs:122-130`。

因此验收和 Master 操作手册必须改成:

- 不是 “复活后查正在跑的 worker 继续等结果”。
- 而是 “复活后通过 conversation/handoff 状态识别上一轮任务可能因 master death 被 reap，运行 `ah ps/logs` 确认状态，然后重新派发丢失任务”。

handoff bundle 中必须有 `inflight_tasks` 段:

```text
inflight_tasks:
- request_id: optional stable id
  agent_id: a1
  prompt_summary: ...
  status_at_cutover: dispatched | queued | unknown
  redispatch_policy: if missing/failed/killed after revive, submit a new ah ask
```

re-dispatch 的幂等性建议:

- 新派任务使用新的 `request_id`，格式 `cutover-<id>-retry-<n>-<agent>`，不要复用已失败 job 的 request_id。
- Master 在重派前先 `ah ps` 和 `ah logs <agent>` 取证，避免重复派仍成功中的任务。注意 master-death 语义下同 session worker 大概率已 KILLED，但 cutover 期旧/新 master 双活窗口仍需人工确认。

## 7. Master-Revive 适配

复用现有实现:

- master pidfd watcher: `src/monitor/master_watch.rs:24-81`。
- revive lock/snapshot/reap/backoff/CAS/spawn: `src/monitor/master_watch.rs:99-286`。
- CAS/generation/backoff/fuse helper: `src/master_revival.rs:95-185`。
- master scope 用 workspace slice + daemon unit dependency: `src/sandbox/systemd.rs:88-123`。

需要新增的适配:

1. 首次 cutover spawn 和 revive spawn 共享同一 master sandbox home。当前 revive 已复用 `<state_dir>/sandboxes/<session>/master`: `src/monitor/master_watch.rs:210-223`。
2. cutover 首次 seed conversation store 到这个 sandbox；revive 不重复 seed，避免覆盖新 Master 后续对话。
3. revive 后通过 handoff bundle / env / conversation 让 Master 知道:
   - 我是 ah-managed Master
   - 上一次 master death 会导致 workers 被 reap
   - 如需完成在途 PM 目标，应重新派发任务
4. fuse/backoff 行为不变；fuse 后 session failed，不做无限重启。

## 8. Tests-first 拆解

### Batch A: schema + fencing

- `schema_has_master_cutovers_table_and_active_unique_index`: DDL 包含表和 partial unique index。
- `master_cutover_claim_is_single_active_per_session`: 同 session 第二个 active cutover 失败。
- `master_cutover_state_transitions_are_cas_guarded`: 错误旧 state 不能推进。

### Batch B: CLI/RPC cutover shape

- `ah_cli_has_master_cutover_subcommand`: clap/parse 覆盖新命令。
- `cutover_uses_start_session_then_spawn_master_with_env`: 假 RpcClient 验证调用顺序和 `extra_env`。
- `cutover_does_not_call_session_kill`: fake client 确认无 `session.kill`。
- `cutover_prints_attach_master_command`: 输出含 `ah attach master --session ...`。

### Batch C: handover

- `cutover_writes_handoff_bundle_to_state_dir`: 内容含 cutover id、session、socket、re-dispatch policy。
- `cutover_seeds_claude_project_conversation_into_dash_escaped_project_dir`: 给定 cwd `/home/sevenx/coding/ccbd-rust` 和旧 conversation store，seed 后文件必须位于 `<master-home>/.claude/projects/-home-sevenx-coding-ccbd-rust/`；同时断言 raw canonical path key 目录不存在或未被使用，防止把 `workspace_trust_key` 误当 projects key。
- `cutover_seed_target_matches_claude_continue_lookup_key`: helper 对不同绝对路径只做 slash -> dash，测试读取路径与 seed 路径相同；若可做轻量 harness，则模拟 `--continue` lookup 从 dash-escaped 目录读到目标 conversation。
- `cutover_falls_back_to_handoff_bundle_when_conversation_store_missing`: 缺会话文件、旧会话路径不可定位，或只能落到非 dash-escaped key 时，不假装 `--continue` 成功，必须降级到 handoff bundle + first prompt。

### Batch D: attach master

- `attach_master_maps_to_master_session_name`: `ah attach master` target 为 `master_<project>`。
- `legacy_attach_agent_still_maps_to_agent_session_name`: 现有 `ah attach a1` 行为不变。
- `attach_master_errors_when_no_master_pane`: 无 session/master pane 给清晰错误。

### Batch E: socket/env

- `spawn_master_pane_accepts_cutover_extra_env`: RPC 参数中的 `extra_env` 进入 command。
- `cutover_master_env_contains_ccb_socket_and_ah_state_dir`: 命令含 `CCB_SOCKET` / `AH_STATE_DIR`。
- `ordinary_start_master_env_unchanged`: 普通 start 不注入 cutover env。

### Batch F: revive acceptance / dogfood harness

- `active_work_master_death_reaps_worker_revives_master_and_requires_redispatch_marker`: ActiveWork case worker KILLED，master revived，handoff/recovery state 标需要 re-dispatch。
- `idle_master_death_reaps_without_revive`: case B 待命态不 revive，沿用 `src/monitor/master_watch.rs:122-130` 语义。
- ignored 真 scope dogfood:
  1. 绿 Master 在 ah pane 派 >10s 真 worker 任务。
  2. 外部 `kill -9` master pid。
  3. ahd 自动 revive。
  4. 复活 Master 识别旧 worker 已被 reap，重新派发该任务直至成功返回。
  5. 待命态 Master 被杀: reap workers，不 revive。

## 9. 验收定义

必须同时满足:

1. 冷启动或已有 ahd 下，旧 Master 执行 `ah master cutover --wait --print-attach`，得到 `ah attach master --session <id>`。
2. 用户 attach 后看到绿 Master 的 `cutover accepted: <id>`。
3. 绿 Master 在 ah pane 内能用 `ah ask` 派发 >10s 真 worker 任务。
4. 任务进行中外部 `kill -9 <master_pid>`。
5. ahd 按 corrected master-death 清理旧 worker，ActiveWork revive master。
6. 复活 Master 通过 handoff/conversation 说明自己重启，运行 `ah ps/logs` 取证，识别旧 worker 已 reap，并重新派发任务到成功返回。
7. case B: Master 待命态被 kill，ahd reap workers 但不 revive master。
8. `session.kill` 未参与 cutover 退出路径。
9. `CCB_SOCKET` / `AH_STATE_DIR` 指向同一 ahd，绿 Master 派单不连错 daemon。

## 10. 风险

| 风险 | 证据 | 影响 | 置信度 | 缓解 |
| --- | --- | --- | --- | --- |
| cutover 期旧/新 Master 双活导致双派 | 现无 cutover fencing；只有进程内锁 `src/master_revival.rs:377-385`、`src/rpc/handlers/sessions.rs:31-42` | 高 | 高 | DB active cutover + handoff prompt + 旧 Master 自我放逐；rollback 显式 CAS。 |
| `--continue` 跨沙箱不恢复旧会话 | master 独立 HOME/CLAUDE_CONFIG_DIR: `src/provider/home_layout.rs:142-167` | 高 | 高 | 显式 seed conversation store + handoff bundle；缺 seed 不算成功 handover。 |
| TTY 盲区 | `ah attach` 只支持 agent: `src/bin/ah.rs:87-90,374-386` | 高 | 高 | 新增 `ah attach master`；cutover CLI wait 到 green ready 再让旧 Master 放逐。 |
| ahd 自身重启时 master 兜底不足 | master-revive 依赖 ahd 存活的 pidfd watcher；startup reconcile 主要收尸 | 中高 | 中 | 后续补 startup reconcile: ahd startup 扫描 ACTIVE session 的 master_pid/pane，缺失且 ActiveWork/recovery marker 存在时触发 revive 或提示 manual `ah master recover`。本设计先列为必须 dogfood 风险，不阻塞 cutover MVP。 |
| Provider 并发/429 | 蓝绿窗口内两个 Claude `--continue` 可能短暂共存 | 中 | 中 | 旧 Master 在绿 Master accepted 后立即停止交互；cutover wait 设置短窗口；失败 rollback。 |
| worker 归属误判 | master-death 会无条件 reap session workers: `src/db/system.rs:224-330` | 高 | 高 | 设计和验收采用 re-dispatch，不依赖旧 worker 继续跑。 |
| helper ccb rollback 与 ah fencing 冲突 | 旧 PLAN 保留 helper ccb rollback；现代码无 ccb hard-fail | 中 | 中 | rollback 必须显式操作；handoff 后默认不再使用 ccb。强阻断 ccb 属于外部规则/alias，不放进 ah MVP。 |

## 11. 实施顺序

1. Schema/fencing helper tests-first。
2. `ah attach master` tests-first。
3. handoff bundle + conversation seed tests-first。
4. cutover CLI/RPC orchestration tests-first，先 fake client 证明不调用 `session.kill`。
5. socket/env 注入 tests-first。
6. recovery/re-dispatch marker tests-first。
7. ignored 真 scope dogfood，按验收定义跑。

## 12. 读码实证摘要

- ah 已有 master pane 创建底层: `src/cli/start.rs:152-165`, `src/rpc/handlers/sessions.rs:191-312`, `src/rpc/router.rs:14-18,76-80`。
- ah CLI 还没有 cutover/spawn-master 对外命令: `src/bin/ah.rs:36-106`。
- nesting guard 只在默认入口，`ah ask/ps/logs` 不走它: `src/bin/ah.rs:138-182,223-224,351-371`。
- `ah attach` 现只映射 agent: `src/bin/ah.rs:87-90,374-386`；master session 名为 `master_<project>`: `src/tmux/mod.rs:31-32`。
- `session.kill` 会杀 master pane/session，不可用于旧 Master 自我退出: `src/rpc/handlers/sessions.rs:74-140`。
- socket/state 解析需要显式注入: `src/cli/rpc_client.rs:102-113`, `src/state_layout.rs:16-47`。
- corrected master-death 已是先 reap 再 revive: `src/monitor/master_watch.rs:99-130`, `src/db/system.rs:166-330`。
