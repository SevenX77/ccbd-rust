# operator 自律原则 —— 已重构,本文件废弃(2026-07-13)

> **本文件的平铺 O1–O9 结构已被用户认定为病根**:细则与顶层原则同一视觉权重,导致执行时细则盖过原则(例:「每次干预=规则修订」压过「用户省心」,把 3 分钟重启膨胀成几小时救火 + 文档)。
> 已重构为**三层渐进式披露**,入口见 [`research/operator/README.md`](operator/README.md)。

## O1–O9 归位对照(找旧条目去哪了)

| 旧条目 | 新家 |
| --- | --- |
| O1 需求可追溯 | `operator/playbook-audit-powers.md` |
| O2 PR 疗效闭环 | `operator/playbook-audit-powers.md` |
| O3 干预=规则修订(**边界已澄清**:只管业务 SOP,不含运维) | `operator/playbook-sop.md` |
| O4 隔离优先 | `operator/playbook-runtime.md` |
| O5 串行 cargo,真验收看并行 | `operator/playbook-runtime.md` |
| O6 main 同步 / 判 base 前 fetch | **移交 master**(SOP 内环),已入 `.ah/rules/master.md` |
| O7 「完成」定义 + 报告禁令 | `operator/playbook-report-escalate.md` |
| O8 同根因≥2轮修机制 | `operator/playbook-sop.md` |
| O9 资源替换朝富余 provider | `operator/playbook-audit-powers.md` |

**新增(旧结构没有的)**:顶层唯一原则「用户省心」的冲突裁决权 + 四问闸门(WHAT/WHEN/WHY/HOW)+ 运行时故障 vs 业务 SOP 的范畴区分——这三样是本次重构的实质补丁,全在 L0(`CLAUDE.md`)与 `playbook-runtime.md`。
