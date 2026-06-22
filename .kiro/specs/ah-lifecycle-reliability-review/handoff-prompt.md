# HANDOFF → 下一个 session：ah Agent 生命周期完成检测 — 可靠性全面梳理

> 写给下一个全新 session 的 Master PM（你）。上一个 session 把 #3（UI-only 完成兜底）合进 main 后清掉了整套 ah、交棒给你。
> 你启动时 ah 是干净的（无 ahd / 无 session / 无 sandbox），需要按下面重新起。

---

## 0. 一句话任务

**把 agent 的完整生命周期 ——「任务发布 → 是否开始 → 任务完成 → 拿到完成结果」—— 的所有检测方法梳理一遍，回答：到目前为止我们测过哪些方法？它们能跑通、达到预期吗？每个的可靠度有多少？这套"多重冗余互相兜底"的体系，真实覆盖到了哪、还有哪些洞。**

最终交付 = 一张**可靠性矩阵** + gap 分析 + 建议（哪些方法是承重的、哪些是兜底、哪里还漏）。

---

## 1. 为什么有这个任务（PM 原话，2026-06-22）

> "我觉得需要全面梳理一下任务发布-是否开始-任务完成-拿到完成结果这一整个生命周期的方法。我们做了好几套冗余方法，就是因为没有100%可靠，希望多重方法可以互相兜底。所以要梳理一下，到目前为止，我们测试过哪些了，他们是否能跑通并达到预期？可靠度有多少？"

核心动机：**没有任何单一信号 100% 可靠**，所以陆续做了多套检测方法层层兜底。但"做了多套"≠"覆盖全 + 可靠"。需要一次系统盘点，搞清楚这套体系的真实可靠性边界。

### 一条贯穿原则（PM point-1，2026-06-22，务必内化）

**每个检测方法必须落在"这个 provider 物理上真能发生什么"上，别为想象出来的中间状态写测试 / 留代码。**

实例：上一个 session 一度把 antigravity 的完成兜底框成"防 reader 读了半截 chunk 的 lag 窗口"。但 antigravity 是 **burst provider（reply+提示符一口气原子吐完，<64KB 一次读光）**，根本不存在"读了一半卡住"的中间态。真实只有两态：reader 整个没读到（chunk0）、或读到完整 burst。梳理时对**每个 provider** 先问"它吐结果是 streaming 还是 burst？有没有日志？hook 能不能用？"，再判每个检测方法对它**是否适用 / 是否在测真场景**。

---

## 2. 生命周期 4 阶段（梳理的骨架）

| 阶段 | 含义 | "成功"判定 = 检测什么 |
|---|---|---|
| **A. 任务发布 (dispatch)** | master → `ah ask` → ahd → tmux send-keys 把 prompt 注入 agent pane | prompt 真注入到**活着的**目标 pane（不是死 pane / 不是错 pane） |
| **B. 是否开始 (started/BUSY)** | agent 收到 prompt 开始干活，状态 IDLE→BUSY | 真的进入工作态（不是漏判成还 idle，也不是假 BUSY） |
| **C. 任务完成 (completion/IDLE)** | agent 干完，BUSY→IDLE | 真完成（不漏判 = 不永卡 BUSY；不早判 = 不把还在干的判成 idle） |
| **D. 拿到完成结果 (reply retrieval)** | 完成后取回 reply 并交付 master | reply 内容真实、完整、交付到位（不丢、不是 UI 噪音、不空） |

---

## 3. 已知检测方法清单（起点 inventory，需你 grep 代码核实 + 补全）

> 下面是上一个 session 已知的方法。**不要当完整真相** —— 你要 grep 代码确认每个真实存在、找出清单外的、剔除已废弃的。

**阶段 A（发布）**
- dispatch atomicity（`tests/dispatch_atomicity.rs`；`src/orchestrator/mod.rs` 的 `resolve_current_dispatch_pane` + pane pid 校验，防注入到死/错 pane —— 上一轮 incident Bug B/C 修过：KILLED slot recycle + pid revalidate）

**阶段 B（开始/BUSY）**
- marker matcher 匹配 busy/working marker（`src/marker/matcher.rs`）
- pane diff watcher 观测 pane 变化（`src/pane_diff/mod.rs`）

