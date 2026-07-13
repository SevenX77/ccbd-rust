# MD2 目标3 设计稿 · `monitor::master_watch` 拆分边界(探测 / 决策 / 执行)

- 执笔:d1(claude,设计主笔,唯一执笔者)
- 日期:2026-07-13
- 状态:**待 operator 亲审**(master 转交;d1 不直接联系 operator)
- 交付路径:`.kiro/specs/ah-modular-decoupling/d1-target3-master-watch-design-2026-07-13.md`
- 上游 gate:MD3(design-before-code)。本稿是 MD2 三目标中**唯一需要独立设计评审**的一个(见 `md2-plan-2026-07-13.md` §目标3),因为 master 自愈是全系统单点,拆错会复现历史 master 死循环 / 双活 / 孤儿壳事故。

---

## MD3 门:本稿读了索引哪些条目、改了哪些

> 依 `research/architecture-index.md` §279:每个设计交付必须先声明读了哪些索引条目、改了哪些;r1 据此审"是否发明了索引里已有的能力"。

**读到的索引条目(作为拆分约束):**
- Layer 2 `monitor`(`src/monitor/master_watch.rs`,5704 行)—— 责任=进程/会话/master liveness 监控、pidfd 注册、进程死亡上的 cascade/revival 钩子。8 个顶层 pub 符号已列明。
- Layer 2 `master_revival`(`src/master_revival.rs`,928 行)—— 责任=**纯状态转移助手**,用于"决定/认领/节流/完成 ah 托管的 master 复活"。
- Layer 2 `master_cutover` —— 与本目标相邻但**不在拆分范围**(cutover 是"未托管→托管"接管,不是死亡复活);仅在诊断"stale-inflight 误判"时引用其 in-flight 语义。
- Capability→Owner 表两行:
  - `Master self-healing (revival)` → `master_revival::{classify_master_death, revive_session_master}`(`src/master_revival.rs`)。
  - `Master cutover` → `master_cutover::write_handoff_bundle` + `rpc::handlers::handle_session_master_cutover`。
- Layer 4 依赖面(master_watch 现实际 reach 到的持久化 owner):`db::master_recovery`(recovery window 状态机)、`db::system`(`clean_worker_runtime_resources_sync` / `cascade_kill_session_agents` / snapshot)、`db::recovery`(spawn spec / 中断 job requeue)。

**本稿主张要改的索引条目(拆分落地后 PR 内同步更新,见索引 §278 freshness 机制):**
- `monitor` 行:`master_watch.rs` 行数将显著下降;pub 符号表基本不变(见 §2 契约不变承诺),但需要新增一行执行子模块(建议 `monitor::master_reaper`)。
- `master_revival` 行:key symbols 需**追加**从 master_watch 上移的决策/围栏谓词(见 §2.2 清单)。
- **不发明任何索引已有能力**:决策去向的是**已存在**的 `master_revival`(索引已登记为"决策助手"),不是新造决策层;物理动作调用的仍是**已存在**的 `db::system` / `tmux` / `sandbox::systemd` / `rpc::handlers::realign::spawn_realign_agent`(见 §5 分层倒挂说明)。

---

## §0 方法与可信度声明("走了源码,不是编的")

