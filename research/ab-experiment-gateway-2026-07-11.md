# A/B 实验协议:Gateway 根治(模块 D / ah#18)— codex+agy 泳道 vs codex 单体

状态:草案,待用户拍板后开跑。2026-07-11 立。承 `research/ab-experiment-r1-outbox-protocol-2026-07-11.md` 的协议范式(两臂对比、监控不停、异常分钟级 push),但**本实验默认两臂并行**(隔离已解决,见 §4)。

## 1. 被测任务(两臂完全相同)

实现 **Plan B Fake Gateway**——per-worker credentials 的根治方案,冻结设计见 `.kiro/specs/ah-per-worker-credentials/design-rev.md`,根因实证见 `requirements.md`「根因(二进制实证)」节 + `research/credentials-phase0-spike.md`。

范围 = design-rev 的 Phase 1–3:
- Phase 1 宿主 HTTP Gateway 核心:持有唯一真凭据 + `/v1/messages` 转发 + header 重写 + **单飞刷新锁**(`RwLock`+`Mutex`+`watch`)。
- Phase 2 worker 侧接入:沙箱 bootstrap 去掉 `.credentials.json`、注入 `CLAUDE_CODE_USE_GATEWAY=1` / `ANTHROPIC_BASE_URL` / 假 JWT、UDS-per-worker。
- Phase 3 失败可观测:seed 令牌被上游吊销时的可辨识错误 + 用户级重登通知。

两臂拿到**同一份 brief、同一冻结设计、同一验收契约**。不同的只有"谁来实现"。

## 2. 两臂定义

### Arm A — codex+agy 泳道(多 agent 交接)
- 席位:`g1`(codex,闸门/RED 验收测试执笔/审计)+ `g1-m1`(antigravity,实施)。
- 流水:g1 从冻结设计写 RED 验收测试并 commit → g1-m1 plan-first 报计划、g1 批准 → g1-m1 纯实施变绿(不得改测试)→ g1 逐单审计 → 收口。
- 工作区:worktree `/home/sevenx/coding/ccbd-rust-wt-gw-a`,分支 `ab/gateway-lane-codex-agy`。

### Arm B — codex 单体(单 agent 全持上下文)
- 席位:`g2`(codex,**独任**:自己从冻结设计写 RED 测试 + 自己实现变绿 + 自验),**无 agy 搭档**。
- 工作区:worktree `/home/sevenx/coding/ccbd-rust-wt-gw-b`,分支 `ab/gateway-solo-codex`。
- 注:`g2-m1`(antigravity)本实验**停用/闲置**,不属于 Arm B。

### 统一终审
- `r1`(claude,只审不写)对**两臂各自**用**同一把尺**审(scope/spec 稀释/测试锚定/回滚自检),各出 ACCEPT/REJECT;两臂都收口后再出一份**头对头对比裁决**(哪一臂的实现更好 + 逐条理由)。这就是本次"最后都让 Claude 审"的 SOP(细则见 `.ah/rules/r1.md`)。

## 3. 度量(代码质量 = 头条终裁;可靠性/成本 = 并列必记轴)

**终裁问题(用户 2026-07-11 定案):同一冻结设计下,codex+agy 多 agent 交接泳道 vs codex 单体,谁写出的代码质量更高。** r1 按下述维度头对头判(每维给证据):
- **正确/能工作**:编译过、CI 测试绿(CI 是权威,见 §4);实现真满足冻结设计的契约。
- **scope 贴合**:diff 恰好覆盖设计、无越界改动、无 spec 稀释(悄悄放宽 fail-closed/加例外)。
- **测试质量**:验收测试锚定契约边界可观测行为(非实现内部状态);回滚自检真变红(空转测试=质量差)。
- **设计落地质量**:单飞刷新锁/UDS 隔离/header 重写等关键点实现是否稳健、边界处理是否完整。
- **可读性/地道**:与周边代码风格一致、命名、错误处理、无死代码。

