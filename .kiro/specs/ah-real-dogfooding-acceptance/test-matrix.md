# ah 真 dogfooding 验收测试矩阵 v2

> 铁律: 全矩阵使用真 `ahd` binary + 真 `ah` CLI + 真 claude/codex/gemini provider。禁止用 in-process harness、`rpc_raw`、fake LLM provider、手写 `insert_event` completion、手工改 DB 状态来替代验收。所有 `ah ask` 必须经 UDS 到 `ahd`; completion 必须来自真 provider stdout -> tmux/FIFO -> `agent_io::reader` -> `events` -> `state_machine` -> `event.subscribe`。
>
> 本轮必须消除上个 closure 的 3 个边界: 1) in-process RPC harness; 2) fake provider 协议层替代真 provider; 3) stuck 只验逻辑不验真实进程/health 行为。若因 OAuth/外部账号限制无法跑真 provider, 该测试标为 BLOCKED, 不得用 fake 结果冒充 PASS。

## 机制事实锚点

- 隔离不是 bwrap/mount namespace。PR2 T4 后默认无 bwrap; 隔离模型是 provider HOME/env 重定向 + OAuth symlink + systemd scope。证据: `scripts/mvp13-e2e-sandbox.sh:3-5`, `src/provider/home_layout.rs:118`, `src/provider/home_layout.rs:144`, `src/provider/home_layout.rs:165`。
- agent/provider 进程默认由 `systemd-run --user --scope` 包装; unsafe 模式 `CCBD_UNSAFE_NO_SANDBOX=1` 会绕过 scope。证据: `src/sandbox/systemd.rs:7-31`, `src/sandbox/mod.rs:19-44`。
- tmux server 也可被 `systemd-run --user --scope --collect` 包住。证据: `src/tmux/scope.rs:21-39`, `src/tmux/session.rs:13-25`。
- session/master/agent 清理是组合机制: systemd scope + pidfd SIGKILL + tmux `kill-pane`/`kill-session`。证据: `src/db/system.rs:128-174`, `src/monitor/master_watch.rs:38-56`, `src/rpc/handlers.rs:100-138`。
- 状态一致性依赖 `state_version` CAS。证据: `src/db/state_machine.rs:79-146`。
- SQLite 使用 WAL + busy_timeout。证据: `src/db/mod.rs:71-78`。

