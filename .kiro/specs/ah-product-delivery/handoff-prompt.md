# Handoff Prompt — ah 产品交付 (Master PM, 续接 session)

> 写于 2026-06-18. 上一个 session 出现 malformed tool-call bug (上下文退化征兆), 切新 session 续接.
> 你是 **Master PM** (环境无 `CCB_CALLER_ACTOR` / 无 `AH_MASTER_ROLE`). 按 `~/.claude/` 宪法 + 本项目 `CLAUDE.md` 工作.
> 核心信条 (PM 原话): "用户参与的只有需求挖掘和敲定目标, 怎么实现目标应该全是主控和 agents 自动根据 sop 完成的事情." 自驱所有 HOW, 只在真正的目标层/设计地基问题 escalate.
> **真相来源 = 进程树 / tmux pane / systemd / 文件系统 / git diff, 绝不信 ah/ccb 状态自报.**

---

## 任务顺序 (PM 2026-06-18 拍定, 严格按序)

1. **先把合并解决** (PR #53 → main)
2. **master 切换到 ah** (Step-4 終极验收 dogfood, task #26)
3. **hook 设计落地** (借鉴 cmux 的 push 完成信号)

---

## 优先级 1: 完成 PR #53 合并到 main

### 背景
- 分支: `feat/step4-master-self-switch`, PR #53 (base main, OPEN). 这是 PM 已授权合并的 Step-4 milestone ("前面的实现合到 main").
- 已 commit 在分支上: `3258304` (pr4e 并行 flake 隔离修复 — 已验证 5/5 轮并行绿).
- **未 commit 的工作改动** (a1 正在做的 r1 reap-race 修复, 接手时可能已跑完):
  - `src/db/system.rs` (产品修: worker tmux session 拆除必须在 `clean_worker_runtime_resources` 返回前完成)
  - `src/db/agents_lifecycle.rs`
  - `tests/r1_master_exit_shutdown.rs` (断言调整)

### r1 reap-race 根因 (已确诊)
CI 跑 `cargo test --all-targets` (并行). `tests/r1_master_exit_shutdown.rs` 4 个测试稳定 FAILED (两次独立 run 一致, 0.3s). 真失败只有 1 个 (`active_master_raw_exit_reaps_old_worker_then_revives_master:421` — DB active-count 已到 0 但 worker tmux session 还没拆), 其余 3 个是 `DEV_STATE_LOCK` 被毒化的连带 PoisonError. 上一 commit (bec9770) 能过是因为旧 plugin_drift 挂起 10s 改变时序遮住了这个 race; plugin_drift 修好后 race 稳定暴露. a1 判定为**产品回收顺序 bug** (不是测试太严), 修在 `src/db/system.rs`.

### 当前精确状态 (2026-06-18 22:50, session 退化重置时定格)
- **已 commit 在分支** (`3ef426c`): 第 1 轮 reap-ordering 修 (agents_lifecycle.rs cleanup-before-commit + system.rs 回归测试 + r1 断言初版). **原 `:421` tmux race 已消除**.
- **但 CI 全并行 (`--all-targets`) 又冒 2 个跨 binary 隔离缺陷** (单 binary 复现不出):
  - `second_daemon...:435` — `ccbd_process_count()` 按 ahd binary path 数全机器进程, 把别的 test binary 的 daemon 也数了 (2≠1).
  - `active...:488` — `active_agent_count==0` 瞬时断言 race 了 revive 的 worker reprovision.
- **a1 第 2 轮修在 disk 上未 commit** (`tests/r1_master_exit_shutdown.rs`, ~49+/16-): **内容已 PM review 确认正确** (按 brief 精确命中): `ccbd_process_count(state_dir)` 改按 `AH_STATE_DIR={state_dir}` needle 隔离自己 daemon; daemon spawn 传 `.env("AH_STATE_DIR", state_dir)`; 加 `wait_for_agent_session_replaced`; `active_agent_count` 改 `wait_for_active_agent_count(&state_dir, 1, …)` (a1 判定 revive **会** reprovision → 落定为 1). **但未跑测试验证** (a1 被 reset 杀在验证途中, 无存活 cargo/rustc).
- **helper ccbd tmux server 已 DOWN** ("no server running on .../tmux.sock"), a1/a2/a3 都没了, **要先重启 ccbd 才能派单**.

### 你接手要做的
1. **重启 helper ccbd** (派单渠道): `command ccb`(前台, 别 `&` detach). 确认 `command ccb ps` 三 agent idle.
2. **决定 a1 第 2 轮未验证改动**: 内容 PM 已 review 正确 (上面). 两条路任选:
   - (a) **PM 直接 commit by-name + push 让 CI 验** (CI `--all-targets` 全并行 = 真正的验证环境; 这是 test-only 改动, 不碰 src, 推 PR 分支不是 main, 安全可逆). 然后盯 CI.
   - (b) 不放心就**派 a1 用 `CARGO_BUILD_JOBS=1 cargo test --release` (全 target 单 cargo 进程) 复现+验证**再 commit.
   - 推荐 (a): test-only + CI 是权威并行 gate + 内容已 review.
3. **commit by-name**: `git add tests/r1_master_exit_shutdown.rs` (绝不 `-A`/`.`), commit (含 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`), `git push origin feat/step4-master-self-switch`.
4. **盯 CI 到全绿**: `gh run list --branch feat/step4-master-self-switch --limit 3`. push 触发 push + pull_request 两个 run, 都要绿. 真失败先 `gh run view <id> --log-failed` 诊断, 不盲目 re-run. CI 设 `CCB_TEST_SKIP_REAL_PROVIDER=1` 跳 real-provider 测试 (本地全 target 跑会撞 real_gemini 失败, 那是外部不是 r1, 别混淆).
5. **合并**: CI 全绿 + main 最近 3 次绿 (`gh run list --branch main --limit 3`) 后, **PM 已授权**用 `gh pr merge 53 --squash` (Step-4 milestone). **绝不带红 CI 合 main** (宪法5). 合完报告 PM (人话, 现状/根因/下一步).

---

## 优先级 2: master 切换到 ah (Step-4 終极验收 dogfood, task #26)

### 目标
ah (Agent Hypervisor, Rust) 立项終极验收 = **Master PM 自己跑在 ah 上而不是 ccb 上**. 即用 ah 替代当前 helper ccbd (Python `/home/sevenx/.local/bin/ccb`) 来派/管 a1/a2/a3.

### 现状 (接手前先 grep `.kiro/specs/ah-product-delivery/` + memory `project_ah_product_delivery_phase` 重建)
- 三个 provider (codex/agy/claude) 已 dogfood-proven (Case B/C/D). claude provider = PR #46 merge 60cc416.
- master-death revive/resume 已闭环 (本 PR #53 reap-ordering 修复是其一部分).
- **剩余 gate**: ahd 自身可重启 service 未实现; **master-OOM vs 反孤儿级联杀 是唯一真决策点** (memory `project_master_death_corrected_semantics` + `project_ah_session_watch_cascade_defeats_revive` — 已派 a1 研究 + a2 设计在途); Step3 并发峰值 OOM smoke 未跑; Step4 master 自换被 Step3+master-OOM 设计 gate.
- **dogfood 纪律** (memory `feedback_dont_test_ah_with_ccb`): 不拿 ccb 测 ah (用病人测医生). master 切换验收必须真用 ah 派 agent. dogfood 用隔离 ahd state+socket (`AH_STATE_DIR`/`CCBD_STATE_DIR` + `tmux -L` 唯一名 + trap cleanup + 只 kill 精确 DAEMON_PID); 生产 ahd 是 systemd unit `ahd.service` (当前 inactive) — 要 `systemctl --user stop ahd.service` 不只 kill. **绝不** pkill/killall claude/codex 全局; 注 OOM 用 `kill -9 <pid>` 精确, 不用全局压力.
- **provider 护栏** (memory `project_gemini_deprecated_antigravity_target`): gemini 在 ah **弃用**, 严禁再对 gemini 投 dogfood/修复; 目标 provider = antigravity (agy v1.0.3 已装+鉴权在).

### 接手要做的
合并 PR #53 后, 按 SOP-08 12 步闭环自驱推进剩余 gate. master-OOM vs 级联杀 决策点 a1 研究 + a2 设计在途, 收敛后实施. 别让任何 agent idle.

---

## 优先级 3: hook 完成信号设计落地 (借鉴 cmux)

### 来源 / 动机
PM 看了 `docs/competitive/cmux-vibeyard-borrowable-for-ah.md` 后指定要落地这条. **痛点实地复现**: 本 session 派 a1 用 `ccb ask --wait`, job 卡 `running` 十分钟但 a1 pane 早 idle (completion-lag); ah 自身历史也有 completion-detection bug. 根因: ah/ccb 现在是**「拉」**模型 — 靠读 agent 日志/transcript 判断完成. cmux 用**「推」** — agent 完成时 hook 直接推事件给编排层, 无滞后无误判.

### 设计要点 (借鉴, 不抄)
- **机制**: hook-based push completion signal. agent 一完成, 由它自己的 hook 触发事件直达 ahd, 取代/补强现在的 pull 日志检测.
- **跨厂商成立** (ah 护城河不能丢): codex/claude 有 hook; 目标 provider antigravity **自带 hook 引擎** (`jsonhook.JSONHookSpec`). 设计必须三 provider 通用, **不退化成单厂商子 agent**.
- **跟 completion v2 关系**: ah 已有 completion-detection v2 (codex `task_complete` + claude `stop_reason` log 主信号, memory `project_ah_completion_v2_log_signal_verified`). push hook 是补强/替代这套 pull. 设计说清两者关系 (push 为主 + pull 兜底? 还是 push 完全取代?).
- **红线**: cmux 是 **GPL-3.0** — 只学机制、ah 自己重写, **绝不抄源码**. (vibeyard 部分 MIT 可参考代码.) ah 护城河 = 跨厂商 agent 总线, 借鉴不能丢这个.

### 接手要做的 (新颖/架构级, 走 SOP-08 §1.1 完整设计环)
1a a1 research (现有 completion v2 代码 + 三 provider hook 能力 grep 实证) → 1b a3 audit → 1c **a2 出思路** (第一性原理: push vs pull / 跨厂商抽象 / 主兜底关系) → 1d a1+a3 audit 思路 → 1e a1 写 design.md → 1f a3 audit → 收敛后 test-first 实施. **设计阶段 PM 不自己拍方案** (宪法7), 传话给 a2 + 事实校验.

---

## 标准约束 (必须遵守, 逐条)

- **派 agent 渠道**: helper ccbd (Python, `/home/sevenx/.local/bin/ccb`) 派 a1/a2/a3. **绝不动它 / 绝不改 `.ccb/ccb.config`**. socket `/home/sevenx/coding/ccbd-rust/.ccb/ccbd/tmux.sock`; a1=%4 codex, a2=%3 gemini, a3=%2 claude.
- **派单姿势**: 前台 `command ccb ask --wait --timeout 600 a1 "..."` 或 Bash `run_in_background:true` (仍 attached). **绝不** `&`/nohup detach (reparent PID1 → 污染 ahd.owner → master-lock 拒派). 长 prompt (>500 字符) 写 `/tmp/*.md` 让 agent 自己读.
- **ccb completion-lag**: `--wait` 可能超时但 agent 仍在跑 — **pane 是真相**, 不信 ccb status. phantom job `command ccb pend a1` 查 id + `command ccb ask cancel <id>` 清.
- **VPS cargo 必须串行**: `CARGO_BUILD_JOBS=1 cargo test/build --release ... -- --test-threads=1`. **保留 --release**. 并行多个 cargo 进程会 OOM 杀主控+崩 ccbd. (单个 cargo 进程内部并行测试 OK, 验 race 时用.)
- **角色**: PM **绝不写 src/tests** (派 a1). 但 PM 为 dogfood 验收**可以** build/run ah (dogfood 是 PM 验收动作不算写业务码). 派 a2 prompt 必含边界关键词 (不改文件/不 ccb ask/不 commit/中文).
- **git**: 永不 `git add -A`/`.` (按名 add); 永不 force-push main; 永不 amend published commits; 永不跳 hooks/CI. commit 结尾 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- **merge 策略**: 开发期 `gh pr merge --merge` (merge commit); milestone `--squash`. **合 main 须 PM 显式 ack** (PR #53 已 ack).
- **escalation**: audit/CI/dogfood 发现的问题 = 下一轮派 a1+a2 input, **不抛 PM 拍工程细节** (宪法9). 只真目标层/设计地基崩 escalate.
- **OAuth-only** 所有 agent, 绝不 API key, 绝不 silent fallback.
- **autonomous loop**: 派 ccb 任务后 60s ScheduleWakeup poll (`<<autonomous-loop-dynamic>>`), 主动 capture pane 验证. 沟通用人话 (现状/根因/下一步), 引用 file:line.
- **`ah`/`ahd` 两个 binary**: `target/release/ah` (CLI) + `target/release/ahd` (daemon). **绝不在没 `AH_STATE_DIR` 下裸跑 `target/release/ahd`** (它会 daemonize).

## 重建上下文必读
- memory: `project_ah_product_delivery_phase` (主进度) / `project_master_death_corrected_semantics` / `project_ah_session_watch_cascade_defeats_revive` (级联杀决策点) / `feedback_dont_test_ah_with_ccb` / `project_gemini_deprecated_antigravity_target`
- 文档: `docs/competitive/cmux-vibeyard-borrowable-for-ah.md` (hook 借鉴) / `.kiro/specs/ah-product-delivery/` (全 spec)
