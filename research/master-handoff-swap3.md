# Master 交接 — 换血#3(2026-07-10)

> 你是换血#3 后的新 master(空白会话)。新二进制 = main `97104cd`(含感知 C1/控制面 D1 三条 PR #136/#137/#138 + issue #13 respawn-storm 修复 #141)。这份文档是前任 master 在冻结点写的完整交接。前两轮交接文档 `research/master-handoff-swap1.md`/`master-handoff-swap2.md` 仍可读作背景,但**权威现状以本文为准**——distinguish：swap2 交接时三份 kiro spec(C/D/凭据)还在等放行，本轮 C1/D1 已经放行、走完全流程并 merge，凭据 spec 已改版但未排期，见下文 §五。

## 一、本轮已完成 + 已合入 main

**Phase 1（感知仲裁器 C1 + 控制面 D1）**：
| PR | 内容 | commit |
|---|---|---|
| #136 | perception-arbiter Phase 1（C1 写闸门 `db::perception::gate` + C2 `perception_events` 通道走既有 events 表 + CI grep 规则禁止 gate 外直写 `agents.state`） | `93c2bf3`（merge） |
| #137 | control-plane-refactor Phase 1（D1 `JobStatus` 状态机 + `transit_job_state` 单写闸门 + 两条 gap-patch：排队原因可观测性、cancel 驱动者+超时接管） | `fdc4566`（merge） |
| #138 | C1 checker 基线维护（D1 的 `db/job_state.rs` 测试夹具被 C1 冻结白名单误判违规，跨泳道 merge 碰撞，补一条 file-scoped 精确匹配基线条目） | `0f529ff`（merge） |

两条泳道都走完执笔权全流程（gatekeeper 写 RED → 实施变 GREEN → gatekeeper 审计 ACCEPT，含至少一次真实 REJECT 返工：C 泳道抓到 CI 脚本通配符 fail-open、D 泳道抓到 requeue 绕过闸门+CI 排除过宽两处真 bug，均已修复复审 ACCEPT）。

**Issue #13（ah 公开仓）respawn-storm 全闭环**：
- **现象**：追加 sibling agent block 后 `ah up` 触发全栈 respawn storm，压垮 tmux server。
- **research**（g1 亲证）：operator 原假设"hash 吃了 sibling"被 g1 用真实 SHA-256 输出**推翻**——`compute_config_hash` 是纯 per-agent 输入。真根因是 spawn 侧存的 hash 算【注入后有效 env】、realign 侧比对只算【裸 ah.toml env】，两侧永不相等 → **每个 agent 首次 `ah up` 必判 DRIFT**，与是否追加 sibling 无关。
- **design**：g1 出边界草案，o1 做一轮对抗质疑（5 条：`AH_AGENT_ID` 排除/`IS_SANDBOX` 排除/`config.env` 全员同 drift 是否可接受/client 侧 merge 信任边界/哈希写入权分裂），g1 终裁——3 条采纳（含修正 o1 对 realign 循环"并发"的误判，其实是串行）、1 条坚守己见反驳 o1（`IS_SANDBOX` 用 o1 自己的原则反驳 o1）。
- **RED**：g1 commit `c6ed45b`，`tests/issue13_respawn_storm.rs`，真实走 `agent.spawn`/`session.realign` RPC，非绕过路径 seed 假 hash。
- **GREEN**：commit A `af32fa8`（server 侧收拢 merge + 注入前抓裸 env 喂指纹 + 唯一哈希写者）+ commit C `217b59c`（500ms 错峰）。**由 g1 亲自实施**（原计划 g1-m1 做，但被 #13 自身 catch-22 挡死，见下§二），g2 跨泳道审计 ACCEPT（含额外 RED 诚实性核查，因 RED 作者=实施作者，独立性打折但 g2 兜底核实）。
- **CI 修复**：PR #141 CI 抓到 `tests/mvp9_acceptance.rs::test_launcher_passes_merged_env_to_agent_spawn` 断言旧线格式（真 gap 非回归，g1 GREEN 自验漏跑集成测试），g1 补 commit `88f306a`，全量 `cargo test` 串行验证 1481 passed/0 failed。
- **merge**：`97104cd`。public `SevenX77/ah` issue #13 已评论+关闭。
- **follow-up 已立账，未做**：#139（commit B，SIGKILL orphan-reap，需 systemd-scope e2e 才能诚实 RED，拆出去防止拖累干净的核心 PR）+ init-probe 200ms 轮询无退避另一放大器；#140（`grand_tour_realign_extra_matrix` 预先存在的 flaky e2e，与本次改动无关，BUSY-fixture 时序问题）。

## 二、当前拓扑真相（换血前冻结快照）

