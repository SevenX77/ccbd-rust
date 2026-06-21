# ah master cutover 继任 master 就绪门设计

## 1. 背景与痛点

当前 `ah master cutover` 已经有 `master_cutovers` 表和 PREPARING/SPAWNING/VERIFYING/ACTIVE 状态，但 VERIFYING 不是实际就绪门。表结构允许 `VERIFYING` 与 `ACTIVE`，也用唯一索引约束一个 session 只能有一个活动 cutover：`src/db/master_cutovers.rs:7`, `src/db/master_cutovers.rs:10`, `src/db/master_cutovers.rs:31`。

现有 cutover flow 会创建全新 session 和 cutover id：`src/rpc/handlers/sessions.rs:490`, `src/rpc/handlers/sessions.rs:491`；写 handoff bundle：`src/rpc/handlers/sessions.rs:561`；seed Claude conversation：`src/rpc/handlers/sessions.rs:570`；spawn declared workers：`src/rpc/handlers/sessions.rs:576`; 再把 cutover 从 PREPARING 推到 SPAWNING：`src/rpc/handlers/sessions.rs:594`。

真正问题在 spawn master 后：代码写入新 master pid/generation/pane 并进入 VERIFYING：`src/rpc/handlers/sessions.rs:613`，随后只打一条 readiness stub 日志：`src/rpc/handlers/sessions.rs:627`，再直接 `VERIFYING -> ACTIVE`：`src/rpc/handlers/sessions.rs:628`。这导致 ACTIVE 只证明进程被 spawn，不证明继任 master 已能接管 PM。

同时，cutover handler 是同步 RPC：`src/rpc/handlers/sessions.rs:472`, `src/rpc/handlers/sessions.rs:476`。RPC server 每连接 task 内直接 await dispatch：`src/rpc/mod.rs:41`, `src/rpc/mod.rs:59`；client 端 `rpc_call` 阻塞读完整响应且无显式 timeout：`src/cli/rpc_client.rs:115`, `src/cli/rpc_client.rs:137`, `src/cli/rpc_client.rs:150`。因此 readiness timeout 必须由 cutover 自身实现。

## 2. 核心契约

新契约：

`ACTIVE <=> 继任 master 已被证实能接管 PM`

对 Claude master，证实条件为：

- 继任 master 进程存活。
- 继任 master 交互 bootstrap 已走到末尾。
- 继任 master 能通过 `CCB_SOCKET` 回连 ahd。
- ack payload 的 `cutover_id` 等于 ahd 注入的 `AH_CUTOVER_ID`。
- ack handler 校验该 cutover 当前仍处于 `VERIFYING`。
- 若有 handoff，上报 ack 的动作发生在读取 handoff/接收 takeover 指令之后。

对 non-claude/custom/bash master，若无法执行 in-band ack，则只能降级为 Route A 存活/稳定探测；这种 ACTIVE 语义必须标注为 `readiness_mode = "probe"`，只代表存活级保证，不代表 PM 上下文已接管。

禁止 `ACTIVE-but-unverified`。ack 成功或 probe 成功才 ACTIVE；timeout、ack 校验失败、VERIFYING 期 master 死亡都进入 FAILED 并触发 scoped rollback。

## 3. 状态机改造

### PREPARING

职责：

- 创建新 session：`src/rpc/handlers/sessions.rs:501`。
- claim cutover，并初始写入 PREPARING：`src/db/master_cutovers.rs:62`, `src/db/master_cutovers.rs:92`, `src/db/master_cutovers.rs:101`。
- 注入 `AH_STATE_DIR`、`CCB_SOCKET`、`AH_CUTOVER_ID`、`AH_MASTER_HANDOFF`、`AH_MASTER_ROLE`：`src/rpc/handlers/sessions.rs:531`, `src/rpc/handlers/sessions.rs:534`, `src/rpc/handlers/sessions.rs:537`, `src/rpc/handlers/sessions.rs:539`, `src/rpc/handlers/sessions.rs:542`。
- 准备 master sandbox home；当前 master home layout 用 `"claude"`：`src/rpc/handlers/sessions.rs:289`, `src/rpc/handlers/sessions.rs:290`。
- 写 handoff bundle：`src/master_cutover.rs:43`, `src/master_cutover.rs:47`, `src/master_cutover.rs:83`。
- seed Claude conversation：`src/master_cutover.rs:88`, `src/master_cutover.rs:104`, `src/master_cutover.rs:116`。