**阶段 C（完成/IDLE）—— 冗余最多的一层，重点**
- **log-event 主信号**：codex 写 `task_complete`、claude 写 `stop_reason`(end_turn/tool_use)，log monitor 读 transcript/log 判完成（见 memory `project_ah_completion_v2_log_signal_verified`，commit eaad842 修过 claude 跨 tick armed guard bug）
- **hook-push（push completion）**：agent stop hook 主动推完成信号；`ah.toml` 的 `hook_push_providers` 控制哪些 provider 走（claude/codex 行，**antigravity 物理不行** —— agy 生成中途被 SIGKILL）
- **pane marker matcher**：底部视窗匹配 idle prompt marker → 判 idle
- **pane stability terminal**：pane content hash 稳定 ≥N秒 + log mtime 静默 → 强制 fallback 完成（`src/.../pane_stability*`，源自旧 ccb Bug Y 修复）
- **UI-only pane recapture 兜底（#3，刚合 829eba8）**：UiOnly+BUSY agent 复用 pane_diff 的 30s capture，底部视窗连 `AH_UI_COMPLETION_STABLE_TICKS`(默认2) 次稳定匹配 → 标 idle；reply 从 pane `distill_reply` 刮，刮不到 → STUCK(`UI_COMPLETION_RECAPTURE_PROMPT_ONLY`)。这是 antigravity 唯一的完成路径
- **idle anti-pattern**：区分真 idle vs 假 idle（codex 漏 U+2022 bullet 致 2s 假完成 bug，已修 7976fbf，见 memory `project_ah_codex_premature_completion_bug`；claude 空 anti_pattern 疑似同类 sibling 待确认）

**阶段 D（拿结果）**
- `collect_reply` 从 pipe 读 chunks（`src/db/jobs.rs`，日志 `collect_reply complete ... chunk_count raw_bytes_total reply_len`）
- `distill_reply` 从 pane 刮 reply（UI-only 路径用）
- mailbox 交付（`~/.ccb`?/ah 的 mailbox inbox/outbox）

---

## 4. 已知 bug / 现状（引用，需你复核是否仍成立）

- ✅ FIXED：codex 过早判完成（idle anti_pattern 漏 bullet）— 7976fbf，dogfood 复验过
- ✅ FIXED：claude 跨 tick armed guard 退化 UI 兜底 — eaad842
- ✅ MERGED：#3 UI-only pane recapture 兜底 — 829eba8
- ❌ **OPEN（本 handoff 顺带要修，PM "合完再修"）**：**master_watch 重启不重装探针** — master 存活探针只在建 master 那刻 arm 一次（`sessions.rs:395`/`:898`，全 RPC 事件，无 startup），ahd 重启 reconcile 不为继承的 ACTIVE master 重 `pidfd_open`+重 arm，且无周期巡检兜底（`master_process_is_alive` 仅 cutover 用一处）→ 死 master + 活 worker + ahd 零检测/零复活/零 reap。详 memory `project_ah_master_watch_not_rearmed_on_restart`。**修向**：(1) startup reconcile 对每个 ACTIVE session 重 `pidfd_open`+重 arm，那刻已死则立即走死亡处理；(2) orchestrator tick 加周期 master 存活巡检兜底。这虽是 master 生命周期不是 agent 生命周期，但同根（"探针只在创建时装一次、重启不重装、无周期兜底"），梳理时把它当同类问题的一个实例
- ⚠️ 相关未结：`project_ah_session_watch_cascade_defeats_revive`（session_watch 级联杀 defeat revive）、`project_master_death_corrected_semantics`（reap 语义）、`project_orphan_scope_reconcile_unwired_bindsto_supersedes`

---

## 5. 每 provider 的物理现实（梳理前先填这张，point-1 原则落地）

| provider | 输出形态 | 有日志? | hook-push 可用? | 完成检测主路径 | 备注 |
|---|---|---|---|---|---|
| **codex** | streaming | 是(transcript/log) | 是 | log-event `task_complete` | 有 idle anti_pattern 坑 |
| **claude** | streaming | 是(transcript) | 是 | log-event `stop_reason` + hook-push | 跨 tick guard 修过 |
| **antigravity (agy)** | **burst（一口气吐完）** | **否** | **否（SIGKILL）** | **pane recapture 兜底（#3）** | **唯一 UI-only，最脆弱，重点验** |
| ~~gemini~~ | — | — | — | — | **已弃用，严禁再投**（memory `project_gemini_deprecated_antigravity_target`） |

> 这张表是你判"某方法对某 provider 是否适用 / 是否在测真场景"的依据。每格都要 grep 代码 / dogfood 实证，别照抄。

---

## 6. 要交付什么

