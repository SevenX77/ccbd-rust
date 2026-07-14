# 剩余任务模块化重编排 — rebatch plan（2026-07-10，master 回执）

用户拍板:剩余任务**按大模块分组**,一个大模块内全部改动做完后**统一跑一次 cargo 编译+测试**(不再逐任务红绿)。本文=模块分组表 + 每模块任务清单 + 排期。

## 分组原则
按**域**聚合 + **文件重叠**归同模块(重叠文件必须同模块,否则收口 cargo 会互撞);域不重叠的模块 = 可双 worktree 并行。

## 模块表

| 模块 | 域 | 任务 | 主要文件（不重叠保证并行） | worktree/分支 |
|---|---|---|---|---|
| **A** | 完成判定/状态机 | P0-3 迟到完成白名单对称化 + orphan `mark_agent_idle_recaptured_*` 三件套清理 + 300s log 静默超时 + unsafe_no_sandbox flag 透传 | `db/state_machine.rs`、`provider/health_check.rs`、`completion/*` | wt-modA / feat/modA-completion-statemachine |
| **B** | 进程环境/生命周期 | 身份显式注入替代 cgroup 嗅探 + tmux 清理兜底 + C2 teardown 两机械向量(pkill 模式收紧 + unit BindsTo) | `platform/linux/identity.rs`、`agent_io/registry.rs`、kill/teardown 站点(`rpc/handlers/agent.rs`/`sessions.rs`/`orchestrator/mod.rs` kill 段)、systemd unit 模板 | wt-modB / feat/modB-process-env |
| **C** | 恢复/熔断 | P0-2(熔断清零洞 + 认领 cancel)——**在途,照现流程收尾,不重编排** | `orchestrator/mod.rs`、`db/recovery.rs`、`db/jobs.rs` | wt-p02(已存在) |

**A × B 文件不重叠**(A=state_machine/health_check/completion；B=identity/registry/teardown-kill/home)→ 可 a1+a2 双 worktree 并行,各自收口只跑一次 cargo。**C(P0-2)与 A/B 无重叠**(recovery/jobs/orchestrator-respawn),独立收尾。

> ⚠ 需实施者收口前复核的潜在重叠点:B 的 teardown-kill 段若触及 `orchestrator/mod.rs`,与 C(P0-2)改的 `orchestrator/mod.rs:798` respawn 段是否同文件?**同文件不同函数可容忍(收口各自 cargo),但 B 要等 C 合入后再基于新 main 拉分支**,避免 rebase 冲突。排期已按此串。

## 每模块 TDD/执笔权流水线(模块粒度)
1. **a4 一次性写全模块契约 RED 测试** + 必要 unimplemented 缝桩,一个 commit;可只跑受影响文件的**定向编译核**(cargo check 局部),**不跑全量**。
2. **a1(或 a2)把模块内所有任务全部实施完**(纯实施,不碰测试文件)。
3. **收口统一一次** `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`(串行),全模块 RED→GREEN **一次出证**。红绿证据**按模块批出**,不逐任务。
4. **a4 按模块整批审**实施 diff(diff 大分文件审,但 cargo 只收口跑);ACCEPT→operator 推 PR+auto-merge。
5. 每模块 = **一 worktree + 一分支 + 一 PR**。

## 模块 A 任务清单(完成判定/状态机)
- **A1 P0-3 白名单对称化**:`state_machine.rs:1142/1162` 迟到完成接受门——P0-1 删了 PANE_DIFF_STUCK 生产源后只剩 HEALTH_CHECK_STUCK,做对称化清理(去掉已死的 PANE_DIFF_STUCK 分支/白名单不对称)。
- **A2 orphan recapture 清理**(backlog #14):P0-1 后 `mark_agent_idle_recaptured_with_pane` / `_health_check_with_pane` 等三件套无调用者=死码,删净(删前 grep 确认零 caller)。
- **A3 300s log 静默超时**:既有待办——完成层日志静默达 300s 的权威超时处置(与感知设计轮的 Unknown 预算方向一致,但本模块只做既定的 300s 机械门,不做设计轮的大改)。
- **A4 unsafe_no_sandbox flag 透传**(backlog #8):`check_pending_tasks_from_log_root` 硬编码 `unsafe_no_sandbox=false` → 透传真 flag(a4 P0-1 审时提的非阻断项)。
- a4 契约 RED:各项一条断言(对称化后无 PANE_DIFF_STUCK 迟到接受、orphan 函数不存在/无 caller、300s 超时触发权威处置、unsafe flag 透传生效)。

## 模块 B 任务清单(进程环境/生命周期)
- **B1 身份显式注入**(§一.10):`platform/linux/identity.rs` daemon 靠 `/proc/self/cgroup` 嗅探自身 unit → 改**显式参数注入**,禁环境嗅探(防测试子进程误认活栈身份 teardown 掐死活栈)。
- **B2 tmux 清理兜底**(§一.12):`agent_io/registry.rs:145-160` expected_pid 存在但已死时 `kill_*_if_owned` 失败无兜底、session 泄漏 → 补兜底。
- **B3 C2 teardown 两向量**(§三.2):e2e teardown 逃逸(已证实 CONFIRMED,panic/failure 路径漏杀 ahd)——两机械向量:pkill 模式收紧 + unit BindsTo 关系修正。
- a4 契约 RED:身份注入不再嗅探 cgroup(注入值优先)、tmux 兜底覆盖 dead-pid 泄漏、teardown 在 panic/failure 路径也杀净 ahd。

## 未归入 A/B 的项(评估结果)
- **per-worker 独立凭据**(§一.13,§三.4):OAuth 凭据 symlink→per-worker。域=凭据隔离,文件 `provider/home_layout.rs`,与 A/B 不重叠但属**§三.4 第二批**(非本机械批)→ **另立模块 D**,排在 A/B 之后。
- **test-hygiene #6**(--lib 起真 tmux):测试基建大改,跨多测试文件→**另立**,排后(与 B 的 tmux 相关但改的是测试层,不塞 B 的生产修)。
- **C2 fix 已并入 B(B3)**;C2 investigation 已完成(证据存档)。
- **设计线**:感知+控制面设计轮(a3 辩论进行中)、C1 空壳 daemon 设计、host-parity #7 → 走**设计线**,不在机械模块内(设计轮收敛→spec→再排实施)。
- **db/ 重命名归位**(§三.5):长期,最后。

## 排期
1. **现在**:P0-2(模块 C)按现执笔权流程收尾(a1 实施在途)→ 我亲验 → a4 审 → operator 推合入。设计轮(a3)并行跑,不受影响。
2. **P0-2 合入后**:operator 加 a2 实施位 → 开 **模块 A(a1)+ 模块 B(a2)双 worktree 并行**(基于含 P0-2 的新 main 拉分支)。每模块:a4 写全模块 RED → a1/a2 实施全模块 → 收口各自跑一次 cargo → a4 批审 → PR。**A/B 文件不重叠,cargo 只在各自收口排队(全机单跑铁律)。**
3. **A/B 合入后**:模块 D(per-worker 凭据)、test-hygiene;设计轮收敛稿进 spec 后排设计线实施。

## 铁律（不变）
- **全机 cargo 单跑**:任何时刻只有一个 cargo 在本机跑,串行 `CARGO_BUILD_JOBS=1`;A/B 并行实施但收口 cargo 排队(不同时跑)。
- **执笔权**:a4 写测试闸门 + a1/a2 纯实施不碰测试文件 + 同实例不自审。
- **禁后台跑测试**;模块粒度红绿,证据按模块批出。
