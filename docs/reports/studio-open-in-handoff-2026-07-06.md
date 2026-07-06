# ccbd-rust (ah) Handoff — runtime 状态语义补 Starting 相位 + worker root 逃生口归位 + 发布 ≥1.3.1

> 原始版本写于 2026-07-06 会话内的临时 scratchpad（session-local temp dir，不持久），现补挪进仓库固化。
> 下面按当前真实状态标注每条任务的完成情况——**T1/T2/T3/T4 四条本任务在写这份文档时都还没做**；
> 同一天的会话里另外修了三个独立缺陷（见文末「已完成的旁支修复」），不要把两者混为一谈。

## 0. 背景:2026-07-06「Open in Claude Code」事故(已在 Studio 侧止血,根修在本仓)

Studio(agent-harness 仓)的「Open in Claude Code」把 skill 工作区交给 ah 编排:master = 交互
Claude,worker = clotho/lachesis/atropos 三个带技能的 claude agent。事故链:

1. **PR #99(runtime events stream,1.3.0→1.3.1)** 上线后,Studio(agent-harness #443)订阅
   `ah events --format json` 并「快照判 stale 即自动 cleanup(`ah stop`)」。
2. `ah start` 的冷启动窗口:`session.create` 先把 session 落成 ACTIVE,`session.spawn_master_pane`
   之后 master tmux 才活(`src/cli/start.rs` 顺序:session.create → spawn_master_pane → agent.spawn×N)。
   这段窗口的 runtime snapshot = `ahd_has_inventory=true, master_tmux_alive=false` ——
   **和 stale 残骸在快照布尔上完全同形**。
3. 于是 Studio 把自己正在启动的 daemon `ah stop` 了,终端里阻塞在 spawn_master_pane RPC 上的
   `ah start` 被断连:`rpc_call` 读到 0 字节 → `invalid JSON response from daemon: EOF while
   parsing a value at line 1 column 0`,exit 3(`src/cli/rpc_client.rs` 的 InvalidJson 分支)。
4. 叠加缺陷:worker 以裸 `claude --dangerously-skip-permissions` 在 WSL root 下 spawn,claude CLI
   直接拒跑退出("--dangerously-skip-permissions cannot be used with root/sudo privileges"),
   worker 秒死被 REAP,`ah start --wait`(等所有 agent 就位)报 `AGENT_NOT_FOUND` exit 2。

