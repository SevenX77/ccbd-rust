# Arm A(ah 泳道)观测记录 — 只读,零干预

- 射令注入 master:2026-07-11T04:52:01Z(计时以 master 派出泳道首单为准,以 job 记录时间戳校准)
- base commit:97104cd;工作区 /home/sevenx/coding/ccbd-rust-wt-ab-a;分支 ab/r1-outbox-lane
- brief:task-brief-arm-a.md(md5 40905510be0ee12b0d8f6ebf2033dd2d)
- 泳道:g1(claude 闸门,opus4.8 xhigh)+ g1-m1(antigravity 实施);派单前双 /clear(上下文卫生,拉平两臂起点)
- 诚实性备注:g1 是 R1 设计的执笔者,虽经 /clear 清上下文,模型对自己写过的设计仍可能有零星熟悉度;记录在案供终报评估
- operator 干预记录:(应恒为空;若有=DNF)

## 事件流(operator 只读观察追加)
- 2026-07-11T04:53:09Z job_b67b3a25 → g1 DISPATCHED(泳道首单,**计时起点**)
- 2026-07-11T05:55:00Z job_b67b3a25 g1 COMPLETED(疑似闸门开工单/规划步,非终点)
- master 收口核验(产物轨,不只信 job 状态):`wt-ab-a` 分支 `ab/r1-outbox-lane` 工作树干净,末 commit `f3267c6`(commit 时间戳 2026-07-11T05:54:09Z,-07:00 时区)。RED→GREEN 轨迹完整可查:`fba52ad`/`0c2c3e2`(R1-T1)、`9969827`/`8e6097a`(JC-1)、`dbb636a`/`df3b48b`(R1-T2~T4:cold-scan/reap/dead-letter/ordering)、`d6cf27d`(接线)、`f3267c6`(COMPLETION-REPORT)。`COMPLETION-REPORT.md` 已落盘(14391 字节)。**结论:这条 COMPLETED 是真终点,不是规划步误报**——所有 GREEN commit 时间戳均早于/等于末 commit,产物轨与声明一致。operator 干预记录:空。master 本次运行期间未触发任何 ghost-text/kill+up 类 SOP(未撞见需要处理的卡单)。
- 2026-07-11T06:05Z operator 结算判读:**方法保真度偏差**——g1 闸门全程单干(1h01m,04:53:09→05:54:09Z),g1-m1(agy)零参与,不符 Arm A 定义(claude+antigravity 泳道)。根因=operator 实验设计漏洞:冻结 brief 因"两臂字节级相同"约束不含泳道角色分工,而射令又禁止 master 加框架,泳道方法上下文被剥离;非泳道抗命、非中继失真。本次产物保留,记为参考臂 A′(= opus4.8-xhigh 单体 via ah,含 TDD 全轨迹、COMPLETION-REPORT、12G 真实编译)。是否重跑正规 Arm A 待用户裁决。

