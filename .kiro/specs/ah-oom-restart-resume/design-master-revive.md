# ahd 存活 + master OOM 后自动复活续干设计

## 1. 目标与非目标

目标承接 PM 定调：ahd 尽量不被 OOM 杀；正常被杀的是 master；ahd 检测到 master 死亡后复活 master 继续干；workers 不动、不因 master 单独死亡被级联杀掉。

非目标：不解决 ahd 与 master 同时死亡的全局雪崩恢复。该双死场景没有外部 Harness 接手，属于显式未恢复残留；本设计通过 OOMScoreAdjust 把它压到低概率，而不是声称 ahd 死后仍能恢复。

## 2. 继承字段表

`sessions` 当前 schema 来自 `src/db/schema.rs:8-16`：

| 字段 | 当前定义 | 本设计是否改 | 说明 |
| --- | --- | --- | --- |
| `id` | `TEXT PRIMARY KEY` | 不改 | session 身份。 |
| `project_id` | `TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE` | 不改 | 项目关联。 |
| `master_pid` | `INTEGER NOT NULL` | 语义补齐，不改类型 | 当前 `create_session_sync` 写入 `0`，见 `src/db/sessions.rs:54-57`；本设计要求 master spawn/revive 后维护为当前 master pane pid，并用作 CAS。 |
| `master_pane_id` | `TEXT` | 不改 | 当前 `handle_session_spawn_master_pane` 会更新，见 `src/rpc/handlers/sessions.rs:242`；revive 后继续更新为新 pane。 |
| `status` | `TEXT NOT NULL DEFAULT 'ACTIVE'` | 不改 | 作为意外死亡 vs 有意关闭的主判据。 |
| `config_hash` | `TEXT` | 不改 | 继续表示 master 配置指纹。 |
| `created_at` | `INTEGER NOT NULL DEFAULT (unixepoch())` | 不改 | 创建时间。 |

## 3. [NEW] 字段

| 字段 | 类型/默认值 | 迁移 | 理由 |
| --- | --- | --- | --- |
| `[NEW] sessions.master_retry_count` | `INTEGER NOT NULL DEFAULT 0` | `ALTER TABLE sessions ADD COLUMN master_retry_count INTEGER NOT NULL DEFAULT 0` | 记录连续 revive 失败次数，支撑退避与 5 次熔断。 |
| `[NEW] sessions.master_next_retry_at` | `INTEGER NOT NULL DEFAULT 0` | `ALTER TABLE sessions ADD COLUMN master_next_retry_at INTEGER NOT NULL DEFAULT 0` | 下次允许 revive 的 unix epoch 秒，避免 crash loop 立即重启。 |
| `[NEW] sessions.master_generation` | `INTEGER NOT NULL DEFAULT 0` | `ALTER TABLE sessions ADD COLUMN master_generation INTEGER NOT NULL DEFAULT 0` | 区分旧 watcher、revive task、confirm timer 与 realign 产生的不同 master 实例；单靠 `master_pid` 会受 pid 复用与未维护历史影响。 |
| `[NEW] sessions.master_last_exit_reason` | `TEXT` | `ALTER TABLE sessions ADD COLUMN master_last_exit_reason TEXT` | 诊断字段，记录 `OOM_OR_CRASH`、`INTENTIONAL_KILL`、`REVIVE_FAILED`、`FUSED` 等，不参与核心 CAS。 |

以上都是加列，不改变现有字段类型与含义；不引入 `[BREAKING]` schema 修改。

## 4. 核心机制设计

### 4.1 master 死亡入口改为状态判定

当前 `spawn_master_pidfd_watch_task` 在 pidfd readable 后直接 cascade kill，见 `src/monitor/master_watch.rs:36-47`，随后 kill worker pane/session，见 `src/monitor/master_watch.rs:49-57`。本设计改为：

1. watcher 携带 `session_id`、`expected_pid`、`expected_generation`。
2. pidfd readable 后先查 `sessions.status/master_pid/master_generation`。
3. 若 `status != 'ACTIVE'`，按有意关闭处理，只清理该 generation 的 monitor key，不 revive。
4. 若 `status == 'ACTIVE'` 且 `master_pid == expected_pid` 且 `master_generation == expected_generation`，判定 master 意外死亡，启动 `revive_master_task`。
5. 若 CAS 不匹配，说明已有更新 master 或 realign 抢先成功，旧 watcher 退出但不得影响新 monitor。