- **g1**（claude）：IDLE，已 `/clear`，ctx 0%。
- **g2**（claude）：IDLE，已 `/clear`，ctx 0%。
- **o1**（antigravity）：IDLE，**未清**（按 operator 指示跳过，避免打断潜在在途状态）。
- **g1-m1**（antigravity）：**KILLED**。原因：#13 本身的 catch-22——活栈没有干净的单 agent revive 路径（`ah up`/`ah start` 都会对**全部**已声明 agent 重算 hash、触发正在修的同一个 storm 类问题），revive g1-m1 就是在正在修的 bug 身上跑一次实测，operator 裁决弃救，改由 g1 亲自实施 GREEN。**换血后新二进制生效、hash 不对称已修**，可以安全 revive。
- **g2-m1**（antigravity）：**假 BUSY 状态挂了约 12 小时**（`ah ps` 显示 `BUSY`，实际早已空闲，job `job_ab76c3c4` 卡在 `DISPATCHED` 永远到不了终态——agy 实施位turn-end 后不归 IDLE 的已知协议缺陷）。**不用手动处理**，换血整栈重启会顺带清掉。
- **a1-a5**：旧代拓扑（换血#1 之前编号）残留，**KILLED**，与当前 g/o 拓扑无关，纯历史噪声，可忽略。

## 三、本轮踩坑与教训（给新 master 避雷）

1. **改公共契约（RPC payload/线格式）必须跑全量 `cargo test`，不能只跑触及模块**。g1 在 GREEN 阶段自验时只跑了 lib 单测 + 特定 e2e，漏了 `tests/` 目录下的集成验收套件，导致 PR #141 CI 红（`mvp9_acceptance.rs` 断言旧线格式）。这不是实现错了（新线格式是设计正确演进），是自验范围不够。**以后凡是跨进程/client-server 契约类改动，派单时明确要求全量串行测试收口，不要信"触及模块测试绿了就够了"**。

2. **单 agent revive 存在结构性 catch-22，暂无 CLI 解法**：`ah agent` 只有 `notify`（生命周期事件通知，不是 spawn）；`ah up`/`ah start` 都会对 `ah.toml` 里**全部**声明的 agent 重新走 `session.realign`（`ah start` 的"已存在 session"分支和 `ah up` 调的是同一个 RPC），没有任何 CLI 层面能只 revive 一个缺失 agent、不触碰其它已在跑的。issue #13 修复后这条路径本身不再假 DRIFT，但"只想救一个 agent 却要重算全员"这个语义耦合还在。**已记入 hardening backlog**：ah 需要一个"单 agent respawn" CLI/RPC，只补缺失 agent、不 reconcile/drift 其它——如果有余力可以现在排期，不算阻塞项。

3. **pend 哨兵对 agy（antigravity）实施位的"假完成/假 BUSY"免疫力为零**，这轮至少撞见 4 次：
   - agy 完成一轮工作后不归 IDLE，卡在 `BUSY`/`Matched` 状态，新 job 排队到该假 BUSY 后面永远派不下去（`ah pend` 会一直等到超时，而不是报告真相）。
   - g1-m1、g2-m1、o1 都各自撞过至少一次。
   - **监控铁律不变但要重申**：pend 哨兵只当**超时兜底**用，agy 实施位的真实进度锚 **fix worktree 的 `git log`/`git status`**（HEAD 变了=真干完了），不能锚 `ah ps` 或 job 状态。
   - **释放假 BUSY 的正确姿势（按严重程度递进）**：先 `tmux send-keys -t <pane> '/clear' Enter` 等新 banner；如果 `/clear` 之后 `ah ps` 仍显示 BUSY 或 pane 冒出了旧会话的幽灵残留提示词（这轮真实撞见过一次：`/clear` 后立刻弹出了换血#2 之前的 A4 时代旧 prompt 残留），直接 `ah kill <agent>` + `ah up`（`ah up` 只会重建刚 kill 的这个/这几个 agent，其余显示 `SKIPPED_BUSY` 的不受影响——但换血前这条本身受 #13 影响，见上条；换血后 `ah up` 应该完全安全了）。
   - 另有一次 claude 系 gatekeeper（非 agy）撞见"幽灵占位文本卡住派单"——pane 停在上一轮任务的旧建议性文本（比如"ACCEPT 确认，通知 operator 开 PR"）没被清空，新 prompt 排队但物理没打进 pane。同样用 `/clear` 解，Esc/C-u 对这类幽灵无效。

4. **对 killed agy agent 手动 revive 前，先确认没有更省事的正规路径能用**——这轮为了救 g1-m1 一度想手搓裸 JSON-RPC 直连 `ahd.sock` 调 `agent.spawn` 绕过 `session.realign`，被 operator 叫停（活体验证正在修的 bug，风险不对称）。**任何时候要绕开正规 CLI 直连内部 RPC/socket，先跟 operator 确认**，不要因为"技术上可行"就自行执行——这类操作的风险等级高于常规派单。