### SPAWNING

职责：

- `PREPARING -> SPAWNING` 仍用 CAS：`src/rpc/handlers/sessions.rs:594`, `src/db/master_cutovers.rs:117`。
- spawn master pane：`src/rpc/handlers/sessions.rs:602`。
- 获得 pid/generation/pane，并把 spawn metadata 写入 cutover row：`src/rpc/handlers/sessions.rs:603`, `src/rpc/handlers/sessions.rs:608`, `src/rpc/handlers/sessions.rs:613`。

改造点：`spawn_prepared_master_pane` 仍可创建 pane、记录 runtime，但不能在这里 arm revive watcher。当前 watcher 在 spawn 内立即注册：`src/rpc/handlers/sessions.rs:408`, `src/rpc/handlers/sessions.rs:410`，必须推迟到 ACTIVE 后。

### VERIFYING

职责：

- 进入 VERIFYING 后启动 ahd 内部 readiness wait。
- Claude/ack 模式：等待 `master.ack_ready` RPC。
- Probe 模式：执行低层 pane pid/capture 稳定探测。
- timeout 建议默认 120s，可配置为 `[master] readiness_timeout_s`，允许 30-600s；Claude 首次 onboarding 可慢，120s 是默认，不是硬编码上限。

失败规则：

- timeout：`VERIFYING -> FAILED`。
- ack 校验失败：不改变状态或直接标 FAILED，按错误类型区分。错误 cutover_id/非 VERIFYING 只拒绝该 ack；当前 cutover 超时后统一 FAILED。
- VERIFYING 期新 master 进程死亡：FAILED + scoped rollback，不 revive。

### ACTIVE / FAILED

ACTIVE：

- 只在 readiness 成功后 CAS `VERIFYING -> ACTIVE`：`src/db/master_cutovers.rs:117`, `src/db/master_cutovers.rs:126`。
- ACTIVE 后 arm master pidfd watcher。
- `reap(old_master)` 才允许继续。

FAILED：

- 当前失败路径只把当前状态标 FAILED 并声明 old master left running：`src/rpc/handlers/sessions.rs:654`, `src/rpc/handlers/sessions.rs:656`, `src/rpc/handlers/sessions.rs:657`。
- 新设计要在 FAILED 前后执行 scoped saga rollback，且只清理本次 `session_id/cutover_id` 资源。

## 4. 六条需求落实

### 4.1 Route B ack 握手

[NEW] 新增 RPC method：`master.ack_ready`。

payload：

```json
{
  "cutover_id": "cutover-sess_...",
  "session_id": "sess_...",
  "pid": 12345,
  "readiness_mode": "ack",
  "handoff_path": "/.../cutovers/.../handoff.md",
  "observed_socket": "/.../ahd.sock"
}
```

必填字段：`cutover_id`。`session_id` 可由 cutover row 反查，但 CLI 可一并传入用于日志诊断。`pid` 可选，仅做诊断，不作为身份认证；身份边界来自本地 UDS、注入 env 和 cutover state。

router 落点：

- 在 `METHODS` 加 `"master.ack_ready"`；当前 method 白名单在 `src/rpc/router.rs:15`，`session.master_cutover` 在 `src/rpc/router.rs:19`。
- import 新 handler；现有 handler import 区在 `src/rpc/router.rs:3`。
- dispatch 分支加 `"master.ack_ready" => handle_master_ack_ready(...)`；现有 dispatch 从 `src/rpc/router.rs:78` 开始。

handler 落点：

