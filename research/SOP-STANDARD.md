# 工程 SOP 标准

**这份文档**:定义本项目一个改动「从派单到合入、算不算完成」的工程环路标准。它是**系统级规则**——只讲工程 SOP 本身(完成定义、生命周期、护栏、追溯、监控、测试政策),与具体项目拓扑(哪些席位、哪个 provider 坐哪个角色)无关。给 master 与 workers 用:master 是本 SOP 的全生命周期 owner,worker 在本 SOP 下执行被指派的单条任务。项目拓扑、席位分配、provider 选型不在此文档,在 [master.md](../.ah/rules/master.md);本文档只规定"任何拓扑下都成立"的工程标准。原则层裁决依据见 [USER-GOALS-AND-PRINCIPLES.md](USER-GOALS-AND-PRINCIPLES.md),需求逐条追溯见 [REQUIREMENT-LEDGER.md](REQUIREMENT-LEDGER.md)。

**索引**

- **完成的两级口径** — 代码闭环与实证闭环的精确定义,以及二者差额如何记成验证债。
- **全生命周期** — 一个改动从设计到合入的环路,以及 master 对 CI 状态的感知责任。
- **PR 生命周期与护栏** — worker/master 的分工、合入护栏、交接显式化、auto-merge 门。
- **需求追溯** — 每 spec 的需求基线要求与需求变更记录纪律。
- **执笔权** — 哪些产物归严谨 agent 执笔、谁写测试、谁审。
- **监控:锚定产物轨** — 为什么 job 状态不可信、真相锚在哪、每单的兜底机制。
- **cargo / 测试政策** — 迭代期与收口点的测试口径、串行约束、与 CI 对齐。
- **派单纪律** — 共享 git 树下的 worktree 调配、base 选取、扇出复核。
- **本地 main 同步** — merge 后同步本地 main 的触发点与无损姿势。
- **度量** — 代码质量终裁与三轴并列必记的失败数据。

## 完成的两级口径

"完成"在本 SOP 下有两级精确口径,不可混用:

**代码闭环** = **CI 绿 + 审核位 ACCEPT + 合入 origin/main**。这是 master SOP 工程环路的收口点。push 分支、开 PR、agent 停下都不是代码闭环——只有改动进了 origin/main 且经过审核,才算 master 侧交付完成(裁决依据见 [USER-GOALS-AND-PRINCIPLES.md](USER-GOALS-AND-PRINCIPLES.md) B2)。

**实证闭环** = **dogfood 活栈端到端亲验**,即目标描述的行为在真实运行环境里端到端发生、且留有可复核证据。这是用户视角的"完成",归 operator 做系统层终验;master 不代替 operator 宣布实证闭环。

**为什么两者不等价**:CI 用 `CCB_TEST_SKIP_REAL_PROVIDER=1` 显式跳过需要真实 provider 二进制、OAuth 凭据、tmux、systemd 的活栈探针测试(`tests/mvp*_real_*.rs` 一类)。因此 **CI 绿只证明"代码与非活栈测试自洽",不证明"行为在真实栈里真发生"**。代码闭环 ≠ 实证闭环。

**验证债**:代码闭环与实证闭环之间的差额,即"已合入但尚未经活栈 e2e 论证"的部分。每个未经活栈论证就代码闭环的改动,**在 merge 当时即记验证债**,内容为两项:①**必验断言**——要在真实栈里观察到的具体行为;②**挂靠节点**——这条断言挂在哪个 dogfood 运行节点上验。merge 时不写验证债 = 违规,等同把"没验的当验了"。

## 全生命周期

一个改动的完整环路(角色名见 [master.md](../.ah/rules/master.md),此处只讲环节):

设计管线(课题→发散/红队→执笔收敛→辩论到冻结) → 实施派单 → 测试 → 审核 → 开 PR → 盯 CI → 修 → CI 绿 + ACCEPT → auto-merge 合入 → 汇报。