Studio 侧已合的止血(agent-harness PR #457):事件快照只投影状态、不再自动 cleanup;生成的
ah.toml 顶层加 `[env] IS_SANDBOX = "1"`(**这是 T2 说的次优方案,handoff 明确要求换成本仓 provider
层的正规实现**)。

## T1(未做):RuntimeState 补 `Starting` 相位

**现状证据**(`src/runtime_events.rs`,#99 引入):
- `let ahd_has_inventory = sessions.iter().any(|s| s.status == "ACTIVE");`(≈L146)
- `let master_tmux_alive = ahd_has_inventory && all_active_masters_alive;`(≈L211)
- `runtime_state = if active { Active } else if ahd_has_inventory { Degraded } else { Inactive }`(≈L214-218)
- 快照里其实已带 `sessions[].master_state`、`agents[].state`(SPAWNING 等),原始信息不缺,缺的是
  **顶层相位归纳**——冷启动被归进 Degraded,消费方只看顶层布尔/状态就必然误判。

**要做的**:
1. 给 `RuntimeState` 增加 `Starting`:session ACTIVE 且 master 尚未活、但处于**启动预期内**
   (master_state 仍在 spawn 流程 / 距 session 创建未超过 readiness_timeout)→ `Starting`;
   超时仍未活、或 master 曾活后消失 → 才是 `Degraded`。判定依据用 DB 里已有的 master
   state/generation/时间戳,不要靠猜。`worker` 侧同理:agent 处于 SPAWNING 且在预期内不该把
   `active=false` 解读成残骸。
2. 语义定义写进本仓设计文档(README/docs 里 runtime events 一节)+ CHANGELOG。
3. 测试:仿照 `runtime_events.rs` 现有 tests 补 `starting_snapshot_is_not_degraded`、
   `master_died_after_alive_is_degraded`、超时转 Degraded。
4. **消费方契约**:Starting 落地后通知 Studio 把 Open/Attach 的 stale 判定升级为
   「Degraded 才可清理、Starting 不许动」;Studio 设计文档
   `docs/studio/mvp1/03_regions/copilot/ah-orchestration-design.md` §4「诊断结论(2026-07-06)」
   已预留这句话。

## T2(未做,PM 2026-07-06 点名指出):worker 的 root 逃生口归位到 provider 层

**现状证据**:
- 整个 ah 源码 **没有任何 IS_SANDBOX 字样**(git grep 为证);worker spawn cmd 是裸
  `claude --dangerously-skip-permissions`。
- master 能活纯粹因为 Studio 手写的 master cmd 里有 `export IS_SANDBOX=1`。
- ah 本来就把 worker 放进自管 sandbox HOME(`/root/.cache/ah/sandboxes/<hash>`,
  `src/provider/home_layout.rs` 一带)——「我在受管沙箱里,可以信任 skip-permissions」这个知识
  属于 ah 的 claude provider,不属于每个 host 的配置模板。
- **临时方案已在 agent-harness #457 落地**(Studio 生成的 ah.toml 顶层加
  `[env] IS_SANDBOX = "1"`)——这正是本条 handoff 要求废止、换成正规方案的次优实现,PM 已明确
  指出「T2 还没做」。

**要做的**:
1. claude provider 构建 spawn env 时(`src/rpc/handlers/agent.rs` env 组装一带,
   `build_agent_spawn_env_vars_*`),当 provider=claude 且带 `--dangerously-skip-permissions`
   启动、HOME 指向 ah sandbox 时,注入 `IS_SANDBOX=1`。加测试:spawn env 断言含该变量。
2. 落地后**发 issue/PR 提醒 agent-harness** 删掉模板里的 `[env] IS_SANDBOX = "1"`
   (agent-harness #457 加的),不要静默留双份来源。
3. **⚠️ 2026-07-06 排查发现主仓根(非 worktree)有 18 文件、1170+ 行未提交改动**(provisioning
   方向:`home_layout.rs`、`init_probe_task.rs`、`sessions.rs`、`master_watch.rs` 等)。已核实边界:
   该 WIP 的 diff 里 **0 处 IS_SANDBOX**(不是 T2 的实现),且 **T2 主落点
   `src/rpc/handlers/agent.rs`、T1 落点 `src/runtime_events.rs`、白名单 `src/provider/manifest.rs`
   都不在 WIP 涉及文件里**——T1/T2 可以在从 `origin/main` 切的干净 worktree 里正常做,唯一的
   外围重叠是 `home_layout.rs`(T2 只读它作证据、不必改它)。规矩:在 worktree 干活,别动主仓根
   工作区,更不要 reset/discard 那堆 WIP(归属未确认)。

## T3(未做):公开发布 ≥1.3.1,解掉「手工 cargo build」单点

**现状证据**(2026-07-06,当天曾出现过):
- `/usr/bin/ah` = 1.3.0-rc.1(公开安装);`/root/.cargo/bin/ah` 一度是手工 `cargo install` 出来的
  1.3.1/1.3.2/1.3.3(2026-07-06 会话里又追加合了 #100 事件过滤 bug 修复 + #101 断流重连修复,均只
  合进本仓 `origin/main`,**没有走 cargo-dist 发布到公开的 `SevenX77/ah` 仓**)。
- Studio launcher payload 硬性要求 `ah >= 1.3.2`(agent-harness #458 提的门槛,随 T2/T1 修完可能
  还要再提),PATH 把 `$HOME/.cargo/bin` 排最前。若没有手工 build,报错提示让用户跑
  `scripts/install-claude-code-wsl.ps1`——装回来的还是旧公开版本,**死循环**。

**要做的**:
1. 把含 T1/T2 的版本走 cargo-dist release 发到 SevenX77/ah(版本号接着当前 Cargo.toml 走,
   发布前跟 CHANGELOG 对齐)。
2. 明确唯一安装落点:installer(AH_INSTALL_DIR,PR #97)与 `cargo install` 双源并存会让
   版本漂移不可见——至少在 `ah doctor` 里把「PATH 上生效的 ah/ahd 与其版本、systemd unit
   ExecStart 指向的 ahd 路径」打出来,双源不一致时给告警。unit 是 `ah` 生成的
   (`src/systemd_unit.rs`),ExecStart 固化绝对路径,升级后旧 unit 可能仍指旧二进制。

## T4(未做):小项

- **`ahd` 不认 `--version`**:`ahd --version` 会直接把 daemon 拉起来(2026-07-06 排查时踩到,
  意外在 default state dir 起了个真 daemon)。给 ahd 加标准 `--version`/`--help`,未知 flag 报错退出。
- **daemon 半途死掉的 RPC 错误不可诊断**:`rpc_call` 读到 0 字节报 `invalid JSON response from
  daemon: EOF...`,用户完全无法定位。空响应应单独成错:「daemon closed the connection without
  replying(可能被 stop/重启)」并提示查 `journalctl --user -u <unit>`。
- **e2e 残留**:`/home/ahe2e/ah-tell-master-e2e/run-20260704-095857/bin/ahd` 有多个进程常驻
  (2026-07-06 两次核对均为 4 个 ahd;#98 的 e2e 跑完没收),清理 + 让 e2e runbook 带 teardown。
- **主仓根未提交 WIP 归属**:2026-07-06 排查时发现的那 18 个文件、1170+ 行改动(见 T2 注记)——
  先确认这是谁的任务、进展到哪,再决定收进分支还是废弃,别让它继续裸放在主仓根工作区。

## 验收标准

1. 在 `/mnt/d/coding/skills/story-deconstruction-v3/subgraph/text-segmentation`(事故现场)用
   Studio 生成的 config 跑 `ah --config <cfg> start --wait`:exit 0,master READY,
   `tmux -L ahd-<hash> ls` 可见 master + agent_clotho/agent_lachesis/agent_atropos 全活。
2. `ah events --format json` 在整个启动过程中的快照序列:冷启动窗口 `runtime_state="starting"`,
   master 活后 `"active"`;人为 kill master 后变 `"degraded"`。
3. 公开 install 脚本装出来的 `ah --version` ≥ 本次发布版本,`ah doctor` 能揭示双源/版本漂移。
4. 全套 CI 绿 + CHANGELOG 记录。

## 已完成的旁支修复(不属于 T1-T4,是同日排查中发现的独立缺陷)

这些已经合并进 `origin/main` 并验证过闭环,记在这里避免和上面的 T1-T4 混淆:

- **`ah events` workspace 过滤 bug**(PR #100,1.3.2):`cmd_events` 拿 config 父目录当
  workspace_path 传给 `runtime.subscribe`,daemon 按 `projects.absolute_path = ?1` 过滤 inventory,
  与 session 记录的真实工作区路径永不相等 → 快照恒空、Studio 状态永远显示 inactive。已去掉该过滤。
  **这不是 T1 的 Starting 相位实现**,只是修复了一个让验证 T1 变得不可能的前置阻塞项。
- **`CLAUDE_CODE_OAUTH_TOKEN` env passthrough**(同 PR #100):加进 `ENV_PASSTHROUGH` 白名单,
  配合 agent-harness 的 auth 桥(`claude setup-token` 存 Windows env、WSLENV 转发),master/worker
  可以继承长效登录态,不用在 WSL 里重复登录、也不用复制含 refresh token 的 `.credentials.json`
  (会员两侧刷新互踩、同分钟被清空,2026-07-06 实证)。
- **`ah events` 断流退出 bug**(PR #101,1.3.3):守护进程 `ah stop` 或重启会关闭订阅流,原实现
  把这当「正常结束」直接退出,导致 GUI 侧的状态订阅冻结在最后一帧。改成断流后发一条本地 inactive
  快照(down-edge)再持续重连。