本稿所有"某函数在第 N 行 / 调用谁"的断言,d1 都用 `rg` + `Read` 亲验过当前 `main`(post PR#151)源码,不凭记忆、不凭旧 spec。历史 spec(下引)里的行号是**事故当时**(PR#52 / 产品交付期)的旧文件,与当前 5704 行文件**不对应**;凡引用历史 spec 处均标注"[incident-era 行号]",凡引用当前源码处均为亲验行号。这一区分本身是本稿的一个核心发现(见 §3 引子)。

历史事故档案(operator 点名必须显式引用,均已通读):
- `research/incident-stale-session-kill-cascade-2026-07-08.md` —— 共享 tmux socket 上 stale `master_pane_id='%0'` 复用 → 误杀活 master。
- `.kiro/specs/ah-oom-restart-resume/research-master-death-corrected.md` —— "清理先于复活、idle 不复活"的 corrected 语义来源。
- `.kiro/specs/ah-oom-restart-resume/design-master-death-corrected.md` —— snapshot-before-cleanup / real-reap-before-backoff / teardown 内序 的时序不变量总纲。
- `.kiro/specs/ah-product-delivery/revive-master-readiness-zombie-design.md` —— "活 ≠ ready";readiness gate 必须先于 COMPLETED;**重启 readiness 重装 probe** 回归点。
- `.kiro/specs/ah-product-delivery/master-oom-vs-cascade-coordination-design.md` —— in-process lock 不是权威状态;DB `master_recovery_windows` 状态机替代;generation fencing 是命门。
- `.kiro/specs/ah-product-delivery/recovery-reinsert-atomicity-design.md` —— reprovision 期 KILLED agent 删除经 FK `ON DELETE CASCADE` 连删 job → 原子替换窗口。
- `research/ah-master-death-cutover-incident/dead-master-transcript-f3808d36.jsonl`(+ 同目录 `findings.md` / `RESTART-BRIEF.md`)—— 真实故障:cutover 失败后 reap-old-master 定时器未取消 → 孤儿;且点名回归测试 `master_revive_stale_inflight_dispatch_failure_does_not_overwrite_requeued_job`。

---

## §1 现状边界诊断(Q1)

### 1.1 结论先行

`master_watch.rs` 的 5704 行里,**≈59% 是 inline test**(生产码 1–2323,测试 `mod tests` 起于 2325)。真正的耦合病灶不在"行多",而在:**探测 / 决策 / 执行三层被两个"上帝函数"按语句顺序拧成一股**——`revive_master_after_exit_windowed`(574–927,约 350 行)与 `resume_master_recovery_readiness`(1026–1210,约 185 行)。这两个函数各自从头到尾把"读快照→物理清 worker→判 idle/active→退避重判→认领→记账/熔断→拉起进程→装探针→等就绪→失败则级联+收割→重灌 worker→完成窗口"一条龙写死在一个函数体内。

**"决策本该在 `master_revival`、现在内联在 `master_watch`"的实测量**:约 8–10 个纯 DB 状态谓词/围栏 + 2 处内联分支(合计约 200 行生产码)属于决策层却长在 master_watch(明细见 §1.4)。`master_revival`(928 行)本身已经是干净的纯决策助手,承接得下这些。

### 1.2 三层在文件里各自的位置(亲验行号)

**(A) 探测层 / probe —— 这是 master_watch 的合法身份(索引 Layer 2 原话)**

| 符号 | 行 | 做什么(纯观察) |
| --- | --- | --- |
| `master_process_is_alive` | 152 | `pidfd_open(pid).is_ok()` 探活 |
| `spawn_master_pidfd_watch_task` | 418 | pidfd `readable().await` → 死亡即路由 |
| `patrol_active_masters_once` | 121(`pub(crate)`) | 巡逻:`monitor::contains(key)` 缺失则补探,`!master_process_is_alive` 则路由死亡 |
| `master_watch_patrol_loop` | 383 | 巡逻循环(secondary net) |
| `rearm_active_master_watches_on_startup` | 110 | 重启后逐 active session 重装探针 |
| `arm_or_route_master_watch` | 159 | 开 pidfd + 校 pane pid → 装表或路由死亡;**并在 MASTER_VERIFYING 窗内 resume readiness(233–273)** |
| `stored_master_pane_still_matches` | 277 | tmux `get_pane_pid` 与存储 pid 比对(防 PID 复用误判) |
| `revive_master_readiness_ack` | 1574 | 读 transcript assistant-progress + 围栏 + 探活 |
| `revive_master_readiness_probe` | 1631 | 连续 3 次稳定非空 capture(内容盲降级探针) |
| `ReviveMasterReadinessProbeState` | 1608 | 探针稳定态 |
| `wait_for_recovered_workers_ready` | 1905 | 轮询 worker 状态到 ready/超时 |

**(B) 决策层 / decision —— 部分已委派 `master_revival`,部分内联(病灶)**

已正确委派到 `master_revival`(master_watch 只调用):`classify_master_death`(用于 69、647)、`try_claim_master_transition`(667)、`record_master_revive_attempt`(687)、`complete_claimed_master_transition`(842)、`confirm_master_stable`(2182 via timer)、`master_spawn_lock`(67、510)、`query_master_revive_next_retry_at`(635)、`remove_master_monitor_key_if_generation_matches`(457/464/490/1342)。

内联在 master_watch、本应属决策层的(§1.4 详列)。

**(C) 执行层 / execution —— 大多是对下层的 glue,是 master_reaper 的候选**

| 符号 | 行 | 物理动作 | 真正干活的下层 |
| --- | --- | --- | --- |
| `revive_master_after_exit_windowed` 内 spawn 段 | 769–835 | 组 env/creds/命令 + tmux `ensure_session`/`spawn_window`/`set_pane_title` | `sandbox::systemd::master_command_with_env`、`tmux::*` |
| `build_master_revive_command` | 2196 | 命令组装 | `systemd::master_command_with_env` |
| `inject_master_continue_instruction_best_effort` | 2225 | 向复活 pane 注入"继续" | `tmux::send_keys_*` |
| `reap_failed_revive_master_best_effort` | 1276 | 围栏→SIGKILL→杀 pane→撤探针 | `sigkill_*`(1392)、`kill_*_pane`(1440)、`monitor::*` |
| `reap_claimed_revive_master_after_error_best_effort` | 1212 | 出错路径收割已认领 master | 同上 |
| `reprovision_declared_workers_after_master_revive` | 2001 | 重灌声明的 worker | `rpc::handlers::realign::spawn_realign_agent`(**跨层,见 §5**) |
| `restore_killed_worker_spawn_spec` | 2145 | reprovision 失败回滚 spec | `db::recovery`/`db::agents` |
| `write_master_revival_redispatch_marker` | 2295 | 落 redispatch marker 文件 | `std::fs` |
| `spawn_master_confirm_timer` | 2174 | 60s 后 `confirm_master_stable` | tokio + `master_revival` |
| 物理清 worker(在 windowed 内) | 597 | 级联清 worker runtime(非终态) | `db::system::clean_worker_runtime_resources_sync` |
| 失败/超时级联 | 1057/1100/1182 | `cascade_kill_session_agents` | `db::system::cascade_kill_session_agents` |

### 1.3 两个上帝函数如何"拧成一股"(耦合的具体形态,不泛泛)

`revive_master_after_exit_windowed`(574–927)的语句顺序**本身就是安全契约**:

```
585  snapshot_master_death_session_activity        [探测/读:清理前快照]
586  begin_master_recovery_window_for_snapshot     [决策/写:开窗]
597  clean_worker_runtime_resources_sync           [执行:物理清 worker,非终态]
615  if IdleNoWork → close + FAILED + return        [决策:idle 不复活,内联分支]
626  mark phase WORKERS_REAPED                       [决策/写]
633  backoff: 读 next_retry_at → sleep → 重判       [决策+时序,内联]
667  try_claim_master_transition                     [决策:CAS 认领(委派)]
687  record_master_revive_attempt → Spawn/Fused/Stale[决策:节流/熔断(委派)]
743  write_master_revival_redispatch_marker          [执行:落 marker]
769  组 env/creds/sandbox home                        [执行:spawn 准备]
825  ensure_session + spawn_window                    [执行:物理拉起进程]
841  get_pane_pid                                     [探测]
842  complete_claimed_master_transition(None→851 杀孤儿 pane)[决策+执行]
860  mark phase MASTER_RUNNING                         [决策/写]
867  inline SQL: persist master_cmd                    [执行/写,内联 SQL]
875  pidfd_open + register + spawn_master_pidfd_watch  [探测:重装探针]
894  inject_master_continue_instruction                [执行]
914  resume_master_recovery_readiness(→ 第二个上帝函数) [探测+决策+执行]
925  spawn_master_confirm_timer                        [执行]
```

`resume_master_recovery_readiness`(1026–1210)同构:`no-budget→cascade+reap`(1050–1078)、`readiness 失败→cascade+reap`(1093–1123)、`mark ready`(1125)、`reprovision`(1139)、`worker readiness gate 超时→cascade+reap`(1164–1202)、`complete window`(1203)。

**耦合的本质不是"函数太长",而是三层沿着一条时序穿插。** 历史事故(§3)证明这条时序的每一步顺序都有安全含义。这带来 §2 的核心设计约束:**不能按"把所有执行搬到 X、所有决策搬到 Y"来切**,否则一条本地可核对的时序会被打散到三个模块,失去"对着不变量清单逐行验"的能力——这正是 operator 担心的"拆丢一段"。

### 1.4 内联决策清单("有多少决策没委派给 master_revival",亲验)

以下均为**纯 DB 状态谓词/转移**,无 tmux/进程副作用,却长在 master_watch,按分层应属 `master_revival`(或作为其薄适配器):

| 内联决策 | 行 | 性质 |
| --- | --- | --- |
| idle-vs-active 分支 | 615–625 | 决策分支(用 snapshot.classification) |
| backoff + 重判控制流 | 633–666 | 决策 + 时序 |
| `master_runtime_matches`(围栏) | 1754 | 纯 DB 谓词(active+pid+gen) |
| `master_runtime_generation_matches`(围栏) | 1352 | 纯 DB 谓词(pid+gen) |
| `master_recovery_verifying_window_expected_generation` | 1804 | 纯 DB 读 |
| `master_recovery_effective_readiness_timeout` | 1823 | 时序决策(组合 `db::master_recovery::effective_readiness_timeout`) |
| `recovered_workers_ready_sync` | 1880 | 纯 DB 谓词 |
| recovery-window 相位包装(begin/mark/fail/complete/non-revive-terminal) | 1839–1975 | DB 写决策(薄适配 `db::master_recovery`) |
| `mark_session_closed_after_idle_master_death` | 1977 | 内联 SQL 写决策 |
| inline SQL:persist master_cmd | 867 | 内联 SQL 写 |

体量:约 200 行生产码。这就是"决策内联"的实测答案——**不是全部决策都没委派**(核心的 classify/claim/record/fuse 已在 `master_revival`),而是**围栏谓词 + 相位写 + 两处内联分支**这一层没委派,散在 master_watch 里,恰好是历史上出 bug 的那些围栏(generation fencing、readiness 窗、idle 判定)。

---

## §2 拆分方案(Q2)

### 2.1 核心设计原则:"保住 saga,抽走 step"(sequence-preserving extraction)

§1.3 已论证:master 自愈的安全性 = 两个上帝函数里那条**语句时序**。因此拆分的第一约束不是"分层纯度",而是:

> **那条有序时序(the saga / 编排配方)必须继续作为一个本地可核对的整体存在,并且它调用的每个叶子操作(每个探测、每个决策、每个动作)按分层归位。绝不能把时序打散成"跨三个模块的接力"。**

对照 `md2-plan-2026-07-13.md` §目标3 的粗边界假设——"master_watch 只做探测 + 调用 master_revival 做决策 + 调用 monitor/platform 做物理动作"——这句话的落点正是:**编排(那句"调用")留在 master_watch,被调用的决策叶子去 master_revival,被调用的执行叶子去执行层。** 本稿在此基础上细化,不重新发明边界。

### 2.2 目标结构(三层归位)

**① `master_revival.rs`(决策层,扩容)—— 接收 §1.4 上移的纯决策叶子**

上移清单(全部是纯 DB 谓词/转移,无 tmux/进程副作用,与 `master_revival` 现有 `classify_master_death`/`query_master_runtime` 同族):
- `master_runtime_matches`、`master_runtime_generation_matches`(两个 generation 围栏)→ 合并进 `master_revival` 的 runtime 查询面(可与 `query_master_runtime` 归并为一组围栏谓词)。
- `master_recovery_verifying_window_expected_generation`、`recovered_workers_ready_sync`、`master_recovery_effective_readiness_timeout` → recovery-window 只读决策。
- recovery-window 相位写包装(`begin/mark/fail/complete/non-revive-terminal`)、`mark_session_closed_after_idle_master_death` → 作为 `master_revival` 对 `db::master_recovery`/`db::sessions` 的**决策适配器**(把散落的 inline SQL 收进决策层,消灭 867、1977 两处裸 SQL)。
- idle-vs-active 分支、backoff 判定 → 抽成 `master_revival` 的**返回枚举的纯决策**(例:`fn plan_after_worker_reap(...) -> ReapFollowup { CloseIdle, ProceedRevive, WaitBackoff{secs}, StaleAbort }`),让 master_watch 的 saga 变成"取决策→按决策 act",而不是内联 if/sleep。

**② `monitor::master_reaper.rs`(执行层,新增)—— 承接"拉起进程 / 清理级联"物理动作叶子**

这是 plan 里悬而未决的"要不要新增第三个模块"的答案:**建议新增,但作为第二阶段(见 §2.4)。** 归它的叶子:
- spawn 复活 master(env/creds/sandbox home 组装 + `build_master_revive_command` + tmux ensure/spawn/title/get-pid + 装探针 + inject continue)。
- `reap_failed_revive_master_best_effort` 全链(围栏→`sigkill_failed_revive_master_process`→`kill_failed_revive_master_pane`→撤探针)+ `reap_claimed_revive_master_after_error_best_effort`。
- `reprovision_declared_workers_after_master_revive` + `revive_reprovision_one_worker` + `restore_killed_worker_spawn_spec` + `collect_master_revive_recovery_intents_before_reprovision`。
- `write_master_revival_redispatch_marker`、`spawn_master_confirm_timer`。

落点选择:**放 `src/monitor/master_reaper.rs`(monitor 层内)**,不放 top-level。理由:这些是 daemon-local 的进程/tmux 物理动作(SIGKILL、pane kill、spawn),正是索引里 monitor 的职责("cascade/revival hooks on process death");而 `master_revival` 是 top-level 是因为它是**纯 DB 决策**(无副作用),两者分层性质不同。(此落点是 d1 建议;MD3 要求实施者在 PR 里对最终落点给出理由,r1 核。)

**③ `master_watch.rs`(探测 + 编排 saga,瘦身)—— 保留身份 + 保留时序契约**

保留:§1.2(A) 全部探测叶子(pidfd watch task、patrol、startup rearm、liveness/pane 探测、readiness ack/probe)+ 两个 saga 协调函数(`handle_master_death_detected`、`revive_master_after_exit_windowed`、`resume_master_recovery_readiness`),但后者重构为**读起来就是不变量清单**的有序协调器:每一步是一次对 `master_revival`(决策)或 `master_reaper`(执行)的具名调用,顺序**逐字保留**。

**为什么 saga 留在 master_watch 而不是再拆第四个"coordinator 模块":** 时序契约必须与喂给它的探测层相邻,且必须能"对着历史不变量清单在一个文件里逐行审"。再拆一层只会增加跨模块散射面,与 §2.1 原则相悖。

### 2.3 pub 契约不变承诺(给 r1 / client)

索引登记的 8 个 pub 符号 + `pub(crate) patrol_active_masters_once` 的**签名与语义不变**;4 个外部调用点(亲验)不受影响:
- `src/bin/ahd.rs:111` → `rearm_active_master_watches_on_startup`
- `src/orchestrator/mod.rs:79-80` → `resolve_master_watch_patrol_interval` + `master_watch_patrol_loop`
- `src/rpc/handlers/sessions.rs:27` → `spawn_master_pidfd_watch_task`
- `src/monitor/session_watch.rs:142` → `master_process_is_alive`

`MasterDeathSource` 枚举、`monitor_key`(见 §5 死符号)保持导出面。**对外行为零变化,与 plan §执行约束"目标3 RPC 协议契约对外不变"一致。**

### 2.4 分阶段落地(风险管理,d1 强烈建议)

operator 明言"拆错=HA 故事崩"。因此建议把目标3 拆成**两个前后依赖的 PR**,而不是一刀切:

- **PR-A(本设计推荐先做,低风险高收益)**:只做 **① 决策上移 + ③ saga 重构成有序协调器**,**不新增 master_reaper**;执行叶子先在 master_watch 内抽成具名 private helper(名字对齐 saga 步骤),留在原文件。
  - 为什么低风险:纯机械——(a) 把 §1.4 的内联决策表达式**在同一位置**替换成对 `master_revival` 的具名调用;(b) 把执行块抽成同文件 private fn。**零跨模块散射**,时序在原函数里逐字可见。48+10 个 inline test 全部原地不动;`super::` 导入仅需为上移的决策 fn 改成从 `master_revival` 引入。
  - 收益:直接消灭 plan 点名的耦合违规("决策本该在 master_revival"),并把 saga 变成可对不变量清单逐行审的形态。
- **PR-B(follow-up,需自己的 MD3 gate)**:把 PR-A 抽好的执行 private helper **上提**到 `monitor::master_reaper`。仅在 PR-A 的**并行 CI 绿**被实证后再做(§4)。
  - 为什么后置:跨模块搬移执行链(reap/reprovision)才是真正提高"拆丢一段"概率的动作,单独一个 PR、单独一次并行 CI 验证,回滚面小。

**备选(一刀切,d1 不推荐):** ①②③ 一个 PR 完成。收益是一次到位,代价是"决策上移的 diff"和"执行跨模块搬移的 diff"混在一起,review 时很难把"行为保持"和"结构移动"分开核,恰好是 §3 最怕的场景。若 operator 出于节奏偏好选一刀切,§3 的零回归论证仍成立,但 §4 的并行 CI 门必须一次覆盖全部三层。

> **d1 裁量点(需 operator 拍)**:PR-A/PR-B 两段式 vs 一刀切。d1 推荐两段式;这是节奏/风险偏好的目标层选择,留给 operator。

---

## §3 零回归论证(Q3,重点)

### 3.0 引子:当前代码已经是"corrected 设计"的成熟体,拆分的任务是"别把成熟体拆退化"

一个必须先讲清的关键事实(两位 research 助手交叉确认 + d1 亲验):**operator 点名的历史事故 spec,行号引用的是 PR#52 / 产品交付期的旧 master_watch(那时它没有 recovery window、没有 readiness gate、没有 real-reap)。当前 5704 行文件已经把那些 corrected 语义全部实现进去了**,而且——正是问题所在——**是以"两个上帝函数里的语句顺序"这种形态实现的**。

所以本节四类败因的论证结构统一为:

1. **该败因的防护当前长在哪**(亲验 file:line);
2. **历史 spec 怎么说这个顺序不能动**(incident-era 引用);
3. **本拆分会不会改变这条行为路径**;
4. **保证"拆后还能拼回同一正确顺序"的机制**;
5. **钉住它的测试**。

贯穿四类的**同一个机制保证**:§2.1 的"saga 留在一处、逐字保序"原则 + §2.4 PR-A 的"决策在同一位置换成具名调用、执行抽成同文件 helper"——**PR-A 不移动任何一条语句的相对顺序**,它只是把"内联表达式"换成"具名调用"、把"内联块"换成"具名 helper 调用",调用点位置不变。因此 PR-A 对四类败因的行为路径是**恒等变换**。PR-B 的跨模块上提才有非平凡风险,分别在每类下点名。

---

### 3.1 败因(a):stale-inflight 误判

**当前防护(亲验):**
- `classify_master_death`(`master_revival.rs:61`)对 **非 ACTIVE → IntentionalExit**、**cutover in-flight → IntentionalExit**(`:89`)、**pid/gen 不匹配 → Stale**(`:92`)。这是死亡路由的第一道闸(master_watch.rs:69 调用)。
- generation 围栏:`master_runtime_matches`(1754)、`master_runtime_generation_matches`(1352),在 readiness 循环(1590/1647)与失败收割(1283)前必查——**绝不对 stale 代 window 动作**(对应 coordination-design [incident-era] "generation fencing 是命门")。
- reprovision 原子性:`collect_master_revive_recovery_intents_before_reprovision`(2117)在**重灌前**把中断 job intent 抓进内存,再由 `spawn_realign_agent` 走原子替换——对应 `recovery-reinsert-atomicity-design.md` 的"同 id 重插、并发 `query_job` 永不见 None"。

**历史 spec:** `incident-stale-session-kill-cascade-2026-07-08.md`(stale `master_pane_id='%0'` 复用误杀)+ `recovery-reinsert-atomicity-design.md`(KILLED agent 删除经 FK cascade 连删 job)+ 真实 transcript 点名的回归测试 `master_revive_stale_inflight_dispatch_failure_does_not_overwrite_requeued_job`(现 `master_watch.rs:4243`)。

**拆分影响:** `classify_master_death` 已在 `master_revival`,不动。两个 generation 围栏是 §1.4 上移项——PR-A 把它们移进 `master_revival`,**调用点(1590/1647/1283)行为不变**(同一谓词、同一入参、同一返回)。reprovision 的 intent-capture-before-reprovision 顺序在 PR-A 不动;PR-B 把整条 reprovision 上提 master_reaper 时,**capture→replace 的先后 + 原子替换必须整体搬,不可只搬一半**(见下方机制)。

**保证机制:**
- PR-A:围栏上移是纯函数搬家,`master_runtime_matches` 的 SQL 谓词(`status='ACTIVE' AND master_pid=?2 AND master_generation=?3`)逐字不变。恒等。
- PR-B:reprovision 上提时,`collect_..._before_reprovision`(2117)、`revive_reprovision_one_worker`(2094)、`restore_killed_worker_spawn_spec`(2145)**必须作为一个整体单元搬进 master_reaper**,且 saga 里"先 capture 后 reprovision"的调用顺序(§1.3 中 resume_readiness 的 2028→2043 顺序)不变。**原子替换本身不在本拆分范围**(它在 `spawn_realign_agent`/`db::recovery::replace_killed_agent_and_requeue_job_sync`,不属 master_watch),拆分**不触碰**它——这是最强的零回归保证:名 `stale-inflight` 的原子性根本不在被拆的文件里。

**钉住的测试:** `master_revive_stale_inflight_dispatch_failure_does_not_overwrite_requeued_job`(4243,且是唯一带 `#[serial_test::serial(global_env)]` 的)、`master_revive_worker_reprovision_requeues_captured_interrupted_job`(4819)、`master_recovery_cutover_inflight_does_not_create_recovery_window`(2691)、`revive_master_readiness_*` 系列(1590/1647 围栏)。

---

### 3.2 败因(b):restart 不重装探针

**当前防护(亲验):** 这是当前代码**已经专门修过**的路径(revive-master-readiness-zombie-design [incident-era] "点2/P4b 回归点")。三重网:
1. `rearm_active_master_watches_on_startup`(110)→ 逐 active session `arm_or_route_master_watch`(159):重开 pidfd(179)→ 校 pane pid(204)→ **装表 + spawn watch task(220–232)**,或若已死则路由死亡(181–191/204–214)。
2. **在 MASTER_VERIFYING 窗内 resume in-flight readiness**(233–273):重启若撞见一个仍在就绪等待的窗,不 complete、不重判活即 ok,而是**重新装 probe/ack 等待**——精确对应 zombie-design [incident-era]:"活+窗 MASTER_VERIFYING(非终态)→重启 readiness 等待(重 arm probe/ack),不 complete"。
3. `master_watch_patrol_loop`(383)→ `patrol_active_masters_once`(121):`monitor::contains(key)` 缺失即补探(secondary net),防 pidfd 未注册/注册失败/重启只靠 patrol 的窗口(coordination-design [incident-era] `session_watch.rs:65-100` TOCTOU 讨论)。

**历史 spec:** zombie-design 点2/P4b 回归点(必随门改)+ coordination-design §1/§4(ahd 是常驻 service,lock 丢失后 session_watch 会误判无 revive-in-flight 而级联)。

**拆分影响:** 探测层(1、3)整体留在 master_watch(§2.2③),**不移动**。(2) 的 resume-readiness 依赖 `master_recovery_verifying_window_expected_generation`(1804,§1.4 上移决策)+ `resume_master_recovery_readiness`(1026,saga,留 master_watch)。PR-A 后:`arm_or_route_master_watch`(159)的 233–273 段仍在原位,只是其中 `master_recovery_verifying_window_expected_generation` 变成对 `master_revival` 的调用。恒等。

**保证机制:**
- 三重网全部属探测/编排层,§2.2③ 明确它们**留在 master_watch**,拆分不碰。这是四类里拆分影响面最小的一类。
- **红线**:PR-A/PR-B 都**不得**把 `rearm_active_master_watches_on_startup`→`arm_or_route_master_watch`→`resume_master_recovery_readiness` 这条重启恢复链的任一环挪出 master_watch,否则重启后"探针重装"与"就绪续等"会跨模块,失去本地时序可核性。
- 若 PR-B 把 spawn-watch-task 之后的 readiness 续等错误地划给 master_reaper,会重演"活→误 COMPLETED";因此 **readiness 续等归探测/编排(master_watch),不归执行层**——这条边界必须在 PR-B design gate 里显式声明。

**钉住的测试:** `startup_rearm_active_master_registers_watch_and_later_exit_routes_existing_path`(3799)、`startup_rearm_dead_master_immediately_routes_existing_path`(3847)、`startup_rearm_resumes_readiness_for_alive_verifying_window`(3898)、`startup_rearm_hung_verifying_window_fails_then_cascades`(3962)、`startup_rearm_skips_terminal_or_non_verifying_window`(4028)、`patrol_detects_dead_active_master_when_monitor_missing`(4096)、`pidfd_and_patrol_double_fire_only_handles_once`(4147)。**这 7 个测试正是 (b) 类的活体不变量清单**,PR 必须让它们在并行下全绿(§4)。

---

### 3.3 败因(c):cascade 击败 revive

**当前防护(亲验):** 当前代码已把 coordination-design 的 corrected 语义落地:
- 复活路径清 worker 用的是 `clean_worker_runtime_resources_sync`(597,`db::system:393`)——**非终态清理**(不写 `sessions.status='KILLED'`,ActiveWork 时 `preserve_session_anchor=true`,见 603 传参),而**不是**终态的 `cascade_kill_session_agents`。这正是 research-master-death-corrected [incident-era] "拆分 worker cleanup,不改 sessions.status" 的落地。
- 级联抑制改由 **DB 权威 `master_recovery_windows` 状态机**决定(begin/phase/complete),不再靠 in-process `master_spawn_lock` 单独判定(coordination-design [incident-era] "lock 不是权威状态")。`session_watch` 依 window 决定 defer。
- 受控级联只在 **readiness 失败后**发生:`resume_master_recovery_readiness` 的 1057/1100/1182 三处 `cascade_kill_session_agents`,每处都**先 `fail_master_recovery_readiness`**(1093)、**后 generation-fenced `reap_failed_revive_master_best_effort`**(1106)。级联不再"抢在 revive 之后二次杀活 worker",而是"revive 判定失败后收尾"。

**历史 spec:** `incident-stale-session-kill-cascade-2026-07-08.md`(cascade + 并发 kill 流交错 → 死 master → 连坐 worker → revive 被 reap)+ coordination-design run2("约 5.5s 后 anchor cascade 杀掉刚复活的 w1")+ research-master-death-corrected("cascade 的 `status='KILLED'` 结构性击败 revive")。

**⚠️ 命名陷阱(research 助手交叉提示,必须写进设计):** `recovery-reinsert-atomicity-design.md` 里的 "cascade" 指 **SQLite FK `ON DELETE CASCADE`**(事务内连删 job),**不是**本类的 anchor cascade。本类 (c) 只对应 anchor cascade(`cascade_kill_session_agents` / `"ANCHOR_UNIT_STOPPED"` / `"MASTER_REVIVE_*_TIMEOUT"`);FK cascade 归 (a) 的原子性,不要混。

**拆分影响:** 三个关键点——(i) 清 worker 用非终态原语、(ii) window 状态机权威、(iii) readiness-fail→fail-window→fenced-reap 的顺序——(i) 调用 `db::system`(不属被拆文件),不动;(ii) window 相位写是 §1.4 上移项;(iii) 是 `resume_master_recovery_readiness` saga 内序。

**保证机制:**
- (i) 最强保证:`clean_worker_runtime_resources_sync` vs `cascade_kill_session_agents` 的**选择**写在 saga 语句里(597 vs 1057),二者都是 `db::system` 的既有原语,拆分**不改调用哪一个**。PR-A 保序 → 选择不变。
- (ii) window 相位写上移到 `master_revival` 决策适配器后,`session_watch` 读的还是 `master_recovery_windows` 表(DB 权威),**跨进程一致性靠 DB 不靠内存**,模块归属变化不影响。
- (iii) PR-A 保序,`fail_readiness`(1093)→`cascade`(1100)→`fenced reap`(1106)三步相对顺序逐字不变。PR-B 若把 `reap_failed_revive_master_best_effort` 上提 master_reaper,**必须保证 saga 里仍是"先 fail-window、再 cascade、再 reap"**——reap 的 generation 围栏(1283)是防"reap 到新 generation 活 master"的命门,和 cascade 的先后不能倒。

**钉住的测试:** `master_revive_lifecycle_hung_claude_ack_timeout_fails_and_cascades`(3286)、`master_revive_lifecycle_anchor_decision_expired_window_cascades_now`(3420)、`worker_readiness_timeout_reaps_failed_revive_master`(3671)、`revive_readiness_timeout_does_not_complete`(2823)、`master_recovery_master_watch_failure_marks_window_failed`(2657)、`master_recovery_fuse_marks_window_fused`(2745)。

---

### 3.4 败因(d):failed-revive 孤儿壳

**当前防护(亲验):** 每一条从 revive saga 退出的**失败路径都配一个 generation-fenced 真收割**:
- 顶层 catch:`revive_master_after_exit_locked`(526)在 windowed 返回 Err 时 → `mark FAILED`(551)+ `reap_claimed_revive_master_after_error_best_effort`(563,claimed_generation=expected+1)。
- readiness/worker-gate 失败:1063/1106/1188 三处 `reap_failed_revive_master_best_effort`。
- finalize 撞 stale:`complete_claimed_master_transition` 返回 None(842)→ **杀孤儿 pane**(851)。
- reap 链本体:`reap_failed_revive_master_best_effort`(1276)→ 围栏 `master_runtime_generation_matches`(1283)→ `sigkill_failed_revive_master_process`(1392,含 `monitor::with_borrowed` pidfd SIGKILL + libc fallback + ESRCH 幂等)→ `kill_failed_revive_master_pane`(1440,`kill_pane_if_owned`)→ `remove_master_monitor_key_if_generation_matches`(1342,撤探针)。

这精确回应 design-master-death-corrected [incident-era] "防僵尸命门是进程和 registry 真清理,而不只是 DB 标 KILLED",以及 transcript 真实事故(cutover 失败后 reap-old-master 定时器未取消 → 孤儿;这里对应的是 revive 失败后**主动**收割而非留孤儿)。

**历史 spec:** research/design-master-death-corrected(fuse 只 DB-mark ≠ real reap)+ zombie-design(false COMPLETED = 永久 master 侧僵尸)+ transcript findings Bug A(reap 定时器未取消 → 孤儿)。

**拆分影响:** 整条 reap 链(1212–1459)+ finalize 孤儿 pane kill(851)是执行层叶子,**是 PR-B 上提 master_reaper 的主要内容**——这也是本类拆分风险最高处。

**保证机制:**
- PR-A:reap 链在 master_watch 内先抽成具名 helper,**每个失败退出点仍在原位调用它**(561/1063/1106/1188/851),恒等。
- PR-B(高风险,专门论证):把 reap 链上提 master_reaper 时的**不可拆分单元**是——`fence(1283) → sigkill(1392) → pane-kill(1440) → remove-watch(1342)` 这四步的**顺序与"每个失败退出点都要调用它"的完备性**。
  - **"拆丢一段"的具体形态**:若上提时漏掉某个失败退出点(例如只把 readiness 失败路径改成调 master_reaper,却漏了 `revive_master_after_exit_locked` 顶层 catch 的 563),就会出现"某条失败路径不再收割 → 孤儿壳"。
  - **防丢机制**:(1) reap 链上提为**单一 pub(crate) 入口**(如 `master_reaper::reap_failed_revive_master(ctx, session, pid, gen, pane)`),master_watch 所有失败退出点都调它,靠"只有一个入口"杜绝"改一半";(2) 依赖**既有的 reap 事件录制器** `FailedReviveMasterReapEvent`(1461)+ `record_failed_revive_master_reap_event`(1509)——现有测试(`revive_failure_reaps_orphan_gen2_master` 等)正是断言这些事件按序发生,上提后测试仍绑同一入口,**任何漏调都会让对应测试失败**;(3) 围栏 `master_runtime_generation_matches` 上提到 `master_revival`(§1.4)后,master_reaper 调它,fence 语义不变。
- **额外保证**:reap 是 best-effort(不 `?` 传播),拆分不得把 best-effort 变成会 panic/传播错误的形态,否则一个 reap 子步失败会中断后续 reap 子步 → 又留孤儿。上提时保持"每个子步独立 best-effort + 各自 tracing::warn"的结构。

**钉住的测试(这组是 (d) 类的活体不变量,PR 必须全绿):** `revive_failure_reaps_orphan_gen2_master`(3568)、`revive_failure_master_reap_is_generation_fenced`(3626)、`revive_error_reaps_claimed_gen2_master`(3645)、`worker_readiness_timeout_reaps_failed_revive_master`(3672)、`happy_revive_readiness_path_does_not_reap_master`(3721,**反向**:成功路径绝不误收割)、`master_revive_lifecycle_happy_claude_ack_completes_without_zombie`(3137)、`test_master_revive_fuse_reaps_worker_and_does_not_spawn`(5502)。

---

### 3.5 四类小结:拆分风险的分布

| 败因 | 防护当前所在层 | PR-A 影响 | PR-B 风险 | 拆分风险等级 |
| --- | --- | --- | --- | --- |
| (a) stale-inflight | 决策(已委派)+ 原子性(在 spawn_realign,不属本文件) | 恒等(围栏上移) | 中(reprovision 整体搬) | **低** |
| (b) restart 不重装探针 | 探测/编排(留 master_watch) | 恒等 | 低(明令不外移恢复链) | **最低** |
| (c) cascade 击败 revive | 执行选择(`db::system` 原语)+ window 状态机 + saga 内序 | 恒等 | 中(reap 与 cascade 保序) | **低–中** |
| (d) failed-revive 孤儿壳 | 执行(reap 链,PR-B 主搬对象) | 恒等(先抽同文件 helper) | **高(单入口 + 完备性)** | **中–高** |

**总判断**:PR-A 对四类均为恒等变换,可安全先行。PR-B 的 (d) 类是全拆分的最高风险点,靠"单一 reap 入口 + 既有 reap 事件测试 + 完备性覆盖每个失败退出点"三重防丢;这也是为什么 §2.4 强烈建议 PR-B 独立成 PR、独立过并行 CI。

---

## §4 验证计划(Q4:并行 CI 下也绿)

### 4.1 "并行 CI"到底是哪个 job(亲验,不臆测)

operator 要求"这两套 inline test 在**并行 CI**(不是本地串行)下也绿"。亲验后关键事实:

- **并行门已存在,就是 `.github/workflows/ci.yml:41` 的 `cargo test --all-targets`**——它**没有** `--test-threads=1`,`cargo test` 默认线程数 = 逻辑 CPU 数,即**这一步本来就是并行跑 lib inline test 的**。master_watch(48 个 `#[test]`/`#[tokio::test]`)+ master_revival(10 个)inline test 就在这一步并行执行。
- **本地收口才是串行**:plan §执行约束的 `CARGO_BUILD_JOBS=1 cargo test --workspace --test-threads=1` 是**串行**的——这正是 operator 说的"串行掩盖过并发 bug"的那个串行。
- 因此 Q4 的答案不是"去搭一个并行 job",而是:**(1) 保证拆分不破坏 ci.yml:41 已有并行门的绿;(2) 消除一个当前靠串行/巧合掩盖的并发脆弱点,让并行绿是"可靠的"而非"碰巧的"。**

### 4.2 当前并行脆弱点(亲验,这是 operator 直觉的实锤)

| 全局可变状态 | 位置 | 并行安全? | 说明 |
| --- | --- | --- | --- |
| `MASTER_SPAWN_LOCKS`(`static`) | `master_revival.rs:12` | ✅ 按 session_id 键 | 唯一 session id 的测试互不干扰(仅永不驱逐,轻微内存增长) |
| `FAILED_REVIVE_MASTER_REAP_RECORDERS` | `master_watch.rs:1469` | ✅ 按 session_id 键 | 同上 |
| `REVIVE_MASTER_READINESS_{PROBE,ACK}_OVERRIDES` | `master_watch.rs:1669/1674` | ✅ 按 session_id 键 | test seam 的**正确范式** |
| tmux server | 测试用 `TmuxServer::new(&state_dir)` | ✅ socket 名派生自唯一 tempdir | 每测试独立 socket → 并行安全 |
| pidfd registry `monitor::register/remove` | key=`master:{session}:{gen}` | ✅ session 唯一 | 并行安全 |
| **进程级环境变量** | 见下 | ❌ **进程全局,是唯一真隐患** | |

**环境变量隐患(核心)**:`AH_MASTER_REVIVE_READINESS_TIMEOUT_SECS` 被 **7 个测试**用 `EnvVarGuard::set` 改写(`master_watch.rs:2824/2899/2972/3053/3899/3963/4029`),它们**都没有** `#[serial]` 标记;而**只有一个**测试(`4242`,改 `CCB_TMUX_ENTER_DELAY`)带 `#[serial_test::serial(global_env)]`。也就是说:代码库**已经知道** global-env 是并发隐患(为一个测试上了 `serial_test`,`Cargo.toml:66` 有 `serial_test="3"`),却**漏掉了这 7 个 readiness-timeout 测试**。它们当前并行不炸,靠的是"都设成同一个值 `"10"`"这种**巧合**,加上 `std::env::set_var` 在多线程下本就是数据竞争(读到默认值 vs `"10"` 的窗口会让 readiness 超时判定漂移 → 偶发 flaky)。这就是"串行掩盖并发 bug"的活样本。

`master_watch.rs:393`(`AH_MASTER_WATCH_PATROL_SECS`)、`db/master_recovery.rs:49/57`(`CASCADE_DEFER_SECS`/`READINESS_TIMEOUT_SECS`)是这些 knob 的读点。

### 4.3 拆分对并行安全的硬约束 + 主动加固

**硬约束(拆分必须遵守,否则新增并行 bug):**
1. **不得新增任何进程级可变 static / 环境变量依赖。** master_revival / master_reaper 里所有 test seam **必须按 session_id 键**(照抄现有 `*_OVERRIDES` 范式),不得引入新的进程全局旋钮。
2. 上移决策/执行叶子时,**保持它们无隐藏进程全局态**(现有 reap 录制器、readiness override 都是 session 键,搬家时键不变)。
3. 保留所有 `cleanup_test_tmux_server` 调用(2500/2894/…):`ci.yml:46-47` 的 `test_no_orphan_tmux_after_test_suite --ignored` 是独立 CI 门,拆分若漏掉某测试的 tmux 清理 → 孤儿 tmux server → 这个门直接红。**拆分不得动测试的 tmux 生命周期。**

**主动加固(d1 建议纳入 PR-A,直接服务"并行下可靠地绿"):**
- 把 `AH_MASTER_REVIVE_READINESS_TIMEOUT_SECS` 的读点**追加一个 session 键的 test override**(照抄 `revive_master_readiness_probe_override` 范式:`resolve_master_revive_readiness_timeout_secs()` 在 `#[cfg(test)]` 下先查 session 键 override,查不到再读 env)。让那 7 个测试改用 session-scoped override 而非改写进程 env。
  - **为什么优于"给这 7 个测试都打 `#[serial]`"**:打 serial 会把它们**串行化**——那恰恰是把并行覆盖降级回串行,与"证明并行下也过"背道而驰。session-scoped override 是**移除**全局依赖,让它们**真并行安全**,并行门才是可靠绿而非巧合绿。
  - 风险:这是 test seam + 一个 resolver 的 `#[cfg(test)]` 分支,零生产行为变化,低风险;但**属于 test 基建改动**,建议在 PR-A 里作为独立 commit,便于 review 与回滚。

### 4.4 PR 里"证明并行下也过"的具体步骤(实施者照做,r1 核)

1. **本地并行复现(不是串行收口)**:
   - `cargo test --lib monitor::master_watch::tests master_revival` **不带** `--test-threads=1`(默认多线程),连续跑 ≥20 次无 flake;
   - 追加一轮高压:`--test-threads=16`(或 `> CPU 数`)放大调度交错,再连跑 ≥20 次。
   - 这一步的目的就是**主动否证** 4.2 的 env 隐患:加固前若能复现 flaky、加固后连续绿,即实证"串行掩盖的并发点已消除"。
2. **收口仍双跑**:plan 要求的 `CARGO_BUILD_JOBS=1 cargo test --workspace --test-threads=1`(串行,catch 顺序依赖)+ 上面的并行跑(catch 竞争)**都要绿**,两者都进 PR 收口证据。
3. **CI 证据**:PR 的 `ci.yml:41 cargo test --all-targets`(并行)+ `ci.yml:46-47` tmux 孤儿门,均绿。master 作为盯 CI 人(CLAUDE.md master 铁律)确认这两步绿才算 (Q4) 闭环。
4. **PR-B 单独重跑全部上述步骤**(尤其 (d) 类 reap 测试的并行跑),因为 PR-B 才是跨模块搬 reap 链、最可能引入"某失败路径漏收割"的 PR。

> **不做隐性截断**:若实施中发现某个 readiness/reap 测试在并行下无法稳定(例如依赖真实 tmux 时序),**必须显式登记为验证债并说明,不得偷偷打 `#[serial]` 蒙混过关**——打 serial = 假装并行绿。这条写进 PR 交接。

---

## §5 审计中发现的现存问题(不属拆分范围,但拆分不得静默掩盖)

MD2 是"纯搬移不改行为"。以下三处是 d1 走源码时发现的**现存**问题,**拆分不修、但也不得静默带走**——每条要么在 PR description 显式登记为"保持现状/已知问题",要么单开 issue,不许趁拆分悄悄改了行为:

1. **死符号 `master_watch::monitor_key`(494,无 generation)**:亲验**无任何生产/测试调用者**(`rg` 确认;它不在 tests 的 `super::` 导入表里;`session_watch.rs:67/127` 的 `monitor_key` 是该文件自己的同名参数,非此函数)。与之并存的是 `master_revival::master_monitor_key(session, gen)`(带 generation,全局在用)。**建议**:拆分时顺手删或至少标注,别让"搬家"把死代码搬进新模块伪装成活的。
2. **分层倒挂:monitor(Layer 2)→ rpc::handlers(Layer 1)**:`master_watch.rs:26 use crate::rpc::handlers::{RealignAgentParams, spawn_realign_agent}`——reprovision 反向调用 RPC handler 层。把 reprovision 上提 master_reaper **不修复**这个倒挂(仍旧向上调 `spawn_realign_agent`)。**建议**:PR-B 显式记录该跨层耦合为"已知、超出本轮范围",不得假装不存在;真正解耦需把 `spawn_realign_agent` 的可复用核心下沉,属 Wave 2 议题。
3. **每次 revive 造新 `ClaudeGatewayService`**:`ctx_from_watch_parts`(414)与 `reprovision`(2041)都 `Arc::new(ClaudeGatewayService::new())`,而非共享 daemon 的 gateway service。这是**行为可疑点**(复活/重灌路径拿的是全新 gateway,可能丢 worker 注册缓存),但**修它=改行为,不属 MD2**。**建议**:登记为独立 issue,拆分时**保持现状**(照搬 `ctx_from_watch_parts` 的构造),绝不趁拆分"顺手修好"——那会把结构 PR 变成行为 PR,污染 §3 的零回归论证。

---

## §6 自我红队 + 覆盖诚实度(反讨好)

**本任务无 o1 发散/对抗材料**:operator 直接把 target3 派给 d1 作唯一设计 gate,未附 o1 divergence brief。故本节不是"采纳/驳回 o1 反方",而是 d1 对自己方案的红队(反讨好铁律:不为收敛而收敛)。

**自我红队(我的方案最可能错在哪):**
1. **"saga 留一处"会不会只是把上帝函数换个说法留着?** 部分成立。PR-A 后 `revive_master_after_exit_windowed` 仍是最长的函数,只是从"内联一切"变成"具名调用序列"。反驳:这是**刻意**的——时序契约必须单点可核;真正的收益是"每步是具名决策/执行调用"后,行数下降 + 可对不变量清单逐行审,而不是追求函数变短。若 operator 认为"函数不够短=没解耦",那是把行数当指标,与 §2.1 原则冲突,需 operator 明确取舍。
2. **新增 master_reaper 是否值得?** 可辩。执行叶子大多已是对 `db::system`/`tmux`/`spawn_realign_agent` 的 glue,单独成模块的收益主要是"saga 读起来是纯编排"+"执行叶子可独立单测"。若 operator 认为收益不抵跨模块搬 reap 链的风险,**PR-B 可以不做**,只做 PR-A(决策归位 + saga 重构),master_watch 仍显著改善。这是我把它设成第二阶段的原因——它是可选的。
3. **决策上移到 master_revival 会不会让 master_revival 膨胀成新上帝?** 需警惕。上移的是**纯谓词/薄适配器**(约 200 行),`master_revival` 现 928 行、结构清晰,承接后仍是"决策助手"。但相位写适配器有把 master_revival 拉向"知道 recovery window 细节"的倾向;边界要守住:**master_revival 只暴露"给我一个决策/写一个状态转移",不暴露 saga 编排**。若实施中发现适配器开始承载顺序逻辑,就是越界信号。
4. **(d) 类 PR-B 的"单入口"防丢,够不够?** 这是我最不放心的一环。单入口能防"改一半",但防不了"某个失败退出点从一开始就没接入 reap"(现存逻辑里若已有这种洞,拆分会原样带走)。缓解:PR-B 必须附一张"revive saga 全部失败退出点 × 是否调 reap 入口"的对照表,r1 逐格核——这比"信任单入口"更硬。**这张表是 PR-B 的验收硬件,我在此显式要求。**

**覆盖诚实度:**
- **亲验(走了源码)**:master_watch.rs 生产码 1–2323 全读;master_revival.rs 生产码 1–534 全读;两文件结构 `rg`;调用图(4 个外部调用点、决策委派点、DB reach、分层倒挂)`rg` 亲验;并行相关全局态(env/static/tmux/CI)`rg` + 读 `ci.yml`/`Cargo.toml` 亲验;§3 四类的当前防护 file:line 全部亲验。
- **依赖二手(research 助手 + 历史 spec)**:7 份事故档案的机制细节与 incident-era 行号——我核对了其中的**符号仍存在于当前源码**(如 `confirm_master_stable`@291、`complete_master_recovery_window_for_master_watch`@1958、点名测试@4243),但**未逐行重读 7 份 spec 全文**,采信了两位 research 助手的抽取 + 引文。若 operator 要更高保真,可让我逐份复核。
- **最弱区域(我主动点名)**:
  1. **master_watch.rs 测试段(2325–5704,约 3379 行)我只读了测试**名与关键断言点(§3 钉住的测试),**未逐个读完 48 个测试体**。我对"这些测试覆盖了四类不变量"的判断基于测试名 + 抽样读(如 readiness/reap 系列),不是逐测试通读。拆分实施时,实施者需确认每个上移符号的 `super::` 导入改对——这层我给了方向但没给逐行 diff。
  2. **PR-B 的 reap 链跨模块搬移**我给了"单入口 + 完备性表"机制,但**没有画出 reap 链上提后的完整新签名/新调用图**——那是实施设计(tasks 级),超出本边界稿;我的边界稿到"哪些叶子归哪层 + 保序机制"为止。
  3. **`spawn_realign_agent` 的原子替换内部**(reinsert-atomicity 的真正落点)我**未展开读** `db::recovery::replace_killed_agent_and_requeue_job_sync` 全文——我的零回归论证在 (a) 类恰恰依赖"它不在被拆文件里",所以不需要动它;但若 operator 想确认原子性本身无恙,那是另一条审计线,不在 target3 拆分范围。

**一句话给 operator**:本稿的骨架结论——**"当前代码已是 corrected 设计的成熟体,四类败因防护是两个上帝函数里的语句时序;因此按'保 saga、抽 step、决策归 master_revival、执行归新 master_reaper、分两阶段'来拆,PR-A 是恒等变换、PR-B 用单入口+完备性表防丢"——是走了源码的,不是编的**;最该被 operator 挑战的是 §2.4 的两阶段 vs 一刀切(节奏取舍)和 §6.4 的 (d) 类防丢强度。

