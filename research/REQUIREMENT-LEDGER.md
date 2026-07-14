# ah 需求追溯账本 · REQUIREMENT-LEDGER

**全项目唯一需求进度总账。operator 与 master 都要读、都要更新。operator 负责优先级编排。**
最后全量盘点:2026-07-12(4 路只读盘点 agent 扫全部 ~41 个 kiro spec)。

## 怎么用
- **每条需求一个条目**:需求原话 / kiro spec / 已完成 / PR 号 / 各阶段测试(单测·CI·tier-3)/ 状态 / 优先级 / owner。
- **不许靠记忆答"哪些做完了"** —— 一切以本账本 + git merged PR 号为准。半途任务必须留痕(状态=HALF)。
- **状态**:DONE(合入+验收) / ACTIVE(在跑) / HALF(做一半留一半) / BLOCKED(卡依赖/决策) / PENDING(登记未开工) / SUPERSEDED(废弃/被替代)。
- **测试三格**:单测(unit)/ CI(并行绿才算)/ tier-3(真二进制 spawn 真 worker 端到端)。**铁律:tier-3 是硬门,绝不只 CI 绿放行。**
- **文档滞后勾稽**:多个 spec 的 tasks.md checkbox 未回勾但 PR 已 merged——**判定以 git merge 为准,不以 checkbox 为准**。
- 每次 merge / 换血 / 决策当场更新,不攒。master 见 `.ah/rules/master.md` 已挂"必读必更新"铁律。

---

## 一、优先级速览(operator 编排)

| 优先级 | 需求/spec | 状态 | 关键 PR | 单测 | CI | tier-3 |
|---|---|---|---|---|---|---|
| **▶ 当前焦点** | 模块化解耦 + 架构地图索引 ah-modular-decoupling(用户 2026-07-12:"先把模块化做完,再开双轨") | ACTIVE(单焦点,开足codex) | 无 | ❌ | ❌ | ❌ |
| **P0** | 凭据机制第一性重做 per-worker-credentials(direct-dir 方案,2026-07-13 收口) | CI绿+合入,tier-3待operator真机 | #151(MERGED) | ✅ | ✅ | ⏳待operator真机 |
| **P1·冻结** | 编排底座第一性重构(双轨的轨B,o1+d1设计→c2实施) | PAUSED(设计draft未冻,待恢复) | 无 | ❌ | ❌ | ❌ |
| **P1** | Windows 原生(tmux→ConPTY,用户"我要用") | HALF | #90/#91/#132 | 编译门✅ | ✅编译 | ❌ M1/M2未启 |
| **P1** | 编排可靠性 orchestration-reliability(含6病例+R-DYN-1+agy检测器) | ACTIVE(纯backlog) | 仅#141 | ❌ | ❌ | ❌ |
| **P2** | 完成协议 completion-protocol(显式完成取代推断) | ACTIVE | #142(R1臂) | 部分 | 部分 | ❌未跑完 |
| **P2** | v1 对外发布 v1-public-release | ACTIVE | #144等 | — | — | 未逐条追溯 |
| **P2** | r1-outbox-followups(孤儿事件终态+reap接线) | ACTIVE零实施 | 前置#142 | ❌ | ❌ | ❌ |
| **P3** | 感知仲裁 perception-arbiter(4阶段只做Phase1) | HALF | #136 | ✅P1 | ✅P1 | ❌ |
| **P3** | control-plane-refactor(只Phase1) | HALF | #136/#137/#138 | ✅P1 | ✅P1 | ❌ |
| **P3** | unified-capture(Phase2落,Phase4治理延后) | HALF | (main commits) | 部分 | — | ❌ |
| **P3** | prompt-handler(Phase1交付,PR4b/PR6b延后) | HALF | (pr4a报告) | ✅Mock | ✅ | ❌真机 |
| **P3** | studio-req1 provisioning(代码闭环,真机门未签) | HALF | #94-97/#105 | ✅ | ✅ | ⏳等用户签 |
| **P3** | macos-port(边界+进程监视,服务层未做) | HALF | #70/#72/#74 | ✅ | ✅ | ❌ |
| **P3** | product-delivery(伞形,多子设计未闭) | HALF | 跨#110-141 | 部分 | 部分 | 部分 |
| **P3** | job-events(表落地,投影层新设计) | HALF | #114/#142 | 部分 | — | ❌ |
| **P4** | self-knowledge-skills(核心done,T5/T6尾巴) | ACTIVE | #108/#109 | ✅ | ✅ | — |
| **P4** | sop-required-checks-automerge(dev配了,pack文档未发) | HALF | 无(配置) | — | — | — |
| **P4** | real-dogfooding-acceptance(主bug闭,残留) | ACTIVE | orphan系列 | ✅ | ✅ | ✅部分 |

