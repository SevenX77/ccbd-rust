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
