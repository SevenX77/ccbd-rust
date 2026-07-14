operator 修正令 #3(Arm A 泳道机制补充;归因=operator 规则缺口,不计 Arm A 账):

事实:g1 两次在只交 RED 契约后收口(最新 job_00b9dbe6,COMPLETION-REPORT 诚实标注"implementation not yet completed"),未派 g1-m1。根因核验:席位规则给了 g1"③派 g1-m1"的**职责**,但通篇未写**机制**(`ah ask` 命令);g1 沙箱环境已亲验可达真栈(AH_STATE_DIR+PATH 均通)。规则盘上已补(`.ah/rules/g1.md` ③的机制条),但按 spawn 物化规律,跑着的 g1 收不到——由你把机制送达。

执行:

1. **续单 g1(新 job,ah ask)**,任务文本如下(一字不改):

「续令:你上一单的 RED 契约(c041f96)已就绪,但你的收单 SOP ③-⑤ 未走完——生产实施须由你派 g1-m1 完成,整条流水收口才算完成。派单机制补充(此前规则漏写,现已补):在你终端直接跑 `timeout 30 ah ask g1-m1 '<实施 brief 全文>'` 拿 job_id,再 `timeout 7200 ah pend <job_id>` 等回复;计划审批、实施审查往返同此机制。这是你的职责授权,不是越权。现在继续:按你规则的派活三律给 g1-m1 写实施 brief(零歧义、拆小、plan-first 先审计划),驱动实施到满足原任务 AC-1~AC-6 与 §4 完成定义(COMPLETION-REPORT.md 收尾),再回本单收口。工作区与全部约束不变。」

2. 挂 pend 哨兵(预算 7200s)。注意:g1 这单期间会自己挂 `ah pend` 等 g1-m1——**g1 长时间无 pane 输出但其 pend 子进程存活 = 正常等待,不是挂死**;判停摆看 g1-m1 侧产物轨。
3. observations.md 记:泳道机制缺口事件(归因 operator 规则缺口;g1 两次半程收口均不计其账,但**交接往返轮数照常计入 Arm A 成本轴**)。
4. g2 续单(job_9beb0b75)已 COMPLETED——按既有流程亲验其复核圈产物(偏差清单+修正 commit+COMPLETION-REPORT 是否齐),结果记 observations;若 Arm B 已达收口条件,**按射令 v2 第 7 步独立走 push+CI**(ff-only 护栏),CI 结果回灌 g2;不等 Arm A(两臂各自独立收口是协议原状,人为同步会扭曲耗时轴)。