**master 对 CI 状态负全程感知责任**:从 push 触发 CI 起到终态,master 必须始终知道每个在途 PR 的 CI 是红是绿,不依赖 operator 转达。**CI 红 = 有 bug = SOP 的内环**,不是外部事件:第一动作是派实施者修,修完复推,直到绿。开完 PR 就当完成、把 CI 红当"回头再看"都是违规。

## PR 生命周期与护栏

**分工**:worker 只在自己的 worktree 里 commit,不碰 gh、不 push。push 触发 CI、开 PR、盯 CI、挂 auto-merge 全归 master。分支收口后由 master 亲自 push 指定功能分支触发 CI,再开 PR,再对每个 PR 挂 CI 哨兵盯到终态。

**合入护栏(铁律)**:

- 只 push 实施任务指定的功能分支,push 前确认分支只含本任务新 commit。
- **ff-only,永不 `--force`**。
- 永不 push / merge 到 main 分支本身;合入只经 PR 的 auto-merge 完成。

**交接显式化(铁律)**:每个 PR 开出时必须显式记录三元组——**谁开的 / 谁盯 CI / 谁验收**。任何一元悬空即违规。**悬空**的精确含义:某环节没有被明确指派给某个具体席位,于是 fall-through 到"没人管"——没人盯 CI 的 PR 会烂在红里,没人验收的 PR 会带缺陷合入。三元组齐全是开 PR 的前置条件,不是事后补记。

**auto-merge 门(铁律)**:auto-merge 只在审核位明确 ACCEPT **之后**才允许挂。合入的与门 = **CI 绿 AND 审核 ACCEPT**,缺一不合。严禁开 PR 时预挂 auto-merge——预挂等于"CI 绿即合",会和审核终审赛跑,把带缺陷代码抢先合入。若发现某 PR 审核未完却已预挂,第一动作是先撤销 auto-merge,再等审核。

## 需求追溯

**每个 spec 必须先有 requirements.md 需求基线**,凡源自用户指令的需求逐条带用户原话出处;没有需求基线的 design 不许开工。需求文档规范见 [USER-GOALS-AND-PRINCIPLES.md](USER-GOALS-AND-PRINCIPLES.md) C1(kiro EARS 三件套,拿不准以官方公开文档为准)。

**削减 / 推迟任何基线需求 = 在 requirements.md 落一条"需求变更记录"**:削了什么、依据什么原则、谁定的、推迟到哪个登记点。没有变更记录的削减即视为需求静默丢失,违规。

**"后续 / 可选 / 首期不做"是登记点不是终点**:每个 defer 项必须进 spec 的 tasks.md 或 backlog,且带明确 owner。没有登记簿的"后续"等于蒸发,禁止。

## 执笔权

严谨性要求高的产物——tasks.md、TDD 框线、spec 硬流程、闸门 / 验收测试代码——归严谨 agent(codex 为主,claude fallback)执笔(裁决依据见 [USER-GOALS-AND-PRINCIPLES.md](USER-GOALS-AND-PRINCIPLES.md) C6)。发散型 agent(antigravity)只有辩论席与实施席,**不执笔任何闸门产物、不做闸门**。

**测试真实性防线**:

- 泳道内验收 / 闸门测试代码由 gatekeeper 写——gatekeeper 先写 RED 测试并 commit,实施者纯实施变 GREEN,**实施者不得增删改测试文件**。让实施者自己写验收测试 = 实施者自证,违规。
- 单独实施者是全链路自写自测(RED + 实现 + 自验),这是自证风险最高的一环,交付后必派审核位审、**绝不同实例自审**。
- 实施者细粒度内部单元测试可自写,但不计入验收证据。

## 监控:锚定产物轨

监控锚在**产物轨**,不锚 job 状态。

**为什么不信 job 状态**:控制面 job 状态双向撒谎——既有 turn-end 的假 COMPLETED(agent 其实还没真产出),也有写完产物却永不收口的假 BUSY / Deferred(产物已完整落盘、job 仍卡着不翻)。**唯一地面真相 = 工作区 git HEAD 变更 + 约定落盘文件 + pane 实际内容**;job 状态无论显示什么都只当提示,不当证据。

**处置纪律**:

