# Step-9 ROUND 4 — live re-dogfood of STUCK-rescue fix (sess_c95c8099, ahd pid 4127242, build 15:56)

> 监督方验证用。结论: **源头修复确证 live 生效 (ROUND2/3 硬失败已消除)**, 但 **recapture-path 完成 (BUSY→IDLE + job COMPLETED) 未能物理触发**, 且发现一个真实接缝 (seam)。Step 2 recapture-完成这一条**未过**。Step 3 **已过**。

## 环境确认
- binary `~/.local/bin/{ahd,ah}` mtime = Jun 22 15:56 (fresh build), running ahd pid 4127242 起于 15:56:52。
- per-tick watcher 每 30s tick (journald 实证, busy_agents=N)。
- a3 = antigravity (UiOnly), tmux pane %3, socket `ahd-9819d8d7587886a9`。
- reader 机制: ahd 持 `a3.fifo` 读端 (O_RDWR, fd 22, 永不 EOF), tmux `pipe-pane -t %3 -O 'cat > a3.fifo'` 喂数据。kill pipe = reader 断流。

## 三次实测

| # | 手法 | 结果 | journald 铁证 |
|---|---|---|---|
| 1 | kill pipe **派单前** | chunk_count=0; recapture **swallow**, a3 **永 BUSY** | `UiOnly scan a3 scan=Matched consecutive_ticks=1→2` 然后 `recapture matched but ... no-op (changes=0): already idle or swallowed`; collect_reply `chunk_count=0`; a3 BUSY 2min+ 从未 STUCK |
| 2 | kill pipe **生成后** (~1s) | reader 已抓全 (chunk_count=27) → **live reader 完成** (非 recapture) | `collect_reply chunk_count=27 reply_len=15`; 生成期 `scan=NoMatch consecutive_ticks=0` → **Step 3 防早转 PASS** |
| 3 | kill pipe **生成中** (~2.2s, 大输出 600) | kill 落在 gemini "思考期" (burst 前) → chunk_count=0 → 同 #1 swallow, 永 BUSY | `consecutive_ticks=1→2` + `no-op (changes=0)`; collect_reply `chunk_count=0` |

## 物理已证 (ROUND 3 验证表, 部分)
- ✅ **源头修复 live**: a3 **整 session 从未被标 STUCK** (`health_check.rs:46` UiOnly 跳过 completion-staleness)。ROUND2/3 的 <60s STUCK 硬失败**已消除**。
- ✅ **race/计数器修复 live**: watcher query BUSY a3, `consecutive_ticks` 存活 1→2 (不再一 STUCK 就掉出观测)。
- ✅ **recapture 判定 fire**: tick 2 触发 recapture 决策 (每 2-tick 周期)。
- ✅ **Step 3 防早转**: 生成期 (`esc to cancel` anti-pattern) `scan=NoMatch`, recapture 不误转 IDLE; 完成后才转。
- ✅ **不破坏 live/正常路径**: reader 活时 live 完成, recapture 让位。

## 未证 + 真实接缝 (seam)
- ❌ **recapture-path 完成 (BUSY→IDLE + job COMPLETED via recapture)** 未能物理触发。