1. **可靠性矩阵**：[生命周期阶段 × 检测方法 × provider] → 每格标：① 是否有该机制 ② 是否有测试覆盖（单测/集成/dogfood，给文件路径）③ 测的是不是真场景（按 point-1 原则）④ 跑通吗 ⑤ 可靠度评估（High/Medium/Low + 依据）⑥ 已知洞
2. **gap 分析**：哪些阶段×provider 组合没有任何可靠检测；哪些"冗余兜底"其实是空的（测了假场景 / 没接进主路径）；哪些方法互为兜底、哪些是单点
3. **建议**：哪些方法承重必须硬化、哪些可砍（测假场景的）、哪里要补
4. **顺带修** master_watch 重启不重装探针 bug（§4 OPEN 那条，PM "合完再修"）
5. 全程按 SOP-08 12 步走（research→design→impl→tests-first→audit→PR→dogfood goal-verify）

---

## 7. 当前仓库 / 环境状态（你启动时）

- main top = `829eba8`（#3 已合，CI 绿）。分支 `feat/ah-hook-push-completion` 已 push、可删可留
- **ah 已被上一个 session 清空**（ahd 杀掉 / sessions 清 / sandboxes 清 / 泄漏的 tmux socket 清）。你要重新 `ah start` 起一套干净的（codex a1/a2 + antigravity a3 + claude a4，按 `ah.toml`）
- `ah.toml` 里 `hook_push_providers = ["claude","codex","antigravity"]` 是上一轮 #3 dogfood 加的（让 antigravity 走 hook-push→失败→兜底）。梳理时复核这配置是否仍合理
- 已知 tmpdir/socket 泄漏 bug（`docs/upstream-ccb-bugs/tmux-scope-and-tmpdir-leak-bugs.md`）：起 ah 前可能又积了一堆 `/tmp/tmux-1001/ahd-*` 死 socket，dogfood 完记得清

---

## 8. 工作方式（硬约束，别违反）

- **dogfood，不要用 ccb 测 ah**（memory `feedback_dont_test_ah_with_ccb`）：验证用 `ah ask` 派 a1-a4，不是 ccb
- **gemini 已弃用**，目标 provider 是 **antigravity**，严禁再对 gemini 投 dogfood/修复
- **VPS cargo 必须串行**：`CARGO_BUILD_JOBS=1` + `cargo test ... -- --test-threads=1`
- **master 是 PM 不写代码**：src/tests 派 a1(codex)；设计派 a2... 但 **a2=gemini 已弃用** → 新颖设计走 SOP-08 §1.1（research 派 a1 / 思路这块暂时主控+a1+a3 收敛，或按当时可用的设计 agent）；机械改直接 a1
- **不 `ah master cutover` / `ah up`**（Bug A reap-after-fail）
- **OAuth-only，禁 API key**
- **commit footer**：`Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` + `Claude-Session: <你的 session 链接>`
- **merge 到 main 须 PM ack**（SOP-05 三连绿才合）；feature 分支自动 commit/push 不用问

---

## 9. 证据源指针

- **memory 索引**：`~/.claude/projects/-home-sevenx-coding-ccbd-rust/memory/MEMORY.md`（尤其 `project_ah_completion_v2_log_signal_verified` / `project_ah_codex_premature_completion_bug` / `project_ah_master_watch_not_rearmed_on_restart` / `project_ah_product_delivery_phase`）
- **#3 完整 handoff + 证据**：`.kiro/specs/ah-hook-push-completion/`（step9-redogfood-backstop-handoff.md + REAL-*.txt fixtures + SUPERVISOR-EVIDENCE-*.md）
- **product-delivery 现状**：`.kiro/specs/ah-product-delivery/handoff-prompt.md`
- **完成检测代码**：`src/pane_diff/mod.rs`（recapture 兜底 + 测试）/ `src/db/state_machine.rs`（状态机 + distill）/ `src/db/jobs.rs`（collect_reply）/ `src/marker/matcher.rs` / `src/provider/manifest.rs`(`CompletionSignalKind`) / log monitor 相关
- **master 生命周期**：`src/monitor/master_watch.rs` / `src/master_revival.rs` / `src/monitor/session_watch.rs` / `src/db/system.rs`(reconcile_startup) / `src/rpc/handlers/sessions.rs`
- **CI 跑法**：`.github/workflows/ci.yml`（`cargo test --all-targets`，`CCB_TEST_SKIP_REAL_PROVIDER=1` 让 real-LLM 测试早返）

---

## 10. 第一步建议

1. 读完本 handoff + §9 全部 memory
2. 重新 `ah start` 起干净 ah，`ah ps` 确认 a1-a4 IDLE
3. 派 a1 做 research：grep 全仓，把 §3 inventory 补成完整真相（每个方法 file:line + 测试文件 + 接没接进主路径）
4. 填 §5 provider 物理现实表（每格 dogfood/grep 实证）
5. 按 SOP-08 走完整流程产出 §6 矩阵 + gap + 建议 + master_watch 修复
