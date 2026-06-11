# ah 真 dogfooding 验收测试矩阵

## A. 配置隔离 (Configuration Isolation)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| A1 | Agent ENV 隔离 | `ah attach a1` 然后 `env \| grep -E "CLAUDE_CONFIG_DIR\|CODEX_HOME\|GEMINI_CLI_HOME"` | 各自指向 `/tmp/ahd/.../sandboxes/a1/` 等隔离路径，绝不指向 `~/.claude/` 或混串。 | 防止 a1 的设置串染给 a2 或覆盖系统全局配置。 |
| A2 | Master/Agent 身份隔离 | Master 终端中执行 `env \| grep CCB_CALLER_ACTOR`；Agent Tmux 中执行同样命令。 | Master 无此变量，Agent 必须有且等于其 `agent_id`。 | 决定了 Claude 走 PM `CLAUDE.md` 规则还是 Worker 铁律。 |
| A3 | OAuth Credential 同步 | 登录 a3(claude)，在 a3 的 sandbox 里 `cat .claude/.credentials.json`，然后看 a1/a2。 | a3 生成了 token，a1/a2 通过 symlink 或复制获得了同样有效 token。 | 一次登录，全局可用，但又不破坏 ENV 隔离。 |
| A4 | 敏感文件防误删 (Migration) | 在 a2(gemini) 的 sandbox 放一个假的旧 config JSON，触发 `ah up` 或强制重启。 | 旧的 plain-text token 不被 provider 内置的 migration 删掉或覆盖掉全局文件。 | 过去发生过 gemini-cli 删 plaintext 凭证的事故。 |

## B. kill/exit 进程清理 + 无泄漏 (Process & Resource Cleanup)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| B1 | Master 异常退出 (Cascade) | 启动 master 和 `ah start`。通过 `kill -9 <master_pid>` 模拟 OOM 或强制断开。 | 1. `ah ps` 显示所有 agents 变为 `KILLED`。2. `tmux ls` 里无 `ccbd-tmux-*` session。 | 防主控掉线留孤儿，这是导致 VPS "两周必崩" 的首因。 |
| B2 | Agent 子进程(孙进程)清理 | 让 a1 执行 `bash -c "sleep 1000 &"` 后，调 `ah kill a1`。 | `ps -ef \| grep sleep` 找不到对应进程。cgroup 级的 systemd scope 被销毁。 | Agent 后台任务不准在 kill 后变成僵尸或孤儿，吃光 PID 限额。 |
| B3 | Daemon(ahd) 崩溃恢复 | `kill -9 $(pgrep ahd)`，然后重启 `ahd`。 | 1. 之前的 tmux session 和 scope 全成孤儿。2. 启动后自动跑 `reconcile_startup`。3. `tmux ls` 旧 session 全清，`systemctl --user list-units` 无旧 scope。 | 守护进程自己崩是常态，必须能自愈不堆积雪球。 |
| B4 | 物理残骸清理 | `ah kill --session <id>` 后，检查 `/tmp/ahd/` 或配置的 state_dir。 | `sandboxes/<session>/` 目录被删，`pipes/<agent>.fifo` 被删。 | FIFO 不清会导致下次同名启动卡死，目录不清吃磁盘。 |
| B5 | 优雅退出 vs 强杀 | 分别对 ahd 发送 SIGTERM 和 SIGKILL。 | SIGTERM 走 `cascade_kill` 有序清理；SIGKILL 走 B3 的重启自愈清理。 | 验证 Systemd stop 时是否触发正确 Drop/Signal hook。 |

## C. SOP 执行 (SOP Execution)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| C1 | Ask/Pend 顺畅度 | `ah ask a1 "echo OK" --wait` | 主控不挂起，无 `command ccb ask cancel` 救场，直接返回包含 `OK` 的 `COMPLETED`。 | 核心调度能力。 |
| C2 | 多并发任务互不干扰 | 同时跑 `ah ask a1 "sleep 5"` 和 `ah ask a2 "sleep 5"`，连续 `ah pend`。 | SQLite 中 `jobs` 表两条记录互不阻塞，a1 和 a2 分别完成。 | 验证并发下 `pubsub` 不会把 a1 的事件误判给 a2。 |
| C3 | 隐式 Output Chunk | a1 执行长任务时，`ah logs a1` | 能实时看到 tmux pane 内容，不管任务完没完成。 | 主控可观测性。 |