## A. 配置隔离 (7 个测试点)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| A1 | Agent provider HOME/env 隔离 | `CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah ask a1 'printf "HOME=$HOME\nCLAUDE_CONFIG_DIR=$CLAUDE_CONFIG_DIR\nCODEX_HOME=$CODEX_HOME\nGEMINI_CLI_HOME=$GEMINI_CLI_HOME\n"' --wait`; 对 a2/a3/a4 重复。 | 真 provider 输出中各 agent 的 `HOME` / provider config env 指向各自 sandbox/home; claude 看 `CLAUDE_CONFIG_DIR`, codex 看 `CODEX_HOME`, gemini 看 `GEMINI_CLI_HOME`。不得出现同一 agent 复用 master 原始 home。锚点: `home_layout.rs:118,144,165`。 | 防止多 agent 共用配置目录造成 token、session、prompt cache 污染。 |
| A2 | OAuth 凭证主动破坏测 | 先记录 master 原始凭证 hash: `sha256sum ~/.claude/* ~/.codex/* ~/.gemini/* 2>/dev/null`. 再让 agent 真执行删除/改写尝试: `ah ask a3 'rm -f "$CLAUDE_CONFIG_DIR"/* 2>/dev/null; echo done' --wait`; gemini/codex 同类。最后重算 master 原始凭证 hash。 | master 原始凭证文件 hash/mtime 不变; agent sandbox 内可删的是 symlink/拷贝/隔离层, 不能破坏 host 原始凭证。若 provider 正常运行需要写 token, 写入必须限定在 agent HOME/env 指向路径。 | Gemini/Codex/Claude CLI 可能主动写凭证, 只测“能读”不够。 |
| A3 | 工作目录为项目根而非 mount namespace 幻觉 | `ah ask a1 'pwd; git rev-parse --show-toplevel 2>/dev/null; ls -la' --wait`。 | `pwd` 是 `ah start` 的项目根或 config 指定 absolute_path; 不要求 bwrap 隔离, 但不应默认进入 `~` 或全局 `.ccb`。锚点: `src/cli/start.rs:36-61`, `src/rpc/handlers.rs:730-747`。 | 验证 agent 在正确 repo 工作, 避免误改用户其他目录。 |
| A4 | 多 Agent 并发初始化 IO 竞态 | 使用真实 `ah.toml` 4 agent, 执行 `./target/release/ah --config "$TEST_CONFIG" start --wait`; 同时外部 `find "$STATE_DIR" -maxdepth 4 -type d | sort`。 | 4 个 agent 都到 `IDLE`; sandbox/home/config 目录全部存在; daemon log 无 file busy / duplicate sandbox / lock 错误。 | provider home materialization 并发最容易出竞态。 |
| A5 | 隔离区污染残留验证 | `SESSION_ID=$(ah ps | grep -oE 'sess_[a-f0-9-]+' | head -1); ah kill --session "$SESSION_ID"; ah --config "$TEST_CONFIG" start --wait`; 对比新旧 sandbox dir。 | session kill 后旧 agent sandbox 被移除或新 session 使用新隔离目录; 新 agent 不继承旧 `PROMPT_PENDING`/cache 锁。锚点: `src/rpc/handlers.rs:118-125`。 | 防止旧状态污染新任务。 |
| A6 | Master 自身配置隔离 | `ah start --wait` 后在 master pane 中 `env | grep -E 'CLAUDE_CONFIG_DIR|HOME'`; 或读 daemon log spawn cmd。 | master 的 `CLAUDE_CONFIG_DIR` 指向 master sandbox `.claude`, 不是任一 agent sandbox。锚点: `src/rpc/handlers.rs:235-256`, `src/sandbox/systemd.rs:37-57`。 | master 与 worker 权限边界必须分开。 |
| A7 | unsafe 模式隔离降级声明 | 对比默认启动和 `CCBD_UNSAFE_NO_SANDBOX=1` 启动: `ah ask a1 'echo $HOME $CLAUDE_CONFIG_DIR $CODEX_HOME $GEMINI_CLI_HOME' --wait`。 | 默认模式必须 systemd scope + provider env; unsafe 模式允许无 scope, 但验收报告必须显式标注“不证明子孙进程收割”。锚点: `src/sandbox/mod.rs:19-44`。 | 避免把 unsafe smoke 当生产隔离验收。 |