5. **RED 作者=实施作者时（本轮 g1 因 catch-22 被迫自己实施自己的 RED 契约），审计方必须额外核实 RED 测试本身的诚实性**（是否同义反复、是否被实施阶段悄悄削弱），不能只做常规 diff 审计。这轮 g2 承担了这层额外职责，也真的抓出并独立复核了 g1 主动披露的两处设计缝隙（recovery 路径的 env 存储语义、pre-existing flaky e2e）——这套"实施者主动披露疑点、审计方独立复核而非走过场"的模式值得延续。

## 四、换血后新二进制生效的行为变化

- **`ah up` 不再假 DRIFT 风暴**：config-fingerprint 现在两侧（spawn 存储 / realign 比对）都只吃 `merge(config.env, agent.env)` 裸配置，排除所有运行时注入的派生值（`CCB_SOCKET`/`AH_ROLE`/`AH_SESSION_ID`/`AH_AGENT_ID`/`HOME`/`CLAUDE_CONFIG_DIR`/`CODEX_HOME`/`IS_SANDBOX`）。未改配置的 agent 首次 `ah up` 也不会再被误判 drift。可以放心用 `ah up` 做真实的配置对齐（含 revive g1-m1/g2-m1）。
- **realign 重生错峰**：多个 agent 真需要同时 realign 时，respawn 之间有 500ms 最小间隔，不会再瞬时并发拉爆 tmux server（但 commit B 的孤儿 pane 回收还没做，见 #139，realign 判 drift 时旧进程仍不会被真实 SIGKILL，只是不再一次性爆量）。
- **job 状态机有了单写闸门**：`jobs.status` 只能经 `transit_job_state` CAS 变更，非法迁移在闸门层拒绝；`ah cancel` 对失联 agent 有超时强制接管（不再无限期挂等 agent 侧确认）；调度延迟现在有可观测信号（`dispatch_deferred` 事件，不再是纯 `tracing::info!` 静默）。
- **感知层写闸生效**：`agents.state` 直写在 gate 模块（`db::perception::gate`）外被 CI grep 规则禁止（Phase 1 只建闸门，还没做 Phase 2 生产者迁移，现有 ~40 处历史直写仍是基线豁免状态，尚未收缩）。

## 五、悬而未决方向（等 operator/用户拍板，你不要自己排期）

1. **v1.5.1 发版**：待用户点头，operator 独占 push/PR/发版权限，你不参与。
2. **模块 D（per-worker 凭据隔离）未排期**：spec 在 `.kiro/specs/ah-per-worker-credentials/`，design.md 已废弃指向 `design-rev.md`（o1 出的修订稿，Plan B "Fake Gateway" 已选定，Plan A MITM 已否决）。Phase 0 spike 已证实原生 OAuth-refresh 重定向不可行（CLI 二进制硬编码域名白名单）。Phase 1 尚未开工，等 operator/用户方向。
3. **感知设计轮（perception-arbiter 剩余 Phase 2-4）收敛稿待转 spec 排期**：Phase 1（C1/C2）已完成落地，Phase 2（生产者迁移）/Phase 3（tri-state+budgets+epoch）/Phase 4（hook 归属）仍是 spec 里的"outline only"状态，未进入实施排期。
4. **Windows 原生移植优先级最高（用户标记）**：spec 在 `.kiro/specs/ah-windows-native/`，含 design.md/m0-spec.md/m1-spec.md/research.md，具体排期待 operator/用户对齐。
5. **模块台账**：`research/MODULE-STATUS-LEDGER.md`，operator 维护，merge 后会更新——orientation 时读一遍，不要自己另起一份。
6. **#14/#15（ah 公开仓小 bug）**：operator 正在和用户对齐下一步是先扫这两个小 bug 还是直接推进感知层 Phase 2，你到岗时如果还没定，主动问一句，不要自己猜着排期。

## 六、下一步建议（给新 master）

1. `ah ps` 核对拓扑（g1/g1-m1/g2/g2-m1/o1 五个 agent），确认换血后的 PID/pane 都是新的（旧 PID 应该全部 KILLED）。
2. g1-m1/g2-m1 revive：新二进制下 `ah up` 应该安全了，但**revive 后先验一次 rust toolchain 在位**（`cargo --version`、`RUSTUP_HOME`/`CARGO_HOME` 能解析）再派 GREEN 类任务——这条坑换血#2 交接就提过，本轮 g1-m1 也确实在自己的沙箱里撞见过 rustup 无默认 toolchain（不是 revive 导致的，是沙箱本身没设默认，`cargo +stable` 显式指定即可绕过，别跑 `rustup default` 改宿主配置）。
3. 向 operator 确认 §五 的悬而未决方向优先级，不要自己排期开工。
4. 继续守泳道层级化边界（g1/g2 终裁各自泳道内事务——测试契约疑问、签名适配分歧走 `.lane-question`，你只原样转派或处理跨泳道/需升级的事）。
5. 监控习惯延续 §三.3 的教训：agy 实施位锚 git HEAD 不锚 job 状态；claude 闸门位 pend 可靠、但仍要警惕 pane 幽灵占位文本这一类物理层面卡单（跟 job 状态是否可信是两回事）。