## D. 生命周期推进 + 检测 (Lifecycle & States)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| D1 | CRASH 恢复 (Provider 死亡) | `ah attach a1`，然后在里面直接 `kill -9 $$` 杀掉 provider bash。 | 1. `ah ps` 显示 `CRASHED`。2. `ah up` 后，它自动变为 `SPAWNING -> IDLE`，产生新 PID。 | provider 假死/真死是痛点，恢复机制必须管用。 |
| D2 | STUCK 检测与逃逸 | 强行向 a1 tmux 里 `send-keys` 一直占住 prompt 不让出 marker，干等 300s (或环境 override 的 30s)。 | `ah ps` 自动变成 `STUCK`，主控收到 push 事件。 | 解决 "Thinking hang 14m+" 无通知盲区。 |
| D3 | Prompt 交互打断 | a1 进要求用户输入的停顿 (`PROMPT_PENDING`)，主控跑 `ah cancel`。 | a1 的 job 取消，回到 `IDLE` 或被 `KILLED` 重生。 | 防交互锁死整个调度队列。 |

## E. 隐蔽角落 / 异常路径 (Obscure Corners)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| E1 | daemon 未启动时交互 | 不起 `ahd`，直接执行 `ah ps` 或 `ah ask a1 ...` | CLI 返回友好的 `DaemonNotRunning` 错误，不是 hang 住或 panic。 | 基础容错。 |
| E2 | Ask 幽灵 Agent | `ah ask not_exist_a99 "hello"` | 立即报错 "Agent not found"，且不在 SQLite 留下幽灵 Job 记录阻塞队列。 | 验证路由白名单与 SQL 约束。 |
| E3 | Kill 已死 Agent | `ah kill a1`，再执行一次 `ah kill a1` | 第二次正常返回，不崩溃，底层 `mark_agent_killed_sync` 幂等生效 (返回 changes=0)。 | 重复操作的防抖。 |
| E4 | Stale Socket 反复建 | 不杀 daemon，强行删 `ahd.sock` 后新起一个 `ahd`。 | `reconcile_startup` 探测到另一个 ccbd 存在，警告退出，或者强行踢掉上一个并接管（看目前设计）。 | 防双脑裂 (split-brain) 写坏同一个 SQLite。 |
| E5 | 极大包投递截断 | `ah ask a1 "$(cat large_1MB_file.txt)"` | Tmux 里 provider 收到完整内容，无 bash `Argument list too long` 错误，没被 tmux buffer 截掉尾巴。 | 解决旧 ccb 的长 Context 投递损坏 Bug (TD-008)。 |

## F. 资源泄漏 / 压力 (Resource Limits)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| F1 | 反复启停泄漏探测 | 跑脚本循环 100 次: `ah start; ah kill --session` | 1. `ls /tmp/ahd/sandboxes` 和 `pipes/` 空。2. `systemctl --user list-units` 无泄露。3. 内存持平。 | 长寿项目最怕慢性 OOM 和 tmp 文件刷爆 inode。 |
| F2 | SQLite WAL 膨胀 | `ah watch a1` 1 小时不断刷大量 log。 | `ahd.sqlite-wal` 文件不会无限增长，在设定的 checkpoint 后合并入主文件。 | 数据库自身管理问题。 |

## 我认为最高风险的 5 个角落 (优先测)

1. **Master 退出的 Cascade 信号丢失**: SSH 会话断开引发 SIGHUP，导致 `ah start` (master) 异常掉线。如果 `master_pidfd_watch` 没兜住这个，整个 session 就成孤儿。
2. **`systemd-run` 孙进程逃逸**: Agent 内部启动的守护进程 (如 rust-analyzer / tsserver)。当 `ah kill` 时，这些是否随着 scope cgroup 被内核杀掉，或者它们 `nohup` 逃逸了？
3. **Daemon Panic 时的 Tmux 残留**: 当 `ahd` 因为某种 bug 触发 Rust panic，`Drop` (如 SandboxDirGuard) 是否能有效清理环境？
4. **长耗时操作与 UDS 超时**: `ah up` 正在物化多个 provider 的 Sandbox (涉及大文件复制)，客户端 socket timeout 报错了，但 daemon 还在做事。
5. **并发 `Ask` 的 SQLite Busy**: 多个 master 或并发脚本高频写入事件/证据时，SQLite 的 `SQLITE_BUSY` 锁死 daemon 线程。

## 真启动姿势建议 (基于 e2e 脚本)

要在 playground 真起并验证，需严格按照以下干净顺序：
1. **清理环境**: `pkill -f ahd; pkill -f ah; rm -rf target/dev_state; tmux kill-server` (确保全净)。
2. **起守护**: `CCB_ENV=dev AH_STATE_DIR="$(pwd)/target/dev_state" ./target/release/ahd &` (带正确环境)。
3. **起项目**: 建一个 `test_project`，写 `ah.toml`。
4. **配 Master 身份**: 开个新终端，模拟 master，**不要**导出 `CCB_CALLER_ACTOR`。执行 `ah start --wait`。
5. **多窗口观测**:
    - Win 1: `ah ps` (持续刷新看状态)
    - Win 2: `systemctl --user list-units '*ccbd*'` (看 cgroup scope)
    - Win 3: `sqlite3 target/dev_state/ahd.sqlite "select * from events order by seq_id desc limit 10;"` (看真脉搏)
