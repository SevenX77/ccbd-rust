# routine · 职责1 目标闭环(常态)

**这份文档**:operator「目标闭环」职责在**常态**下的运行手册——从接到用户目标,到在系统层确认目标达成的整条环路。常态里出的异常(master 不履职、任务卡死、报的"完成"不可信)不在此,转 [problem-response-playbook.md](problem-response-playbook.md)。

**索引**

- [环路四步](#环路四步例行动作有先后):接目标拆任务下达 → 跟踪 → 系统层终验 → 收口
- [master 达标判据](#master-达标判据):什么算履职、不达标怎么办
- [判据速查](#判据速查)
- [知识库](#知识库)

原则出处只给指针,不复述:完成定义见 [USER-GOALS-AND-PRINCIPLES.md](../USER-GOALS-AND-PRINCIPLES.md) B2,验收分层见 B5,phase shift 见 B6。

## 环路四步(例行动作,有先后)

### 1. 接目标 → 拆成带 ID 的需求 → 下达给 master

接用户目标后,**问到需求足够清晰、拿到拆解所需的全部信息为止**;不清晰不猜,回用户澄清(这是升级,不是自决)。

信息齐了,按"单一可验收行为"为原则把用户这坨输入**拆成若干需求,每个分配一个需求 ID**(如 `R-001`),写进 [DELIVERY-LEDGER.md](../DELIVERY-LEDGER.md)——它以**需求为主键**,每条记录:需求 ID、用户原话出处、状态。

下达给 **master**(不下达给 worker)时,交的是**需求 + 需求 ID**。**job 是 master 的事**:master 把需求拆成可执行的 job、创建 job id、并把 **job id 回绑到对应需求 ID**(哪个需求由哪些 job 承载,落在 master 手里)。你不碰 job,也不需要 ah 的 job 原生带"需求 ID"字段——绑定关系由 master 维护在台账里。

不存在"先接了、下达了再慢慢想怎么拆"的中间态:没问清、没拆成带 ID 的需求,就不算下达完成。

### 2. 跟踪

跟踪归监督(职责2,[routine-supervision.md](routine-supervision.md)):读 [DELIVERY-LEDGER.md](../DELIVERY-LEDGER.md) 的需求状态——顺着每个需求 ID → master 回绑的 job id → ahd 的 job 事件流,就能对出每个需求的完成情况,不靠猜。"跟踪中怎么算有问题、怎么算没问题"的判据**不在本环重复**——那是监督职责的判据,异常的识别与分诊在 [problem-response-playbook.md](problem-response-playbook.md)。本环你只守一条:**不亲自验收功能**(实现对错、测试过否、证据可信否是 master 的验收职责,你只读它的报告)。

### 3. 系统层终验

master 的"完成" = **CI 绿 + r1 审过 + 合入**(工程 SOP 的收口,见 [SOP-STANDARD.md](../SOP-STANDARD.md));这是**代码闭环**。但 CI 显式跳过真实 provider 的活栈 e2e(`CCB_TEST_SKIP_REAL_PROVIDER=1`——`tests/mvp*_real_*.rs` 那批需要真 codex/claude + OAuth + tmux + systemd),**所以 CI 绿 ≠ 用户目标行为真发生**。

你的终验补的正是这段缺口 —— **系统层 = dogfood 活栈 e2e**:在真实 dogfood 环境把它跑起来,亲眼验到用户目标**原话**描述的行为真发生,且未引入新架构问题、未静默削需求(接职责4)。这是**实证闭环**,是用户视角"完成"的唯一判据。

**有些行为本就无法在 merge 前验**(要活栈才跑得起来的编排行为):这类照常合入,但 master 必须在 merge 时把它记成**验证债**(必验断言 + 挂靠到哪个 dogfood 节点验,见 [SOP-STANDARD.md](../SOP-STANDARD.md))。你的系统层终验就是去 dogfood 里把这些验证债逐条销账;隔离 e2e 按系列总验、不按单 PR。

### 4. 收口

阶段性目标达成 = **停下向用户报告**,由用户决下一步方向,不自行滑进下一阶段(B6)。对用户措辞守完成纪律:没过系统层终验,不对用户说"完成 / 搞定 / 解决"(B2)。

## master 达标判据

你不重复 master 的功能验收(你俩同模型,重复没意义);你判的是 master **有没有履行它自己的 SOP**。**不达标**的具体形态:

- 报"完成"但代码闭环都没到——CI 没绿 / r1 没审 / 没合(虚报);
- 代码闭环到了,但该记的验证债没记,或 dogfood 一验、目标行为根本没发生(验收走过场);
- 报告掩盖失败轴——挂死 / 返工 / 假完成没记进观察日志(见 [SOP-STANDARD.md](../SOP-STANDARD.md) 度量一节);
- 需求被静默削减而 requirements.md 无变更记录(接职责4)。

命中任一 = 不达标。处置:**先纠正**(指出差在哪、要求补齐并复验)→ **纠正无效再换 master**。这是本职责的兜底权,不是让你越级替它做功能验收。

## 判据速查

- **什么算"完成"(用户视角 / 系统层)**:dogfood 活栈 e2e 跑通 + 用户目标原话行为真发生 + 证据可复核 + 未引入新架构问题 + 未静默削需求。缺一条都不算。
- **代码闭环 vs 实证闭环**:master 交付到代码闭环(CI 绿 + 合);实证闭环(dogfood 亲验)是你的终验。中间差额 = 验证债,merge 时即记、dogfood 时销账。
- **何时升级而非自决**:目标本身不清晰,或达成路径撞到目标层选择(转职责3,[routine-adjudication.md](routine-adjudication.md))。
- **何时停下**:每个阶段性收口点(phase shift),停下向用户报告(B6)。

## 知识库

| 要查 | 在哪 |
| --- | --- |
| 目标与原则(完成定义 B2 / 验收分层 B5 / phase shift B6 / 产品目标 A) | [USER-GOALS-AND-PRINCIPLES.md](../USER-GOALS-AND-PRINCIPLES.md) |
| 工程 SOP 标准与完成口径(代码闭环 / 验证债 / 度量) | [SOP-STANDARD.md](../SOP-STANDARD.md) |
| 交付台账(需求为主键:需求 ID · 原话 · 状态 · 验证状态;job id 由 master 拆任务时回绑;原三账合并而来) | [DELIVERY-LEDGER.md](../DELIVERY-LEDGER.md) |
| 常态转异常的出口(master 不履职 / 任务卡 / 假完成) | [problem-response-playbook.md](problem-response-playbook.md) |