**优先级理由**:P0=用户当前唯一在推的关键路径(凭据急件),发版被它 hold。**首任务**=用户 2026-07-12 明示"换血#6 后第一个任务应该是模块化梳理"——即 06:14 双轨里"轨A 存量逐个落实"的**第一项**具体化为「模块化解耦 + 架构地图索引」(spec `.kiro/specs/ah-modular-decoupling/`,requirements.md 已建,design 待 o1+d1);多开 codex 并行、Claude 省用、换任务清 ctx。与在飞的 P0 凭据收口的先后是 open operator 决策。P1=①Windows 用户明示最高优先"全力打通我要用"、真核心 M1/M2 还在起点;②orchestration-reliability 是可靠性风险最集中处(6 个 root-cause 确认的病例全部已知未修 + 整套 reconciler 零实现 + 用户新提的动态拓扑)。P2=方向性推进中、非阻塞。P3/P4=分阶段落地、多数卡"实证门/真机门"或显式延后。

---

## 二、P0 — 凭据机制第一性重做 · ah-per-worker-credentials 【CI绿+合入,tier-3待真机】

- **需求原话(北极星,用户 2026-07-12)**:"用户只想登录一次——用他平时自己用的那个 CLI 登录一次就够了。之后 ah 栈里 N 个 claude worker 全部骑在这一次登录上干活,既不逼用户为每个 worker 重复登录,也不能因为多 worker 并发把用户(或彼此)登出。"
- **不变量**:P1="没有任何 worker 能独立发起一次会轮换服务器端令牌血缘的刷新";P2=凭据失败可观测、可独立恢复。
- **2026-07-12 晚设计第三次重开(取代下方 Layer1/2/3 Gateway 旧记录)**:operator 逆向 claude 2.1.207 二进制 + 用户真机 drvfs spike,证伪"共享一份文件无效"的旧论断(claude 自带 mtime 重读+竞态保护+跨进程锁+原子写),排除 apiKeyHelper,用户裁定 SSOT=Windows 真实凭据文件本身、拒绝独立副本方案。**Gateway 三层(#146/#147/#149 merged 但从未端到端激活过任何 claude)整体撤,不再是实施依据。**
- **新方案(direct-dir + CLAUDE_SECURESTORAGE_CONFIG_DIR)**:每 claude 席位注入 `CLAUDE_SECURESTORAGE_CONFIG_DIR=<共享真目录>`(仅凭据存储位置),`CLAUDE_CONFIG_DIR=<每沙箱>/.claude` 不变(settings/规则/session/trust/MCP 各自隔离,已逆向证实正交)。共享目录必须 direct-dir(目录本身指向真实路径,非文件级 symlink——drvfs spike 证实 rename 会替换 symlink 写不穿真文件)。配置化:`ah.toml [providers.claude].shared_credentials_dir`,fail-closed 校验(空串/相对路径/不存在/非目录/symlink 全拒)。
- **交接链路(2026-07-13)**:c1 出 design.md 收敛稿 → d1 轻审补一处漏项(两条主 spawn 路径 Gateway 拆除)→ c2 实施(worktree `feat/claude-shared-credentials-dir`)→ r1 六轮审查(REJECT×4,每轮抓到不同类漏网:R1 两个集成测试拿 None 打 fail-closed claude 路径;R3 生产 `run_master_cutover` 漏发字段导致真实 master cutover 对 claude 必炸;R5/R6 `prepare_home_layout("claude",...)` 系列顶层 wrapper 全仓穷举清零)→ R6 ACCEPT,CI 全绿 → **PR #151 已 squash-merge(`3a7bb5a`)**。
- **机制修正(obs 2026-07-13)**:此 PR 暴露"本地只 cargo check 看不到测试调用点断裂"的机制漏洞,operator 已在 `.ah/rules/master.md` 加硬门——改公共签名前 push 前必须 `cargo check --workspace --tests`。
- **未完成**:**tier-3 真机验收三条硬门未跑**(①worker 骑用户单次登录到 IDLE;②刷新新RT原地写Windows真文件;③第二 worker/用户宿主不登出),runbook 已交 `.kiro/specs/ah-per-worker-credentials/tier3-runbook.md`,执行归 operator(Win11/WSL2 真机)。**merge ≠ 完成,tier-3 三条全 PASS 才算完成。**
- **PR**:#151(MERGED,`3a7bb5a`)。废弃网关 #146/#147/#149(merged 但回滚停用,从未端到端激活过 claude,历史记录见下)。
- **测试**:单测✅(1065 lib passed + 58 集成测试文件全绿)/ CI ✅(全平台绿)/ **tier-3 ⏳待operator真机**。
- **验证债(已签字,沿用)**:F5 上游 invalid_grant 残余赌注不花真凭据验(仅 Layer2 类代理停用时需回验,当前方案已无 Layer2,大概率不适用);D-1 从"零本机登出"松动为"接受残余窄竞态"(ahd 宕机+AT过期+用户正用原生CLI 三条件同时发生的物理天花板)。
- 参考:requirements.md「2026-07-12 晚」三个冻结块、design.md 顶部冻结章节、tier3-runbook.md;历史(Layer1/2/3 Gateway,已作废)见 design-rev.md、convergence/divergence-2026-07-12、spike-1-2-report、incident-2026-07-12-gateway-bridge-ahd-current-exe.md。

---

## 三、P1 — Windows 原生 · ah-windows-native 【HALF ⭐用户最高优先】

- **需求原话**:"first user-usable Windows MVP remains M0 + M1 + M2 because production agent/master spawning still depends on the M2 ConPTY multiplexer"。
- **已完成**:M0.5 ConPTY spike(#90)+ M0 编译门(#91,`cargo check --target x86_64-pc-windows-msvc` CI 绿)+ gateway Windows cfg 修补(#132)。
- **未完成**:**M1(Win32 adaptors:HANDLE/JobObject/TaskScheduler COM/Named Pipe IPC)未启动;M2(ConPTY 多路复用,tmux→ConPTY 硬核心)未启动。** m0 列 5 条 carry-forward 全 deferred。
- **PR**:#90/#91/#132 merged。**测试**:M0 编译门✅;M1/M2 运行时 smoke ❌。
- **备注**:用户标"全力打通我要用",但真产品化核心还在起点。**这是 P1 但离交付最远的一项。**

---

## 四、P1 — 编排可靠性 · ah-orchestration-reliability 【ACTIVE·最大未偿债】

- **需求原话**:"Every destructive action against an agent runtime resource MUST pass all applicable ownership layers before it can reap/kill/stop/remove"(D1 三层 ownership gate,fail-closed);"ahd MUST run a continuous reconcile loop"(R,5s tick,把 ahd 从 reactive 变 reconciler);"Evidence counts toward completion only if T_evidence > T_dispatch"(FS 新鲜度)。
- **已完成**:**无**(tasks 全 `[ ]`;`reconcile_active_agents_once`/`agents.spawned_at` 在 src 中不存在,整套 reconciler 零实现)。
- **PR**:仅 #141 修了 respawn storm(ah#13),属外围。
- **状态**:ACTIVE,纯 backlog。

### ⚠️ 已知但未修的 6 个病例(root-cause 确认,无实现 PR)——可靠性风险最集中处
| 病例 | Bug | 修向 |
|---|---|---|
| dispatch-ack-race | prompt 未落 pane 即 STUCK,phantom DISPATCHED job 永卡 | bounded IDLE wait + STUCK 非死端 |
| realign-atomicity(ah#16) | realign 非原子 delete-then-spawn,末位 agent 蒸发 | 两阶段替换+session锁 |
| stuck-false-positive-log-monitor | 300s log-monitor 硬超时 + health_check `.or()` stale marker 遮 live | 两处 fix 已定 |
| lane-completion-triple-failure | agy Stop 钩子静默 + log 300s 超时 + 催单器盲注入击穿 | 未修 |
| recovery-reinsert-vs-cancel-race | reinsert 不查 cancel_requested → 取消任务被重投跑完 | 未修 |
| respawn-pane-name-mismatch | respawn 后 tmux 重名并存,感知按名投错席 | 未修 |

### ⚠️ 新增(用户 2026-07-12)+ operator 现场诊断 —— 两条**不同类**需求,已拆分归档
> R-DYN-1 是**控制能力**(新命令),AGY 是**可靠性缺陷**(检测器 bug)——不是同类,不放一个 spec。
> - R-DYN-1 → `.kiro/specs/ah-agent-lifecycle-control/requirements.md`(Requirement DT.1-DT.5)
> - D-AGY-COMPLETION → `.kiro/specs/ah-orchestration-reliability/requirements.md`(Requirement AGY)
- **R-DYN-1(用户原话)**:"1个是重启单个agent，还有一个需求是随时改拓扑，热启动新的agent，或者随时停掉单独的agent"。当前 ah 只有 `ah up`(整体reconcile,带ah#16危害)/`kill`/`start`,缺:①`ah agent restart <id>` 原子重生单个 ②运行中改拓扑 ③热插新agent ④优雅停单个。**PENDING。**
- **D-AGY-COMPLETION(operator 亲验推翻"o1卡死")**:o1 假 BUSY/Deferred **不是进程卡死**——agy 活着响应("ping"秒回"Yes")、早已 concluded;卡的是 ahd 完成检测器判 Deferred + 自动注入"等cargo test"催促(该任务纯markdown无后台命令)→ 假BUSY死循环。**解法=poke一turn+Esc即转IDLE,不需 ah up。** 根因=PR#122 yield-and-wait 检测器过度触发。**PENDING(根因已定位)。** 是 [[project_ah_agy_wait_deadlock_false_busy]] 根因层。

---

## 五、P2/P3 — 进行中 & 半途

- **completion-protocol【ACTIVE】**:显式完成声明(`ah job done/fail` 走 outbox)取代"turn 静止即完成"。design 冻结、R1 参考臂 #142 merged;主体(R2/G4/证据闸门/R3拆除)tasks 标"framing only 未排期",等 operator 亲验。tier-3 dogfood 未跑完。
- **v1-public-release【ACTIVE】**:"external integrator 可编辑 per-agent rule docs;kernel+scenario 分层;unknown provider→硬报错不静默回退bash"。kernel/scenario 经 plugin-bundle+builtin skills 落地,v1.6.0(#144)已出;design 5 点未逐条追溯闭环。只有 design.md 无 requirements/tasks。
- **r1-outbox-followups【ACTIVE零实施】**:R1 孤儿事件永败要终态归档(现场 sess_334718e9 FK 永败无限重试);R2 reap-on-RPC-success 接线(#142 已写未接线);R3 两条代码观察。前置 #142 merged,follow-ups 待派单。
- **perception-arbiter【HALF】**:C1 单点写门(唯一函数可写 agents.state)+ C6 STUCK→Stalled 非死端。4 阶段只落 Phase 1(#136);Phase 2 生产者迁移/Phase 3 tri-state/C8 cgroup PoC(commit 195cbe2 **dangling 未合main**)全未做。
- **control-plane-refactor【HALF】**:D1 JobStatus 状态机门 + CI 禁直写 `UPDATE jobs SET status`。只 Phase 1 落地(#136/#137/#138);**Phase 4 的 agent-state `VERIFYING`/`FAILED_VERIFICATION` 未实现**(仅 master cutover 上下文有 VERIFYING);Phase 2/3/5 未做。
- **unified-capture【HALF】**:三类脆弱屏幕表面(StartupReadiness/RuntimeMarker/ReplyExtraction)纳入"学习→Rust校验→回填"闭环。Phase 2 通用抓取 + StartupReadiness 闭环落地;Phase 4 治理引擎(confidence/quarantine/LRU)显式延后。
- **prompt-handler【HALF】**:can-input 探针就绪门 + SPAWNING/BUSY→IDLE 职责分离。Phase1 物理底座交付(CI Mock 全绿);**PR4b(自学习DB+Haiku慢路径)、PR6b(真机parity)、PR8(通用migration框架)显式延后**。真 LLM parity 未跑。
- **studio-req1-provisioning【HALF】**:`ah setup --check/--fix/--json/--resume`(read-only 除 --fix)。Phase0/1/2 代码闭环(#94-97/#105);**Phase2 真机门未签(等用户 Win11/WSL2 runbook 签 PASS,v1.3.0-rc)**;Open-in T3/T4 pending。
- **macos-port【HALF】**:Linux 行为为兼容 oracle,先抽平台 trait 零行为变更。7 trait 仅 ProcessWatcher(kqueue,#74)真实现;launchd服务/scope容器/doctor 未做。
- **product-delivery【HALF】**:伞形可靠性 spec(OOM自恢复/master生命周期/recovery-reinsert原子性/idle-crash-revive 等)。多子设计分批落地(#110/#121/#141等),若干 design 未实施/未实证闭环;以 2026-07-11 handoff 为准。
- **oom-restart-resume【HALF/SUPERSEDED】**:facet A(worker OOM自愈,三provider dogfood-proven)+ facet B(`Restart=on-failure`)DONE;**facet C(ah-job 在途续接)折入 Step-4 未单独交付**,依赖 master 重生(已随 self-switch 落地)。唯一待用户 goal 点:"resume 续断点"=provider上下文恢复(已闭)还是 ah-job级追踪(深改)。
- **job-events【HALF/ACTIVE】**:`job_transitions` 表 + runtime snapshot v2 投影(job_events[]+cursor+resume)。表落地(随 state-contract);完整投影层是 2026-07-11 新设计,收尾度待核;与 #142 交叉。
- **self-knowledge-skills【ACTIVE】**:三 builtin skills(ah-config/ah-runtime-state/ah-operate)+ kernel 索引落地(#108/#109);T5 plugin 一键安装 / T6 公开仓同步 尾巴未确认。
- **real-dogfooding-acceptance【ACTIVE】**:真 ahd/CLI/provider 矩阵,禁 fake。BUG-1(kill 泄漏孤儿孙子进程)已修+实证+回归;残留 scope failed-unit 清洁化 + 真 provider OAuth 受限测点 BLOCKED(不冒充 PASS)。
- **sop-required-checks-automerge【HALF】**:auto-merge 前必须 required checks。dev 仓 main 已配 `test` 必过;pack kernel 层 prerequisite 文档条目未随版本发布。

---

## 六、DONE(已合入,压缩清单)

| spec | PR | 备注 |
|---|---|---|
| isolation-core | #6 | 删bwrap→env隔离;其 OAuth symlink 正是 P0 要推翻的病根 |
| core-fixes | (早期) | Req1-4 全 Implemented;守护进程奠基 |
| evidence-statemachine | #16 | PR-1a/1b 合入;tasks 未回勾(文档滞后) |
| hook-push-completion | #55/#60/#84/#122/#123 | agy 假完成门;部分被 completion-protocol 继承 |
| dogfooding-closure | #30/#31/#32/#40 | fake provider,0介入 dogfood-8 |
| full-e2e | #22/#23/#25 | 14步主线+6case分支矩阵(mock) |
| A3-adversarial-review | 无(纯审) | 反哺 C/D spec 收敛 |
| lifecycle-reaping | #130-133 | 动态 service unit 检测,无硬编码残留 |
| pr6-recovery-resume | (case_11绿) | claude `--continue` resume |
| pr7-oom-resume | #37 | ahd自OOM+codex/agy resume |
| state-contract | #112-119 | job_transitions表+CLOSED+status--json;v1.4.0 |
| master-self-switch | #37/#52/#53/#71/#121 | 真master切ah-managed+corrected death;宽版 |
| master-tell-observability | #98 | ah tell master + BUSY/IDLE |
| pr4b-builtin-layer | #13/#15 | builtin rules 铺底 |
| pr4c-hooks-plugins | #17 | hooks+plugins 双侧物化 |
| pr4d-auto-provisioning | #19 | git-based plugins 自动补齐 |
| pr4e-up-fingerprint | #20(+#141修) | config_hash+ah up 指纹审计 |
| pr1b-readfirst-hook | #18 | Read-first evidence writer |
| pr5-rename | #27 | ccbd→ah/ahd Phase A |
| plugin-bundle-pr3 | #80 | codex bundle 物化 |
| plugin-bundle-pr5 | #83 | realign/recovery重物化+CLI |
| pr4-orchestration | (早期#13等) | 三层隔离基座;凭据模式已被P0取代 |

---

## 七、SUPERSEDED
- **pr8-migration-config**:以 bwrap 挂载为隔离前提,项目已 pivot 到 env-var 隔离;内容被 v1-public-release 的 kernel/scenario 吸收。仅一份 design 笔记。

---

## 八、跨账观察(维护时注意)
1. **`job_transitions` 表被 3 spec 共享**(state-contract 落地 / job-events 投影 / r1-outbox #142 未来重指向后 drop),schema.rs:258 标了迁移债——作为"跨 spec 载体演进"单独追踪,任一 spec 宣称完成时别漏后续 drop。
2. **反噬链**:isolation-core 的 OAuth symlink(DONE)是 P0 要推翻的病根;evidence-statemachine/hook-push 的证据闸门/完成检测被 completion-protocol 升维继承——"旧实现→新总纲重构"的继承关系。
3. **requirements.md 普遍缺失**:大量 spec 只有 design/tasks/research,无独立 requirements.md 承载用户需求原话。**追溯审计的结构性缺口**——新 spec 应先立 requirements.md。
4. **文档滞后**:多个 DONE spec 的 tasks checkbox 未回勾;per-worker-credentials 的 tasks.md 仍写废弃网关。判定一律以 git merge 为准。
5. **两个 HALF 卡"实证门"**:windows-native(M1/M2硬核心未启)、studio-req1(真机runbook待签)——都是"代码闭环≠实证闭环"。
