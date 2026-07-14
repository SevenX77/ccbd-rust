# 阶段计划 — 感知/完成协议设计轮(master 起草,2026-07-11)

来源:operator dispatch + `research/stage-brief-perception-protocol-2026-07-11.md`。本文只是泳道分工与顺序,不重复 brief 里已钉死的内容(仲裁器四题、收敛稿骨架不重开)。

## 一、范围复述(不重开)

设计轮产出 `.kiro/specs/ah-completion-protocol/{design.md, requirements.md, tasks.md}`,覆盖 brief §三 五项缺口:
1. R1 G1 hook 投递可靠化(outbox/ACK/重放,归属竞态沿用仲裁器 design Q4 答案)
2. R2 显式完成协议本体(头号)+ reply 载荷归属显式化
3. G4 控制路径自检
4. 物理证据闸门(job 级,两护栏)
5. R3 拆除计划与替代信号覆盖表(只设计,不实施)

新题(未收敛,需双盲发散+互审):per-provider 完成缺口矩阵(agy 无 Stop-hook 等价物、codex task_complete 语义边界)。

## 二、执笔权落地

- spec/design/tasks 执笔:g1、g2(claude 闸门)。两条泳道各领一部分,互相交叉审阅后合并,不由发起泳道自己终裁自己的部分。
- o1(agy):只坐辩论席,发散纪律=只给问题不给结论+显式反讨好,不落 spec 正文。
- g1-m1/g2-m1(agy 实施位):设计轮不派单,标准 IDLE 待命——实施器归下一阶段(operator 亲验 design 初稿后)才启动。

## 三、泳道分工

**Track A(g1 执笔)**:R1(G1 outbox/ACK/重放,归属竞态复用仲裁器 design Q4)+ G4(控制路径自检:hook 配置 diff + 合成触发 + 接线完整性断言,三档启动检查 + 深检挂 `ah doctor`)。
产出草稿:`design-draft-track-a-g1.md`(同目录)。

**Track B(g2 执笔)**:R2(显式完成协议本体,含 reply 载荷归属显式化)+ 物理证据闸门(job 级,is_mutating 静态标注 + 2 次 nudge 后第三次放行上抛)+ R3 拆除计划与替代信号覆盖表。
产出草稿:`design-draft-track-b-g2.md`(同目录)。

**Track C(新题发散,双盲互不可见 + g2 起草 + g1 复核)**:per-provider 完成缺口矩阵。**operator 复核纠偏(2026-07-11):单盲(仅 o1)违反"双盲重合"纪律,已补第二盲稿,人选按 operator 指示优先模型多样性(claude > 同 agy 双盲)。**
1. o1 出第一份发散稿(只列问题/风险,不给结论)→ `divergence-provider-matrix-o1.md`(已完成,产物轨已验)。
2. **g1** 出第二份发散稿,与 o1 互不可见(prompt 不透露 o1 稿内容/任何已有倾向)→ `divergence-provider-matrix-g1.md`(顺带于 Track A 之后,已排队)。
3. 两稿到齐后,比对重合与分歧,记入 `convergence-provider-matrix.md`(谁来写:g2,因为它是矩阵结论的起草人,顺带完成比对)。
4. g2 基于合并后的问题清单起草矩阵结论,并入 Track B 草稿对应小节。
5. g1(跨泳道)复核该小节,可挑战/推翻,记录终裁分歧。

## 四、时序

1. **现在启动,可并行**:Track A(g1)、Track B(g2)、Track C 第 1 步(o1 发散)三者同时开工——纯 markdown 设计,不碰共享 git 树的可变文件,无需 worktree 隔离。
2. Track C 第 1 步完成后,g2 把矩阵结论并入 Track B 草稿(约在 Track B 主体完成前后,不阻塞 Track A)。
3. **交叉审**:Track A 草稿完成后由 g2 复核,Track B(含 Track C 结论)完成后由 g1 复核——都是"跨泳道审对方产物",不自审。
4. **合并**:两条草稿 + 交叉审意见收敛后,由 g1 执笔合并为正式 `design.md`(结构对齐仲裁器 design 的 must-answer 体例),g2 执笔 `requirements.md`/`tasks.md` 骨架(每项标注实证计划挂哪个 dogfood 节点,遵守验证债纪律)。
5. 汇报 operator,design 初稿亲验后才开实施(g1-m1/g2-m1 现在保持 IDLE)。

## 五、监控与纪律延续

- pend 哨兵按预算挂(设计轮无 cargo 编译,单条预算下限仍 900s,按内容量可上调)。
- claude 闸门位真实进度锚 pend 收口 + 落盘文件出现;agy(o1)真实进度锚落盘文件出现,不锚 job 状态(o1 历史撞过 turn-end 假完成)。
- 双盲发散强制反讨好:o1 的 prompt 里不透露 g2/operator 已有倾向,显式授权推翻。