## B. kill/exit 进程清理 + 无泄漏 (9 个)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| B0 | 完整进程树 baseline diff (硬证明) | 杀前采集: `AH_PID=$(pidof ahd)`, `MASTER_PID=$(pgrep -f 'claude.*remote-control' | head -1)`, `pstree -ap "$MASTER_PID"`, 递归 `ps --ppid` 收集 master/provider/tmux pane 全部子孙 PID; 同时记录 agent scope: `systemctl --user list-units --type=scope | grep -E 'ccbd-agent|ahd-tmux'`, `systemd-cgls <scope>`, `cat /sys/fs/cgroup/.../<scope>/cgroup.procs`。杀后对每个 PID `kill -0 "$pid"`。 | 杀后所有 baseline PID 都 ESRCH; 对应 scope 的 `cgroup.procs` 为空或 scope 已消失; `systemd-cgls` 不再列出 provider 子树。禁止只 `grep sleep` 作为证明。 | 0 孤儿唯一硬证明, 防止 cargo/node/sleep 任意名进程逃逸。 |
| B1 | Master 正常 `/exit` 级联清理 | 在 master pane 输入 `/exit`; 外部 tail daemon log, 同时跑 B0 baseline diff。 | `master_watch` 通过 pidfd 捕获 master 退出, `cascade_kill_session_agents` 触发, 5s grace 后若无 active agents 则 daemon shutdown。锚点: `src/monitor/master_watch.rs:10-56,82-119`。 | 日常退出路径。 |
| B2 | `kill -9` 强杀 master 进程 | `kill -9 "$MASTER_PID"` 后跑 B0 baseline diff, 并看 daemon log。 | pidfd readable 后 cascade kill; agent state 变 `KILLED`; pane/session 被杀; daemon 是否退出取决于 `auto_shutdown_on_master_exit`。 | 强杀场景防孤儿。 |
| B3 | 孙子进程逃逸 (systemd scope) | 默认非 unsafe 模式下: `ah ask a1 "bash -lc 'sleep 1000 & disown; echo spawned' " --wait`; 记录新增 sleep PID 和所在 cgroup; `ah kill --session "$SESSION_ID"` 后跑 B0。 | sleep PID 必须在 agent/systemd scope 内, session kill 后 ESRCH; cgroup 空。若 sleep 逃出 scope, FAIL。 | 后台 dev server / cargo watch 是最高风险泄漏源。 |
| B4 | `ahd` 异常断开与 socket 清理 | `kill -9 "$AH_PID"`; 检查 `$STATE_DIR/ahd.sock`; 重启 `CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ahd`; 再 `ah ping`。 | stale socket 不阻塞重启; startup reconcile 标记死 agent 或清理 orphan scope。锚点: `src/bin/ah.rs:236-241`, `src/bin/ahd.rs:35-70`, `src/db/system.rs:276-314`。 | 宕机恢复能力。 |
| B5 | `ah stop` 优雅释放 | `./target/release/ah stop`; tail daemon log; 检查 tmux server/socket/session。 | `system.shutdown` 触发 `cleanup_tmux_resources`, 执行 tmux `kill-session` + `kill-server`, socket 删除。锚点: `src/bin/ahd.rs:100-149`。 | 正常关闭路径确定性。 |
| B6 | SIGHUP / 关闭终端窗口 | 在承载 master 的终端直接关闭窗口或 `kill -HUP "$MASTER_PID"`; 观察 master 是否退出; 若未退出, 记录状态。 | 若 master 退出, pidfd 路径同 B1 PASS; 若 master 忽略 SIGHUP 且仍活, ah 不应误 cascade kill, 但矩阵报告必须标注“pidfd 只捕获死亡, 不捕获 hang/忽略 SIGHUP”。 | 真实用户关窗口常见, 不等于 kill -9。 |
| B7 | tmux detach 不应误判退出 | `ah attach a1` 后 `Ctrl-b d`; master pane 同类 detach。 | detach 后 provider/master 进程仍活, pidfd 不触发, agent/master state 不应被标 KILLED/CRASHED; 无 orphan 增长。 | detach 是正常观察行为, 不能被当退出。 |
| B8 | master 自身 hang | 让 master 卡住但不退出: 在 master pane 触发长时间无响应或 `kill -STOP "$MASTER_PID"`; 观察 `ah ps`/日志。 | 当前 `master_watch` 只有 pidfd 死亡检测, 不应声称能发现 hang。若无健康检测, 本项预期为 GAP/FAIL, 记录需补 master health。 | pidfd 只看进程死活, 不看逻辑健康。 |
| B9 | 单 agent kill 与 session kill 差异 | `ah kill a1` 后检查 a1; 再 `ah kill --session "$SESSION_ID"` 检查全 session。 | `ah kill a1` 只杀单 agent; `ah kill --session <session_id>` 才杀 master+所有 agents。锚点: `src/bin/ah.rs:64-70,456-479`。 | 避免验收命令误杀范围。 |

