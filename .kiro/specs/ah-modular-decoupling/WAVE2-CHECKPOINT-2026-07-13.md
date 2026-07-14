# Wave-2(MD2 行为保持解耦)检查点 — 2026-07-13

**背景**:用户 2026-07-13 指令把栈 redirect 去修两个代码 issue(#29 可观测性 + master 换 codex 参数化),Wave-2 暂停。本文件保住 Wave-2 在停顿点的全部状态,防止 redirect 后遗忘/孤儿化。恢复 Wave-2 时**先读本文件**。

## 已完成但未收口(需收口:PR→CI→merge)

### Wave-2 candidate1 — `db/system.rs` 拆分【DONE,差 PR】
- **分支**:`feat/md2-wave2-system-split`,HEAD=**a8d4970** `refactor(db): split system startup and sweep modules`,**已 push 到 origin**。
- **内容**:`src/db/system.rs`(~3491 行)拆出 `src/db/system/{dump,startup,sweep}.rs`;cascade/master-death 生产代码 + 红线测试**保留**在 system.rs;加了 facade re-export 编译锚测试 `facade_reexports_sweep_stale_tmux_sockets_sync_old_path_compiles`;更新 `scripts/ci/check_state_write_gate.sh` baseline(startup SQL 路径迁移)。
- **实证**(c1 本地,已落盘日志):`cargo check` 绿;`cargo test --lib db::system -- --test-threads=1` = 47 passed / 0 failed;`CCB_TEST_SKIP_REAL_PROVIDER=1 CARGO_BUILD_JOBS=1 cargo test --workspace -- --test-threads=1` = 1070 passed / 0 failed / 全 target 绿。行为保持双保险:红线 rg 确认 cascade/master-death 符号仍在 system.rs 原位。
- **欠账**:**无 PR**。master IDLE 未收口(收口断点:c1 干完 23min 无人开 PR)。恢复动作 = `gh pr create` → 等 CI 绿 → merge。**本地串行绿≠并发安全,以 CI 并行绿为真验收。**

## 停顿点其它未推/未合状态
- **本地 main 领先 origin/main 两个 docs 提交**(未推):`74bc952 docs: design db system split boundaries`、`b9c9af3 docs: d1 brief for Wave-2 candidate1`。是 d1 直接提交到本地 main 的设计文档。恢复时决定推还是并进某 PR(注意"陈旧本地 main"陷阱,选 base 前 `git fetch`)。
- **PR #160** open,`docs: MD2 Wave-2 plan draft (unapproved)` — 计划草稿,与实现分支分开。

## 剩余 Wave-2 候选(未开工,来自 master orient / survey)
- 低风险数据层:`db/state_machine.rs`(2978)、`rpc/handlers/sessions` 剩余。
- **高风险(设计稿先发 operator + 合并前 gate)**:`db/jobs.rs`(2662,派单引擎)、`provider/home_layout.rs`(3039,凭据物化+current_exe 区)。`db/state_machine.rs 算不算碰派单核心`是待裁开放问题。

## 恢复 Wave-2 的第一动作
1. 收口 candidate1:为 `feat/md2-wave2-system-split` 开 PR,盯 CI,绿则 merge(操作在 master 的 SOP 内环)。
2. 推/处置本地 main 领先的两个 docs 提交。
3. 再从剩余低风险候选继续,高风险目标走 operator gate。