## Arm A run 2(正规泳道跑法)
- 2026-07-11T05:59:32Z 射令 v2 注入 master:worktree 重置(新分支 ab/r1-outbox-lane@97104cd,build cache 已清=冷启动对齐);修正=派单须显式声明泳道角色(g1-m1 agy 执笔,g1 闸门拆解/审/终裁不亲写);g1/g1-m1 双 /clear(g1 携 A′ 上下文必须清);污染备注:g1 一小时前亲手实现过同一任务(A′),/clear 后权重层熟悉度不可清零,记录在案
- 2026-07-11T06:00:16Z job_2a6d61a8 → g1 DISPATCHED(泳道首单,**run 2 计时起点**)。派单文本 = 泳道角色声明(g1-m1 执笔/g1 拆解审阅终裁,不亲写生产代码)+ 冻结 brief 全文一字不改附后。g1 派单前已确认 ctx 归零(/clear 生效),g1-m1 同步 /clear。
- 2026-07-11T06:00:39Z job_2a6d61a8 → g1 DISPATCHED(run 2 泳道首单,**计时起点**)
- 06:10Z job_2a6d61a8 g1 COMPLETED@~9.5min——但产物轨零 commit、g1 还挂着 Explore 子任务(68.6k tokens)在跑:典型 F2(turn-end)≠F3(任务完成)误报,正是本 spec 要治的病;泳道仍在作业中,等 master 按产物轨纪律自行判断(零干预)
- 2026-07-11T06:11Z master 核验:哨兵在 job 转 COMPLETED 时唤醒(job_2a6d61a8),但产物轨为空(base commit 无新提交,0 commit)——`ah ps` 显示 g1/g1-m1 皆 IDLE。capture-pane 核实:g1 并非真结束,而是主动 spawn 了一个背景 Explore 子代理("Map outbox/notify/daemon code surface"),自己 yield 等待其完成,子代理完成后 CLI 自动续接 g1 的 turn(pane 显示 "✽ Understanding contract & mapping code…" 正在继续思考),todo 列表第 1 项已勾、2-5 项待办。**这是 job 状态说谎的又一活例(非阻塞式,不需要 /clear 类 SOP)**:job 层面的 COMPLETED 对应的是 g1 某个中间 turn 边界,不是真任务终点;未做任何介入(不 /clear、不重派、不催促),按"状态作废、等真产出"处置。master 改用产物轨轮询(等 COMPLETION-REPORT.md 落盘或 8h 到点)作为下一次唤醒依据,不再依赖 job/pend 状态。
- 06:34:15Z job_020e6306 → g1-m1 DISPATCHED(闸门→agy 实施位派活,泳道角色分工本轮生效)
- 09:18:50Z 实施段取证:agy 活跃实现仅 3m58s(06:34:15→06:38:13),之后 cargo test --lib 悬挂在自写 test_ledger_on_conflict_and_applied ≥2h40m(进程实测),agy 无超时静候;泳道看门狗未触发;operator 早前'迭代慢'判读已在 progress-time-table.md 更正;零干预维持
- 2026-07-11T09:20Z master 修正:上一条"COMPLETION-REPORT.md 落盘=真终点"结论有误。产物轨核实 HEAD=`5ae42f0`(commits: `05fd0e4` RED g1-authored、`09f5120` GREEN、`5ae42f0` docs),但该 COMPLETION-REPORT.md 是 **g1-m1(实施位)自报**"ready for gatekeeper review (g1 acceptance audit)"——不是闸门终裁,只是实施者提交审阅的标志,不满足冻结 brief 完成定义第 5 条(闸门终裁后声明完成)。capture-pane 核实 g1 当时仍在真实审阅中:跑 g1-m1 新增单测时抓到一个真死锁(`cargo test --lib` 挂死,futex 互斥重入,g1-m1 自己新写的测试踩了 DB 锁重入),g1 诊断后 kill 掉了这棵挂死的子进程树,判定为"泳道内自愈自己代码猴子的失控子进程,非 operator/master 介入、未碰活栈",随即重新派回 g1-m1 修复。**这是本轮实验里第二次撞见 job/产物信号说谎(这次是实施者自报完成早于真终点,而非 job 状态早于 turn 真结束)**,与本 spec 要治的病同源。master 未做任何介入(既未 /clear 也未催促也未替 g1 做判断),按"零裁决纯中继"把这次 kill 判定完全留给 g1 自己的泳道权限。等待信号从"文件存在"改为"HEAD 稳定 + g1/g1-m1 皆 IDLE 持续 10 分钟"或 8h 到点。
- 09:22:36Z job_c34fa980 → g1-m1 QUEUED(泳道侧新单入队——闸门疑似开始处置卡死,方法内自愈迹象;g1-m1 仍 BUSY 挂死中)
- 09:27:51Z job_9b6d6811 → g1-m1 QUEUED(第二张排队单);用户质询点入档:agy 实施零 commit(5 文件一坨悬空),brief '红绿轨迹入 commit 史'符合度缺陷,记质量终评;对照 A′ 逐步 8 commit
- 09:29:11Z 反转:挂死 2h40m 的 cargo(2502365)于 ~09:19Z 消失,agy 随即复活,一坨提交 09f5120(feat 全量实现)+ 5ae42f0(COMPLETION-REPORT),transcript 09:28:09Z 自宣完成"13 tests passed"——13≠全量套件,brief 全量测试符合度存疑,记质量终评;停摆哨兵同时报 gate 的 nudge 单 c34fa980 排队超 5min(挂死期间闸门两张 nudge 至今未落,现成 stale 单,看泳道自己怎么消化)
- 09:30:24Z 泳道自愈闭环:闸门撤销两张 stale nudge 单(9b6d6811/c34fa980 双 CANCELLED),哨兵 RECOVERED busy=1——闸门大概率经产物轨(git HEAD 变化)发现实施位收工,进入审核段
- 09:40Z 双侧 transcript 取证(agy brain + g1 claude session,均有时间戳,只读):①agy 冻结窗 06:38:00→~09:19 transcript 零步——不是慢,是整机挂起在"等后台 cargo test --lib"上(06:37:33 起后台任务,06:38 最后一步"I will wait"),cli 日志仅剩 6 分钟级空转轮询;②解冻主因=g1 闸门 09:19 亲手 kill 挂死测试进程树(futex 死锁诊断链 09:17:52→09:19:57 全留痕),非 agy 自愈;agy 进程复活后 09:19:19/30 一坨双 commit;③09:23 起框架 completion-nudge 死循环:ah 每 3-4s 注入"The job is still open. Wait for the background command…",agy 逐条复读"all 13 tests passed. Commit 5ae42f0"数十遍(step 494-564+)——job 无法关闭(agy 无 hook/无 done 通道)=R2 病的活体,烧 token 型 livelock,后被 g1 撤单+收工指令终结;④g1 监护行为实录:06:34 布 Monitor→06:36 验实施位真开工→07:35 1h 例查(判"在推进")→08:36 2h 例查(判"实现基本完成但无 commit,存疑")→09:17 2.75h 升级取证→kill 解卡→复审并抓到第二次死锁(reaper)→撤 stale 单→亲验死锁根因。结论:闸门并非不闻不问,是按 1h 监视窗节奏巡检;缺陷=节奏粒度(1h 窗)对 4 分钟级实施太粗+Monitor 只订阅"状态变化/commit"而挂死恰恰不产生这两种信号,2h40m 损耗全落在这个盲区里。
- 09:41:42Z job_6894c02e → g1-m1 QUEUED:**闸门 REJECT #1,单点返工单**。g1 审核结论=核心 R1 实现验收全绿(journal-first/去重台账/冷扫重放/错误本/reap),但抓到 1 个真回归:agy 把 agent.notify RPC 快路径改成了去磁盘重读 outbox 文件,违反 design R1-Q1 step 3("process as today,成功后 reap"),导致既有单测 1 条 FAILED+丢 params reply(此前被死锁单测掩盖看不见)。返工单质量高:引设计条文、限定单文件单分支改法、给两条精确验收命令、明令"别再跑 --workspace 卡住自己"。另 g1 自述"死锁我已修"——闸门按泳道惯例做了审阅中微小修正(修 agy 自写死锁单测),属泳道内权限。审核段流程(独立验收测试→抓回归→定向返工)正是 A′ 单体没有的第 ⑤ 步,时间成本与质量收益记终评。
- 09:48Z 停摆哨兵 STALL(排队超 5min):返工单 job_6894c02e QUEUED 无法派发。根因链坐实(读库+pane 实证):①原实施单 job_020e6306 仍 DISPATCHED 且 cancel_requested=1 挂着——agy 无 ACK/关单通道,CANCEL_REQUESTED 永远收不了尾;②框架 completion-nudge 持续每隔数秒注入"The job is still open…",agy 持续复读,livelock 已 ≥25min 未停(09:47 pane 实拍);③g1-m1 因此假 BUSY(Deferred),新单派不进;④g1 此刻 IDLE 裸等。= dispatch-ACK 竞态 + agy 假 BUSY + cancel 不可达三病并发,全是本 spec(R2 显式完成声明/JC 关单契约)的活体标本。零干预维持,看泳道(g1 的停摆感知或 master)是否自愈;若至 8h 顶未解=该段 DNF 结算。