- 放在 `src/rpc/handlers/sessions.rs`，原因是 cutover 状态、session、spawn metadata 都在该模块内聚合，现有 `handle_session_master_cutover` 也在此处：`src/rpc/handlers/sessions.rs:472`。
- handler 读取 `cutover_id`，DB 查询 cutover row，要求 state == `VERIFYING`，再写入 ack 事件。
- 为避免 handler 自己直接 ACTIVE，设计为：handler 只把 ack 送入 ahd 内部 `pending_ack` registry/channel，或写 `ack_ready_at` 字段；状态转换仍由正在运行的 cutover saga 完成，保持单写者。

CLI 落点：

- `MasterCmd` 当前只有 `Cutover`：`src/bin/ah.rs:145`, `src/bin/ah.rs:148`。
- 新增 `AckReady { cutover_id: Option<String> }`，CLI 默认从 `AH_CUTOVER_ID` 读取；也支持显式 `--cutover-id`。
- main dispatch 的 `MasterCmd` match 在 `src/bin/ah.rs:211`。
- 复用 `UnixRpcClient`：`src/cli/rpc_client.rs:68`；调用 `RpcClient::call`：`src/cli/rpc_client.rs:83`；底层 JSON-RPC 发送在 `src/cli/rpc_client.rs:115`。
- `resolve_socket_path_for_config` 已优先读取 `CCB_SOCKET`：`src/cli/rpc_client.rs:102`, `src/cli/rpc_client.rs:103`，满足继任 master 从 sandbox 回连 ahd。

### 4.2 ack 焊点

决策：Claude master 的第一阶段使用 ah 可控 handoff + master rules 文本指令作为软焊点；同时设计一个后续可升级的 hard hook：在 sandbox home 种受控 slash command/bootstrap 脚本。当前 1e 设计要求先落 Route B，因此主路径不选择 wrapper 包 `master.cmd`。

理由：

- `/remote-control` 只是默认命令字符串，不在本 repo 实现：`src/cli/config.rs:169`, `src/cli/config.rs:170`。不能把 ack 焊在不可控 slash command 内部。
- ah 已可靠控制 env：`src/rpc/handlers/sessions.rs:531` 到 `src/rpc/handlers/sessions.rs:542`。
- ah 已可靠控制 handoff bundle 内容：`src/master_cutover.rs:43`, `src/master_cutover.rs:51`, `src/master_cutover.rs:83`。
- ah 已可靠控制 Claude conversation seeding/fallback prompt：`src/master_cutover.rs:88`, `src/master_cutover.rs:146`, `src/master_cutover.rs:151`。
- ah 已可靠控制 master sandbox home，并能 materialize Claude master rules；master home layout 当前传入 `"claude"`：`src/rpc/handlers/sessions.rs:289`, `src/rpc/handlers/sessions.rs:291`，master role 下只为 Claude 写 master rules：`src/provider/home_layout.rs:309`, `src/provider/home_layout.rs:313`, `src/provider/home_layout.rs:321`。

落地点：

- 更新 `write_handoff_bundle` 的 handoff body，在 constraints 顶部加入强制 takeover 步骤：读取 handoff、确认接管、立即执行 `ah master ack-ready --cutover-id "$AH_CUTOVER_ID"`。
- 更新 Claude master rules，在 master role rules 中加入同样的第一步约束。
- 更新 `seed_claude_project_conversation` fallback prompt，让无 conversation seed 时第一提示也包含 ack 命令。

可靠性契约：

- 对 `master.provider = "claude"` 且使用 ah sandbox home 的 master，Route B ack 是产品契约；ah 只在收到 ack 后 ACTIVE。
- 该契约仍依赖 Claude 遵从文本/bootstrap 指令，因此要配 timeout；不 ack 就失败回滚。
- 若后续要硬保证，新增 home seeding：写入 sandbox 内受控 slash command 或 bootstrap script，并把默认 master cmd 指向该受控入口。wrapper 包 `master.cmd` 暂不选，因为它会改变用户自定义命令语义，且容易破坏 shell quoting。

### 4.3 non-claude / bash 降级契约

[NEW] 建议新增 `master.provider` 字段，默认推断：

- 显式 `master.provider = "claude"`：Route B ack。
- 未显式 provider 且 `master.cmd` 首词为 `claude`：兼容推断为 `claude`，Route B ack。
- 其他命令或 `master.provider = "bash" | "custom"`：Route A probe。

