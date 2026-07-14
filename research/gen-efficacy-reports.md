# 版本换血疗效报告(operator 维护 · 每代一份阶段性报告)

**用途**(用户 2026-07-11 要求):每次版本更新换血之后,观察该版本所修问题的**真实表现**,以阶段性报告形式记录。与疗效账本(`research/dogfood-ledger-*.md`,病例流水/发生率计数)分工:账本记原始病例,**本文记判决**——每项修复"预期 → 实际 → verdict"。

**纪律**:
- 换血开窗当日:列出该代**每项修复的预期表现 + 必验断言**(验证债从 outstanding/§G 继承,不许漏项)。
- 关窗(下次换血前)必须出**判决报告**;窗口超 3 天先出中期报告。
- verdict 词汇表:**治愈-实证**(断言过,有证据)/ **改善**(发生率降,量化)/ **无效**(病仍在)/ **恶化** / **未观测**(窗口内没遇到触发场景——如实写,不许算"治愈")。
- 每份报告要**推送用户**(阶段性汇报),不是只落盘。
- 证据规格:pane capture / DB 行 / 观察日志条目号,不凭印象。

---

## Gen-1/2/3 追溯摘要(判决散落在疗效账本各关窗段,此处收拢结论)

| 代 | 载荷 | 判决要点 |
|---|---|---|
| Gen-1(换血#1) | #126 scanner 修 | G1 误闩 6/6→0 **治愈-实证**;但**半换血事故**(只换 ah 没换 ahd)使 #127/#128 归因作废——教训:先验载荷真在跑的二进制里,再谈疗效 |
| Gen-2(换血#2) | #126-#130 全量 | G2 垃圾reply假完成 **治愈**;P0-2 cancel 正样本 1 例 **实证**;暴露 ro_binds 秒死新 bug(开窗即修);agy 语义假完成 3/3 **无效**(预期内,无对应修复) |
| Gen-3(换血#3) | #131-#134 = v1.5.0 | ro_binds 修 **治愈**;agy 假完成/占道 2 例 **无效**(对照组,预期内);幽灵文本第三路径(dispatch 就绪复查)**新发现**;#13 catch-22 活体现形 |

## Gen-4 报告(换血#4,2026-07-11 开窗;载荷 = #136/#137/#138/#141)

### 开窗预期表(每项修复的必验断言)

| # | 修复 | 预期真实表现 | 必验断言/观测口径 | 当前 verdict |
|---|---|---|---|---|
| 1 | #141 respawn storm(公开仓 #13) | `ah up` 不再假 DRIFT 风暴;真配置改动仍触发 drift;多 agent 重生错峰 ≥500ms | 活栈 `ah up` 全员 NO_CHANGE + pane pid 零变化 | **治愈-实证**(开窗当日,见观察日志 #41;剩"真 drift 仍触发"待一次真配置改动机会验证) |
| 2 | #141 连带:单 agent 恢复 catch-22 消失 | kill 单 agent 后可安全 `ah up` 补回,不殃及全栈 | 下次单 agent 事故时实测 | 未观测 |
| 3 | #137 D1 job-state 闸门 | job 非法状态跳变被闸;卡 DISPATCHED 族/状态竞态写发生率下降 | 对照 Gen-2/3 的 DISPATCHED 僵尸单发生率;events 里非法迁移拒绝记录 | 未观测 |
| 4 | #136/#138 C1 感知写闸 | gate 外直写 agents.state 被 CI+运行时拦;幽灵/banner 文本族误闩发生率下降 | 幽灵文本族(G1 变种)再现时看写闸是否拦截并响亮告警 | 未观测 |
| 5 | `ah start` unit 化(随新血启动路径生效) | ahd 自死后 systemd 自动拉起(Restart=on-failure) | 下次 ahd 意外死亡时验自愈;或压测窗口人工 kill 验 | 未观测 |
| 6 | **对照组(预期无效)**:agy 语义假完成/假 BUSY | 无对应修复,预期继续发生 | 发生即记账本,不记本代过失 | 待录 |
| 7 | 验证债连带(§G):Fix C 真 CLI 场景 / A/B 看门狗真触发 / G2 检测器假 COMPLETED 对照归零 / 每角色模型配置 / REVIVE_IDLE 链路 | 见 handoff 域 6 | 各自断言见 outstanding-problems §G 31-36 | master=sonnet5 **实证**(statusline);其余未观测 |

### 病例流水(窗口内)
(记于疗效账本 Gen-4 段,判决时引用)

### 关窗判决(2026-07-11,换血#5 前;窗口仅 ~1 天,样本量小,如实降置信)

| # | 修复 | verdict | 证据 |
|---|---|---|---|
| 1 | #141 respawn storm | **治愈-实证** | 开窗当日活栈 `ah up` 全员 NO_CHANGE + pane pid 零变化(观察日志 #41);"真 drift 仍触发"一项由换血#5 自身验证(hook 命令 wire 格式变更=真配置改动,预期全员 drift 重生) |
| 2 | 单 agent 恢复 catch-22 | **未观测** | 窗口内唯一单 agent kill(g1-m1,A/B 终止)按用户指令不补回,无 `ah up` 补回场景 |
| 3 | #137 D1 job 状态闸 | **未观测**(无负样本) | 窗口内 cancel/kill 路径两例(020e6306 FAILED、6894c02e CANCELLED)走的都是合法迁移;无非法跳变被拒记录。注意:agy turn-end 假 COMPLETED 是"语义谎报"不是"非法迁移",D1 管不到,不算无效 |
| 4 | #136/#138 C1 感知写闸 | **未观测**(无触发) | 窗口内幽灵文本族再现 1 例(g2 "push it" ghost text 卡 25min),但那是 dispatch-ACK 竞态(ah#17 第三路径),不经写闸路径;写闸本体无拦截机会 |
| 5 | ah start unit 化自愈 | **未观测** | 窗口内 ahd 零意外死亡 |
| 6 | 对照组:agy 语义假完成 | **如预期继续发生** | A/B run 2 多例 turn-end 假 COMPLETED + 假 BUSY 冻结 2h41m(观察日志 #42 族);**新发现:claude(g2)也出同族标本**(后台跑测试+提前收尾,job 翻 COMPLETED)——实锤"停下==完成"是结构病非 agy 特有,佐证连根撤 pane 推断的裁决 |
| 7 | 验证债 §G | master=sonnet5 **实证**;其余 **未观测** | statusline 亲验 |

**窗口新发现(非本代载荷,已转化为 Gen-5 载荷/待办)**:①agy Stop hook 注入链 0% 送达(obs #43)→ #143 已修,换血#5 生效验证;②dispatch-ACK 幽灵文本第三路径再现 → ah#17 仍待做;③仓库无 required checks,auto-merge=立即合 → 已配 main `test` 必过(SOP 基建)。

**结论**:本代载荷正主(#141)治愈实证;感知层 C1/D1 两块地基整窗无触发场景,疗效判定顺延至 Gen-5 窗口(不许算治愈)。

---

## Gen-5 报告(换血#5,2026-07-11 开窗;载荷 = #142 R1 outbox A′ + #143 agy hook 修复 + 拓扑改组 codex 闸门×2 + r1 claude 审核位)

### 开窗预期表

| # | 载荷 | 预期真实表现 | 必验断言/观测口径 | 当前 verdict |
|---|---|---|---|---|
| 1 | #143 agy hook 绝对路径 | agy Stop hook 真送达 daemon:`agent.notify` 出现 antigravity 组 | journalctl 见 agy 侧 `received agent.notify`;hooks-debug 日志出现**非 replay** 组织条目 | 开窗待验 |
| 2 | #143 timeout 5000→5 | agy hook 超时不再 83min 同步阻塞 | 沙箱 hooks.json 材料化值 =5 | **实证**(开窗当日:三个新 agy 沙箱全部=绝对路径 `/home/sevenx/.local/bin/ah` + timeout 5) |
| 3 | #142 R1 outbox 冷扫 | ahd 重启后 outbox 目录残留被 replay/reap/隔离 | 换血#5 重启即首个真触发:重启后 outbox 目录清空或 dead/ 归档 | **首触发实证**(开窗当日:旧 master 停机瞬间 userpromptsubmit hook journal-first 落盘——恰是 R1 要保的 daemon 停机窗口;冷扫 replay FK 失败→正确 retry_deferred=1,无热循环。**新观察项**:死 session 孤儿事件 FK 永败,永不成功也未进 error-book,冷扫每次重启都会重试一次——需要 max-attempts→error-book 或 dead/ 归档兜底) |
| 4 | #141 真 drift 触发(Gen-4 遗留断言) | hook wire 格式变更 → `ah up` 全员 drift 重生,且错峰 ≥500ms 无风暴 | 换血#5 `ah up` 输出 + respawn 时间戳间隔 | **未按预期路径观测**:daemon 重启触发 master-death 全栈连坐(SIGTERM→清 tmux→master 死→级联清 worker,设计语义),`ah start` 全新起栈,drift-respawn 路径未走到;断言顺延至下次"活栈不重启 daemon 只 `ah up`"的真配置改动 |
| 5 | 拓扑:codex 闸门×2 + r1 | codex 闸门按同纪律运转;r1 只审不写 | 首个 PR 周期观察 | 开窗待验 |
| 6 | 感知 C1/D1(Gen-4 顺延) | 同 Gen-4 #3/#4 | 同 Gen-4 口径 | 顺延观测 |

### 病例流水(窗口内)
(记于疗效账本,判决时引用)

### 关窗判决
(下次换血前填写并推送用户)
