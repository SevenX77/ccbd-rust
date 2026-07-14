# 射令:A/B 实验开跑(Gateway 根治;operator → master,2026-07-11)

你的场景层规则已更新:**先完整重读 `.ah/rules/master.md`**(新增:A/B 两臂拓扑、度量轴记录职责、cargo 政策、push 下放护栏),再执行下述序列。实验协议 = `research/ab-experiment-gateway-2026-07-11.md`;冻结 brief = `research/ab-experiment-gateway/task-brief-frozen.md`。

## 执行序列(顺序执行,每步验证再下一步)

1. `ah ps` 确认 g1 / g1-m1 / g2 全 IDLE。**g2-m1 本实验停用:全程不派、不清、不动。**
2. 上下文重置:对 g1、g1-m1、g2 逐个 `/clear`(只清 IDLE;tmux send-keys 姿势见你规则;等每个 pane 出现全新 CLI banner 再继续)。
3. **派单(两臂并行、同一 brief)**:把冻结 brief 文件**全文一字不改**作为任务文本,分别 `ah ask` 发给 **g1(Arm A)** 和 **g2(Arm B)**。不加角色框架、不加你的解读、不增删一字——角色由席位规则自带。长文本投递用你规则的 Write→load-buffer→paste 姿势,派后验证 job 落库 + prompt 真落 pane。
4. 每单**立刻**挂 pend 哨兵(你规则的机械姿势;首单预算 7200s)+ 无信号超预算闹钟。
5. 开 `research/ab-experiment-gateway/observations.md`:记两臂派单时间戳(job_id + UTC),照 r1-outbox 的观测格式持续追加(挂死/livelock/假完成每例带时间戳+损耗时长;交接与 REJECT 轮数)。
6. 观察模式:监控锚**产物轨**(git HEAD/落盘文件),job 状态只当提示;发现挂死/livelock/假完成/越权 **≤15min** 落 `.operator-question` + 记 observations;**不中途干预两臂实施方法**。
7. 臂内一轮收口(该臂报 commit 完毕待 CI)后:按你规则 **push 该臂分支**(仅 `ab/gateway-lane-codex-agy` / `ab/gateway-solo-codex`,ff-only,push 前核 `git log origin/<branch>..HEAD`),把 CI 结果回灌该臂。
8. 两臂各自 ACCEPT 后按协议走 r1 终审 → 头对头对比裁决;全程异常走 `.operator-question`。

确认收到后立即执行;每步完成追加进 observations.md。
