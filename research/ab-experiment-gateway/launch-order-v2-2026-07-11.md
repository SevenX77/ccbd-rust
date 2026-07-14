# 射令 v2:A/B 实验开跑(Gateway 根治;operator → master,2026-07-11 整栈重启后)

你是全新 master(栈 sess_7163d948,4 席:g1/g1-m1/g2/r1,无 o1/g2-m1)。你的场景层规则(`.ah/rules/master.md`)已含 A/B 拓扑、push 下放护栏、cargo 政策——先读一遍。实验协议 = `research/ab-experiment-gateway-2026-07-11.md`;冻结 brief = `research/ab-experiment-gateway/task-brief-frozen.md`。前轮事故背景(必读,防重蹈):观察日志 #47-#49——两臂曾在主树混写、codex 曾无视 worktree 钉死 commit 本地 main(现已装 pre-commit 闸拦 main commit)。

## 执行序列(顺序执行,每步验证再下一步)

1. `ah ps` 确认 g1 / g1-m1 / g2 / r1 全 IDLE。
2. **派单(两臂并行、同一 brief)**:把冻结 brief 文件**全文一字不改**作为任务文本,分别 `ah ask` 发给 **g1(Arm A)** 和 **g2(Arm B)**。不加角色框架、不加解读、不增删一字。长文本用 Write→load-buffer→paste 姿势;派后验证 job 落库 + prompt 真落 pane。
3. 每单**立刻**挂 pend 哨兵(首单预算 7200s)+ 无信号超预算闹钟。
4. **首触点核查(新增,≤10min 硬闹钟)**:派单后 10 分钟内亲验两臂的**第一笔文件改动落在哪棵树**——Arm A 必须在 `/home/sevenx/coding/ccbd-rust-wt-gw-a`、Arm B 必须在 `/home/sevenx/coding/ccbd-rust-wt-gw-b`;主树(`/home/sevenx/coding/ccbd-rust`)出现 src/tests 改动 = 立刻 ESC 打断该臂 + 落 `.operator-question` 上报,**不等 15 分钟**。
5. 续记 `research/ab-experiment-gateway/observations.md`:两臂派单时间戳(job_id + UTC)、后续所有异常(时间戳+损耗时长)、交接与 REJECT 轮数。
6. 观察模式:锚产物轨(两臂 worktree 的 git HEAD/落盘),job 状态只当提示;挂死/livelock/假完成/越权 ≤15min 落 `.operator-question`;**不中途干预两臂实施方法**。
7. 臂内一轮收口(该臂报 commit 完毕待 CI)后:push 该臂分支(仅 `ab/gateway-lane-codex-agy` / `ab/gateway-solo-codex`,ff-only,push 前核 `git log origin/<branch>..HEAD`),CI 结果回灌该臂。
8. 两臂各自 ACCEPT 后按协议走 r1 终审 → 头对头对比裁决。

确认收到后立即执行;每步完成追加 observations.md。