新增字段的原因：

- 当前 `MasterConfig` 没有 provider 字段，只有 cmd/enabled/hooks/plugins：`src/cli/config.rs:23`, `src/cli/config.rs:29`, `src/cli/config.rs:31`。
- 当前 cutover request 的 master params 也没有 provider：`src/rpc/handlers/sessions.rs:441`, `src/rpc/handlers/sessions.rs:444`。
- master home layout 当前硬编码 `"claude"`：`src/rpc/handlers/sessions.rs:289`, `src/rpc/handlers/sessions.rs:291`。

Route A 机制：

- 使用 spawn outcome 中的 pane id/pid。
- 复用 tmux pid/capture primitives：`src/tmux/session.rs:281`, `src/tmux/session.rs:582`, `src/tmux/session.rs:658`。
- 做一个 master 专用 probe，不直接复用 agent init probe task。agent init probe 绑定 `agent_id`、DB、provider、marker matcher、idle state：`src/provider/init_probe_task.rs:105`, `src/provider/init_probe_task.rs:109`, `src/provider/init_probe_task.rs:112`, `src/provider/init_probe_task.rs:116`。
- 可抽出稳定判定思想：连续命中才 ready；现有 `record_readiness_match` 使用 `STEADY_COUNT`：`src/provider/init_probe_task.rs:301`, `src/provider/init_probe_task.rs:303`。

降级 ACTIVE 语义：

- `readiness_mode = "probe"` 时 ACTIVE 只代表“pane pid 存活 + 可 capture + capture 稳定/非空”，不代表 handoff 已读，也不代表 PM 语义已接管。
- CLI 输出必须显示 degraded readiness，避免用户误以为是 Claude ack 级别保证。

### 4.4 VERIFYING 超时与 `--wait`

cutover 内部新增 `ReadinessWait`：

- 输入：`cutover_id`, `session_id`, `pane_id`, `new_pid`, `generation`, `mode`, `deadline`。
- Claude ack mode：监听 ack registry/channel，或者轮询 DB ack 字段。
- Probe mode：按固定间隔 capture pane，直到稳定或 timeout。
- timeout 到达：CAS `VERIFYING -> FAILED` 并 rollback。

默认 timeout：

- `master.readiness_timeout_s = 120`。
- 配置范围建议 30-600。
- 120s 兼容 Claude 首次启动、conversation load 和 `/remote-control` onboarding；太短会把慢启动误判为失败。

`--wait` 行为：

- 当前 request 有 `wait` 字段：`src/rpc/handlers/sessions.rs:461`, `src/rpc/handlers/sessions.rs:464`，CLI 传入 wait：`src/bin/ah.rs:273`, `src/bin/ah.rs:291`, `src/bin/ah.rs:302`。
- 当前代码丢弃 `wait` 和 `print_attach`：`src/rpc/handlers/sessions.rs:642`。
- 新行为：带 `--wait` 时 RPC 阻塞到 ACTIVE/FAILED，并用 CLI exit code 反映成败。
- 不带 `--wait` 时早返回 `cutover_id/session_id`，状态在后台 saga 继续；需要后续 `[NEW] ah master status --cutover-id` 查询，status 子命令不作为本设计必须落地项。

行为标注：这是 Step-4 charter 内部行为变更，因为 master 自托管就绪本来就是本阶段目标。

### 4.5 revive arm 时机修正

决策：主修正选择“推迟 watcher 注册到 ACTIVE 后”，并加一个防御性 cutover-state gate。

证据：

- 当前 spawn 内获得 pidfd 后立即 `monitor::register`：`src/rpc/handlers/sessions.rs:408`, `src/rpc/handlers/sessions.rs:409`。
- 同一位置立即 `spawn_master_pidfd_watch_task`：`src/rpc/handlers/sessions.rs:410`。
- 但 cutover 要到后面才进入 VERIFYING：`src/rpc/handlers/sessions.rs:613`, `src/rpc/handlers/sessions.rs:626`。
- watcher 收到进程退出后会 classify 并 revive：`src/monitor/master_watch.rs:60`, `src/monitor/master_watch.rs:61`, `src/monitor/master_watch.rs:62`。
- classifier 只查 `sessions.status/master_pid/master_generation`：`src/master_revival.rs:61`, `src/master_revival.rs:70`；只要 session status 是 ACTIVE 且 pid/generation 匹配就 Revive：`src/master_revival.rs:85`, `src/master_revival.rs:88`。