6. **实操**: 在 Master 端 `ah ask a1 "xxx" --wait`，触发上面设计的边角测试。

---

# 实测结果 (Live Dogfooding Findings, 2026-06-01)

> 主控 (Master PM) 真跑 `ah` 二进制做产品级 dogfooding 的物理实证记录。真相来源 = 进程树 / tmux / systemd / 文件系统, 不信 ccb/ah 状态自报。
> dogfooding 用 **真 `ah` 二进制** (不是拿 ccb 测 ah — 那是"用病人测医生")。hermetic bash provider 只能验 spawn/lifecycle/cleanup (B/F); ask-reply (C) 与配置/OAuth (A1/A3/D3) 必须真 provider (codex/gemini/claude), 一次一个, 见下。

## 通过 dogfooding 找到并修掉的真实缺陷

| # | 缺陷 | 类型 | 物理实证 | 修复 |
|---|---|---|---|---|
| BUG-1 | 项目目录 basename 含 `.` → 所有 agent spawn 失败 (`TMUX_COMMAND_FAILED`) | 实现缺陷 | `master_session_name` 未过滤, tmux `-t` 把 `.` 当 window.pane 分隔符 → 会话建得出却寻址不到 | `4051f59` `sanitize_tmux_name` 单一契约边界过滤 + 单测 |
| BUG-2 | `ah up` 100% 坏 (硬编码 session_id `"default"` → `IPC_INVALID_REQUEST`) | 实现缺陷 | 真 session 是 `sess_<uuid>`; up 从不 `session.list` 解析真 id | `53df604` `resolve_realign_session_id` 走 session.list + 单测 |
| BUG-3 | 每次 master 死亡 + 每次 `ah stop` 泄漏 `ahd-session-<sessid>.service` systemd anchor 单元 | 实现缺陷 | anchor 回收没 co-locate 在 session→KILLED 转换处: `cascade_kill_session_agents` (master_watch 触发, 也是 `ah stop` kill master 后触发) 标 KILLED + 回收 agent scope 但**不回收 anchor**; 仅 `handle_session_kill` 回收 | `6f6c69c` cascade `Some(daemon_marker)` 分支补 `stop_session_anchor_with_runner` (门控 daemon_marker) + ahd shutdown net 二级兜底; 见 §F1 |

## B. 进程清理 — 实测

- **B1 单 agent kill** ✅ PASS: `ah kill a1` → `state=KILLED`; a1 inner bash + fifo 进程都被 reaped; `agent_a1` tmux 窗口消失; **a2/a3 仍 IDLE 未受牵连** (无误 cascade 到 sibling)。
- **B (daemon teardown via `ah stop`)** ✅ PASS (进程/tmux/socket/scope 层): 0 孤儿进程 (ahd + tmux server + master/a2/a3 inner bash 全 reaped); tmux server 死; Unix socket 文件删除; `ahd-tmux-<hash>.scope` 消失。
- **B (systemd 单元层)** ✅ PASS (BUG-3 修复后): `ah stop` 回收 per-session anchor 单元, 0 残留 (见 §F1 修复 + 行为层复验)。

## F. 资源泄漏 — 实测 (§F1 = BUG-3)

