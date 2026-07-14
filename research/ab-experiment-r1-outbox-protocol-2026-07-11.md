# A/B 对照实验协议 — R1 outbox:ah 泳道 vs 单体 Claude Code(2026-07-11,operator 起草)

用户指令原文级:同一任务,设计全部完成(kiro spec 四件套)后,建两个 worktree 用两种方法实施:①ah 泳道(claude code+antigravity);②Claude Code 单实例独立完成全部步骤(opus 4.8,effort high)。观察最终完成情况:质量、时间、token 消耗。**禁止任何中间干预**。

这也是"配置包真实目标=保真 vs 规模"命题(便宜/异构编排 vs 单强 agent)的第一个受控实验。

## 一、前置条件

- `.kiro/specs/ah-completion-protocol/` 四件套定稿(requirements.md / design.md / tasks.md + 收敛记录),经 operator 亲验。
- 实验任务 = **R1 outbox 子系统**(用户拍板):hook 投递事务化(outbox journal→投递→ACK→commit 后 reap、事件 id 幂等账本、重启冷扫重放、错误本),以定稿 tasks.md 中 R1 条目为唯一 scope 依据。
- 冻结一份任务 brief(`research/ab-experiment-r1-outbox/task-brief-frozen.md`),两臂拿到的文本**字节级相同**,仅"工作目录/分支名"一段按臂替换。brief 冻结后不再改动。

## 二、两臂定义

| | Arm A(ah 泳道) | Arm B(单体) |
|---|---|---|
| 方法 | master 派单给一条泳道:claude 闸门(执笔审查/终裁)+ agy 实施位,泳道内生机制(TDD 红绿、gate 审、pend 哨兵、ah 自愈)全部照常——它们是方法本体 | 单个全新 Claude Code 实例,model=opus-4.8,effort=high,自己完成全部步骤:读 spec→TDD→实施→测试→自审→commit→自声明完成 |
| 工作区 | worktree `wt-ab-a`,branch `ab/r1-outbox-lane` | worktree `wt-ab-b`,branch `ab/r1-outbox-solo` |
| 起点 | 同一 base commit(定稿时的 main HEAD) | 同上 |
| 监督 | 零 operator 干预;master 只做方法内建的派单/收口 | 零监督,bypass permissions,跑到自声明完成或超时 |

## 三、控制变量与红线

1. **禁止中间干预(用户令)**:实验开始后 operator/用户对两臂零输入、零提示、零救援。臂内**原生**自愈(ah 的 redispatch/revive/SOP 机制)属于方法自身,允许;需要人手动救才活=记 DNF(带部分产物入评)。
2. **cargo 串行铁律**:两臂环境各自 `CARGO_BUILD_JOBS=1` + `--test-threads=1`(brief 内写死)。
3. **两臂串行执行**(A 先 B 后):单核 cargo+OOM 线的 VPS 上并行会互抢资源,同时污染两臂的时间测量;串行下各臂独占资源,wall-clock 可比。
4. 超时硬顶:每臂 **8 小时**(从 brief 注入起算),超时记 DNF。
5. 实验期间 operator 只做只读观测(pane capture/git log/日志采集),全部观测落 `research/ab-experiment-r1-outbox/observations-arm-{a,b}.md`。

## 四、度量口径

- **时间**:brief 注入时间戳 → 最终 commit + 完成声明时间戳(以 git commit 时间、job 记录、transcript 时间戳互证)。
- **token**:
  - Arm B:该实例 transcript JSONL usage 字段全量累加(input/output/cache 分列)。
  - Arm A:闸门 claude transcript usage 累加 + agy 侧以 brain/<conv>/.system_generated/logs 可得计量为准;master 派单/收口开销单列。
  - 两臂口径有天然差异(异构 provider),终报报**原始 token 分列 + 估算成本**,不做单一数字硬比。
- **质量**(两臂完成后统一事后评,评前不看过程):
  1. 全量 `cargo test` 串行绿否(同机同基准);
  2. spec 符合度:按定稿 design/tasks 的 R1 验收条目逐条 PASS/FAIL;
  3. 北极星 §五 第一条实测:任一时刻 `kill -9` ahd 再拉起,事件流无洞(outbox 重放可证);
  4. 独立评审:未参与任一臂的 fresh 评审实例 + operator 亲验,缺陷计数分级(blocker/major/minor);
  5. 测试质量(红绿轨迹、断言实质性)与代码体量作为辅助指标。

## 五、产出

`research/ab-experiment-r1-outbox/`:task-brief-frozen.md、observations-arm-a.md、observations-arm-b.md、终报 REPORT.md(质量/时间/token 三维对照表 + 判决 + 对"保真 vs 规模"命题的证据陈述)。

## 六、时序

1. 设计四件套定稿 + operator 亲验(在途)。
2. 冻结 task brief;建两 worktree;记录 base commit。
3. Arm A 跑到收口(或 DNF)→ 只读采数。
4. Arm B 跑到收口(或 DNF)→ 只读采数。
5. 统一质量评审 → REPORT.md → 推送用户。