## C. SOP 执行 (6 个)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| C1 | 正常单 ask 闭环 | `./target/release/ah ask a1 "请回复一行 READY" --wait`; 同时 `ah watch a1 --since-event-id 0`。 | CLI 打印 terminal job payload; DB job 状态 `COMPLETED`; agent 最终 `IDLE`; completion 由真 provider output 触发, 非手写 DB。锚点: `src/bin/ah.rs:413-431`, `src/bin/ah.rs:523-545`。 | 最基础可用性。 |
| C2 | Physical Evidence 拦截 | 通过需要证据的 job 设置或 RPC 标记 `job.mark_requires_evidence`, 让 agent 只文字回复不写文件/不跑测试。查 `ah logs a1`。 | 插入 `evidence_denied` event, 文本含 `SYSTEM DENY: Missing physical evidence...`; job 不应被直接 completed。锚点: `src/db/state_machine.rs:28,321-324,455-496`。 | 防止口头完成。 |
| C3 | 忙时请求排队 | 启动长任务: `ah ask a1 "sleep 30; echo done"`; 立刻 `ah ask a1 "second"`。 | 第二个 job 进入 QUEUED 或明确排队; 不串进当前 pane; first 完成后再 dispatch。锚点: `src/db/jobs.rs` claim/dispatch, `src/orchestrator/mod.rs`。 | 并发安全。 |
| C4 | 不存在 agent | `ah ask ghost "do it" --wait`。 | 立即 RPC/CLI error, 非空等; exit code 非 0; message 指明 agent not found。 | 边界输入处理。 |
| C5 | 中途 cancel | `JOB=$(ah ask a1 "sleep 30; echo done" | sed -n 's/job_id=//p')`; `ah cancel "$JOB"`; 后查 `ah ps`, `ah logs a1`。 | job `CANCELLED` 或 `CANCEL_REQUESTED` 最终收敛; agent 回到合理状态; 无残留 sleep, 用 B0 子 PID 证明。锚点: `src/rpc/handlers.rs:1019-1054`。 | 用户反悔与卡死恢复。 |
| C6 | 完整多 agent SOP-08 真协作 | 真跑一轮: `ah ask a2 "写设计..." --wait`; `ah ask a1 "按设计实现..." --wait`; `ah ask a3 "audit diff..." --wait`; 重复到 report。全程记录 `ah ps`, `ah logs`, DB events。 | a2/a1/a3 都是真 provider; 13 步状态流转完整; 主控不手工 capture-pane verify、不 loop poll、不 cancel stuck; 所有 completion 由真 output 触发。 | 这是 dogfooding 最终目标, 覆盖单 ask 不覆盖的长链协作。 |

## D. 生命周期推进 + 检测 (6 个)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| D1 | 全新 provider home / onboarding | 暂移 provider token/config, `ah --config "$TEST_CONFIG" start --wait`; 若卡住, `ah ps`, `ah logs <agent>`; 用 `ah prompt resolve <agent_id> --action '<json>'` 或 `--keys '<keys>'`。 | 状态准确进入 `PROMPT_PENDING` 或 `UNKNOWN`, 不黑洞; resolve 后可推进到 `IDLE`。CLI 真实写法锚点: `src/bin/ah.rs:117-128`。 | 首次启动最容易卡登录/信任提示。 |
| D2 | provider 主进程崩溃状态感知 | 找 agent pane pid: `tmux -S /tmp/tmux-$(id -u)/<socket> display -p -t agent_a1 '#{pane_pid}'`; `kill -9 <pane_pid>` 或杀 provider 主 PID。 | agent pidfd watcher 标记 `CRASHED`, exit_code/error_code 如实记录; 不依赖“remain-on-exit 退出码”作为主路径。锚点: `src/monitor/agent_watch.rs`, `src/rpc/handlers.rs:842-917`。 | provider died 必须及时上报。 |
| D3 | STUCK 假死判断 | 让 agent 进入无输出 hang: `ah ask a1 "bash -lc 'echo start; sleep 9999'" --wait`; 观察 `AH_STUCK_THRESHOLD_SECS` 默认/override。 | 超过阈值后 agent `STUCK`, `event.subscribe(event_kind:["stuck"])` 可收到 frame; 不无限等待。锚点: `src/pane_diff/mod.rs`, `src/provider/health_check.rs`。 | 防无限 hang。 |
| D4 | 缺失 provider binary 启动失败 | 临时 PATH 去掉 provider 或配置不存在 provider, `ah --config bad.toml start --wait`。 | 状态进入 `CRASHED`/`UNKNOWN` 或 CLI 明确失败; `ah ps` 可见; daemon 不 panic。 | 启动失败可解释。 |
| D5 | DB state_version CAS | 并发执行: `for i in {1..20}; do ah kill a1 & ah up --force & done; wait`; 后查 DB events 与 `ah ps`。 | 无互相覆盖的非法状态; `state_version` 单调; 无 panic。锚点: `src/db/state_machine.rs:79-146`。 | 并发状态一致性。 |
| D6 | 真 completion 检测 | 对真 provider 执行会产生清晰终态的 prompt: `ah ask a1 "输出 DONE 后停止" --wait`; 同时 tail DB `events` 和 `ah logs a1`。 | 真 output 被 reader 捕获为 `output_chunk`; job 完成、agent 回 `IDLE`; 不允许 `insert_event`/fake marker/harness 参与。锚点: `src/agent_io/reader.rs`, `src/db/events.rs`, `src/db/state_machine.rs`。 | 上个 closure 的命门, 必须真验。 |

