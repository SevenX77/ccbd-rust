# MD2 Wave-2 计划草案(待 operator 审)

Status: **草案,未派任何实施**。写于 Wave-1(target1-3 四个 PR,#153/#155/#156/#158)全部合入 main 之后,operator 下达"选下一批 2-3 个 1000+ 行 ownership center,优先数据层"指令时的 survey 结果。operator 随后下达"这轮做完就停,不开 Wave-2 实施"——本文件是那个"做完"的落盘,供下一轮直接接续,不用重新 survey。

来源:`research/architecture-index.md` capability→owner 表 + 本次对候选文件的函数级 survey(`wc -l` + `grep '^pub'` 实测,2026-07-13,HEAD 87bd01d)。

## 候选行数复核(实测,非索引旧数字)

| 文件 | 行数 | 说明 |
| --- | --- | --- |
| `src/provider/home_layout.rs` | 3039 | 未变(Wave-1 未碰) |
| `src/db/system.rs` | 3491 | 较 MD1 索引记录的 3485 略涨(target3 期间/之后有其它改动) |
| `src/db/state_machine.rs` | 2978 | 未变 |
| `src/db/jobs.rs` | 2662 | 未变 |
| `src/rpc/handlers/sessions.rs` | 1869 | target2(#155)已拆过一轮(cutover handler 移出),现低于 2000,暂不够格当 Wave-2 主目标 |
| `src/rpc/handlers.rs` | 1426 | 门面文件,暂不够格 |

## 三个候选 + 风险分级(operator 指令:触及 master 自愈/凭据/派单核心 = 升级)

### 候选1:`db::system.rs`(3491L)—— 低风险,拟自闭环

**函数级实测**(`grep '^pub'`)显示四类现成边界,和 MD1 索引描述一致:

- **启动 reconcile**:`reconcile_startup_sync`、`reconcile_startup_sync_with_state_dir`、`reconcile_orphan_scopes_sync`、`reconcile_orphan_scopes_with_runner_sync`、`reconcile_master_recovery_windows_sync`、`reconcile_active_agents_to_crashed_sync`,+ 对应 async 包装 `reconcile_startup*`。
- **system dump**:`system_dump_sync` / `system_dump`。
- **级联清理**:`cascade_kill_session_agents_sync`、`cascade_kill_session_agents_with_runner_sync`、`clean_worker_runtime_resources_with_runner_sync`、`clean_worker_runtime_resources_sync`、`snapshot_master_death_session_activity`、`session_agent_ids_sync`、`stop_session_anchor_for_session_sync`,+ async 包装。
- **socket/沙盒目录清扫**:`sweep_stale_tmux_sockets_sync`、`remove_agent_sandbox_dir_sync`、`remove_agent_sandbox_dir_preserving_home_sync`。

**⚠️ 一处必须标注的耦合(不是我杜撰,是 target3 设计稿已经记录过的事实)**:`cascade_kill_session_agents_sync`(:188)硬编码 `UPDATE sessions SET status='KILLED'`——这正是 d1 target3 设计稿点名"结构性打败 revive"的那个函数,`monitor::master_watch`/`master_reaper` 的 saga 里会调用这条链路做 worker 级联清理。这个函数**物理上住在 db/ 层**,但**语义上是 master 自愈 saga 的一部分**。

**边界假设**:
- 第一刀只切**启动 reconcile + system dump + socket/沙盒清扫**三类(它们之间、与级联清理之间耦合最弱,纯粹是"运维/诊断/清扫"职责),各自搬进 `db::system::startup`(或类似命名)/ `db::system::dump` / `db::system::sweep` 子模块,`db::system` 保留门面 re-export,不改任何函数体、不改调用点签名。
- **级联清理(cascade_kill_session_agents* 一族)先不动**,留在 `db::system` 原处——不是因为技术上难拆,是因为它是 master 自愈 saga 的耦合点,按 operator "触及 master 自愈=升级" 的标准,这部分如果要拆,应该走 target3 那一档的设计稿+合并前 gate,不该混进"低风险自闭环"批次里顺手带走。
- 这样处理后,Wave-2 候选1 的范围是"启动 reconcile / dump / sweep 三类"的**纯搬移**(不动级联清理),风险定级低,可以自闭环。

### 候选2:`db::jobs.rs`(2662L)—— 触及"派单核心",拟升级

`dispatch_job_to_agent_sync`、`claim_next_job_sync`、`insert_job_sync`、`mark_job_completed`、`mark_job_failed`、`request_dispatched_job_cancel`——这些函数**字面意义上就是"派单"**(job dispatch 引擎本体)。另外还混了 `collect_reply_for_dispatched_job`/`distill_reply`/`strip_ansi_escapes` 这类回复抽取/文本处理逻辑,与"派单状态机"是不同职责,是天然的解耦候选,但**不建议按低风险自闭环处理**。

**边界假设(草)**:分离出(a)job 持久化状态机(insert/claim/dispatch/complete/fail/cancel/requeue,核心 CAS 逻辑)与(b)回复抽取/文本处理(distill_reply/strip_ansi_escapes 一类,纯函数、无状态、可安全独立)。(b) 部分风险低,(a) 部分因为直接触及"谁能领到下一个任务"这个派单核心决策,按 operator 标准应该跟 target3 走同一档流程:**d1 先出设计稿 → 设计稿发 operator 过目 → 过了才派 codex 实施 → 合并前再过一道 gate**。

### 候选3:`provider::home_layout.rs`(3039L)—— 触及"凭据",拟升级

`materialize_auth_file_with_ladder`(凭据材料化)、`prepare_claude_home_layout_with_gateway`(Claude gateway 环境接线)、`build_ah_hook_command`(hook 桥接命令解析——本轮 MD1 索引里记录过的 current_exe 事故落点)——这几条全部直接碰凭据/gateway 桥接。`prepare_home_layout_*` 那组 7 个组合爆炸的重载变体(`_with_role`/`_with_extensions`/`_for_slot`/`_and_claude_credentials` 排列组合)本身就是设计问题不是纯搬移问题,需要先决定收敛成 builder 模式还是别的形态。

**边界假设(草)**:同 db::jobs.rs,按 operator 标准升级处理——d1 先出设计稿(需要包含:①7 个 `prepare_home_layout_*` 重载怎么收敛;②凭据材料化/gateway 接线这部分的公共面边界;③current_exe 陷阱在重构后是否还成立的零回归论证,参照 target3 §3 的论证结构)→ 发 operator → 过了才派实施 → 合并前再一道 gate。

## 排序建议(暂定,operator 审批后可调整)

1. **候选1(db::system,低风险子集)先做**——不需要 d1,不需要升级流程,直接自闭环,验证"数据层拆分"这条新流程本身能不能跑通(类似 Wave-1 用 target1 pilot 验证 worktree→PR→CI→r1 流水线的逻辑)。
2. **候选2(db::jobs)和候选3(home_layout)的设计稿可以并行起**(都需要 d1,但 d1 单线程,需要排队或者一个稿子接一个稿子写)——这两个都要先过 operator 设计门,不急着抢时间,可以等候选1 跑完、或者提前把设计稿写出来攒着。

## 未决事项(留给下一轮/operator 定)

- `db::state_machine.rs`(2978L)本轮未纳入正式候选——它是 agent 生命周期的中枢,几乎所有模块都依赖它的状态转移语义,虽然物理上是"数据层",但语义上影响面接近"派单核心"级别(状态是否 IDLE 直接决定能否被派单)。建议下一轮明确问 operator 这个算不算"触及派单核心"该升级,还是按纯数据层处理——本文件先不下判断,留白。
- Wave-1 踩过的坑已机制化进 `.ah/rules/master.md`(派单前 worker 要读的 brief/design/index 必须先 commit 到 main,不能只在本地工作树)——下一轮派单前会照做,不用重复交代。

## 状态

**未派任何实施。三个候选 + 风险分级为草案,等 operator 审后再动。** workers 全部 IDLE,栈已收干净。