`ah start`×多轮 → `ah stop` 后, `systemctl --user list-units 'ahd-*'` 残留 6 个 `ahd-session-sess_*.service`。
逐个 `systemctl --user show` 实证: `ExecStart=/usr/bin/true` + `RemainAfterExit=yes` 的 oneshot anchor, **ControlGroup 为空 → 0 进程 / 0 cgroup 内存**。
→ **不是进程/内存/fd 泄漏, 是 systemd 单元表泄漏** (注册被 `active(exited)` 永久 pin 住, 反复 start/stop 无限累积)。
矩阵 F1 ("100× start/kill 无泄漏"): 进程/内存层一直 **PASS**, systemd 单元层修复前 **FAIL** → 修复后 **PASS**。
**真根因 (比初判更深)**: session 的 ACTIVE→KILLED 转换只发生在一处 — `src/db/system.rs:146 cascade_kill_session_agents_with_runner_sync` 的 `UPDATE ... status='KILLED'` (handle_session_kill 也 delegate 到它)。该 cascade 标 KILLED + 回收 agent scope 但**从不回收 session anchor**。`master_watch` 在 master 死时触发它 → 每次 master 死亡都泄漏一个 anchor; `ah stop` 会 kill master tmux session 从而触发同一 cascade, 所以也泄漏。
初版 shutdown-only 修 (ahd.rs `cleanup_tmux_resources` 查 `status='ACTIVE'`) **被竞态打败**: cascade 先把 session 标 KILLED, shutdown net 再查 ACTIVE 就查不到。journal 实证序列: `received SIGTERM` → `master_watch: cascading session kill` → (无 anchor stop)。
**修复 (`6f6c69c`)**: 把 anchor 回收 co-locate 到 cascade 的 `Some(daemon_marker)` 分支 (`stop_session_anchor_with_runner`, best-effort, 门控 `daemon_marker.is_some()` — anchor 只在 daemon 跑 systemd 下创建, 正是 daemon_marker=Some 时)。保留 ahd shutdown `cleanup_session_anchors` 作二级兜底 (覆盖 ACTIVE-但无 cascade 的边角)。不删 handle_session_kill 既有回收。
**行为层复验 (PASS)**: systemd-launched ahd (under_systemd=true) → `ah start` 复用 daemon 建 anchor (1) → `ah stop` → anchor 回收 (0), 系统级 0 残留; journal: `INFO ah::db::system: stopped session anchor during cascade ... unit=ahd-session-sess_<uuid>.service`。
**自动化复验**: lib 446/0; r1_master_exit_shutdown 4/0, r1_shutdown_cleanup 1/0, r1_session_lifecycle 6/0, orphan_reap 3/0, pr4a_lifecycle_contract 9/0。

## A2 身份隔离 — 机制澄清 (矩阵措辞需更新)

矩阵 A2 原写 "Agent 必须有 `CCB_CALLER_ACTOR=agent_id`" — 这是 **legacy ccb** 的机制。实测真跑 `ah`:
- `/proc/<pid>/environ` 实证: master 与全部 3 个 agent **都没有 `CCB_CALLER_ACTOR`**; grep 全仓 `CCB_CALLER_ACTOR` 在 ah src/tests **出现 0 次**。
- ah 的 master-vs-worker 区分走 **config-home 布局**: `src/provider/home_layout.rs` 的 `HomeLayoutRole::Master → MASTER_RULES` / `Worker → WORKER_RULES`, 把不同的规则文件 seed 进各自隔离的 provider 配置目录 (CLAUDE_CONFIG_DIR 等)。
- 即 ah 的 A2 = "worker 的隔离配置家目录里是 WORKER_RULES, master 的是 MASTER_RULES" — 跟 A1 (provider 配置目录隔离) 同源, **bash provider 没有配置家目录 → A2/A1 必须真 provider 才能验**。
- **待澄清 (设计层, 非主控自决)**: config-home rules 是否足够替代 ccb 的运行期 `CCB_CALLER_ACTOR`? agent 进程运行期是否需要知道"我是 a1 还是 a3"(而不只是"我是 worker")? 这是 ah 替代 ccb 的身份机制设计问题, 需 a2 设计输入 (见 task 收尾)。

## 待真 provider 阶段 (一次一个, OAuth, 避免 3 并发 OOM)