### 4.2 MF-1：专用 revive 路径，不裸调 `handle_session_spawn_master_pane`

`handle_session_spawn_master_pane` 当前会直接 `ensure_session`/`spawn_window`，见 `src/rpc/handlers/sessions.rs:227-239`；它只更新 `master_pane_id`，见 `src/rpc/handlers/sessions.rs:242`，并注册无 generation 的 `master:<session_id>` key，见 `src/rpc/handlers/sessions.rs:260-262`。`monitor::register` 是覆盖插入，见 `src/monitor/mod.rs:64-69`；旧 watcher 退出时会 `remove` 同一个 key，见 `src/monitor/master_watch.rs:66`。

因此新增专用路径 `revive_session_master(session_id, expected_pid, expected_generation, cause)`：

1. 获取 `master_spawn_lock(session_id)`，覆盖 auto-revive、realign master spawn、手动 master respawn。
2. DB CAS：只在 `status='ACTIVE' AND master_pid=? AND master_generation=?` 时进入 revive；否则返回 `STALE`.
3. 清理旧 master runtime：best-effort kill 旧 `master_pane_id`，清理旧 monitor key，但 remove 必须带 generation 检查，不能让旧 watcher 删除新 key。
4. 复用 session 的 master sandbox 与 config，不重新创建新 CONFIG_DIR。
5. spawn 新 master pane，读取新 pane pid。
6. 单事务更新 `master_pane_id`、`master_pid`、`master_generation = old + 1`、`master_retry_count`、`master_next_retry_at`。
7. 用带 generation 的 monitor key 注册新 pidfd，例如 `master:<session_id>:<generation>`；诊断 `list_keys` 可继续兼容显示。

初始 `session.spawn_master_pane` 也应抽出共享的低层 `spawn_master_runtime`，但普通创建路径与 revive 路径入口分开：普通路径允许准备新 master HOME；revive 路径禁止重建 transcript 所在 CONFIG_DIR。

### 4.3 MF-2：revive 复用 master CONFIG_DIR/transcript，不重建

当前 master spawn 会在 `src/rpc/handlers/sessions.rs:201-218` 解析 master sandbox 并调用 `prepare_home_layout_with_extensions("claude", ..., HomeLayoutRole::Master, ...)`。该函数会创建 sandbox home，见 `src/provider/home_layout.rs:114-118`；Claude 分支会创建 `.claude`、`projects`、`session_env`，见 `src/provider/home_layout.rs:149-155`；会写内置规则与 settings，见 `src/provider/home_layout.rs:156-162`、`src/provider/home_layout.rs:555-580`；`ensure_json_file` 对已存在文件直接返回，见 `src/provider/home_layout.rs:970-978`。

实证结论：现有 Claude prepare 流程没有整体 `remove_dir_all(.claude)`，但它会重写 `CLAUDE.md`、settings、hook/plugin 链接，并重新物化 auth 链接。revive 的硬要求是复用旧 sandbox HOME 与 `.claude` CONFIG_DIR 中的 transcript/session state，不删除、不换路径。

设计约束：

1. revive 路径必须使用原 session 的 `resolve_sandbox_dir(state_dir, session_id, "master")`，不创建新的 role/id 目录。
2. revive 路径默认不调用完整 `prepare_home_layout_with_extensions`；只计算同等 env override：`HOME=<sandbox_home>` 与 `CLAUDE_CONFIG_DIR=.claude`，保持 `src/provider/home_layout.rs:164-167` 的效果。
3. 若后续实现需要补齐缺失 auth/rules，只能走幂等、保留 transcript 的 `prepare_existing_master_home_for_revive`，禁止删除 `.claude/projects`、conversation/transcript/state 文件。
4. tests-first 必须先放置 sentinel transcript 文件，触发 revive 后断言文件仍存在且 mtime/内容未被重建。

### 4.4 MF-3：续断点是未验证假设，不是 ahd 语义保证