## Arm A run 2 终局
- 10:11:30Z **用户下令终止**("先停掉泳道的任务,不用再做了没有意义")。operator 执行:cancel 返工单 job_6894c02e(CANCELLED)、kill g1-m1(KILLED,livelock 随之断);原实施单 job_020e6306 在 kill 路径下收为 FAILED。停摆哨兵 RECOVERED(pend=0 busy=0)。master 已通告收队。
- **run 2 结算:DNF(用户在假BUSY停摆期终止)**。终态产物:分支 ab/r1-outbox-lane,HEAD=5ae42f0(05fd0e4 RED 契约 g1 亲笔 / 09f5120 agy 实现 / 5ae42f0 报告);g1 审核已出 REJECT #1(agent.notify 快路径回归)但返工未执行,该回归仍在 HEAD 里;agy 自写死锁测试已被 g1 修掉。全量 workspace 套件从未跑绿。
- 计时总账:06:00:39 起至 10:11:30 终止,历时 4h10m51s;其中有效工作 ≈42m(闸门契约 32m19s + 交接 1m17s + agy 活跃实施 3m58s + 闸门取证/审核散段),其余 ≈3.5h 全部损耗在挂死测试等待(2h41m)+ nudge livelock/假BUSY 停摆(~50m)。

## 参考臂 A′ 收尾(operator 令:验收合入 main)
- 2026-07-11 worktree 切至 `ab/r1-outbox-ref-aprime`(HEAD `f3267c6`),`ab/r1-outbox-lane`(实验证据)保留未动。
- g2 独立审阅(g1 因执笔禁自审)—— **ACCEPT**,重点核查项(RPC 快路径 reap 语义)未犯回归:`handle_agent_notify` 未改动,纯 params 就地处理,不读盘;reap-on-success 干脆未接线(比设计更保守,非退化)。回滚探针验证去重测试非空转。2 条非阻塞小修正记录在 `worktree/REVIEW-ARM-APRIME-BY-G2.md`。全量 workspace 串行测试作为最终门正在跑,通过后 push+PR。
- 2026-07-11 全量串行门(g2 亲自执行,`wt-ab-a`)：`CARGO_BUILD_JOBS=1 cargo test --workspace -- --test-threads=1` exit 0，**1501 passed / 0 failed / 35 ignored，跨 62 个测试二进制**，全部 `test result: ok`；grep 命中的 "FAILED/error" 字样均为假阳性（测试名含 `error::tests::...` 或预期内 WARN 日志）。工作树仅 `REVIEW-ARM-APRIME-BY-G2.md` 未跟踪，无脏改动。**结论落地方式说明**：g2 完成测试后的汇报文本卡在 pane 输入行未提交（ghost-text 症状，operator 巡检抓到），master 直接从 pane capture 里读取到完整结果文本（含逐条核验），已 `/clear` 解卡，未要求 g2 重跑——数据来源是 g2 亲自产出的真实终端输出，不是转述。
- 2026-07-11 分支 `ab/r1-outbox-ref-aprime` 已 push origin;**PR #142** 由 operator 开出并挂 auto-merge(merge commit),CI 跑中。master 环境无 gh 认证,push+回报交由 operator 开 PR/合并,已成为固定 SOP。**A′ 收口完成**（待 CI 绿）。