- **假 COMPLETED 不重派**:状态作废即可,agent 上下文完好还在干活,重派会打断真实进度;等真产出,并把该例记入失败数据。
- **每单挂 pend 哨兵兜底(机制,每单强制)**:派单拿到 job_id 后立刻用后台任务挂一个带超时的 pend——后台任务退出会物理唤醒 master:正常退出 = job 收口(仍去产物轨亲验),超时退出 = 停摆警报(先看 pane 真相再分诊)。哨兵是机械闭环,保证任何结局都会唤醒 master,裸等在机制上不再可能;没挂哨兵不结束回合。
- **禁裸 `ah ask --wait`**:无超时的 `--wait` 撞上"假 BUSY 永不收口"的 job 会把 master 自己钉死——前台一直挂着,连自己的升级闹钟都跑不了,模块彻底失去自恢复能力。派单一律走"`ah ask` 拿 job_id + 后台 `timeout … ah pend`"两步;`--wait` 唯一可接受形态是外包超时的 `timeout <预算> ah ask … --wait`。
- **挂死盲区主动闹钟**:挂死不产生任何信号,状态订阅监控不到。对每个在途实施单,除 pend 哨兵外再挂一个"无信号超预算"闹钟,到点亲查 pane + 进程真相,不纯等事件。
- **阻塞出口约定**:worker 有阻塞时落盘 worktree 根的约定文件(worker 落 `.operator-question`,泳道实施者落 `.lane-question` 给自己的 gatekeeper);上级对"下级在等"有监控盲区,落盘比在 pane 里干等可靠。

## cargo / 测试政策

**分阶段口径**:实施迭代期间只用增量 `cargo check` + 单测试定点复现(防 OOM、防逐 task 浪费);**模块 / PR 收口点(push 前)本地跑一次全量 `cargo test`**,一把拿到完整失败清单、一次改全、再 push。全量测试只在收口点跑一次,不逐 task 跑。

**串行约束**:本机 cargo 必须串行——`CARGO_BUILD_JOBS=1` + `--test-threads=1`,防并行 cargo 撑爆内存杀主控(见 [USER-GOALS-AND-PRINCIPLES.md](USER-GOALS-AND-PRINCIPLES.md) C11)。

**与 CI 对齐**:收口点全量测试必须带 `CCB_TEST_SKIP_REAL_PROVIDER=1`,与 CI 的 `test` job 口径一致——CI 显式跳过活栈 `real` 探针测试;本地不带这个 flag 会在与改动无关处看到资源竞争造成的偶发红,误当回归浪费排查。本地收口口径对齐 CI 实际门,不多测也不少测。

**收口硬门(铁律)**:改动公共函数签名 / 公共符号 / fail-closed 契约(如新增必填字段)时,收口点这一次必须是**真跑 `cargo test`,不是 `cargo check`**。判据:`cargo check` 只编译不运行,抓得到签名 / 编译类断裂,抓不到运行期失败——fail-closed 契约违背往往是运行期 panic,只有真跑测试才暴露;且裸 `cargo check` 默认不编译 test 目标。

**一次吃干,不许子集修(铁律)**:无论失败清单来自本地全量 test 还是 CI 红日志,都要把整份列表一次改完再推,严禁"只修看见的头几个就复推"。CI 日志本身就是完整清单,本地全量 test 相对 CI 的唯一优势是延迟低,不是"只有本地找得到"。

**同根因返工 ≥2 轮换工具**:同一类失败(调用点 / fixture 没跟上一类)连红第 2 轮就停下换工具 / 换归因(如从"再等一轮 CI"切到"本地全量 test 一次收敛"),不盲目再等第 N 轮(见 [USER-GOALS-AND-PRINCIPLES.md](USER-GOALS-AND-PRINCIPLES.md) C3)。

**串行绿 ≠ 并发安全**:本地串行全量绿只证明"发现了完整失败清单",不证明并发安全;并发安全只有 CI 并行跑能验。**CI 并行绿才是真验收门**,本地全量是"完整失败发现器"。

## 派单纪律