设计：

- `spawn_prepared_master_pane` 返回 pidfd 或可重新打开 pidfd 所需信息，但不注册 watcher。
- cutover readiness 成功并 CAS ACTIVE 后，调用新 helper `arm_master_revival_watch(session_id, pid, generation, pane, cmd, ...)`。
- VERIFYING 期 master 死亡由 readiness wait 观测到，直接 FAILED + rollback。
- 防御性 gate：`classify_master_death` 查询该 session 是否存在 `master_cutovers.state IN ('PREPARING','SPAWNING','VERIFYING')`；若存在，返回 IntentionalExit/Stale，不 revive。

这样 VERIFYING 期死亡语义统一为 cutover failed，而不是 revive。

### 4.6 scoped saga rollback

cutover 创建的是全新 session id：`src/rpc/handlers/sessions.rs:490`，handoff path 也在 cutover id 目录下：`src/rpc/handlers/sessions.rs:496`, `src/rpc/handlers/sessions.rs:498`, `src/master_cutover.rs:47`。因此 rollback 作用域必须严格限定为这个新 `session_id/cutover_id`。

可复用块：

- per-agent runtime cleanup：`src/agent_io/registry.rs:97`；它会 abort reader、删 fifo：`src/agent_io/registry.rs:102`, `src/agent_io/registry.rs:104`；kill agent tmux session：`src/agent_io/registry.rs:113`, `src/agent_io/registry.rs:114`；删 agent sandbox：`src/agent_io/registry.rs:120`, `src/agent_io/registry.rs:123`；清 marker/completion/parser/monitor：`src/agent_io/registry.rs:148`, `src/agent_io/registry.rs:151`, `src/agent_io/registry.rs:152`, `src/agent_io/registry.rs:153`。
- `cascade_kill_session_agents` 可按 session 找 active agents：`src/db/system.rs:395`, `src/db/system.rs:399`，并用 pidfd SIGKILL fallback：`src/db/system.rs:419`，但它会标 session KILLED：`src/db/system.rs:371`, `src/db/system.rs:374`，不适合作为 cutover rollback 的唯一入口。
- `session.kill` 组合了更多清理：cascade：`src/rpc/handlers/sessions.rs:105`；kill agent pane/session：`src/rpc/handlers/sessions.rs:112`, `src/rpc/handlers/sessions.rs:116`；删 agent/master sandbox：`src/rpc/handlers/sessions.rs:121`, `src/rpc/handlers/sessions.rs:124`；kill master tmux session：`src/rpc/handlers/sessions.rs:125`。但它是用户 session kill 语义，太宽，不能直接用于 cutover rollback。
- master pane 可用 `kill_pane`：`src/tmux/session.rs:637`；tmux session 可用 `kill_session`：`src/tmux/session.rs:642`。

新增 helper：`rollback_master_cutover_scope(ctx, cutover_id, session_id, outcome?)`。

逆序补偿：

1. 标记 cutover 正在失败，避免 ack late arrival 被接受。
2. 删除 `state_dir/cutovers/<cutover_id>` handoff bundle 目录。
3. 查询新 session 下 declared workers，逐个调用 `cleanup_agent_runtime_resources(agent_id)`；若 registry 无 entry，再用 `cascade` 的 pidfd/systemd 片段做 fallback，但不得调用整段 `session.kill`。
4. kill 新 master pane；仅使用 `master_cutovers.new_master_pane_id` 或本次 spawn outcome，不按 project_id 模糊杀旧 master。
5. 删除 `sandboxes/<session_id>/master`。
6. 移除该 session/generation 的 pidfd monitor key。
7. session row 标 FAILED/KILLED-internal，cutover row 标 FAILED 或 ROLLED_BACK；状态必须终态。