新 master 命令默认是 `claude --dangerously-skip-permissions --continue /remote-control`，但“复活后接管在途 worker job”不是 ahd 目前保证的语义。RPC 暴露的是 `job.submit`、`job.wait`、`job.cancel`，见 `src/rpc/router.rs:34-36`、`src/rpc/router.rs:96-98`；`job.wait` 必须传已知 `job_id`，见 `src/rpc/handlers/jobs.rs:48-50`；`system.dump` 只导出 sessions/agents 等，不导出 jobs，见 `src/db/system.rs:65-83`。`event.subscribe` 有 backfill，见 `src/rpc/handlers/events.rs:14-20`、`src/rpc/handlers/events.rs:41-49`，但它不能证明复活后的 Claude 一定知道该从哪个 job/event 接回。

因此本文只设计 ahd 侧“master 复活 + workers/job 不被 ahd 破坏”的机制；端到端续断点闭合标为未验证假设：

1. dogfood 验证步骤：创建真实 session，派一个长耗时 worker job，确认 job 处于 `DISPATCHED`；kill master pane/process；观察 ahd revive；确认 workers 未被杀、job 未被标 FAILED；验证新 master 是否真正继续等待/处理该在途 worker job。
2. 判定标准：复活后 master 能基于 `--continue /remote-control` 自己恢复上下文并接回 job，才认为 PM 的“续干”端到端成立。
3. fallback：若 dogfood 证明 master 自身记忆不够，本设计不强行扩大 scope；后续新增 job 查询/恢复 RPC，例如按 session 查询 active jobs、按 last event id 恢复订阅、向 master replay unresolved job context。

### 4.5 MF-4：generation / master_pid-CAS 原语

所有 master 生命周期修改统一走 `try_claim_master_transition(session_id, expected_pid, expected_generation, action)`：

1. `master_pid` 保存当前 master pane pid；普通 spawn 与 revive 成功后都必须更新。
2. `master_generation` 每次成功创建新 master 实例递增。
3. pidfd watcher、revive task、60s confirm timer 都携带 `(session_id, pid, generation)`。
4. confirm timer 到期只在 `(pid,generation)` 仍匹配且 `status='ACTIVE'` 时把 `master_retry_count` 重置为 0。
5. revive task 只在 `(pid,generation)` 匹配时增加 retry/退避并 spawn；spawn 成功后递增 generation。
6. 旧 watcher 只能清理自己的 generation monitor key，不能删除当前 generation。

这解决当前 watcher 无 generation、`master_pid` 初始为 0 且 spawn 不维护导致的踩踏问题；相关现状见 `src/db/sessions.rs:54-57`、`src/rpc/handlers/sessions.rs:250-262`、`src/monitor/master_watch.rs:30-66`。

### 4.6 MF-5：realign 与 auto-revive 幂等互斥

`realign --force` 当前直接调用 master spawn，见 `src/rpc/handlers/realign.rs:101-110`。现有 `session_window_lock` 定义在 `src/rpc/handlers/sessions.rs:30-38`，但 agent spawn 才使用它，见 `src/rpc/handlers/agent.rs:171-184`，master 路径没有锁。

设计：

1. 新增 `master_spawn_lock(session_id)` 或复用 session 级 lock，但语义上明确覆盖 master spawn/revive/realign。
2. realign master force 与 auto-revive 进入同一个锁。
3. 锁内先做 DB CAS；auto-revive 用 expected dead pid/generation，realign 用当前 live pid/generation。
4. 若另一路已成功更新 generation，本路返回 `STALE`/`NO_CHANGE`，不再 spawn 第二个 master。

### 4.7 MF-6：所有有意退出先落 DB KILLED

`session.kill` 已经调用 cascade，而 cascade 先执行 `UPDATE sessions SET status = 'KILLED' WHERE id = ?1 AND status = 'ACTIVE'`，见 `src/db/system.rs:151-158`；但该入口在 cascade 前先 stop session anchor，见 `src/rpc/handlers/sessions.rs:81-83`，实现时应收敛为先 DB 标记、再任何 stop/kill。

`system.shutdown` 当前只是 50ms 后 SIGTERM 自身，见 `src/rpc/handlers/system.rs:10-16`，没有标记 sessions。必须新增 shutdown 前置步骤：

