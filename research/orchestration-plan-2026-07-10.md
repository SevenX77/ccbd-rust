# ah 任务编排计划书(operator 主笔,2026-07-10)

依据:用户 2026-07-09/10 三条拍板——①任务按大模块编排、模块内全改完统一跑一次 cargo;②换血后加一个 claude;③现在空闲的 agent 把后续模块的文档/TDD 前置做起来。整合架构收敛终稿(§三行动清单)、感知收敛终稿(设计轮必答四题)与在途流水。

---

## 一、现状快照(2026-07-10 拂晓)

| 项 | 状态 |
|---|---|
| P0-1(删毒推断器) | ✅ 已合 main(PR #127),CI 绿 |
| P0-2(熔断三层洞+认领 cancel) | ⚠️ RED 测试已 commit(6d4e4c9);**a1 实施停摆**(跑完基线归 IDLE,零改动零 commit)——立即处置项 |
| 设计轮(感知+控制面) | 🔄 已启动,a3 辩论单在跑 |
| live 栈 | 旧二进制(税:a4 每单被闩 4/4、agy 假完成 3 例/晚) |
| 拓扑 | a1(agy 实施)+a3(agy 设计辩论)+a4(claude 审计/测试执笔);codex 配额 ~7/11 回 |

## 二、剩余工作全量清单 → 大模块分组

**每模块 = 一个 worktree + 一个分支 + 一个 PR + 收口一次串行 cargo test --lib。**

### 模块 A · 完成判定/状态机域(机械修,设计不 gate)
文件面:`db/state_machine.rs`、`provider/health_check.rs`、`completion/monitor.rs`

**⚠️ 2026-07-10 修订(a4 前置单 STOP 实证,原四项为过时情报)**:
1. ~~P0-3 白名单对称化~~ → **重铸**:PANE_DIFF_STUCK 生产源在 #127 后仍活(mark_agent_stuck + timer.rs:113/health_check.rs:133 调用),并入"pane 生命周期推断整体删除"裁决线,删残余写入路径(待 master 钉最终形态)
2. 孤儿 `mark_agent_idle_recaptured*` 三件套:确系死码,验收=grep 零生产 caller + 删后 cargo check --all-targets 过(含唯一集成测引用清理),不写行为测试
3. ~~MAX_LOG_MONITOR_WAIT 300s~~ **已修**(a7c9d34,300s→900s,含守卫测试);若要参数化是新契约,master 定
4. ~~health_check 时间戳优先级~~ **已修**(a7c9d34,.or→.max 三源取最大);已有 marker 缺席面测试覆盖

### 模块 B · 进程环境域(机械修,设计不 gate)
文件面:`platform/linux/identity.rs`、spawn 命令构造层、`agent_io/registry.rs`
1. 身份注入:显式 env 注入替代 cgroup 嗅探(identity.rs:3 读 /proc/self/cgroup)
2. tmux 清理泄漏兜底(registry.rs:145-160,kill 失败无 fallback)
3. C2 向量收口(e2e teardown 逃逸分支——master 在查的读单结论并入,若纯机械)

### 模块 C/D · 结构改造(被设计轮 gate,codex 回归后转 spec)
- C:感知仲裁器(单写入口硬约束、perception_events 表、三态+Unknown 预算、Stalled 异常真、epoch 版本化)+ hook 上报归属竞态机制 + 父/子 cgroup 委托布局(先 PoC)
- D:控制面重分层(spawn_realign_agent 出 rpc 层、kill 四处统一、job 状态机替 11 处裸写、F3/F2 事务寄生解耦、db/ 重定位、master_watch 拆分)
- 独立线:per-worker 凭据(设计先行,换血后排)

### 运维/实证线(不占 cargo)
- P1 换血(见 §五)
- §G 实证债 dogfood 矩阵(8 条,换血后在活栈跑)
- 测试卫生:--lib 不该拉真 tmux daemon(排模块 A 或 B 收口后)
- C1 空壳 daemon 累积设计(post-G2,master todo 已有)
- (域外 backlog,不进本计划:Windows 原生线、antigravity 实施管线复测)

## 三、拓扑演进(三阶段)

**现在(换血前)**:a1(agy 实施)/a3(agy 设计辩论)/a4(claude 审计+测试执笔)。空闲产能全部投前置(见 §四)。

**换血时(P0-2 合入边界,一次重启完成两件事)**:
- 新二进制 + 扩拓扑:**+a2(agy 实施位2) +a5(claude,用户拍板新增)**
- 五 agent 分工:a1/a2 = 双实施泳道;a3 = 设计辩论(不变);a4/a5 = 双 claude,按泳道分管测试执笔+审计:a4 管泳道1(给 a1 写 RED、审 a1 实施),a5 管泳道2(给 a2 写 RED、审 a2 实施);e2e 归 a5(a4 P0 期专注审计)。互审可交叉,唯一铁律不变:不审自己产出。
- a5 场景层规则文件:复用 a4.md 内容 + 泳道标注(换血前由 master 备好 .ah/rules/a5.md)

**codex 回归后(~7/11)**:按用户既定"发版后节点 realign"再议——codex 回实施/严审位,agy 收缩,设计轮冻结稿转 spec 由 codex 执笔。

## 四、流水线规则(全程不变量 + 新增)

不变量:执笔权(agy 不执笔闸门;验收测试=claude 写,实施者不碰测试文件)/全机 cargo 单跑/不自审/worker commit 不 push(operator 推,auto-merge)/阻塞落 .operator-question。

新增(用户 2026-07-10 拍板):
1. **模块批量 cargo**:模块内全部任务实施完,收口统一一次 `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`;中途只许定向 `cargo check` 受影响文件;红绿证据按模块批出。
2. **文档/TDD 前置**:agent 空闲即投后续模块的前置产出——工单文档、契约 RED 测试(只 cargo check 定向核编译,不跑全量)。前置产物落各模块 worktree,实施开始前 rebase main。
3. 双泳道并行时,两模块收口 cargo 排队跑(先到先跑,后到等),绝不并行。
4. **每模块换血 + dogfood 观察(用户 2026-07-10 追加拍板)**:每个大模块 PR 合入后都执行一次换血(新二进制按 §六 runbook 重启栈),后续任务即成为该模块修复的 dogfood;operator 维护疗效账本(research/dogfood-ledger-2026-07-10.md),按二进制代次记录病种发生率对照。模块边界换血由该拍板预授权,执行时知会用户,不再逐次请示。

## 五、排期(五个节点,并行泳道图)

```
节点0 立即(今晚)
  ├─ 代码线: 救活 P0-2 实施(a1 停摆→master 续派)→GREEN→a4 审→merge
  ├─ 前置线: a4 空闲窗口写模块A的 RED 契约测试(wt-modA off main,cargo check 定向)
  │          master 备模块B工单文档 + a5 规则文件草稿
  └─ 设计线: a3 辩论(在途)→master 收敛→冻结稿给 operator/用户过目
节点1 P0-2 合入 = 换血#1(操作见下,预计 30-60 分钟,预授权+执行时知会)
  └─ 新二进制 + a1-a5 五agent拓扑 + orientation + 双验拓扑 + §G 抽验(G1/G2 完成检测类先验)
  ★ 换血节拍(用户追加拍板):此后每个模块合入都换血一次(换血#2=模块A后、#3=模块B后、……),
    下一模块流水即上一模块修复的 dogfood,operator 记疗效账本对照病种发生率
节点2 双泳道机械修(换血后)
  ├─ 泳道1: a1 实施模块A(a4 已前置的 RED)→收口 cargo→a4 审→PR→merge
  ├─ 泳道2: a2 实施模块B(a5 补齐 RED——换血后 a5 首单)→收口 cargo→a5 审→PR→merge
  ├─ 实证线: §G dogfood 矩阵在活栈逐条销(不占 cargo)
  └─ 设计线(用户 2026-07-10 改派):**master+a3 抽空即做,不等 codex**——a3 辩论发散/master 收敛执笔,
     模块C/D 写成 kiro spec(.kiro/specs/,必答四题不许悬空);operator 亲验+用户过目后才进实施
节点3 codex 回归(~7/11): 设计冻结稿→spec+TDD 框线(codex 执笔)→模块C/D 排产;拓扑 realign 再议
节点4 模块C/D 结构改造: 按模块批量规则实施;C 里父/子 cgroup 布局先独立 PoC 小单验证可行性再进主实施
```

## 六、换血节点 runbook 要点(operator 亲自执行)

1. 前置:P0-2 合入后 main 构建 release 二进制;master 备好 ah.toml(五 agent)+a5 规则文件+各 agent orientation 文本
2. 顺序:通告 master 收敛在途→`ah stop` 优雅停栈→换二进制→前台 `ah start`(绝不后台)→逐 agent 等 IDLE
3. 双验:`ah ps` 拓扑对表 + tmux list-panes 对表(realign 丢 agent 前科);master orientation 注入(tmux load-buffer 法)
4. 验效:抽验 G1/G2(派 a4 一单看是否再被闩;派 a1 一单看假 COMPLETED 是否消失)——这两条是换血直接疗效,当场可证
5. 回滚线:旧二进制保留原位,异常即换回重启

## 七、风险与对冲

| 风险 | 对冲 |
|---|---|
| 前置 RED 测试与 P0-2 合入后 main 漂移 | 模块 worktree 实施前 rebase;前置只锚契约边界不锚实现细节 |
| 双泳道抢 cargo | 收口排队规则写死进两边 brief;flock 文件互斥兜底 |
| agy 假完成/停摆(节点0-1 期间仍有税) | 产物轨监控+master 按 SOP 续派;换血后消 |
| 换血重启事故(丢agent/级联) | runbook 双验+回滚线;流水收敛后才动手 |
| a5 新会话无前情 | 规则文件+自包含 brief;首单派轻量(模块B RED) |
| 设计冻结稿被 spec 稀释 | 既有管线纪律:冻结稿权威,a3 忠实度对抗审 |

## 八、立即执行项(本文发出即生效)

1. master:救活 P0-2(a1 IDLE 零产出,续派自包含实施单,明确"基线已跑过,直接实施四文件改动到 GREEN+commit")
2. master:给 a4 派模块A前置单(wt-modA,RED 契约测试,cargo check 定向,不跑全量;文件面=§二模块A 四项)
3. master:备模块B工单文档 + .ah/rules/a5.md 草稿 + 换血 ah.toml 草稿
4. operator:P0-2 合入后向用户单独知会换血窗口再执行