## E. 隐蔽角落 / 异常路径 (5 个)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| E1 | 长时间编译 false STUCK | `ah ask a1 "cargo build --release" --wait`; 记录 stdout heartbeat、pane_diff/health events。 | 正常编译不应被误判 STUCK; 若长时间无输出且确实仍在跑, 需记录为 detector gap。 | Rust 项目真实可用性。 |
| E2 | tmux server 被杀 | 找 tmux socket/server PID 后 `kill -9`; 运行 `ah ps`, tail daemon log。 | daemon 不死锁; active agents 降级 `CRASHED`/可解释错误; 后续 `ah stop`/重启可恢复。 | 底层基础设施故障。 |
| E3 | fork 炸弹/逃逸压力 | 低强度安全版: `ah ask a1 "bash -lc 'for i in {1..30}; do sleep 300 & done; echo spawned'" --wait`; 记录所有新增 PID/cgroup; `ah kill --session "$SESSION_ID"`。 | B0 进程树/cgroup 证明所有新增 PID 清空。禁止只 grep `sleep`。 | 最严重资源泄漏风险。 |
| E4 | 多实例并存 | 分别用 `AH_STATE_DIR=$PWD/target/state_a` 与 `AH_STATE_DIR=$PWD/target/state_b` 启两个 ahd; 分别 `ah --config ... start --wait`。 | 两套 `ahd.sock`, `ahd.sqlite`, tmux socket name 独立; kill A 不影响 B。锚点: `src/state_layout.rs:15-18`, `src/tmux/mod.rs` socket hash。 | 多项目/多用户并存。 |
| E5 | SQLite 写争用 | `for i in {1..20}; do ./target/release/ah ask a1 "pwd $i" & done; wait`; tail daemon log。 | 无 `database is locked` panic; WAL + busy_timeout 生效。锚点: `src/db/mod.rs:71-78`。 | 高频并发稳定性。 |

## F. 资源泄漏 / 压力 (4 个)

| # | 测试点 | 怎么真实观察 (具体命令/检查) | 通过标准 | 为什么重要 |
|---|---|---|---|---|
| F1 | Tmux pane/session 重建压力 | `for i in {1..100}; do ah kill a1 || true; ah up --force || true; done`; 记录 `tmux -S <socket> list-sessions/list-panes`。 | pane/session 数不持续增长; 旧 pane 被 kill; daemon log 无连续 cleanup failure。 | 长运行不堆 tmux 资源。 |
| F2 | Daemon FD 泄漏 | 压测前后记录 `lsof -p "$AH_PID" | wc -l`; 期间运行 C6/E5/F1。 | FD 数增长有界, 不随操作线性增长; 无大量残留 UDS connection。 | 防 `Too many open files`。 |
| F3 | DB 体积增长 | 压测前后记录 `du -h "$STATE_DIR/ahd.sqlite"*`, `sqlite3 "$STATE_DIR/ahd.sqlite" 'select count(*) from events;'`。 | 增长符合事件数量; 若无限增长无 pruning, 标为容量风险并给出上限建议。 | 磁盘泄漏。 |
| F4 | master kill 循环 RSS/FD baseline | 循环 50 次: `ah --config "$TEST_CONFIG" start --wait`; 记录 `AH_PID`, `ps -o rss= -p "$AH_PID"`, `lsof -p "$AH_PID" | wc -l`, tmux session count; `ah kill --session "$SESSION_ID"`。 | 50 轮后 RSS/FD/tmux session count 涨幅有界; session kill 路径资源释放独立于 `ah stop` 优雅关机路径。 | cascade kill 是日常清理主路径, 必须单独压测。 |