### 根因 (代码实证, 非猜测)
1. **接缝**: ROUND 3 兜底 `mark_agent_idle_recaptured` 的守卫绕过 (`allow_stuck_recapture`) **只对 previous_state==STUCK 生效** (`state_machine.rs:342`)。但**源头修复让 UiOnly agent 不再进 STUCK, 停在 BUSY**。BUSY 路径仍命中 **prompt-only-reply swallow** (`state_machine.rs:399`)。
2. **prompt-only swallow**: `is_prompt_only_reply("")==true` (`:851`)。reader 全死 → chunk_count=0 → 空 reply → swallow (changes=0)。
3. **结果**: chunk_count=0 的 UiOnly agent → **既不完成 (recapture swallow) 又不 STUCK (源头修复撤了网)** = **静默永 BUSY** (实测 #1/#3 各 2min+ 永 BUSY, 从无信号)。源头修复前它至少会 STUCK (可见失败)。

### 测试盲区
- 单测 `test_ui_recapture_can_mark_stuck_agent_idle_without_opening_live_marker_guard` (`state_machine.rs:1667`) **只覆盖 STUCK→IDLE** (seed `STATE_STUCK` + 非空 chunk)。**无** BUSY→IDLE + chunk_count=0 的测试 — 正是源头修复后的真实形态。两半修复各自验证, **交互 (源头修复把死-reader 路由进 BUSY, 兜底绕过却只认 STUCK) 没被任何 gate 抓到**。

### 测试保真墙 (为何 live 无法证 recapture-完成)
- gemini/antigravity **reply + idle-prompt 一次性 atomic burst 喷出** (输出 < 64KB pipe buffer, reader 一次 drain)。
- 故 kill-pipe 只能二选一: **burst 前** kill → chunk_count=0 (swallow); **burst 后** kill → reader 已抓全 → live 完成。**没有** "抓到 reply 但漏 idle-prompt" 的中间窗 — 而那正是 recapture-完成需要的条件。
- 真实 ROUND1 bug ("漏 final prompt"/lag): reader 活、chunks>0、只是 live matcher 漏匹配 settled idle prompt → recapture (干净 capture-pane 快照) 补上 → collect_reply 返回 chunks>0 (非 prompt-only) → **完成 (不 swallow)**。此路径**代码上成立**, 但 kill-pipe **物理无法复现** (kill 同时清零 chunks)。

## 主控建议 (单方案, 非 ABC)
源头修复 (消除 <60s STUCK) 是大头, 已 live 确证。剩余接缝是源头修复撤掉 STUCK 网后, chunk_count=0 死-reader 路径变成静默永 BUSY。建议:
1. 派 a1 工程 + a2 设计确认接缝分析, 出最小修复: 让 recapture 在 BUSY+2-tick-stable+chunk0 时也能收口 (要么完成-with-pane-reply, 要么重建一个 STUCK 可见网), 不留静默永 BUSY。
2. 补 deterministic 集成测试覆盖 BUSY→IDLE recapture (chunk0 与 chunks>0 两路), 因 live pipe-kill 对 burst provider 物理无法证 recapture-完成。
3. 由监督方裁定: 接缝 (chunk0 永 BUSY) 是否阻塞闭合, 还是 ROUND1 lag (chunks>0 = recapture 正常完成) 已是目标、chunk0 死-reader 属另案。

---

# ROUND 4 接缝修复 — 完成 + 验证 (READY FOR DEPLOY, 未 commit/merge)

## 流水线 (主控自驱, 无 passthrough)
1. a1(工程)+a2(设计) 分析 → 强收敛单方案: complete-with-pane-reply (recapture 用干净 pane 快照经 distill_reply 刮 reply) + STUCK 兜底网, 杜绝静默永 BUSY。
2. a1 实施 (tests-first, 真实字节 fixture — PM 用 live a3 现抓: charlie / prompt-only / wrapped-prompt 三份)。
3. a4 audit → 5 重点 4 PASS + 抓 2 个 must-fix。
4. a1 修 2 must-fix。
5. PM 独立验证 (物理实证, 非 self-report)。

## a4 audit 抓出的 2 must-fix (已修)
- **重点 2 false-STUCK (真 bug)**: `pane_snapshot.contains(prompt)` 逐字匹配, 长 prompt 被 TUI 软换行 → 失败 → 完成的 agent 误标 STUCK + 丢 reply。PM 用 live a3 实锤 (162 字符 prompt 在 150 列软换行, fixture `REAL-a3-idle-longprompt-wrapped.txt`)。
  - 修: 新 `prompt_match_end_byte` (jobs.rs:699) — pane 与 prompt 都空白归一化再匹配, 归一化末位映射回原始 byte offset; gate (`contains_prompt_text`) 与 `distill_reply` 切片共用, 一致。
- **发现 A 死代码+重复 (quality)**: `_inner` 的 `allow_stuck_recapture=true` 分支无调用者 + recapture 整段复制完成逻辑。
  - 修: 抽共享 `mark_agent_idle_matched_conn_inner` (state_machine.rs:608), live(`:590` allow_from_stuck=false) + recapture(`:471` true) 共用; 死代码消除, 无重复。

## PM 独立验证 (物理实证)
- `cargo +stable test --lib` = **701 passed / 0 failed / 3 ignored** (含先前 flaky 的 master_revive, 本次也绿)。
- recapture 12/0 (含新 `ui_only_recapture_completes_busy_job_from_real_wrapped_prompt_pane ... ok`, 断言 reply=="delta")。
- mvp7_acceptance 6/0, pr4c_hooks_plugins 21/0。
- 读码验证: (1) live 路径字节等价 (guards 在 caller 不变, 共享 helper 仅 CAS-gated UPDATE+complete+event, live 传 false=BUSY-only IN-list 同旧); (2) 归一化 offset 映射正确 (char-boundary, checked_sub 兜底, 不 panic)。
- 二进制重编: `target/debug/{ahd,ah}` mtime 17:11 (含接缝修复)。**running ahd 仍 15:56 (无接缝修复) → 需 deploy。**

## 闭合两路证明
- **(b) chunks>0 lag 路 (真 ROUND1 目标)**: deterministic 集成测试 **PASS** (真实字节, `..._when_chunks_prompt_only` + wrapped variant) — **已闭合** (burst provider live 诱不出, 用集成测试是物理墙下的正确方法)。
- **(a) chunk0 死-reader LIVE**: 需 deploy 17:11 build 后由 PM live 证 (kill pipe 产 chunk0 → recapture 从 pane 刮 reply → BUSY→IDLE+交付, 不静默 BUSY/不 <60s STUCK)。