**共享 git 树**:master 与全部 worker 的 cwd 是同一份仓库,两个 agent 不能同时做分支 / commit。git-active 任务用 worktree 隔离派发,worktree 与分支按任务划分、由 master 统一调配(避免两个 worker 撞同一分支,见 [USER-GOALS-AND-PRINCIPLES.md](USER-GOALS-AND-PRINCIPLES.md) C12);纯 markdown 设计任务可并行。派单前把依赖的 brief / design 先 commit。

**选 base 前必 `git fetch`**:任何"某功能在哪个分支 / 选哪个 base"的判断前,先 `git fetch origin` 以 origin/main 为准,绝不信可能陈旧的本地 main——陈旧本地 main 会把 fix 的 base 选进落后于 main 的死分支。

**验收断言外部锚定**:验收断言(测试名 / 文件 / 行为断言)写死在 brief 里,防实施者自证完成;brief 自包含,不假设新会话有前情。

**上下文卫生**:派新任务前,agent 攒了多单未清就先重置会话;**只对 IDLE 席位 `/clear`**,绝不对 BUSY 席位投任何键;清后等 pane 出现全新 CLI banner 再派。投长文本进 pane 用 Write 落盘 + `load-buffer` / `paste-buffer`,绝不用命令替换会执行的内联方式。

**dispatch-ACK 验证**:派单后验证 job 真落库 + prompt 真落 pane——目标未回 IDLE 就派会造"派了但从未开始"的假 STUCK;确认目标已回 IDLE 再派。

**扇出必有合成层复核(铁律)**:多 agent 并行审计 / 编目类任务(枚举模块、扫符号、盘点路径),合成者在把子代理的枚举并进权威文档前,必须自己对子代理声称的"文件路径 / 符号 / 行数"做至少一轮机械复核(`ls` / `rg` / `wc -l` 直接核对,不是读子代理文字判断"看起来合理"),不许把未核的枚举直接送审核位。子代理会撒谎(编造或数字过时),权威产物必须合成者亲自物理核对一遍;审核位是终审兜底,不是唯一核实关口——与 filesystem-verify 同源。收口点对全表(不是抽查)机械对齐应作为合成动作的标准步骤。

## 本地 main 同步

每个 PR 合入 origin/main 后,立即把主树本地 main 同步到 origin/main(触发点 = merge 事件本身,不攒批),避免陈旧本地 main 在下一次选 base 时选错。共享工作树的无损姿势:worker 都在独立 worktree,同步前留恢复点,清理与 origin 逐字节一致的挡路未跟踪文件,再 `git merge --ff-only`。

## 度量

**代码质量 = 审核位头条终裁**(以审核位的头条判决为准,判据见对应角色规则)。

**完成度 / 可靠性 / 成本三轴并列必记**:每例挂死 / livelock / 假完成(带时间戳 + 损耗时长)、交接往返与 REJECT 轮数、有效时间 vs 损耗时间,都要记录。不记 = 对真实败因失明。**失败轴(挂死 / 返工 / 假完成)必进观察日志** [operator-observation-log.md](../logs/operator-observation-log.md),PR 疗效反向定位见 [pr-efficacy-ledger.md](pr-efficacy-ledger.md)。

## 知识库

| 要查什么 | 在哪 |
| --- | --- |
| 项目拓扑 / 席位 / provider 分配 / 设计管线具体走位 | [master.md](../.ah/rules/master.md) |
| 用户目标与工程原则(裁决依据) | [USER-GOALS-AND-PRINCIPLES.md](USER-GOALS-AND-PRINCIPLES.md) |
| 需求逐条追溯(原话 / spec / PR / 验证状态) | [REQUIREMENT-LEDGER.md](REQUIREMENT-LEDGER.md) |
| 模块完成台账(只认已 merge PR) | [MODULE-STATUS-LEDGER.md](MODULE-STATUS-LEDGER.md) |
| 能力→owner 注册表(设计必读) | [architecture-index.md](architecture-index.md) |
| PR 疗效台账 | [pr-efficacy-ledger.md](pr-efficacy-ledger.md) |
| 失败 / 异常观察日志 | [operator-observation-log.md](../logs/operator-observation-log.md) |
