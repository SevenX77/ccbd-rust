# Step 4 — master 自换 ccb→ah (终极 dogfood, req #3)

> **[SUPERSEDED 2026-06-15 by `design.md`]** 本 PLAN 是**窄版** Step-4 (只切派单工具 ccb→ah, master 进程照常启动), 已 3-provider PASS。PM 2026-06-15 拍定本轮目标 = **宽版** (master 进程本身作为 ah 托管 pane 跑 + 被 master-revive 自救保护)。宽版正式设计见同目录 `research.md` → `idea-a2.md` → `1d-audit-convergence.md` → `design.md` (1a-1f 已收敛)。本文件仅作窄版历史参考, 实施以 `design.md` 为准。

> 2026-06-12 主控 PM 自驱计划 (handoff §5 Step-4 / §8: 自驱推进, 不抛 PM 拍工程细节)。

## 目标 (PM req #3)

主控 (本 PM Claude) 把**对 worker 的调度工具**从 `command ccb ask/ps/pend/kill` 切到 `ah ask/ps/pend/kill`, 用整套 ah 跑真实 PM 工作 = 终极 dogfood ("不拿 ccb 测 ah", [[feedback-dont-test-ah-with-ccb]])。

**这是 dispatch-tooling 切换**, 不是"master 进程被 ah 托管"。master 仍是 `claude /remote-control`; 变的是它派活的工具。

## 前置 (全部满足)

- Step 1 provider 全测 = DONE (codex/agy/claude C/D3/E/F)。
- master-side `ah ask/ps/pend/kill/start/stop` 已在 Step 3 大量真用且全 work (无 phantom / 无 cancel 救场)。
- 不依赖 master 自身重生 (SF1) / facet C —— 那些是 OOM 整套重启场景, 本 Step 是稳态 dispatch 切换。

## Cutover 序列 (安全 + 可回滚)

1. **保留 helper ccb 作 ROLLBACK 通道**: 全程不 tear down helper ccbd, 直到 ah dispatch 在 ≥N 个真任务上证明干净。任一 ah dispatch 卡住 → 立即回退 `command ccb ask`。
2. **起持久 ah worker session**: `ah-master.toml` = master(sleep, 占位) + a1 codex + a2 antigravity + a3 claude (antigravity 替代已弃用的 gemini 作设计/分析角色, [[project-gemini-deprecated-antigravity-target]])。
3. **真任务路由过 ah**: 接下来每个真 PM 派单 (audit / research / 实施) 用 `ah ask <agent>` 替代 `command ccb ask`。第一个 dogfood 任务 = 用 `ah ask` 派一个真 audit (本 PLAN 自身 / 交付收尾核验)。
4. **物理实证每次 round-trip**: dispatch → reply 真返回 (含 sentinel) → 0 phantom → 0 cancel 救场 → pane/sqlite 实证 (不信 ah ps 自报)。对照 ccb 头号痛点 (completion phantom / 长 prompt 截断 / 队列 desync) 在 ah 上不复现。
5. **跑够 N 个真任务** (建议 ≥3 异构: codex 实施 + claude e2e/audit + antigravity 设计) 全干净 → Step 4 PASS。

## 回滚

- ah dispatch 任一卡住/失败 → 当轮立即 `command ccb ask` 重派同任务 (helper ccb 全程在线), 记录失败现场 (pane/sqlite), 派 a1 修, 不抛 PM。

## 完成标准

- master 经 `ah ask` 完成 ≥3 个异构真任务, 全 round-trip 干净, 0 phantom / 0 cancel 救场 / 0 截断 / 0 队列 desync; 物理实证。
- 矩阵补 Step-4 行; 与 helper ccb 同期痛点对照。
- 之后可考虑彻底以 ah 替代 ccb 作主控默认调度 (PM 最终拍 milestone squash)。

## 诚实边界

- 本 Step 验**稳态 dispatch 切换**, 不验 ahd-OOM-后 master 整套重生 (= 操作员 `ah start` 重起, handoff §8 PM 模型)。
- "续断点"深层含义 (provider-上下文 vs ah-job-级在途追踪) 仍是**唯一待 PM 答的 goal 点** (上轮已问, 不重复 nag); 答案不阻塞本 Step (dispatch 切换独立于此)。