禁止事项：

- 不触碰 old master pid。
- 不 kill 旧 master tmux session。
- 不处理旧 session workers。
- 不按 project_id 广播式清理。

## 5. reap-gate 次序契约

本设计只定义契约，不实现 reap。

`reap(old_master)` 的前置 gate 是：

```text
master_cutovers.state == ACTIVE
```

因为新 ACTIVE 已恒等于“继任 master 已验证”。worker 全 IDLE 是 reap 的软加项，不是 ACTIVE 条件。原因：worker boot/recovery 不能和 master cutover 就绪死耦，否则某个 worker 慢启动会阻塞 PM 接管，扩大故障域。

后续 reap 实现必须 PM-gated：只有 ACTIVE 后才能考虑 old master teardown；且 reap 的 worker idle 判定只影响是否立即 reap 或延后 reap，不反向改变 cutover ACTIVE。

## 6. [NEW] / [BREAKING] 标注

| 项 | 类型 | 说明 |
| --- | --- | --- |
| `master.ack_ready` RPC method | [NEW] | 新增继任 master 上报就绪入口，注册在 `src/rpc/router.rs`。 |
| `ah master ack-ready --cutover-id` | [NEW] | 新增 CLI 子命令，默认读取 `AH_CUTOVER_ID` 与 `CCB_SOCKET`。 |
| `master.provider` | [NEW] | 建议新增配置字段，用于选择 ack/probe readiness mode；未配置时从 `master.cmd` 兼容推断。 |
| `master.readiness_timeout_s` | [NEW] | 建议新增配置字段，默认 120s。 |
| `ah master status --cutover-id` | [NEW/FOLLOWUP] | 非 `--wait` 异步查询需要，但不作为本设计必须实现。 |
| `VERIFYING -> ACTIVE` 语义 | [BREAKING charter-internal] | 从 stub 变成真实 readiness gate；属于 Step-4 master 自托管目标内行为变化。 |
| `--wait` 行为 | [BREAKING charter-internal] | 从被丢弃变成阻塞到已验证终态，失败时退出码非 0。 |
| VERIFYING 期 master death | [BREAKING charter-internal] | 从可能 revive 改为 FAILED + rollback。 |

## 7. tests-first 清单

下一步先写失败测试：

1. ack 成功：cutover 进入 VERIFYING 后收到 `master.ack_ready`，最终 ACTIVE，response 标明 `readiness_mode = "ack"`。
2. ack timeout：无 ack 到达，cutover FAILED，rollback 只清新 session 资源。
3. ack 校验：cutover 不存在、cutover 非 VERIFYING、cutover_id 不匹配时拒绝 ack，不能 ACTIVE。
4. VERIFYING 期死亡：新 master pid 退出时 cutover FAILED + rollback，不调用 revive。
5. watcher arm：ACTIVE 前不注册 pidfd watcher，ACTIVE 后才注册。
6. scoped teardown：新 session workers/master/handoff/sandbox 被清，旧 master/旧 workers 不被触碰。
7. `--wait`：成功时 0，FAILED/timeout 时非 0；不带 `--wait` 早返回 cutover id。
8. Route A fallback：`master.provider = "bash"` 或 custom cmd 走 probe，ACTIVE 输出 degraded readiness。
9. Claude ack焊点：handoff/fallback prompt/master rules 包含 `ah master ack-ready --cutover-id "$AH_CUTOVER_ID"`。

## 8. 风险与取舍

- 软焊点风险：Claude 可能不遵从 handoff/master rules，或 onboarding 阻塞。处理方式是 timeout + rollback；后续再升级 hard hook。
- wrapper 风险：直接包 `master.cmd` 会改变用户命令语义，尤其 shell quoting/custom command，当前不选。
- `master.provider` 新字段增加配置面，但比猜测命令更可维护；未配置时保留 cmd 首词推断以兼容旧配置。
- Route A 只能给存活级保证，不能假装 PM 已接管；必须在 CLI/API response 中暴露 degraded readiness。
- 推迟 watcher 注册会改变 VERIFYING 期 death 行为，但这是修复竞态所需；ACTIVE 后 revive 语义保持。
- rollback 不能复用 `session.kill` 整体入口，因为它按用户 session kill 语义过宽；需要新写 cutover scoped helper，严格以新 `session_id/cutover_id` 为边界。

