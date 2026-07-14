# operator 规则 · 分层索引

operator 规则按层渐进披露:顶层判断常驻注意力,场景细则按需拉取,证据台账在审计/复盘时查。

## 分层结构

| 层 | 位置 | 内容 | 何时读 |
| --- | --- | --- | --- |
| **L0 顶层判断层** | 项目 `CLAUDE.md`「Operator」节 | 存在意义 + 职责 + 判断框架(四问)+ 保留权力 | **常驻**,session 起手就在 |
| **L1 通用细则** | `research/config-pack/pack/OPERATOR-HANDBOOK.md` | 各场景的通用操作细则(随产品发版,适用于任何用 ah 的项目) | 四问判定场景后 |
| **L1 项目细则** | 本目录 `playbook-*.md` | 本项目特有内容:实锤案例、项目约束、台账落点 | 与对应通用章配套读 |
| **L2 证据/台账** | `logs/`、`research/*-ledger.md`、`research/USER-GOALS-AND-PRINCIPLES.md` | 观察日志、疗效台账、需求总账、用户目标与原则 | 审计/裁决/复盘时 |

上位关系:L0 顶层原则 > L1 场景细则。任何 L1 细则与 L0「用最高效率完成用户的目标」冲突,L0 赢。

## L1 项目 playbook 与通用章对应

| 场景 | 项目 playbook | 通用章(OPERATOR-HANDBOOK) |
| --- | --- | --- |
| 业务 SOP 断点 | [`playbook-sop.md`](playbook-sop.md) | 「场景细则 · 业务 SOP 断点」 |
| 运行时/基础设施故障 | [`playbook-runtime.md`](playbook-runtime.md) | 「场景细则 · 运行时故障」 |
| 汇报 / 目标升级 | [`playbook-report-escalate.md`](playbook-report-escalate.md) | 「场景细则 · 汇报与升级」 |
| 常设审计 / 保留权力 | [`playbook-audit-powers.md`](playbook-audit-powers.md) | 「场景细则 · 审计与保留权力」 |

## O1–O9 归位对照

| O 项 | 归位 |
| --- | --- |
| O1 需求追溯 | 原则通用层已承载;项目落点(REQUIREMENT-LEDGER、kiro requirements.md、hook spec 案例)在 `playbook-audit-powers.md` |
| O2 PR 疗效 | 原则通用层已承载;台账路径与案例(45 PR 盘点、#146)在 `playbook-audit-powers.md` |
| O3 干预=修规则 | 原则通用层已承载;项目实锤(obs #55、obs #59 范畴边界)在 `playbook-sop.md` |
| O4 隔离优先 | 原则通用层已承载;项目案例(ah#18 OAuth symlink、共享 git 树)在 `playbook-runtime.md` |
| O5 串行 cargo | 项目特有约束(VPS OOM),在 `playbook-runtime.md` |
| O6 同步 main | 项目实锤(陈旧本地 main 选错 PR base)在 `playbook-audit-powers.md` |
| O7 完成定义+报告禁令 | 原则通用层已承载,用户裁决在 USER-GOALS B2;tier-3 硬门与案例在 `playbook-report-escalate.md` |
| O8 同根因≥2轮修机制 | 原则通用层已承载,用户裁决在 USER-GOALS C3;实锤(PR#151 账单)在 `playbook-sop.md` |
| O9 资源替换朝富余挪 | 用户裁决在 USER-GOALS B11/C4;项目实锤(PR#151 六轮复审)在 `playbook-audit-powers.md` |

## 已登记欠账(2026-07-14 审阅发现,待用户排期)

| 缺口 | 说明 |
| --- | --- |
| 故障诊断与恢复指令库(incident runbook) | 通用/项目层只有原则,缺"诊断 islanding/socket 冲突/daemon 僵死"的具体命令手册;新 operator 需自行摸索,有误杀风险 |
| tier-3 dogfood 验证的执行入口 | "端到端真验证"是完成硬门,但没有一处写明如何拉起 dogfood 栈、跑哪些命令来亲验 |
| 心跳/哨兵的工程化挂载指南 | 要求"靠机制不靠自律",但未写明新 session 里用什么具体手段挂载(wakeup/后台哨兵),新 operator 上岗时心跳默认缺失 |

## 修订约束(改这套规则时守)

- **分层不重复**:一句话放到任何用 ah 的项目里照样成立 = 通用,写进通用层(`research/config-pack/pack/`),项目层不复制;项目层只收案例、项目约束、台账落点。
- **第一性判据,不写死机械阈值**。唯一例外:用户批准的"最长每 10 分钟"汇报心跳。判据要能自解释,不靠魔法数字。
- **每条项目细则带实锤**(哪次事故、什么代价、对应哪条规则),否则会被当噪音删掉;实锤是规则的免疫力。
- **L0 保持精简**:新增细则默认进 L1;只有改变判断优先级的内容才动 L0。
