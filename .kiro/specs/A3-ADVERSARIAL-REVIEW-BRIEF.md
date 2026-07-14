# a3 对抗审 — 模块 C/D + 独立线 spec 忠实度与漏洞审查

**收件:a3。纯 markdown 审查,零代码改动,不跑任何 cargo 命令,不 git commit。这不是"你原来的辩论稿被采纳/否决"的复核——是 master 收敛+执笔后的 spec 定稿,你要挑漏洞、挑稀释、挑"必答题被悬空"。**

## 待审文件(全部在 `.kiro/specs/` 下,请通读)
1. `.kiro/specs/ah-perception-arbiter/requirements.md` + `design.md` + `tasks.md`
2. `.kiro/specs/ah-control-plane-refactor/requirements.md` + `design.md` + `tasks.md`
3. `.kiro/specs/ah-per-worker-credentials/requirements.md` + `design.md` + `tasks.md`

## 你的输入材料(你自己此前独立写的,master 直接引用了大量内容,可对照原文核对有没有走样)
- `research/perception-divergence-a3-round2-2026-07-10.md`(你的二轮发散,7 题全文——master 在 `ah-perception-arbiter/design.md` 里对你的 §1-4 提案逐条裁决:采纳/修改/否决都写了理由,对 §5-7 则挪去了 `ah-control-plane-refactor` spec)
- `research/perception-final-convergence-2026-07-09.md`(deep-research 裁决终稿,四道设计轮必答题的来源)
- `research/architecture-assessment-converged-2026-07-09.md`(模块 D 的结构判决依据)

## 审查任务(反讨好,推翻问法授权——找不到问题不代表没有,但别硬造)

### 1. 必答四题是否真的"每题在 design.md 里有明确答案,不许悬空"
逐题核对 `ah-perception-arbiter/design.md`:①单写入口硬约束形态 ②各信号类 Unknown 预算 ③父/子 cgroup 委托布局 PoC ④hook 上报归属竞态机制。每题是否有**可执行的具体决定**(不是"留待实现时再定"这种回避)?若你发现某题实际上被文字游戏悬空了(看起来答了,细读发现还是开放的),明确指出。

### 2. master 对你 round-2 提案的裁决是否站得住
你在 round-2 §1 提了三层防御(编译期封装 + CI 静态扫描 + DB Trigger);master 在 `ah-perception-arbiter/design.md` "Q1" 段落只采纳了前两层,**拒绝了 DB Trigger 层**,理由是"单进程 Arc<Mutex<Connection>>下 trigger 买不到额外保证,而且你自己的失效模式分析(§1 失效模式2:授权 token 被其他代码复用)已经削弱了它"。**逐字核对这个拒绝理由是否忠实于你原文的论证**,还是 master 曲解/简化了你的立场。若你认为 DB Trigger 层其实该保留(例如防御纵深有你没写进 round-2 但确实存在的价值),现在提出来,别因为"已经被采纳/否决过"就不说了。

同样核对:Q3(父子 cgroup → 平级兄弟 cgroup)、Q4(outbox 落盘 + cookie 归属)的裁决段落——master 在 Q4 里改了你的机制(不用新造 `CCB_JOB_COOKIE`,复用现有 dispatch 标识符,且留了"具体是哪个现有标识符"给实施者查证)。这个改动是合理简化还是回避了你原方案里"为什么需要一个专门 cookie"的论据?

### 3. 模块 D(control-plane-refactor)有没有被你 round-2 §5-7 的内容稀释
你 round-2 §5(job 状态机)/§6(F3F2 解耦)/§7(仲裁器与状态机协作边界)是 master 在 `ah-control-plane-refactor` spec 里的主要输入。核对 D1/D2/D7 三个 requirement 段落 + 对应 design.md 段落,是否完整保留了你论证里的关键机制(尤其 D2 的"崩溃恢复重发现"要求——你 round-2 §6 失效模式1 明确指出"事务A提交后 daemon 崩溃,job 永远卡 DISPATCHED"这个坑,design.md 里这条是否真的被认真设计了,还是一句带过)。

### 4. 独立线(per-worker-credentials)有没有漏洞
这条不是你写的原始材料,是 master 直接从 architecture-assessment 的 §一.13 展开设计的(copy-on-materialize 方案)。你没有先验立场包袱,正好可以纯挑刺:copy-on-create 是否真的解决了问题,还是只是把 bug 往后挪了(design.md 自己也承认"provider-side session coupling"是未解决的残余风险)——这个残余风险是否被恰当地标注为"已知但不解决",还是被文字掩盖成了"已解决"?

### 5. 跨 spec 一致性
`ah-perception-arbiter` 和 `ah-control-plane-refactor` 两份 design.md 都有"D7/协作边界"相关段落,声称"事件形状留给两边实施者对表,不在这轮定死"——这是合理的留白还是危险的悬空(两边实施者各自假设不同形状,拼接时炸)?你怎么看这个"故意不定死"的决定。

## 输出
一份 markdown 报告,结构参照你以往报告的风格(问法评估 + 置信度 + 具体依据 + 若否决/修改要给出机制),别只是罗列"我同意/不同意"。**写到 `research/a3-adversarial-review-of-c-d-specs-2026-07-10.md`**,完成后回报 master。不确定某处是否是真漏洞还是你自己记错了原文,先用 grep/读文件核对,不要凭记忆断言。