**并列必记轴(不参与"谁代码好"的终裁,但必须全程记录)**——上一次 A/B(R1 outbox)的裁决实际落在这根轴上:泳道臂 DNF,4h10m 里有效工作仅 ~42min(agy 挂死 2h41m + livelock/假 BUSY ~50m),代码质量根本没轮到比。质量对比隐含前提是"两臂都跑完",所以:
- **完成度/可靠性**:各臂跑完没有;挂死/livelock/假 BUSY/假完成每例记时间戳+损耗时长;有效工作 vs 损耗时间比。
- **成本**:交接往返次数、REJECT 返工轮数、token/上下文消耗(Arm B 单体是否撞上下文上限)。
- **失败模式记账**:agy 静默选错不报告、codex 过早判完成、单体上下文耗尽——每例落 `logs/operator-observation-log.md` 标注归属臂。
- 本次 Arm A 同时是对"agy 系列修复(hooks 送达/timeout 单位/完成检测 v2)让泳道能跑完了吗"的疗效验证,结果记 `research/gen-efficacy-reports.md`。

## 4. 隔离与验证(两臂真并行)

- **git 隔离**:两臂各自独立 worktree + 独立分支,git object store 共享但 index/checkout 独立 → 可同时 commit 不撞。破解了"共享 git 树两 worker 不能同时 commit"的老约束。
- **cargo 政策(用户 2026-07-11 定案:不让 agent 做重 cargo)**:两臂**本地只允许 `cargo check`**(纯类型检查,不跑代码/不 codegen/不链接,内存小一个量级 → 无 OOM,**无需 cargo 串行锁**);`cargo test` 与全量构建**一律走 CI**。cargo 不改源码(只写 `target/`;`Cargo.lock` 仅随 `Cargo.toml` 依赖变化而动),两臂独立 `target/` 互不污染。
- **CI = 验证权威;push 下放 master(用户 2026-07-11 批)**:worker 只 commit 不 push;每轮红绿/集成由 **master push 该臂分支触发 CI**(护栏:只准 push `ab/gateway-*` 两分支、ff-only、永不 force/永不碰 main/不 push 别的分支),CI 结果回灌给臂。CI 绿 = 该臂"能工作"的判据(喂给 r1 的质量维度)。operator 只保留有爆炸半径的动作(main/PR 开合/force/发版同步)。共享 admin key 的最小权限清理记模块 D 兄弟债。
- **磁盘**:两份 `target/`(各数 GB)+ 沙箱;实验期留意 `project_ah_sandbox_leak_disk_full`。

## 5. 纪律(实验期)

- **不中途干预方法**:两臂各按自己的流水跑,operator/master 不在中途纠正"该怎么实现";只在**挂死/异常/越权**时介入。
- **监控照常**:master 对全 agent(尤其 agy)的监控不降频;发现停摆/挂死/异常 → ≤15min 落 `.operator-question` + push 用户,不攒不静等。
- **软上限**:单臂 8h 未收口即 checkpoint 上报现状,不硬砍。
- **收口**:两臂各自到 r1 ACCEPT(或落盘"失败+原因")→ r1 出对比裁决 → operator 对胜出臂开 PR。败方分支保留作对照数据,不删。

## 6. 当前就绪状态(2026-07-11)

- [x] 两臂 worktree + 分支已建(`ccbd-rust-wt-gw-a` / `-gw-b`)。
- [x] 席位规则改就位(g2→单体、g2-m1→停用、r1→对比 SOP+附录轴、g1→收单 SOP 焊死、master→A/B 路由+push 下放护栏)。
- [x] 根因二进制实证已补进 spec(requirements.md「根因(二进制实证)」)。
- [x] 冻结 brief 已落盘:`research/ab-experiment-gateway/task-brief-frozen.md`(纯任务、零角色、两臂字节级同一份;工作区由席位规则钉,brief 不载)。
- [ ] **等用户过目 brief 后**:/clear 相关席位 → 同一 brief 分发 g1(Arm A)与 g2(Arm B)→ 挂哨兵 → 开跑。
