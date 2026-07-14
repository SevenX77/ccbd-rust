# PR 疗效台账(operator 维护 · 每 PR 逐项确认效果)

**建账背景**:用户 2026-07-12 问责——"每个落实的 PR 有没有确认过效果?出了 bug 能不能定位到是哪个 PR?" 答案是没有台账。本文件补建初版,覆盖最近 45 个 merged PR(#99–#145;#139/#140 是 issue 非 PR,不在列)。

**维护规则(纪律,违者算 SOP 漂移)**:
1. **每次 merge 当场登记**:新 PR 合入当场追加一行——预期效果(从 title/body 提炼一句话)+ 实证计划(挂靠哪个观察节点:下次换血 / 下次 dogfood / 活栈观察 / 专项演练)。
2. **每次事故当场反向定位**:观察日志新增病例时,当场在"bug→PR 定位索引"补一行——由哪个 PR 引入、哪个 PR 修复;定位不了写"未定位",**不许编**。
3. **每次换血/发版前过一遍验证债**:验证债清单是换血 runbook 的例行项;能在新窗口验的挂进 gen-efficacy 开窗预期表,验不了的写明为什么。
4. **状态只允许四值**:**实证闭环**(有落盘 dogfood/活栈证据)/ **CI 绿仅代码闭环**(测试过但无活栈实证)/ **验证债**(明确该验没验)/ **未观测**(没人看过效果)。严禁把"CI 绿"美化成"实证闭环";拿不准往保守里判。
5. 证据只认落盘出处(观察日志 obs#、疗效账本代次段、handoff/incident 文件、在库测试文件);不凭印象。

**证据源**:`logs/operator-observation-log.md`(obs#)· `research/dogfood-ledger-2026-07-10.md`(疗效账本 Gen-0~4)· `research/gen-efficacy-reports.md`(代次判决)· `research/MODULE-STATUS-LEDGER.md` · `research/incident-*.md` / `research/pr4-reconcile-livefire-incident.md` · `.kiro/specs/*/`。

**最后更新**:2026-07-12(初版补建,operator 审计工装代笔)。

---

## 一、逐 PR 台账(#99–#149,倒序)

> **网关子程序整体裁决(#146/#147/#148/#149)**:方案=Plan B 假网关 per-worker 凭据隔离。**端到端从未激活过一个 claude worker**(三次合入全靠 CI 绿 + 审,缺 A2 tier-3 真二进制 spawn)。#146 激活即秒死→回滚;#147/#149 逐层修(sandbox root / ah_bin resolve)后所有走网关的 claude 仍 <200ms 秒死;**方案已弃**,转凭据第一性重构(Layer1/2/3,见 ah-per-worker-credentials/design-rev.md,PR#150 Layer1 OPEN)。净效果=负(消耗一整个下午 + 回滚 preModD 保活栈)。教训已固化:换血必校验 ahd gateway 符号 + 端到端 spawn smoke。

| PR# | 合并日 | 一句话预期效果 | 实证方式与证据出处 | 状态 | 关联事故/回归 |
|---|---|---|---|---|---|
| #149 | 07-12 | 网关 bridge ah_bin resolve(修 current_exe=ahd)——见本文件末尾专节 | 换血实证:resolver 层修好,但第二层激活缺陷仍在,claude 全部秒死 | **部分闭环·激活仍失败** | Module D 端到端从未激活;方案已弃 |
| #148 | 07-12 | (网关 ahbin v1)误合进落后于 main 的死分支 feat/gateway-graft-modD | 已标 Superseded,由 #149(base=main,v2 语义融合重实现)取代 | **SUPERSEDED** | 陈旧本地 main 致 PR base 选错(记忆档案 project_ah_stale_local_main_wrong_pr_base) |
| #147 | 07-12 | 网关 bridge sandbox root 派生修复(uds sandbox-root regression);merge 2966de4 | 无独立实证:与 #146/#149 同属网关子程序,端到端从未激活 claude worker;随方案弃置作废 | **实证证伪·随方案弃置** | 同 #146 激活缺陷族;缺 A2 tier-3 |
| #146 | 07-12 | 模块 D 凭据网关嫁接:per-worker 凭据隔离(治共享 OAuth symlink 轮换登出=ah#18);merge commit 8f2aab5 | **2026-07-12 换血实证:激活即打死所有 claude worker → 回滚**(见下) | **实证证伪·激活缺陷** | 换血装新 ahd 后 `ah start` → d1(首个 claude worker)`AGENT_UNEXPECTED_EXIT`,session 回滚。根因:bridge 经 `{ah_bin} internal-bridge` 启动,ah_bin=`current_exe()`=`ahd`,但 internal-bridge 只编进 `ah` CLI(符号 ah=13/ahd=0)→ 挂起 → port 永不写 → exit 126。CI 绿因测试从未用真 ahd 端到端 spawn 真 claude worker(current_exe=测试二进制恰含子命令,掩蔽)。已回滚保活栈,ah#18 仍未治。缺陷+修向归档:`.kiro/specs/ah-per-worker-credentials/incident-2026-07-12-gateway-bridge-ahd-current-exe.md`。**教训**:换血必须校验 `ahd` 二进制 gateway 符号 + 端到端 spawn smoke,不能只看 `ah --version` |
| #145 | 07-12 | 场景包 v0.6.0:classic 七席拓扑模板 + dev pack/ 恢复 source-of-truth(回流公开侧 hotfix 漂移) | 无(模板未经过装机验证;发布后外部使用零观测) | **未观测** | 关联历史:场景包漂移+leak-gate bug(记忆档案);发布纪律=双向 direction audit |
| #144 | 07-12 | 版本 1.5.0→1.6.0 记账 + 公开仓 v1.6.0 发布同步 + 3 处 doc-comment 泄漏卫生 | PR body 自述公开仓已发 v1.6.0(cargo-dist);live 栈二进制仍 7bae3b1 未跑 v1.6.0 构建 | **CI 绿仅代码闭环** | 无 |
| #143 | 07-11 | agy Stop-hook 送达链修复:绝对路径 ah 二进制 + timeout 单位秒(治 obs #43 送达率 0%) | **部分实证**:换血#5 三个新 agy 沙箱 hooks.json 材料化=绝对路径+timeout 5(gen-efficacy Gen-5 #2);但核心断言"hook 真送达 daemon"未过——obs #51:g1-m1 hooks-debug 零日志,**hook 根本没执行** | **验证债** | obs #43(病因)、obs #51(修后同晚 agy 侧仍不执行 hook,疑上游) |
| #142 | 07-11 | R1 outbox:journal-first 落盘 + ahd 冷扫 replay/reap/隔离,daemon 停机窗口完成信号不丢 | **首触发实证**:换血#5 重启,旧 master 停机瞬间 hook journal-first 落盘,冷扫 replay FK 失败→正确 retry_deferred=1 无热循环(obs #46;gen-efficacy Gen-5 #3) | **实证闭环** | 自带缺口:死 session 孤儿事件 FK 永败不进 error-book/dead(obs #46);reap-on-RPC-success 未接线(MODULE-LEDGER §三.5) |
| #141 | 07-10 | 修 `ah up` respawn 风暴(公开仓 ah#13):配置指纹两侧同源计算 | 换血#4 开窗当日活栈 `ah up` 全员 NO_CHANGE + pane pid 零变化,旧血同命令=全栈风暴(obs #41;gen-efficacy Gen-4 关窗判决=治愈-实证) | **实证闭环** | 残余断言未验:"真 drift 仍触发 + 错峰 ≥500ms"(Gen-5 走了 daemon 重启连坐路径没走到,obs #44)、"单 agent catch-22 消失"未观测 |
| #138 | 07-10 | C1 写闸检查器把 D1 的 job_state.rs 测试夹具收进冻结基线(解跨泳道 merge 碰撞) | 无活栈证据;Gen-4 关窗=未观测(无触发),Gen-5 顺延(gen-efficacy Gen-4 #4 / Gen-5 #6) | **未观测** | 无 |
| #137 | 07-10 | D1 job 状态机闸门:jobs.status 全部写路径过单一状态机,非法迁移被闸 | 无活栈证据;Gen-4 关窗=未观测(窗口内无非法迁移负样本),Gen-5 顺延(gen-efficacy Gen-4 #3) | **未观测** | 注意:agy turn-end 假 COMPLETED 是"语义谎报"非"非法迁移",D1 管不到(Gen-4 判决原文) |
| #136 | 07-10 | C1 感知写闸:agents.state 迁移全走 gate + 事件通道 + CI 强制 | 无活栈证据;Gen-4 关窗=未观测(幽灵文本病例走 dispatch 就绪复查路径,不经写闸),Gen-5 顺延(gen-efficacy Gen-4 #4) | **未观测** | Gen-3 双 gate 审计曾抓 CI checker fail-open 真 bug 并已在合入前修(obs #38,正样本) |
| #135 | 07-10 | 场景包 v0.5.1:dual-lane 双泳道拓扑模板 + OPERATOR.md 哨兵体系 | 无(pack 模板未装机验证;活栈双泳道是 .ah/rules 直配,非经 pack 安装) | **未观测** | 无 |
| #134 | 07-10 | 发版 v1.5.0(覆盖 #120–#133 感知可靠性系列) | Gen-3 整代活栈跑 v1.5.0(e06b8f9)dogfood,换血首启干净(疗效账本 Gen-3 开窗) | **实证闭环** | obs #34:本 PR merge 时抢跑(check 有实例仍 pending 即合),流程事故自纠 |
| #133 | 07-10 | de-flake orphan-reap 测试(并发收敛容忍,不用 #[ignore]) | 仅 CI;flake 复发与否无人跟踪 | **CI 绿仅代码闭环** | 治 obs #28 的 flake |
| #132 | 07-10 | cfg(unix) 门控 libc::kill 测试,救活 windows-msvc-check | CI 恢复绿(对本 PR,CI 绿即全部预期效果;obs #26/#28 处置记录) | **CI 绿仅代码闭环** | 治 #130 引入的 CI 红(obs #26) |
| #131 | 07-10 | ro_binds 配置在 validation 层大声拒绝(治 scope 属性非法致 agent 秒死) | Gen-3 换血首启干净、同配置不再秒死;gen-efficacy 追溯摘要 Gen-3=治愈 | **实证闭环** | 治 obs #22(Gen-2 首启秒死) |
| #130 | 07-09 | 模块 B:B1 身份注入加固 + B2 测试 tmux 泄漏隔离/自愈清扫 + B3 teardown 逃逸 | 无正式判决:obs #23 设的"Gen-2 核对项"(ahd-* 测试 socket 是否继续新增)从未关账;obs #35 换血#3 仍清出 55 个泄漏测试 daemon(harness 侧不归 B2 管,产品侧疗效未单独判) | **验证债** | 引入 windows-msvc CI 红(obs #26→#132 修)+ orphan-reap flake(obs #28→#133 修) |
| #129 | 07-09 | 模块 A:stuck reason 参数化 + orphan recapture 死码删除(预期 G3 假 STUCK 归零) | Gen-2 开窗预期"G3 归零"但历代关窗从未给 G3 判决项;Gen-2~4 疗效账本无 G3 病例=弱负证据,不够格算治愈 | **验证债** | 治 obs #10(a3 真完成被判 STUCK+FAILED,根因路径本 PR 删除) |
| #128 | 07-10 | P0-2:熔断三层清零洞补上 + 认领时刻 cancel 过滤 | cancel 半边:Gen-2 正样本 job_f6375ac6 QUEUED→CANCELLED 干净落终态(疗效账本 Gen-2,归因有效;Gen-1 正样本因半换血归因作废);**breaker 三层洞半边无触发场景,零证据** | **实证闭环** | cancel 对"僵尸在途单"无认领人仍卡 CANCEL_REQUESTED(obs #31,设计缺口非回归);cancel×dispatch 竞态另案(obs #49/#52,未修) |
| #127 | 07-09 | P0-1:删三个 pane 文本毒推断器(PANE_DIFF_STUCK / UI 完成识别 / …),扫描降级 alert-only | Gen-2 真观察窗:G2 垃圾 reply 假完成 0/2 归零(疗效账本 Gen-2;Gen-1 的 0/2 因半换血归因作废,Gen-2 重验成立) | **实证闭环** | 语义型假完成(回合结束≠完成)不在本 PR 治疗范围,后续各代持续发生(对照组) |
| #126 | 07-09 | Fix C:删 unknown→park 生命周期推断,park 仅白名单(治幽灵/banner 假 PROMPT_PENDING) | Gen-1:G1 误闩 6/6→0/3 治愈-实证(obs #8;疗效账本 Gen-1;半换血更正后归因仍成立——#126 在 Gen-1 daemon 里) | **实证闭环** | 同族第二路径未治:dispatch 就绪复查 pane-diff 仍被幽灵文本击穿(obs #24/#36/#42/#53/#54,= ah#17,非本 PR 回归);§G 残留债"Fix C 真 CLI 场景"未验 |
| #125 | 07-09 | 两只 alert-only 看门狗:QUEUED 饥饿告警 + PROMPT_PENDING 压制升级 | 无:Gen-4 §G 验证债项"A/B 看门狗真触发"至今未观测(gen-efficacy Gen-4 #7) | **验证债** | 无 |
| #124 | 07-09 | REAL pane 夹具搬迁 tests/fixtures + 脱敏 | 仅 CI;a4 逐文件 blob diff 审计 PASS(PR body 自述) | **CI 绿仅代码闭环** | 无 |
| #123 | 07-09 | 删 "5 passed" 过拟合逃生门,transcript 任务信号权威化(#122 追加) | 无活栈判决:Gen-4 §G 债"G2 检测器假 COMPLETED 对照归零"未观测 | **验证债** | G2 检测器审计反转事件:亲验推翻过一次 REJECT、揪出 cancel 正则真 bug(记忆档案,修复去向未记账) |
| #122 | 07-09 | agy pending-task 检测:有后台任务的 turn 不判 COMPLETED(治 yield-and-wait 假完成) | 无正式判决;且后续窗口 agy turn-end 假 COMPLETED 仍多例(obs #42 族;Gen-4 判决#6"如预期继续发生"——账本按"无对应修复"记对照组,但本 PR 的既定目标正是这类假完成)→ **疗效存疑,需专项对照** | **验证债** | pend 哨兵被 agy 假完成击穿(记忆档案);A/B run 假 BUSY 冻结 2h41m(obs #42 族) |
| #121 | 07-09 | 堵 master-revive 降级 spawn 路径的 AH_AGENT_ID 残留泄漏(#120 的独立审计发现) | 仅 CI;revive 降级路径活栈从未触发观测 | **CI 绿仅代码闭环** | 修 #118 引入的泄漏(第二处) |
| #120 | 07-09 | spawn 命令边界统一洗掉继承的身份 env(master 不带 stale AH_AGENT_ID) | 仅 CI;无活栈取证 | **CI 绿仅代码闭环** | 修 #118 引入的泄漏(第一处) |
| #119 | 07-09 | 发版 v1.4.0(state-contract 系列 #112–#118) | PR body 自述"六个契约面隔离 e2e 验过";但计划中的总验 e2e(`tests/e2e_state_contract_final_a4.rs`,见 research/state-contract-e2e-final-plan.md)**从未落盘**;v1.4.0 从未作为活栈代次跑过(换血#1 直接上的 b363dce) | **CI 绿仅代码闭环** | 无 |
| #118 | 07-09 | 全部 7 个 spawn 位注入 AH_AGENT_ID/AH_SESSION_ID/AH_ROLE 身份三元组 | 仅 CI;沙箱内身份 env 实际可用性无落盘取证 | **CI 绿仅代码闭环** | **引入** stale 身份 env 泄漏(worker/master 继承 daemon 侧 AH_AGENT_ID)→ #120/#121 修 |
| #117 | 07-09 | PR4:bare-start 守卫 + orphan-scope reconcile 接线 + 三个 kill 安全修 | 开发期 livefire 事故催生了安全修(research/pr4-reconcile-livefire-incident.md:e2e 测试三轮屠活栈 3/3);合入后"reconcile 删留需压测实证"(记忆档案)从未做;bare-start 守卫无活栈触发记录 | **验证债** | 开发期事故:kill/scope 类 e2e 每次启动 ~30s 后活栈 worker 死一批(已由本 PR 内含修复止血,双 daemon 互不越界压测未补) |
| #116 | 07-09 | de-flake cancel 通知测试(drain 到自己的事件) | 仅 CI | **CI 绿仅代码闭环** | 无 |
| #115 | 07-09 | PR3:`ah status --json` 快照 + `ah ps` status 列/--all | `ah ps` 活栈日用实证(obs #31/#53/#54 均以 ps 输出为取证手段);`status --json` 无活栈使用记录 | **实证闭环** | 无(obs #53 ps 报 BUSY 与 pane 真相不符是上游感知病,非 CLI 病) |
| #114 | 07-09 | PR2b:全部 job 状态变更落 job_transitions 持久事件 + 逐站原子性契约 | 仅 CI;活栈 job_transitions 行正确性从未审计;系列总验 e2e 未落盘 | **验证债** | 无 |
| #113 | 07-08 | PR2a:空闲 master 正常退出落 CLOSED 终态(不再误标 FAILED) | 活栈见过 session CLOSED(obs #44 daemon 重启连坐路径),但本 PR 的目标路径(IdleNoWork 正常退出)从未单独判决 | **验证债** | 无 |
| #112 | 07-08 | PR1:RuntimeSnapshot schema v2 + job_transitions 表 + CLOSED 回填 | 隔离 e2e 在库(tests/e2e_state_contract_pr1_a4.rs);活栈 DB 各代在跑 v2 但无正确性审计 | **CI 绿仅代码闭环** | 无 |
| #111 | 07-08 | de-flake completion-monitor / dispatch-recheck 单测 | 仅 CI | **CI 绿仅代码闭环** | 无 |
| #110 | 07-08 | 所有 kill/级联路径加 pane/session 归属守卫(治 2026-07-08 活栈覆灭事故根因) | 隔离 e2e 在库(tests/e2e_kill_path_ownership_a4.rs);守卫拦截的活栈演练从未做;07-08 后多轮 kill/级联无复发=弱负证据 | **CI 绿仅代码闭环** | 治 research/incident-stale-session-kill-cascade-2026-07-08.md(stale %0 误杀全栈) |
| #109 | 07-08 | ah-config / ah-runtime-state 自知识 builtin skills + kernel 三技能索引 | 无:skill 内容对 master 行为的效果零落盘归因 | **未观测** | 无 |
| #108 | 07-08 | ah-commands master-only builtin skill + kernel 瘦身(master 不再瞎猜命令) | 无:master 正确用 `ah pend` 是 master.md 规则强制的结果(obs #32),没有证据归因到 skill | **未观测** | 无 |
| #107 | 07-08 | dev-programming 场景模板 + 保真测试 | 仅 CI(fidelity tests);模板装机使用零观测 | **CI 绿仅代码闭环** | 无 |
| #106 | 07-06 | T4:`ahd --version` 不再误拉 daemon + RPC EOF 可诊断 + e2e teardown | 无活栈/真机取证;Studio Req1 v1.3.0-rc 整体等 Win11/WSL2 真机 runbook 签 PASS(记忆档案) | **验证债** | 排查期曾在 default state dir 误起真 daemon(本 PR 治因) |
| #105 | 07-06 | 发版 v1.3.4(Studio Open-in root fixes T1+T2) | tag 已出;交付效果(Studio 端到端修好)等真机门 | **验证债** | 无 |
| #104 | 07-06 | T2:claude worker 沙箱 HOME 由 daemon 注入 IS_SANDBOX=1(解 harness 模板耦合) | 无落盘取证(活栈 claude worker 应每天走此路径,但没人验证过注入生效);挂 Studio 真机门 | **验证债** | 治 Studio Open-in 事故根因二 |
| #103 | 07-06 | T1:`ah events` 增派生 starting 相位(冷启动不再被误判 degraded) | 无;挂 Studio 真机门 | **验证债** | 治 Studio 事故:#99 上线后 Studio 把冷启动 runtime 误判并自动 `ah stop` |
| #102 | 07-06 | 落盘 2026-07-06 Studio 交接文档(T1-T4 pending,证据对过 origin/main) | 效果=文档持久化,自证(文件在库) | **实证闭环** | 无 |
| #101 | 07-06 | `ah events` 在 daemon 关流后存活:发 down-edge 快照 + 持续重连(1.3.3) | 无;GUI supervisor 消费端修复效果等 Studio 真机门 | **验证债** | 治 Studio 冻结在最后活跃快照的事故 |
| #100 | 07-06 | `ah events` inventory 过滤修复(Studio 临时 config 目录不再匹配空)+ CLAUDE_CODE_OAUTH_TOKEN 透传(1.3.2) | 无;挂 Studio 真机门;OAuth 透传活栈使用无记录(live worker 走 symlink 凭据) | **验证债** | 治 #99 首个真实消费者暴露的过滤 bug(Studio 状态永远 inactive) |
| #99 | 07-06 | `ah events --format json` 稳定 JSONL 运行时事件源 + runtime.snapshot/subscribe schema v1 | 有真实消费者(Studio 接入)但首用即暴露三处缺陷(#100 过滤、#101 断流、#103 相位);修补后的端到端效果等 Studio 真机门 | **验证债** | 首用引发 Studio Open-in 事故(2026-07-06);修复链 #100/#101/#103/#104/#106 |

---

## 二、验证债清单(所有非"实证闭环"PR;补证动作 + 挂靠节点)

> 排序按扎眼程度。规则:每次换血/发版前过一遍;能挂进下个 gen-efficacy 开窗预期表的当场挂。

**验证债(明确该验没验,16 条)**

1. **#143 agy hook 送达**:材料化过了但送达断言没过,obs #51 显示修后当晚 agy 仍不执行 hook。补证=journalctl 抓 agy 组 `agent.notify` 非 replay 条目;若仍 0,升级为"上游 agy hook 不 fire"独立病案。挂靠:**当前 Gen 窗口关窗判决(gen-efficacy Gen-5 #1)+ agy hooks 专项取证**。
2. **#136/#137/#138 感知 C1/D1 地基**(3 条):两代窗口零触发场景,疗效连续顺延——重构地基到底闸没闸住任何东西,无人知道。补证=D1 等一例非法迁移拒绝记录;C1 等一例 gate 外直写拦截;或专项构造负样本演练。挂靠:**Gen-5/6 关窗判决(已在 gen-efficacy 顺延表)**。
3. **#117 orphan-scope reconcile**:开发期 e2e 三轮屠活栈(pr4-reconcile-livefire-incident.md),合入后"删留需压测实证"从未做。补证=同机双隔离 daemon 压测,验 reconcile/kill 互不越界。挂靠:**专项压测(下次 dogfood 批)**。
4. **#122/#123 agy 完成检测器**:目标病(turn-end 假完成)后续窗口仍多例,疗效存疑但从未正式对照(账本一直按"无对应修复"记对照组,与本 PR 的既定目标矛盾)。补证=定向对照:抓一例 agy yield-and-wait turn,验 detector 是否真挡了 COMPLETED;审计反转揪出的 cancel 正则 bug 修复去向补记账。挂靠:**下次 agy 派单窗口 + 完成协议设计轮**。
5. **#130 模块 B**:obs #23 设的 Gen-2 核对项(测试 socket 是否继续新增=B2 产品侧疗效)从未关账;obs #35 显示测试 harness 侧仍在泄。补证=一个窗口的 /tmp/tmux-1001 socket 计数对照。挂靠:**下次换血 runbook 例行项**。
6. **#129 模块 A(G3 归零)**:预期写了、关窗从未判。补证=在 gen-efficacy 关窗表补 G3 判决项(Gen-2~4 无病例可追认为"改善",但要正式写)。挂靠:**下次关窗判决**。
7. **#125 看门狗 A/B**:QUEUED_STARVATION_ALERT / 压制升级从未见真触发(Gen-4 §G 原债)。补证=等一例真饥饿或构造演练。挂靠:**活栈观察(gen-efficacy §G 继承)**。
8. **#114 job_transitions 持久事件**:活栈行正确性零审计。补证=抽一个真实 job 的 transitions 行与 pane/产物轨对账。挂靠:**下次 dogfood**。
9. **#113 CLOSED 终态**:目标路径(空闲 master 正常退出)未单独判决。补证=一次正常 `ah stop`/空闲退出后查 session 行=CLOSED。挂靠:**下次换血停栈时顺手取证**。
10. **#99/#100/#101/#103/#104/#105/#106 Studio 系列**(7 条,可合并验):全部挂 **Studio Req1 真机门**(Win11/WSL2 runbook 签 PASS,记忆档案已有此节点)。真机门一签,七条一起收账。

**CI 绿仅代码闭环(13 条,债务等级低,列明即可)**

- #144(发版记账;活栈换血到 v1.6.0 构建时自然闭环)
- #133/#116/#111(de-flake 三连:复发与否无人跟踪;补证=CI 连续 N 绿即追认,挂靠 CI 例行观察)
- #132/#124/#107(CI 门控/夹具/模板:CI 即其主要效果面,可接受)
- #121/#120/#118(身份 env 三连:补证=随便进一个活 worker 沙箱 `env | grep AH_` 取证一次,五分钟的事,挂靠下次换血)
- #119(v1.4.0 发版;系列总验 e2e 计划文件 `tests/e2e_state_contract_final_a4.rs` 从未落盘——要么补写要么正式销案,挂靠发版前检查)
- #112(schema v2:随 #114 对账一起收)
- #110(kill 守卫:有隔离 e2e;可选演练=对终态 session 跑 cleanup 验守卫拦截,挂靠专项)

**未观测(7 条)**

- #145/#135(场景包两版:补证=按 README 真装一次机,挂靠 pack 发布纪律)
- #138/#137/#136(见验证债第 2 条,同一批)
- #109/#108(builtin skills:补证=新 master 空白会话问一句 ah 命令看引用来源,挂靠下次换血 orientation)

---

## 三、bug→PR 定位索引(事故反向归因;定位不了写"未定位",不许编)

| 事故(obs# / incident 文件) | 由哪个 PR 引入 | 由哪个 PR 修复 |
|---|---|---|
| Studio Open-in 事故 2026-07-06(状态永远 inactive / 冷启动被自动 ah stop / 断流冻结) | #99 首个真实消费者暴露(缺陷随 #99 上线) | #100(过滤)+ #101(断流)+ #103(starting 相位)+ #104(IS_SANDBOX)+ #106(T4);发版 #105;端到端效果待真机门 |
| 活栈覆灭 2026-07-08(incident-stale-session-kill-cascade):删 2 个终态 session 连坐杀全栈 | 未定位(历史 cleanup 路径 stale pane id 复用) | #110(守卫;拦截演练未做) |
| PR4 livefire 2026-07-09(pr4-reconcile-livefire):kill/scope e2e 三轮屠活栈 worker 3/3 | #117 开发中代码/测试 env 泄漏(两候选根因未终判) | #117 自带三个 kill 安全修止血;压测销案未做 |
| stale 身份 env 泄漏(master/worker 继承 daemon 的 AH_AGENT_ID) | #118 | #120 + #121(第二处由独立审计发现) |
| obs #10:a3 真完成被判 STUCK+FAILED,产出险丢 | 未定位(历史 PANE_DIFF_STUCK 推断路径) | #129(删根因路径)+ #127(毒推断器降级 alert-only) |
| G1 误闩族(Gen-0 6/6:banner/幽灵文本假 PROMPT_PENDING) | 未定位(历史 pane 扫描设计) | #126(Gen-1 实证 6/6→0) |
| G2 垃圾 reply 假完成(Gen-0 5 例) | 未定位(历史 UI 完成识别) | #127(Gen-2 实证 0/2) |
| 幽灵文本击穿 dispatch 就绪复查(obs #24/#36/#42/#53/#54,= ah#17 第三路径;o1 三度卡死) | 未定位(pane-diff 推断历史设计;#126 只治了 scanner 路径) | **未修**(ah#17 / 感知仲裁设计轮) |
| obs #22:Gen-2 首启 agent 秒死(ro_binds 翻成 scope 非法属性) | 未定位(引入窗口=#127–#130 之间,Gen-1 daemon 尚不翻译该配置;未逐 commit 定位) | #131(Gen-3 实证) |
| obs #26:windows-msvc CI 红(libc::kill 无门控) | #130 | #132 |
| obs #28:orphan-reap 测试 flaky | #130 | #133 |
| obs #33:reply 载荷错位(COMPLETED 存 brief 残片) | 未定位(claude provider 抓屏历史设计) | **未修**(模块 C 完成协议 / R1 显式上报方向) |
| ah#13 respawn 风暴 / kill+up 连带 respawn 配对 gatekeeper(obs #30/#37) | 未定位(指纹两侧异源计算,历史设计) | #141(风暴半边 Gen-4 实证;catch-22 半边未观测) |
| obs #43:agy Stop hook 送达率 0%(裸 ah + timeout 单位错) | 未定位(hook 注入实现 PR 未回查) | #143(命令侧已修实证;送达仍未实证,obs #51 疑上游不 fire) |
| obs #46:outbox 死 session 孤儿事件 FK 永败、永驻重试 | #142(自带缺口,合入时 A′ 报告自报延期) | **未修**(R1 follow-up,.kiro/specs/ah-r1-outbox-followups) |
| obs #49:respawn pane 命名错位 / recovery 重投×cancel 竞态 / codex 无视 worktree 钉死 | 未定位(respawn 命名=#30 老 bug 家族;竞态=历史调度设计) | **未修**(spec 病例单已开:ah-orchestration-reliability/*-2026-07-11.md) |
| obs #51:泳道死锁三重奏(agy hook 不 fire × log 监听 300s 放弃 × 硬编码催单文案=指令注入) | 未定位(三层全历史设计;300s=MAX_LOG_MONITOR_WAIT 已知债) | **未修**(spec:lane-completion-channel-triple-failure-2026-07-11.md) |
| obs #52:cancel 占席僵尸→排水积压归档队列,古董 brief 真派发 | 未定位(cancel 副作用面历史设计;cancel 发起方也未定案) | **未修**(并入 recovery-reinsert-vs-cancel-race spec) |
| obs #53/#54:o1 派单三度不落 pane(席位级粘滞疑) | 未定位(ah#17 家族,根因待正式取证) | **未修** |
| obs #47/#48/#50:A/B 两臂混写主树 / stash 扫除活栈配置 / 冻结 brief 引用文档 worktree 不可见 | n/a(operator 部署/操作错误,非产品 PR 引入) | n/a(SOP 已固化;工作区物理闸=pre-commit 本地措施,产品化未做) |
| agy 语义假完成/假 BUSY 占道族(obs #29/#31/#37,Gen-1~4 持续) | 未定位(完成信号=停下推断,结构病) | **部分尝试**=#122/#123(疗效存疑,见验证债 4);根治归完成协议设计轮 |

---

## 四、初版统计(2026-07-12)

- 总 PR:**45**(#99–#145,#139/#140 为 issue)
- **实证闭环:9**(#142 #141 #134 #131 #128 #127 #126 #115 #102)——占 20%
- **CI 绿仅代码闭环:13**
- **验证债:16**
- **未观测:7**

**一句话现状**:有活栈实证的 PR 只有五分之一,且集中在"疗效账本盯着的换血载荷"上;凡是没进 gen-efficacy 开窗预期表的 PR,基本自动滑进验证债/未观测。结构对策=本台账规则 1(merge 当场登记实证计划)把每个 PR 强制挂上观察节点。

## PR #149 (fix gateway ah_bin resolve, base=main, merged b1d67a8) — 2026-07-12
- **预期效果**:修 current_exe=ahd bug,网关 bridge 用正确兄弟 `ah`,claude worker 激活成功。
- **实证(换血)**:resolver 层**已修好**(spawn 用对了 `ah`);但换血暴露**第二层激活缺陷**——所有走网关的 claude(master/d1/r1)仍 <200ms 秒死。
- **状态**:**部分闭环·激活仍失败**。current_exe 是第一层,网关路径还有第二层。**Module D 端到端从未激活过 claude**(#146/#147/#149 全缺 A2 tier-3)。
- **动作**:回滚 preModD 止血;发版 hold;第二层缺陷待隔离复现定位后走 SOP 派修。

## PR #151 (feat: shared secure storage credentials dir, ah#18, base=main, merged 3a7bb5a) — 2026-07-13
- **预期效果**:每 claude 席位注入 `CLAUDE_SECURESTORAGE_CONFIG_DIR=<共享凭据目录>`(只让 `.credentials.json` 指向共享真文件),`CLAUDE_CONFIG_DIR` 保持每沙箱隔离;**替代**并作废整条 Module D 网关方案(#146/#147/#149,gateway/neuter/symlink 全撤)。目标=用户单次登录,多 worker + 宿主 claude 共骑同一份凭据,任一 refresh 原地写回真文件、不互相登出(治 ah#18 根)。
- **代码闭环**:✅ 合入 main,CI `test` 真绿(3m25s pass,非硬合),r1 ACCEPT 后 auto-merge。**config 为 fail-closed**(claude 缺 `shared_credentials_dir` 即 panic,比静默回退 HOME/.claude 安全)。
- **实证计划(挂载节点)**:tier-3 真机(用户 Windows/WSL2)三条——① 真 claude worker 骑用户单次登录起到 IDLE;② 刷新时新 RT 原地写进 Windows 真文件;③ 第二 worker / 宿主 claude 不被登出。**runbook 由 master 产出中;tier-3 执行=operator 在用户机上跑。**
- **状态**:**代码闭环·Linux 激活已实证·Windows tier-3 债未还**(O7:merge≠完成,②③ 未跑)。发版仍 hold 到三条全验过。
- **过程疗效副产**:该 PR `test` job 连红 5 轮同根因(签名+fail-closed 波及全 claude fixture),operator 第 5 轮才归因补编译门([[feedback_verify_full_cargo_test_not_just_lib]] push 前硬门 + O8 纪律);**加门后 c2 第 6 轮一次穷举收敛**(5 轮→1 轮),编译门有效性=已实证。
- **换血#6 激活实证(2026-07-13,session sess_0f31fff6,unit ah-2ee4e0dfc3b5034c)**:停旧栈 → swap `~/.local/bin/{ah,ahd}` 到含 #151 的二进制 → ah.toml 加 `[providers.claude] shared_credentials_dir="/home/sevenx/.claude"` → ah start。结果:
  - **tier-3 条件① Linux 侧 PASS**——7 worker 全 IDLE(含 claude d1/r1,pid 3904298/3904623)+ master(Sonnet 5)IDLE 到 composer,**无 ModD 式秒死**(网关方案在此步从未通过)。
  - **机制真接上**:d1 env 实测 `CLAUDE_SECURESTORAGE_CONFIG_DIR=/home/sevenx/.claude`(共享凭据)+ `CLAUDE_CONFIG_DIR=<沙箱>/.claude`(配置隔离)= 设计原意。
  - **无附带损伤**:operator `~/.claude/.credentials.json` 仍 509B、mtime 未变(未被 clobber/未 stub/未登出);回滚备份留 `~/.cache/ah/operator-swap-backup/`。
  - **仍欠**:② 刷新原地写回、③ 多席位不互登出——需真机时间/Windows 侧,挂用户 Windows dogfood。**对比 #146**:这是凭据方案首次在 Linux 真激活 claude,反向坐实网关方案(#146/#147/#149)"CI 全绿但从没激活"的结构性差异。

## MD1/MD2 模块化解耦 Wave-1(#152/#153/#155/#156/#158,base=main,2026-07-13,session sess_0f31fff6)
- **背景**:用户指令"模块化"。先建架构索引(MD1),再按索引选 god-file 做行为保持解耦(MD2)。资源分配:实施走 codex(充裕侧),claude 只用在 d1 设计判断 + r1 审核(O9)。
- **#152 MD1 架构索引**(merge 93bbb2d):6 层模块图 + capability→owner 表 + ah-CLI/ahd-daemon 进程轴。**疗效**:索引成为 MD2/MD3 的必读地基。**过程副产**:g1(codex)编造 claude_gateway 路径+符号,r1 grep 兜住 + operator 独立 grep 又抓到 5 处陈旧行数 → 固化"扇出审计合成层必须机械复核"铁律(master.md:67)+ 记忆 [[feedback_fanout_audit_synthesis_must_verify]] + obs #56。
- **#153 target1 pilot**(passivate agent_io,merge 0faf8e3):验证整条流水线 worktree→PR→本地全量 cargo test 收口→并行 CI 绿→r1→merge 能跑通。**疗效**:流水线 proven,后续 target 才敢并行。
- **#155 target2**(split master_cutover RPC handlers,merge 7ce2bb5):sessions.rs 域拆分。
- **#156 target3 PR-A**(master_watch saga 抽取,merge 49b2083):决策归位 + 具名 saga,**恒等变换**(纯结构)。
- **#158 target3 PR-B**(reap 链上提 master_reaper,merge ea8e296):MD2 最高风险的跨模块搬移。**operator 亲审 gate**:①完备性表(每失败退出点×是否 reap)我机械复核路由全对;②并行 CI 绿(clean container,ci.yml cargo test --all-targets 无 --test-threads=1)我直接查过 CLEAN;③r1 独立扫描 ACCEPT。**r1 揪出唯一非恒等路径**::671 finalize-stale 从裸 kill_pane 升级为完整 reap(sigkill+撤探针+kill_pane_if_owned,fence=false)——correctness-safe 且更 fail-closed(治"failed-revive 孤儿壳"+"stale pane 误杀活 master"两类事故),但**零测试覆盖**。operator 判决:最高风险路径不接受"合了记债",**打回补钉住测试**(TDD)→ c1 补 revive_finalize_stale_after_spawn_reaps_spawned_orphan_master(断言 Sigkill+WatchRemoved+PaneKill)→ operator 独立复核测试真驱动该路径 → 条件授权 → r1 复核 OK → 合入。
- **代码闭环**:✅ 五 PR 全绿合入 origin/main(HEAD ea8e296)。
- **实证债(未观测,不算治愈)**:①解耦的**可维护性收益**(god-file 变小是否真降低后续改动成本)=需 Wave-2+ 实际改动时对照;②**:671 硬化的真实疗效**=需下一次真实 master-revival finalize-stale 事件**不泄漏孤儿 gen-2 master**才算实证闭环(新单测只证代码路径,不证生产行为);③各拆分模块行为零回归=CI 并行绿是当前最强证据,活栈 dogfood 观察待 Wave 收口换血。
- **状态**:**Wave-1 代码闭环 + 流程门 proven;可维护性与 :671 硬化疗效挂观察**。

## v1.7.0 发版(PR #159 + 公开仓同步,2026-07-13,operator 执行)
- **触发**:用户明确指令发版(原话已授权,operator 之前误当待确认反复停顿=纠偏)。
- **Phase 1 ccbd-rust**:release/v1.7.0 分支 bump 1.6.0→1.7.0 + CHANGELOG → PR #159 auto-merge(CI test 绿)→ tag v1.7.0(merge commit 87bd01d)→ release.yml 成功出 installer。Release live:github.com/SevenX77/ccbd-rust/releases/tag/v1.7.0。
- **Phase 2 公开仓 SevenX77/ah**:无现成同步脚本,operator 用 v1.6.0 共同版本逆向精确 curation manifest。**只同步产品代码**(src/tests/assets/examples/Cargo/CHANGELOG/obs-log),**保留公开侧自维护**(README=写着"private dev repo"绝不覆盖、docs/ROADMAP-docs.md=公开独有、.github/scripts=curated)。**leak-gate 拦下真泄漏**:dev `.ah/rules/master.md` 已变活栈作战手册(g1/o1/d1/c1/c2+obs#55+research/REQUIREMENT-LEDGER 内部路径)→ 踢出同步,保留公开侧干净模板。全树 grep 确认无 .kiro/research/凭据/session-url 漏出。SevenX77 身份、无 session trailer。commit cf1bd90 + tag v1.7.0 → 公开 release.yml 出 installer。Release live:github.com/SevenX77/ah/releases/tag/v1.7.0。
- **预期效果**:用户 Windows 可装 v1.7.0 → 跑凭据 tier-3 ②③(刷新原地写回、多席位不互登出)。
- **实证债**:tier-3 ②③ 仍待用户 Windows dogfood(发版是其唯一前置,现已解除)。发版本身实证=两仓 release + assets 齐全(已验)。
- **机制债(记流程)**:①公开仓 curation 无脚本,靠逆向 v1.6.0 manifest——应固化成 publish-release.sh(含 leak-gate),否则每次发版重手工逆向易出错;②dev `.ah/rules/master.md` 漂成活栈手册,与公开产品模板发散,每次发版都要手动踢——根治=dev 侧把活栈 topology 从 committed master.md 挪走(用 uncommitted 或独立文件)。
- **状态**:**发版闭环(两仓 v1.7.0 live);凭据 tier-3 ②③ 解除阻塞待用户实证**。

## 场景包 v0.7.0 发布(2026-07-13,operator)
- **dev**:PR #161 合入 origin/main(docs-only,CI 绿 auto-merge);source of truth `research/config-pack/pack/` 外科增补 7 条换血周期硬教训(O6/O7/O8/O9 + #157 worktree 前置 + 每代疗效判决报告 + 行为保持重构),不重写既有内容。用独立 worktree 提交,不碰活栈主树。
- **公开仓 SevenX77/ah-scenario-pack**:`publish-scenario-pack.sh v0.7.0` → 双向漂移核对(README/GUIDE/OPERATOR 三文件纯 dev 领先、零公开侧 hotfix 被覆盖)+ leak-gate 四闸过 + 手工补 CHANGELOG v0.7.0 段;SevenX77 身份/无 trailer;main 2bb93f8 + tag v0.7.0(该仓无 release workflow,tag 即发布物)。
- **预期效果**:外部集成方装 pack 即得本轮沉淀的派单/发布/资源/完成判定纪律。
- **实证债**:纪律类内容,疗效=下一个用 pack 起栈的项目是否少踩这几类坑(长周期观察,非即时)。