1. 单事务把所有 `ACTIVE` sessions 标为 `KILLED`，记录 `master_last_exit_reason='INTENTIONAL_KILL'`。
2. 再 stop anchors、kill panes/scopes、SIGTERM ahd。
3. master watcher 如果随后看到 master pidfd readable，因 session 已不是 `ACTIVE`，不得 revive。

## 5. OOMScoreAdjust 设计

ahd systemd 路径：在 `build_ahd_systemd_run_command_with_env` 生成的 `systemd-run --user --unit=ahd.service` 中追加 `--property=OOMScoreAdjust=-900` 或同级保护值；当前属性位置见 `src/cli/start.rs:47-56`。

master systemd 路径：在 `master_command_with_env` 生成的 `systemd-run --user --scope --collect` 中追加 `--property=OOMScoreAdjust=500` 或保持默认 0；当前 scope 构造见 `src/sandbox/systemd.rs:95-100`。master 仍带 daemon unit 依赖，见 `src/sandbox/systemd.rs:113-119`。

覆盖边界：该策略只覆盖 systemd-run 路径。`ah` direct-spawn fallback 不受 systemd 属性保护；若要覆盖 fallback，需要另做 `/proc/self/oom_score_adj` 或父进程写 pid 的方案，本设计不纳入第一阶段。

## 6. 熔断策略

连续 5 次 revive 失败后不抛 PM 决策，直接采用工程默认：

1. `sessions.status = 'FAILED'`。
2. `master_last_exit_reason = 'FUSED'`。
3. 发事件/日志报警，包含 session_id、retry_count、last error。
4. 级联杀 worker 回收资源，因为没有 master 的 worker 在低配机上会变成僵尸资源占用。

退避建议：第 1-5 次分别按 1s、2s、4s、8s、16s 或有上限指数退避写入 `master_next_retry_at`；60s confirm timer 成功后重置 `master_retry_count=0` 与 `master_next_retry_at=0`。

## 7. 测试计划

tests-first 单测：

1. master watcher 对 `ACTIVE` + matching pid/generation 不再 cascade kill，而是调 revive seam。
2. master watcher 对 `KILLED` session 不 revive。
3. `system.shutdown` 先把 active sessions 标为 `KILLED`，再触发 shutdown。
4. revive 成功后更新 `master_pid`、`master_pane_id`、`master_generation`，并注册 generation key。
5. 旧 watcher/timer 在 generation 不匹配时不清理新 key、不重置 retry。
6. 60s confirm timer 只对 matching pid/generation 重置 retry。
7. retry/backoff 超过 5 次后 session `FAILED`，并级联杀 worker。
8. realign 与 auto-revive 同时发生时只有一个 spawn 成功，另一个 CAS stale。
9. revive 复用 master HOME：预置 `.claude` transcript sentinel，revive 后文件不丢、不换目录。
10. OOMScoreAdjust 命令构造：ahd unit 带保护值，master scope 带默认/靶向值。

dogfood e2e：

1. 真 `ah start`，派长耗时 worker job。
2. kill master pane/process，不 kill ahd，不 kill worker。
3. 观察 ahd revive master。
4. 验证 worker 进程、agent state、DISPATCHED job 未被 master death 破坏。
5. 验证复活后的 Claude master 是否真的接回在途 job；这是 MF-3 的验收点。

e2e scope 只覆盖 ahd OOM/master 生命周期边界，不扩展到外部 Harness 或 ahd+master 双死恢复。

## 8. 风险与边界

1. 续断点风险：`--continue /remote-control` 是否足以接回在途 worker job 未验证；ahd 第一阶段只保证不主动破坏 worker/job。
2. 双死风险：若 ahd 与 master 同时被 OOM 杀，本设计不恢复残留；没有外部 Harness。
3. CONFIG_DIR 风险：Claude transcript 具体落点可能随 Claude 版本变化；测试必须用真实 `.claude` 下 sentinel 与 dogfood 验证兜底。
4. pid 复用风险：必须依赖 `master_generation` 与 `master_pid` 双条件，不能只依赖 pid。
5. systemd 覆盖风险：OOMScoreAdjust 只覆盖 systemd-run 路径，direct-spawn fallback 仍弱。

