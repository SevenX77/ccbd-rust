# 病例:recovery 重投 × cancel_requested 竞态(2026-07-11,观察日志 #49②)

## 现象(实证时间线)

1. 两张 job(job_0ac5a283→g1、job_a201a395→g2)DISPATCHED 在途,operator `ah cancel` → `cancel_requested=1`,状态仍 DISPATCHED。
2. cancel 触发 agent kill → respawn(SPAWNING → 新 pid)。
3. **recovery 把 cancel_requested=1 的 DISPATCHED prompt 重投给了 respawn 后的新实例**——两席拿着 brief 重新从头开工(此时新沙箱已带新规则,但那是另一回事)。
4. 新实例干了 ~10 分钟、产出 commit(f174687/0ed41d1)之后,cancel 才在 turn 边界落地:job 先后翻 CANCELLED——**"取消"的单,工作却被完整执行了一遍。**

## 病理

两个子系统各自正确、组合错误:
- recovery/reinsert 的职责:DISPATCHED 单的 prompt 必须真的在 pane 里(治 dispatch-ACK 竞态、respawn 丢 prompt)——单看没错。
- cancel 的职责:标记 cancel_requested,等 agent 认领(#31 已记:对无人认领场景本就乏力)。
- **组合:reinsert 不检查 cancel_requested** → 取消动作反而变成了"重启任务"的扳机。取消的意图被系统亲手推翻。

## 危害

- operator/master 的止损动作(cancel)不可信:cancel 后不能假设工作停止,必须再 kill——但 kill 又触发 respawn,respawn 又触发 reinsert……当前唯一稳妥链是 cancel+kill+看住 respawn(SOP 化的绕行,不是修复)。
- 本例实害:被取消的任务在错误位置(主树)又跑了一整轮并 commit 到本地 main(见 #49③)。

## 修向(设计约束)

- **reinsert 前置检查**:`cancel_requested=1` 的 job 一律不重投——直接收敛到 CANCELLED 终态(fail-closed:宁可少投,不可复活已取消的工作)。
- cancel 语义补强(与 #31 合并考虑):agent 死亡/respawn 即视为"无人认领",cancel_requested 的 DISPATCHED 单在 kill 路径上应同步落 CANCELLED,不留给 recovery 二次解释。
- 回归测试:cancel_requested=1 + agent kill+respawn → 断言 respawn 后 pane 无 prompt 重投、job 终态 CANCELLED(契约边界:pane 内容 + DB 终态)。

## 关联

- `dispatch-ack-race.md`(reinsert 机制的另一面)
- 观察日志 #31(cancel 无人认领)、#49(本例全链)

---

## 变体 B:cancel 占席僵尸 → 队列排水(obs #52,2026-07-11)

**新现象**:cancel 一个长期占席的僵尸 DISPATCHED job,会触发 ahd 对**该席位积压的历史 QUEUED 队列**的无差别排水——把数小时前的过时归档 brief 真 DISPATCH 给 agent,agent 开始执行古董指令。伴随 cancel×dispatch 竞态第二例(`job_6fde2f4c`:`cancel_requested=1` 却被 dispatch 抢先翻 DISPATCHED)。

**根因**:①cancel 的副作用面过大:cancel 占席 job 不应连带排水同席历史 QUEUED;②"僵尸占席"伴生的 QUEUED 归档单被当正常队列消费,无时效/来源校验。

**追加契约(回归)**:
- cancel 一个 job **不得**触发对同一 agent 席位其它 QUEUED job 的自动 dispatch(排水与 cancel 解耦)。
- dispatch 前对 QUEUED job 校验时效/来源:超过阈值年龄或标记为"僵尸伴生归档"的 job 不得被自动 dispatch,需显式重新派发。
- cancel_requested=1 的 job 在任何路径都不得再翻 DISPATCHED(cancel 与 dispatch 的 CAS 原子化)。
