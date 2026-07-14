# playbook · 汇报 / 目标升级(项目层)

进入条件:该对用户说话了——定期汇报、报告一个结果、或把决定升给用户。
通用细则见 `research/config-pack/pack/OPERATOR-HANDBOOK.md`「场景细则 · 汇报与升级」章。

## 裁决依据与升级分界

- 裁决与升级的分界判据在通用章。本项目的裁决依据文件 = `research/USER-GOALS-AND-PRINCIPLES.md`:能从该文档条目推导出答案的由你裁决;推导不出的目标层选择(产品方向、资源边界、停/换栈、发版)升用户。
- 汇报心跳判据在通用章。本项目的挂载手段:harness 的 wakeup 定时 + 后台 pend/watch 哨兵,任一到点或异常都物理唤醒你。
- 向用户展示文档时,将全文直接输出在回复中,禁止仅给路径或链接(`research/USER-GOALS-AND-PRINCIPLES.md` B9)。
- 报告措辞:三段结构(现状/根因/下一步),第一句直接回答用户的问题(同上文档 B8)。

## 「完成」的项目级硬门:tier-3

本项目一个任务算"完成",当且仅当过了 tier-3 端到端真验证:真二进制 spawn 真 worker(dogfood 环境),亲眼验到用户需求原话描述的行为真发生,且未引入新破坏。

下列全部 ≠ 完成,禁止单凭它们对用户说"完成/done/搞定/解决":

- 代码写完了
- PR 开了
- CI 绿了(CI 可能不覆盖真实端到端路径)
- merge 进 main 了
- worker 自报 "done" / pane 停下了(`feedback_completion_root_remove_stop_equals_done`)
- 设计冻结了 / 换血装上了

未过 tier-3,只用精确的半成品措辞并显式标明差哪一步:"已合入,但端到端未验证""代码闭环,实证债未还""Layer1 未合、tier-3 未跑"。宁可啰嗦报"还差 X",绝不含糊报"完成"。每次要写"完成"前自问:tier-3 亲验过了吗?没过就改口。

实锤(2026-07-12,用户纠偏"你总是跟我说完成了,但实际没完成"):凭据网关三个 PR 全 merge、CI 全绿,却从未激活过一个 claude worker(PR#146,Module D);merge/CI 绿被反复当"完成"上报,用户以为全部完成,实际全是半成品。教训:这是报告层的结构病,与 PR 疗效台账分账——台账(O2,见 `playbook-audit-powers.md`)管账本状态,本条管你对用户说的每一句话。
