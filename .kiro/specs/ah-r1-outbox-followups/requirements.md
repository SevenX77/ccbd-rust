# ah R1 Outbox Follow-ups — Requirements

Status: requirements drafted 2026-07-11 (operator), from PR #142 merge-time known gaps + Gen-5 换血#5 首个真实触发的现场观察(观察日志 #46)。Not yet scheduled — awaiting dispatch.

Source material:
- PR #142 `ab/r1-outbox-ref-aprime`(merge 18c9e54)COMPLETION-REPORT §5(自报延期项)
- `logs/operator-observation-log.md` #46(冷扫首触发 + 孤儿事件缺口,2026-07-11)
- g2 交叉审 PR #143 时的两条非阻塞代码观察

## 背景

R1 outbox(journal-first 完成信号传输)已合入 main(#142),换血#5 当天即迎来首个真实触发:旧 master 停机瞬间的 userpromptsubmit hook 事件被 journal-first 落盘,daemon 冷扫正确 replay。但同一现场暴露了设计缺口:该事件所属 session 已 CLOSED,replay 时 FK 约束永远失败,事件永远走 retry_deferred——**永不成功、永不放弃、每次 daemon 重启都重试一次**。

## Requirement R1: 孤儿事件终态归档(max-attempts → error-book)

outbox 中 replay 永败的事件必须有终态,不允许无限重试。

验收标准:
- 每个 outbox 事件带重试计数(持久化,跨 daemon 重启累计)。
- 达到 max-attempts(建议默认 3-5 次,可配)后,事件移入 error-book 或 `dead/` 归档目录,冷扫不再碰它。
- 归档动作留观察痕迹(log line + 事件流条目),操作者能发现"有事件被放弃了"而不是静默消失。
- 区分**可恢复失败**(daemon 忙、瞬时 IO)与**结构性失败**(FK 不存在、session 已 CLOSED):结构性失败可以直接短路进归档,不必耗满重试次数——design 阶段决定是否做这个区分,不做也要写明理由。

现场证据:死 session `sess_334718e9` 的孤儿 userpromptsubmit 事件,FK 失败 → retry_deferred=1,每次冷扫重演(2026-07-11 实录)。

## Requirement R2: reap-on-RPC-success 接线

journal 文件在 RPC 成功送达后应被 reap(删除/归档),该逻辑在 A′ 实现中已写但未接线(COMPLETION-REPORT §5 自报)。

验收标准:
- RPC 成功路径调用 reap;outbox 目录在正常运行时不无限增长。
- 单元测试:模拟"journal 落盘 → RPC 成功 → journal 被 reap";以及"RPC 失败 → journal 保留待冷扫"。
- 与 R1 的归档路径不冲突(reap 只处理成功件,归档只处理永败件)。

## Requirement R3: 两条非阻塞代码观察(g2 审 #143 时留下)

1. `is_ah_owned_hook_item`(src/provider/home_layout.rs:1222 附近):识别 ah 注入 hook 用的是旧字符串匹配器,#143 改绝对路径后匹配器与新 wire 格式可能漂移——统一为单一 source of truth(如共享常量/构造函数),避免"写入格式改了、识别格式没跟上"的同族 bug。
2. `src/bin/ah.rs:608` 附近 debug log 硬编码裸 `ah` argv,与 #143 后实际注入的绝对路径不符——日志应打印真实值,不误导取证。

验收标准:两处修正 + 各一条回归测试(或在既有测试中断言)。

## 边界

- 不扩 scope 到 outbox 传输协议重设计;R1/R2 是收尾接线,不是重构。
- cargo 纪律:按模块批量,与其他小任务攒一次串行全量(CARGO_BUILD_JOBS=1)。
