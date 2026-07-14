# Operator Handoff — 2026-07-11 checkpoint #2(换血#5 / Gen-5 之后;同日第二次交接,本版为准)

> 你是 ah 项目的 **operator**(用户唯一对话面)。用户刚 clear 了上一个 operator 的上下文,本文是完整交接。
> **用户的最高指令(2026-07-11 原话级)**:
> 1. 永远站在**整个项目的架构层面**审视所有模块、功能、bug——**第一性原理,不要打补丁思维**。同族 bug 出现两次=结构病,升维处理;架构域先写理想基准→差距表→收敛,不迁就现状。北极星:`research/perception-layer-first-principles.md`。
> 2. **上下文断了先恢复不重跑**:pane 文本→/resume 旧 session→落盘产物,按序捞;"必须重跑"是最后手段(用户当面纠正过"有毛病啊?不会/resume恢复上下文吗?")。
> 3. **存量任务不许丢**:全部开 kiro spec(需求先写进去)+ 记任务 log;不靠 operator 记忆持久化。
> 4. 用户只在需求/目标层;实施全链路自驱闭环;零干预≠零报告,异常分钟级 push 用户。

---

## 〇、身份与铁律(先读这些,再动手)

- 角色:operator **只动脑不动手**——执行全下派(master→泳道),动手前自问"为何不能派"。发版/PR/合并是 operator 独占(master/worker 无 gh 认证,**是设计不是故障**);**发版必须用户点头**(用户明确"先不发")。
- **PR 归属 SOP(本阶段用户拍板)**:泳道 push 分支+回报 title/body 要点,operator 开 PR + auto-merge;gate 配 gh 凭据等模块 D 落地后给细粒度 PAT,**严禁**全局 `[env]` 塞 token(会泄进 agy 沙箱)。
- **required checks SOP(本阶段用户拍板)**:没配 required checks 时 `gh pr merge --auto`=立即合(#142/#143 实锤);dev 仓 main 已配 `test` 必过;公开仓 SevenX77/ah 故意不配(挡同步 push);属场景包必要条件(spec `sop-required-checks-automerge/` 已开)。
- 记忆系统里有全部行为铁律(auto-memory 会加载),关键几条:亲验不轻信 agent 报告;派单后 in-loop 监控到真结果;投 pane 文本用 Write+load-buffer 绝不 printf;全机 cargo 串行且按模块批量;报告说人话三段(现状/根因/下一步)。
- 模块状态**只认台账** `research/MODULE-STATUS-LEDGER.md`(git merge PR# 为准);每次 merge/换血当场更新。
- 四本账:台账 + 疗效账本 `research/dogfood-ledger-2026-07-10.md` + 观察日志 `logs/operator-observation-log.md`(46 条,当日四要素)+ **疗效报告** `research/gen-efficacy-reports.md`(每代开窗列预期断言、关窗出判决**推送用户**;五级 verdict,未观测不许算治愈)。**Gen-4 已关窗判决;Gen-5 开窗表已建,关窗是你的责任。**

## 一、checkpoint 现状(2026-07-11,换血#5 后)

- **live 栈 = Gen-5**:binary=main **7bae3b1**(v1.5.0 + #136/#137/#138/#141/#142/#143);session **sess_169a3a04**;socket ahd-2ee4e0dfc3b5034c;unit ah-2ee4e0dfc3b5034c.service(Restart=on-failure)。**拓扑(用户指令改组)**:master %0(sonnet5,零裁决纯中继)+ **g1 %1 / g2 %3(codex 闸门)** + g1-m1 %2 / g2-m1 %4(agy 实施)+ o1 %5(agy 设计席)+ **r1 %6(claude opus4.8 xhigh,专职审核位,只审不写,直属 master)**。规则=.ah/rules/{master,g1,g2,r1,o1}.md 已按此改写。master oriented、全员 IDLE、零在途 job。备份 ~/.local/bin/{ah,ahd}.old-gen4。
- **本阶段合入**:PR #142(R1 outbox A′ journal-first)+ PR #143(agy Stop hook 绝对路径+timeout 单位修复)。#143 换血当日材料化实证(新沙箱 hooks.json=绝对路径+timeout 5);#142 冷扫首个真实触发实证(观察日志 #46,附孤儿事件缺口→已开 follow-up spec)。
- **模块完成度**(台账详):ABCD 中 A/B/C ✅;**D(per-worker 凭据)❌ 仅 spec,但 2026-07-11 第三例现场后用户点名、排期应提前(见域 4)**。感知 C1/D1 ✅ Phase 1。#13 修复 ✅。
- **发版债(用户明确暂不发,别催)**:main 领先 v1.5.0 **六个 PR**(#136/#137/#138/#141/#142/#143)。发版时:tag→cargo-dist→三道 leak-gate→同步 SevenX77/ah。
- **任务 log(本阶段建档,与 kiro spec 一一对应)**:①R1 outbox follow-ups(spec `ah-r1-outbox-followups/` 新开);②模块 D 凭据+第三例现场(spec 既有+incident 文件新增);③场景包 required-checks 落包(spec `sop-required-checks-automerge/` 新开);④Gen-5 疗效开窗断言观测;⑤发版债等点头;⑥ah#17 感知第三路径+log-monitor 300s(spec 既有 `ah-orchestration-reliability/`)。
- 交接链:master 侧 orientation 已注入(scratchpad gen5-master-orientation.md);上代清单 `research/outstanding-problems-2026-07-09.md`(大多仍开放,以本文为准)。

## 二、统一遗留清单(按架构域组织——这是审视方式,不是补丁排期表)

### 域 1:感知/完成协议(最深结构病,头号)
**理想基准**:agent 状态与任务完成只由显式协议信号(hook 起止双信号/日志事件)驱动,单写仲裁 FSM,unknown 永不造状态,高可靠信号缺席时响亮降级而非静默回退 pane 推断。收敛稿:`research/perception-final-convergence-2026-07-09.md`(设计轮必答四题:①单写入口硬约束形态;②各信号类 Unknown 预算;③cgroup 委托布局 PoC;④hook 归属竞态)。
- **[设计→实施,头号]** 显式完成协议 R2:停下≠完成已裁决,协议本体未设计未实施。C1/D1 只是地基。**hook 投递 outbox 一洞已被 #142 R1 补上(journal-first+冷扫);G4 hook 配置蒸发自检仍缺**。
- **[现行病,对照组]** agy 语义假完成/假 BUSY:Gen-2→Gen-4 持续;**Gen-4 新发现 claude(g2)同族标本**(后台跑测试提前收尾)——实锤"停下==完成"是结构病非 agy 特有。真信号=产物轨(git HEAD),pend 只兜底(pend 哨兵已被 agy 假完成击穿过)。
- **[现行病]** claude 幽灵文本族第 3 条路径:dispatch 就绪复查 pane-diff 恒拒发(= **ah#17**,任务 log ⑥)。同族三现,归仲裁 FSM,别打第四个补丁。**注意判别法:composer `❯` 后 dim(`^[[2m`)文本是 Claude Code 占位建议≠幽灵输入,capture -e 区分(观察日志 #45,曾浪费 10 分钟)。**
- **[新 issue,前任今晨补档]** ah#19(BUSY 死 turn 无 watchdog,job 挂 60-90min)/ ah#21(respawn 后 INIT_PROBE_TIMEOUT 误杀正在干活的 agent)/ ah#22(master 唤醒文本进 composer 永不回车)——全是感知域结构病的分面。
- **[backlog]** log 监听 300s 硬超时 < 真任务时长 + health_check 时间戳优先级(任务 log ⑥)。
- **[上游+兜底]** agy 空闲自退(REVIVE_IDLE 已合,复验顺延)。**agy Stop hook 真送达=Gen-5 开窗断言 #1,首个 agy 派单必验 journalctl agent.notify。**

### 域 2:控制面/job 状态机(D1 已落 Phase 1)
spec:`.kiro/specs/ah-orchestration-reliability/`(Phase 1-7 框架)。
- **[新 spec,任务 log ①]** `ah-r1-outbox-followups/`:R1 孤儿事件 max-attempts→error-book(死 session FK 永败永重试,观察日志 #46);R2 reap-on-RPC-success 接线;R3 两条代码观察(is_ah_owned_hook_item 旧串匹配器、ah.rs:608 debug log)。
- **[backlog]** dispatch-ACK 竞态(spec 内 dispatch-ack-race.md)/ STUCK 死胡同 part2 / cancel 对僵尸在途单无效 / realign 非原子(= **ah#16**)/ QUEUED 饥饿只告警。
- **[新 issue]** ah#20(ahd 无持久错误日志,ahd.log 恒 0 字节)/ ah#23(ahd.sqlite 27h 涨到 2GB,无 vacuum)——可观测性/存储卫生,适合泳道独立单。

### 域 3:生命周期/teardown/泄漏
- **[语义确认,本阶段实证]** **daemon 重启=全栈连坐重生**(SIGTERM→清 tmux→master 死→级联清 worker,设计语义;换血#4 的 NO_CHANGE 是 `ah up` 无 drift 路径,不可比。观察日志 #44)。换血 SOP 已按此修正。
- **[dev#139]** SIGKILL orphan-reap + init-probe 节流;**[dev#140]** flaky grand_tour(macos/windows checks 因此不入 required)。
- **[设计轮]** C1 空壳 daemon 累积 + C2 teardown 逃逸残余向量。
- **[backlog]** 沙箱 GC(445 个 48G 旧案)/ orphan-scope 回收 unwired / 测试 tmux 泄漏+flock fd 死锁。
- ~~Gen-4 g2 多余 bash pane %3~~ 随换血#5 全灭消失,退役观察项。

### 域 4:凭据/沙箱隔离(**本阶段升级:第三例现场,用户点名**)
- **[模块 D,spec 就绪,排期应提前]** per-worker 独立凭据(`.kiro/specs/ah-per-worker-credentials/`,Plan B fake gateway,phase0 spike 完)= **ah#18**。**2026-07-11 第三例(用户 Win11/WSL2 真机)**:refresh 竞态残根 `expiresAt: 0` 写穿 `/root/.claude/.credentials.json → /mnt/c/Users/test/.claude/.credentials.json`,ah 栈 claude 席(master/d1/g1/g2)全登出,**首次殃及用户宿主机 Windows 的 claude**——爆炸半径越栈,产品缺陷级。证据链+spec 影响:spec 内 `incident-2026-07-11-wsl2-symlink-logout.md`(design 需补 WSL2 验收:任何路径不得写穿宿主 /mnt/c credentials)。任务 log ②。
- **[backlog]** 沙箱种子化缺 rust toolchain;onboarding 种子化欠账。
- **[ah#6]** per-agent CLI settings 只支持 claude。

### 域 5:产品交付(公开仓 SevenX77/ah + 场景包)
- **[用户最高优先级,零进展]** Windows 原生(`.kiro/specs/ah-windows-native/`,tmux→ConPTY);域 4 第三例正是 WSL2 形态踩雷,佐证该方向的现实紧迫性。
- **[host-parity 六连]** ah#7-#12;**[CLI 一致性]** ah#14/#15(泳道热身单);**[docs/doctor]** ah#3/#4/#5。
- **[用户侧]** Studio Req1 v1.3.0-rc 等 Win11/WSL2 真机签 PASS(**注意:第三例登出事故就发生在该真机栈上,是现役阻塞项**)。
- **[场景包,任务 log ③]** required-checks prerequisite 落包(spec `sop-required-checks-automerge/`);pack 仓 open issues #2-#6(哨兵参考实现/分诊树/双泳道 ROLES/指令回执协议/方法论对照研究)。
- **[排最后]** 同 checkout 多 master(用户 worktree 方案自解,撞墙再提)。

### 域 6:验证债(§G 纪律:代码闭环≠实证闭环)
**Gen-4 已关窗**(判决在疗效报告:#141 治愈-实证;C1/D1/unit 自愈/catch-22 均未观测顺延)。**Gen-5 开窗断言(关窗前必验或如实未观测)**:
1. agy Stop hook 真送达(首个 agy 派单验 journalctl `agent.notify` antigravity 组)。
2. #141 真 drift 断言(需活栈**不重启 daemon** 只 `ah up` 的真配置改动;换血#5 走了全重启路径没验到)。
3. codex 闸门×2 + r1 审核位首个 PR 周期运转质量。
4. 感知 C1/D1 活栈行为(Gen-4 顺延)。
5. REVIVE_IDLE 完整链路 / Fix C 真 CLI 三断言 / A/B 看门狗真触发(历史顺延项,逢场景即验)。

## 三、下一阶段怎么开(**方向已定,别自选**)

用户原话:"先把这些做完,然后我会给你下一个任务,**用下一个任务再做 ab 测试**"——前置已全部完成,**下一步=等用户带任务来,拿它在 Gen-5 新拓扑(codex 闸门×2+r1 审核位)上跑 A/B**。既有 A/B 协议参考:`research/ab-experiment-r1-outbox-protocol-2026-07-11.md`(两臂串行、8h 顶、禁中间干预、零干预≠零报告)。
- 开场动作:核活栈(capture master pane,应 standby)→ 向用户报到并**问下一个任务**;若用户先提别的,任务 log ①-⑥ 是可派存量(模块 D 因第三例最有紧迫性,可主动推荐)。
- 发版已押后,别催;Studio Req1 等用户真机 runbook,别催。
- A/B 期间纪律:实验协议禁干预,但监控照常、异常分钟级 push(用户点名"别浪费我的时间,之前的问题根本不用等 6 个小时再发现")。

## 四、权威文档索引

| 文档 | 用途 |
|---|---|
| `research/MODULE-STATUS-LEDGER.md` | 模块→PR→状态台账(唯一事实源,已更至 #143/换血#5) |
| `research/gen-efficacy-reports.md` | 每代疗效判决(Gen-4 已关窗;**Gen-5 开窗中**) |
| `research/dogfood-ledger-2026-07-10.md` | 病种×代次疗效账本 |
| `logs/operator-observation-log.md` | 46 条事故四要素(随公开仓发布) |
| `research/outstanding-problems-2026-07-09.md` | 上代全量遗留(以本文为准) |
| `research/perception-layer-first-principles.md` + `perception-final-convergence-2026-07-09.md` | 感知北极星+收敛终稿(必答四题) |
| `research/ab-experiment-r1-outbox-protocol-2026-07-11.md` | A/B 实验协议模板 |
| `.kiro/specs/ah-r1-outbox-followups/` | **新开**:outbox 收尾三件 |
| `.kiro/specs/ah-per-worker-credentials/`(含 `incident-2026-07-11-wsl2-symlink-logout.md`) | 模块 D spec+第三例现场 |
| `.kiro/specs/sop-required-checks-automerge/` | **新开**:required-checks SOP 落包 |
| `.kiro/specs/ah-orchestration-reliability/` | 控制面 spec(dispatch-ACK/realign/log-monitor) |
| `.kiro/specs/ah-windows-native/` | Windows 原生 research+M0/M1 |
| `/tmp/claude-1001/-home-sevenx-coding-ccbd-rust/c818eb88-1835-42f4-8b02-9c32755ef341/scratchpad/gen5-master-orientation.md` | 现任 master 的 orientation 底稿(scratchpad,易失) |