- **A1** 配置隔离: `CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `GEMINI_CLI_HOME` 各指隔离 sandbox 路径。
- **A2** (真机制): worker config-home 含 WORKER_RULES, master 含 MASTER_RULES。
- **A3** OAuth 同步: 一处登录, a1/a2/a3 都拿到有效 token, 又不破坏 ENV 隔离。
- **D3** 真 `PROMPT_PENDING` → `ah cancel` → IDLE/重生。
- 已知 blocker: 全新 sandbox claude 首跑卡 onboarding 向导 (onboarding-mirror 种子化待补)。

## 自动化层 (necessary-not-sufficient) 复核

- lib 单测 445/0; ~30 个 faithful hermetic e2e 套件全绿 (grand tour 13 ah 命令含 ask / realign_extra 4/4 / pr4a lifecycle)。
- C (ask/concurrent/logs) 已由 faithful Rust e2e (grand tour 注入带 marker 的 provider 输出) 覆盖; 真 CLI 的 C 留待真 provider 阶段顺带验。
- mvp2/3/4/7 acceptance fail = **pre-existing 非本次回归** (HEAD~2 baseline worktree 实证同样 fail): mvp2 = stale harness (假断连 master); mvp3/4 = stale 7s BUSY→UNKNOWN 期望 (现 STUCK 300s/30s); mvp7 = 真 provider smoke (OAuth, 真 provider 阶段)。真行为由更新套件 (r1_master_exit_shutdown / realign_extra / pr4a_lifecycle) 覆盖。

---

# 真 ANTIGRAVITY provider 阶段实测 (2026-06-02)

> gemini provider 在 ah 已弃用 → 全部 dogfooding 针对 **antigravity** (agy v1.0.4, Gemini 3.5 Flash, OAuth)。一次一个 agy (避免 VPS 7.7G OOM)。真相 = `/proc/<pid>/environ` + cache-home 文件系统 + md5, 不信 ah 状态自报。

## 又找到并修掉的真实缺陷 (本阶段)

| # | 缺陷 | 类型 | 物理实证 | 修复 |
|---|---|---|---|---|
| #87 | antigravity init-probe 启动期过早 bail 到 SPAWNING_INTERVENTION | 实现缺陷 | StableUnknownDetector 在 agy 启动 banner 未稳定时 3 扫即判 UNKNOWN | `72dcd59` 注入 10s STARTUP_GRACE (record(capture, elapsed, startup_grace)), elapsed<grace 时不判 UNKNOWN; 3 个 SI e2e 用 Duration::ZERO 保留覆盖 |
| #88 | `ah stop`/master 死亡泄漏 **master** cred-home (满 OAuth) | 实现缺陷 | agent cred-home 被 burn, 但 master 伪 agent 无 teardown 路径 → cache-home 残留 (count 70→72→**72**) | `738f01e` handle_session_kill + ahd cleanup_tmux_resources 对 "master" 伪 agent 走同一 chokepoint (remove_agent_sandbox_dir_sync), 改 pub; 实证 70→72→**70** 双 burn |
| #89 | antigravity worker **治理文件 (WORKER_RULES) 生产路径未落盘** | 实现缺陷 | 真 `ah start` agy worker 的 isolated cache-home **无** `.gemini/AGENTS.md`; 单测假绿 (直调 materialize 绕过生产 prepare_antigravity_overrides) | `5a5ae4a` prepare_antigravity_overrides 透传 role + 调 materialize_builtin_rules(role,"antigravity",home_root) (与 claude/gemini 同模式) + builtin_rules_target 加 `.gemini/AGENTS.md` arm + **集成测试走真实路径** |
| #90 | `handle_agent_kill` 双 kill 第二次误报 `AGENT_NOT_FOUND` (E3) | 实现缺陷 | 真 `ah kill a1` 二次 → rc=2 `AGENT_NOT_FOUND` (应幂等成功); 根因 handlers.rs:1183 把 `changes==0` 一律映射 not-found, 而 `query_agent` :1173 已证行存在 → 此处只可能是"已终态" | `handlers.rs:1183` `changes==0` 改返回 `Ok({state: agent.state})` (幂等成功, 早 return 天然跳过 re-SIGKILL); handler 层测试加 idempotent-repeat + 保留 missing→AgentNotFound; `mvp2_acceptance.rs` ac5 同步断言。lib 467/0 + 真 agy 双 kill 均 rc0 KILLED + missing 仍 AGENT_NOT_FOUND |

> **#89 是 dogfooding 价值的活样本**: 单测必要但不充分 — a1 第一版只直调 `materialize_builtin_rules` 单测过, 但生产 spawn 路径 `prepare_antigravity_overrides` 从不调它 (其它 provider 都调)。只有真跑 `ah start` 物理检查 cache-home 才暴露。修复后加了走 `prepare_antigravity_overrides` 的集成测试堵住该 gap。

## A/C 维度 — 真 antigravity 实测

- **A1 配置隔离** ✅ PASS: 真跑的 agy 进程 (`/proc/<pid>/environ`, comm=agy) `HOME=~/.cache/ah/sandboxes/<hash>` (隔离 cache-home), **无** host-home (`/home/sevenx`) 泄漏。agy 从隔离 HOME 读配置, 不串染 host `~/.gemini` 或 sibling。
- **A2 worker/master 治理隔离** ✅ PASS (#89 修复后): worker isolated cache-home `.gemini/AGENTS.md` == WORKER_RULES (md5 883536e9); master cache-home `.claude/CLAUDE.md` == MASTER_RULES (md5 5ce47f18)。两者隔离共存。
- **A3 OAuth 同步** ✅ PASS (修正判据): antigravity-oauth-token 经 `copy_dynamic_auth_file` **复制** (非 symlink) 进每个 agent 的隔离 home (seeded from 单次 host 登录), 设 0o600。agy 运行时刷新自己那份 copy → 与 host 分叉 (716c52… vs host f0b52a…) = **健康隔离, 非共享失败**。功能有效性由 C1 证 (agy 认证为 "Google AI Ultra" 并回复)。
- **A4 敏感文件防误删 (migration 安全)** ✅ PASS (A3 顺带证): dynamic OAuth 文件**复制不 symlink** → sandbox 端刷新/migration **物理上无法删除或覆盖** host 全局凭证。实证: agy 在 sandbox 改写了自己的 token copy 后, **host token md5 仍 f0b52a… 原封不动**。这正是 ccb 历史上 gemini-cli 删 plaintext 凭证事故的根治。
- **C1 ask-reply 顺畅度** ✅ PASS (**头条结果**): 真 `ah ask a1 "..." --wait` 对真 agy worker, **6 秒**返回含 sentinel 的 reply, a1 直接回 IDLE (sub_state=Matched), **零 phantom job, 零 `cancel`救场**。直接 A/B 对照: 本 session 用 ccb 派 a1 三次都撞 completion-desync 需 `ccb ask cancel`救场; ah 在真目标 provider 上消除了它。
- **C3 logs/可观测性** ✅ PASS: 真 agy worker IDLE 后 `ah logs a1` rc=0 返回 **2720 bytes** 实时 pane 内容 (agy banner + `xingqiqi77@gmail.com` + `Gemini 3.5 Flash` + prompt `>` + state_change `SPAWNING→IDLE reason=SEED_READINESS`)。主控对真 provider pane 实时可观测。

## E 维度 (隐蔽角落) — 真 antigravity 实测

- **E1 daemon 未启动** ✅ PASS: 不起 ahd 直接 `ah ps` / `ah ask a1 --wait` 均 rc=1 + 友好 `ahd daemon is not running at <sock> / Start it with: ah start`, **无 hang(124) 无 panic**。
- **E2 Ask 幽灵 Agent** ✅ PASS: `ah ask not_exist_a99 "hello"` → rc=2 `RPC error -32000: AGENT_NOT_FOUND`; `ah pend not_exist_a99` 含 job_id 行数 = **0** (无幽灵 Job 残留阻塞队列)。
- **E5 大 payload (TD-008)** ✅ PASS (**TD-008 直接对照 ccb**): 真 `ah ask a1 "<8951 字节 prompt>" --wait` → rc=0 **6 秒**返回, agy 回显**结尾** sentinel `QX5LARGE9` (证明 ~9KB prompt 的**最末**那条 FINAL_INSTRUCTION 完整到达 agy, 未截断), pane 内 mid-marker 命中 385 次。ccb 历史痛点 = 逐字 type 长 prompt 致 hang/卡死 (需 /tmp 文件投递绕过); ah 经 send 路径一次性完整投递 ~9KB 不 hang 不截断。
- **D3 prompt-interrupt / cancel→IDLE→重生** ✅ **FULL PASS** (interrupt 效力实测确认): `ah cancel <full_job_id>` 命中 handle_job_cancel:1057 DISPATCHED 分支 → 对 agy pane 发 Escape keysym + spawn settlement watch → **rc=0 `CANCEL_REQUESTED`**, 不崩溃, 零 phantom (stuck=0), agent 复用 (re-ask 答 REBORN_OK)。**关键 disambiguation 实测**: 同一 huge essay 自然完成 = **102s**; cancel 在 agent 真 `BUSY` (生成中) 时发出 → **cancel→IDLE = 1s** (1s ≪ 102s) → **单 Escape 真正中断 in-flight 生成** (不只是请求)。`CANCEL_REQUESTED` (非 `CANCELLED`) 是诚实异步状态。handle_job_cancel 对已终态 job 返回幂等 status (非 error), 与修复后的 handle_agent_kill (E3/#90) 一致。ccb 的 cancel-后-wedge/desync **未出现**。
- **E3 双 kill 幂等** ✅ PASS (#90 修复后): 真 agy 重建二进制实测 kill#1 → rc=0 `state=KILLED`; kill#2 → **rc=0 `state=KILLED`** (修复前 rc=2 `AGENT_NOT_FOUND`); kill 真不存在 agent → **仍 rc=2 `AGENT_NOT_FOUND`** (E2 not-found 路径完好保留)。根因+修法见上 #90 行。矩阵 bar "第二次正常返回" 现达标。

## 本阶段已知 nit / 非阻塞观察

- **mvp11_real_claude `test_claude_spawn_ask_flow` 脆性** (pre-existing, 非 #89 回归): 让真 claude CLI 在自由文本 reply 里逐字回显 `CCB_CLAUDE_MD_MODE`/`CCB_REPLY_LANG`/`CCB_CTX_TRANSFER_LAST_N` 并 string-match。失败点跨 run 漂移 (:96↔:97) = LLM reply 非确定性, 非缺失变量。env 注入本身**确定性单测已过** (`src/sandbox/systemd.rs:623` 断言 `CCB_CLAUDE_MD_MODE=route` 在装配 cmd 内, 在 467/0 绿集中)。建议 followup: 该 e2e 改为直接验 sandbox env (不经 LLM 回显) — 测试质量问题, 非产品缺陷。
- **C1 reply distill prompt-echo 漏** (nit): C1 raw reply 在干净 sentinel 前夹了一段 prompt-echo + `(Google AI Ultra)` chrome。distill_reply 有去 chrome 单测但 live 这例没全清 prompt-echo。功能不影响 (sentinel 在), 收紧 distill 可作 followup。

## Scope-out (本阶段不跑)

- **C2 并发** — 需 2 agent 同时; 双 agy = OOM 风险 (VPS 7.7G)。jobs 表并发不串扰已由 hermetic Rust e2e 覆盖; 真 CLI 双 agy 并发暂不跑 (OOM 约束), scope-out。
- **gemini provider** — ah 内已弃用, 全部 dogfood 转 antigravity (见顶部)。

## 本阶段结论 (2026-06-02)

真 antigravity dogfood 全维度过: A1/A2/A3/A4 (隔离) · C1/C3 (调度+可观测) · D3 (cancel 真中断) · E1/E2/E3/E5 (异常角落)。修掉 4 个真实缺陷 (#87/#88/#89/#90), 全部"真跑 ah + 文件系统实证"发现, 非单测假绿。头条: C1 在真目标 provider 上**消除 ccb completion-desync** (本 session ccb 派 a1 撞 phantom 4× 需 cancel 救场, ah 0×)。已知 nit (非阻塞): mvp11 脆性 e2e · C1 distill prompt-echo · dispatch_atomicity 满载偶发 (隔离 2/2 过, SQLite busy)。

---

# 真 CODEX + CLAUDE provider 阶段实测 (2026-06-11, Step 1)

> Step 1 目标: 给 codex(a1) + claude(a3) 补跑 antigravity 同款 C/D3/E/F 真矩阵, 物理实证消除 ccb 痛点。一次一个真 provider (避免 VPS 7.7G OOM)。
> 二进制: **当前 main HEAD 946adb3 fresh build** (11:18, 旧 Jun-3 binary 早于 completion-v2/handler-split/reconcile 提交故重建); lib **522 passed / 0 failed**。
> 隔离: 各 provider 独立 `AH_STATE_DIR` + 各自 `ahd-<hash>` tmux socket, 与 master 的 ccb (`ccbd` Python + 独立 socket) 完全隔离。真相 = `/proc/<pid>/environ` + cgroup + `tmux -L <sock> ls` + pane logs, 不信 `ah ps` 自报。

## CODEX (a1, 真 codex CLI gpt-5.5, OAuth) — 全维度 PASS

- **A1 配置隔离** ✅: `/proc/3749414/environ` → `HOME=~/.cache/ah/sandboxes/57d17c507a95`, `CODEX_HOME=.../57d17c507a95/.codex`, **无** host-home (`/home/sevenx`) 泄漏。
- **C1 ask-reply 顺畅** ✅ (头条): `ah ask a1 --wait` (sentinel) **3s** 返回, rc=0, sentinel `CDXOK7` 干净回显, agent 回 **IDLE sub_state=LogEvent** (daemon log `reason=LOG_EVENT_TASK_COMPLETE reply_source=log` → completion-v2 原生 log 信号路径, 非 UI 兜底), **0 phantom, 0 cancel 救场**。直接对照 ccb completion-desync (ccb 派 a1 撞 phantom 需 cancel 救场)。
- **C3 logs/可观测** ✅: `ah logs a1` 返回 **62645 bytes** 实时 pane (codex banner + CDXOK7 + state_change LOG_EVENT_TASK_COMPLETE)。
- **D3 cancel 真中断** ✅: 长故事生成 → BUSY → `ah cancel <job>` → `CANCEL_REQUESTED`, **cancel→IDLE = 1670ms** (≪ 4000 词自然完成 >1min → 真中断 in-flight 非仅请求); agent 复用 re-ask 答 `REBORN_CDX`。signal tell: cancel 路径 sub_state=**Matched**, 自然完成=**LogEvent**。
- **E1 daemon 未启动** ✅: `ah stop` 后 `ah ping` → 友好 `ahd daemon is not running ... Start it with: ah start`, 无 hang 无 panic。daemon.log 优雅 SIGTERM 关闭。
- **E2 幽灵 agent** ✅: `ah ask not_exist_a99` → `RPC error -32000 AGENT_NOT_FOUND`, 0 phantom job。
- **E3 双 kill 幂等** ✅: kill#1 `state=KILLED` rc=0; kill#2 `state=KILLED` rc=0 (#90 修复保持); kill 不存在 → `AGENT_NOT_FOUND` (not-found 路径完好)。
- **E5 大 payload (TD-008)** ✅: 8111-byte prompt → **结尾** sentinel `QX5LARGE9` 5s 返回, rc=0 → ~8KB prompt 末条 FINAL_INSTRUCTION 完整到达未截断。
- **F 清干净不留孤儿** ✅: `ah kill a1` → 主 pid 3749414 ESRCH, agent scope inactive; `ah kill --session` → **tmux server gone** (authoritative `tmux -L ahd-ec8badf8d81e02f3 ls` = "no server running"), 0 ah scope 残留; daemon stop 清理幂等。零孤儿。

## CLAUDE (a3, 真 claude CLI, OAuth) — 全维度 PASS

- **A1 配置隔离** ✅: `/proc/3785451/environ` → `HOME=~/.cache/ah/sandboxes/8c4403b18400`, `CLAUDE_CONFIG_DIR=.../8c4403b18400/.claude`, 无 host-home 泄漏。onboarding 已种子化 → spawn 直达 **IDLE/Matched** (无首跑向导卡顿)。
- **C1 ask-reply** ✅ (claude completion-v2 验证): `ah ask a3 --wait` **5s** 返回, sentinel `CLDOK7`, agent **IDLE sub_state=LogEvent** → claude stop_reason end_turn 原生 log 信号在真 dogfood 生效, 验证跨 tick armed-guard 修复 (commit eaad842), 非 UI 兜底。0 phantom 0 cancel。
- **C3 logs** ✅: `ah logs a3` 12609 bytes, CLDOK7 + state_change + LOG_EVENT 可观测。
- **D3 cancel 真中断** ✅: 5000 词长故事 → BUSY → cancel → `CANCEL_REQUESTED`, **cancel→IDLE = 2084ms** (≪ 自然 >1min), 复用 re-ask 答 `REBORN_CLD` (pane 实证)。
- **E1** ✅: `ah stop` → `ah ping` 友好 daemon-not-running。
- **E2** ✅: 幽灵 → `AGENT_NOT_FOUND`。
- **E3** ✅: kill x2 幂等 KILLED/KILLED, ghost AGENT_NOT_FOUND。
- **E5 (TD-008)** ✅: 8905-byte prompt → 结尾 sentinel `QX5CLD9` 4s, 无截断。
- **F 清干净** ✅: `ah kill` 是**异步** (DB `state=KILLED` 先于 OS SIGKILL+scope teardown, 故 kill 后瞬时 `kill -0` 仍 alive — 正常异步窗口); session-kill + daemon-stop 后终态: 主 pid 3785451 **DEAD**, sandbox 8c4403b1 无残留进程, tmux server gone (`tmux -L ahd-48d0bd3cdb072674 ls` = no server), 0 scope, ahd 退出。零孤儿 (eventual-consistency clean)。

## Step 1 结论 (2026-06-11)

**codex + claude + antigravity 三个目标 provider 全部通过 C/D3/E/F 真矩阵, 零 blocker bug。** 头条: C1 在真 codex / 真 claude 上均 **IDLE sub_state=LogEvent** = completion-v2 原生 log 信号路径生效 (codex task_complete / claude stop_reason), 0 phantom 0 cancel → ccb completion-desync 在两个新 provider 上消除。TD-008 (E5 ~8-9KB 不截断) · cancel 真中断 (D3 1-2s ≪ 自然) · 幂等 kill (E3) · 清干净不留孤儿 (F) 全部物理实证。

### 本阶段方法论 nit (主控测试姿势, 非 ah 缺陷)
- `pgrep -f "<pattern>"` 会匹配主控自己 bash eval 命令串里的同一 pattern (假阳性"进程还活着")。教训: tmux 用 `tmux -L <sock> ls`, cgroup 用 `cgroup.procs`, 进程用 `ps comm=`, 不用 `pgrep -f` 在自己也打了的 pattern 上。
- `ah pend` 只吃 `<JOB_ID>` (ah 语义, 与 ccb 的 "name-or-id" 不同); `ah pend <agent_name>` → `empty stream response` (daemon `job_id not found: a1`)。可加友好报错 = cosmetic nit, 非 blocker。`ah pend <job_id>` 工作正常 (返 QX5LARGE9)。
- 优雅 `ah stop` socket 文件未 unlink (残留 `ahd.sock`), 但 `ah ping` 能识别 stale-socket 报友好 not-running → cosmetic nit, 非 blocker。