## 9. Audit 收敛 (1f) 锁定决策 + nit 落实

a3 PM 替身 audit 结论: **零 must-fix, 设计可进 tests-first**。file:line 实质准确, 6 条需求 + reap-gate 机制完整, rollback 安全不变量正确, revive 修复充分。以下为收敛后**锁定决策**(覆盖前文相应处)与必须带进实施的约束:

1. **[锁定] ack 投递机制 = DB 字段, 不用 in-mem channel** (覆盖 §4.1 末两可表述): 继任 master 的 `master.ack_ready` 由 handler 写入 `master_cutovers.ack_ready_at` (+ `readiness_mode`) 持久字段; 正在运行的 cutover saga 轮询/等待该字段, 收到后由 saga (单写者) CAS `VERIFYING -> ACTIVE`。理由: 持久 (ahd 重启不丢) + 单写者 (handler 只记录, saga 只转换), 避免 in-mem channel 在重启丢 ack。

2. **[锁定] master rules = 首选主焊点** (强化 §4.2): `builtin::MASTER_RULES` materialize 进继任 master 的 `.claude/CLAUDE.md` (`src/provider/home_layout.rs:156,313,321`, 经 builtin.rs→`assets/builtin/master_rules.md`), 被 Claude Code 每次 session 启动作为宪法级 project instructions 加载、跨 /clear 与 context 压缩存活, **比一次性 seeded handoff conversation 更硬可靠**。因此: ack 指令以 MASTER_RULES 为**主**焊点, handoff body + fallback prompt 为**加固**。注: claude SessionStart hard hook (`home_layout.rs:160` 已有 `materialize_claude_hooks` 通道) 会在 master 读 handoff/醒入角色**之前**触发, 违反"ack 须在接管指令之后"契约, 故**不选** hook 作主焊点; 软文本路径对"上下文已接管后才 ack"反而正确。

3. **[实施第一不变量] rollback 绝不 kill_session(project)**: cutover 复用 `request.project_id` (`sessions.rs:504`), 故 `master_session_name(project_id)` 对新旧 master 是**同一个 tmux session** (新 master 是其中一个新 window)。rollback **只能** `kill_pane(new_master_pane_id)` 指定新 pane, **绝不** `kill_session(master_session_name(project_id))` —— 否则连旧 master 一起杀。这是实施期必须死守的第一不变量, 必配显式回归测试。

4. **[tests-first 补强]** (覆盖 §7): test 6 增加显式断言"rollback 后同一 tmux session 内的旧 master pane 仍存活"; 新增 probe-mode timeout 用例; 新增 late-ack (FAILED 后迟到 ack 被拒) 显式用例。

5. **[已知 followup, 本设计不实现但显式承认]** ahd 在 VERIFYING 期 (现被拉长到最多 120s) 崩溃 → cutover row 卡 VERIFYING + session 已 ACTIVE + 记了 new master pid + master pane 可能存活但永不 ack + 无 watcher + 无 readiness wait; 重启无 startup reconcile 收拾 (`reconcile_startup_sync` 只管 agents, `system.rs:515-517`) → 永久卡死 + 资源泄漏。rollback 仅在进程内 `Err` 路径跑, 覆盖不到 ahd 崩溃。**Followup**: ahd 启动时扫 in-flight (PREPARING/SPAWNING/VERIFYING) cutover 做 reconcile/rollback。本设计的 readiness gate 不依赖它, 但实施 PR 的 report 必须列此为已知 gap。

6. **[nit] 行号对齐**: `config.rs:23`/`sessions.rs:289` 等几处 off-by-few (实质论断"MasterConfig 无 provider 字段"/"master home 硬编码 claude"均正确), 实施时顺手对齐到精确行 (`config.rs:24-35` struct, `cmd` 在 :29; `sessions.rs:291` 是 "claude", :294 是 `HomeLayoutRole::Master`)。