## 最高风险 5 角落 (优先测)

1. **B0/B3/E3 完整进程树与 cgroup 清空**: 这是 0 孤儿唯一硬证明, 不能用 grep 特定进程名代替。
2. **C6 完整多 agent SOP-08 真协作**: 真 provider、真 UDS、真 completion, 覆盖单 ask 无法证明的长链路。
3. **D6 真 completion 检测**: 必须证明 stdout -> reader -> events -> state_machine, 不能再落回 fake marker/insert_event。
4. **B6/B7/B8 master 非死亡路径**: SIGHUP、detach、hang 都不是 pidfd death, 必须明确能力边界。
5. **A2 凭证主动破坏测**: 读凭证不够, 必须证明 agent 写/删不会破坏 master 原始凭证。

## 真启动姿势 (基于 scripts/mvp13-e2e-sandbox.sh)

在真实项目中做验收:

```bash
export CCB_ENV=dev
export AH_STATE_DIR="$(pwd)/target/dev_state_play"
# 不设置 CCBD_UNSAFE_NO_SANDBOX; 默认 no-bwrap + provider HOME/env 重定向 + systemd-run scope。

cargo build --release --bin ahd --bin ah

CCB_ENV=dev AH_STATE_DIR="$AH_STATE_DIR" ./target/release/ahd > daemon.log 2>&1 &
DAEMON_PID=$!

for i in 1 2 3 4 5; do
  sleep 1
  CCB_ENV=dev AH_STATE_DIR="$AH_STATE_DIR" ./target/release/ah ping && break
done

CCB_ENV=dev AH_STATE_DIR="$AH_STATE_DIR" ./target/release/ah --config "$TEST_CONFIG" start --wait
CCB_ENV=dev AH_STATE_DIR="$AH_STATE_DIR" ./target/release/ah ps
CCB_ENV=dev AH_STATE_DIR="$AH_STATE_DIR" timeout 90 ./target/release/ah ask a1 "echo from isolated a1" --wait

SESSION_ID=$(CCB_ENV=dev AH_STATE_DIR="$AH_STATE_DIR" ./target/release/ah ps | grep -oE 'sess_[a-f0-9-]+' | head -1)
CCB_ENV=dev AH_STATE_DIR="$AH_STATE_DIR" ./target/release/ah kill --session "$SESSION_ID"
```

Master 正确姿势:

- 不手动跑 `claude --config master.json`。
- 通过 `ah --config "$TEST_CONFIG" start --wait` 读取 `[master].cmd` 并调用 `session.spawn_master_pane`。
- 默认 master cmd 是 `claude --dangerously-skip-permissions --continue /remote-control`。锚点: `src/cli/config.rs:174-176`, `src/cli/start.rs:72-90`, `src/rpc/handlers.rs:215-310`。

CLI 真实写法提醒:

- 看 ask 输出必须 `ah ask a1 "..." --wait`, 或先拿 `job_id` 后 `ah pend <job_id>` / `ah logs a1`。
- `ah up --force` 存在但无 `--wait`。
- `ah prompt resolve <agent_id> --action '<json>'` 或 `--keys '<keys>'`。
- `ah kill a1` 是单 agent kill; 整 session kill 是 `ah kill --session <session_id>`。
- `AH_STATE_DIR` 是真 env, 兼容 `CCBD_STATE_DIR`。锚点: `src/state_layout.rs:15-18`。
