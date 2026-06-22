# RE-dogfood 结果 — UI-only completion recapture 兜底 (step9, 2026-06-22)

二进制: ah/ahd 0.9.0 (deploy Jun 22 07:40), session sess_b1f1b4fa, state_dir `~/.local/state/ah/29acbe42/`。
agent lineup: a1/a2=codex, a3=antigravity(UiOnly), a4=claude。

## 结论一句话
**Step 2 硬门槛 FAILED — recapture 兜底在真实 reader-miss 场景下没有触发, a3 完成却停 BUSY→STUCK。**
recapture **逻辑本身是对的** (2 个真实字节单测绿), 但**线上 pane_diff watcher 没把 a3 兜起来** → 运行期(operational) gap, 不是逻辑 gap。#3 **未闭合**。

---

## Step 1 正常路径 — PASS ✅
- `ah ask --wait a3 "reply pong"` (T0=08:28:11)。
- a3 经 **live UI reader** 在 ~15s 内完成: 日志 `collect_reply complete agent_id=a3 screen_text_len=1091 reply_len=4` (08:28:26), `--wait` 返回 "pong" 给 master = reply delivered, a3→IDLE/Matched。
- **零 recapture 日志** → 30s 周期兜底没触发 = 不抢 reader 的活 (正确)。

## Step 2 硬门槛 — FAILED ❌
**制造 reader-miss (可控、faithful)**: `tmux pipe-pane -t %3` 关掉 a3 的 pane→FIFO 管道 → reader 读到 EOF 退出 (reader.rs:110 `Ok(Ok(0))=>break`)。已 verify 这**不影响 recapture 路径**: `agent_io::pane_id(a3)` (TMUX_PANE_MAP) 不被 reader 退出清掉 (reader 退出只 `parser_registry::remove`, reader.rs:201); recapture 用**全新 parser** (pane_diff/mod.rs:127), 不依赖被删的 shared parser。所以 watcher 仍应抓到 a3 并兜底。

**观测** (dispatch job_963ee1cc @08:32:45):
- a3 在 pane 里**完成了** (reply "bravo" + idle `>` prompt + `? for shortcuts ... Gemini 3.5 Flash (High)` 状态行, capture 实证)。
- a3 **停在 BUSY 90s+** (08:32:54→08:34:18 连续 13 次 poll 全 BUSY), 之后 **STUCK** (300s 阈值, ~08:37:45)。
- **整 session 零** `pane diff UI completion recapture marked agent IDLE` 日志。
- 也**无** `pane diff watcher marked agent STUCK` 日志 → a3 的 STUCK 来自 health_check_watcher (HEALTH_CHECK_STUCK, 不在 INFO 级 log), 不是 pane_diff watcher。

## 根因隔离 (逐个排除)
| 假设 | 排除证据 |
|---|---|
| matcher 不匹配真实 idle 画面 | **PM-verified 单测绿**: `marker::matcher::tests::antigravity_real_idle_capture_matches` → Matched (用 ahd `capture-pane -p -S -200` 抓的原样字节 `REAL-a3-idle-capture.txt`) |
| 2-tick recapture 逻辑没触发 | **PM-verified 单测绿**: `pane_diff::tests::ui_only_marker_recapture_completes_real_antigravity_idle_capture_after_two_ticks` → `r2.ui_completed_agent_ids==["a3"]` |
| content_hash 每 tick 翻转 (consecutive_ticks 不累加) | a3 idle pane 连抓 3 次 md5 完全一致 (32fc5ba3...), hash 稳定 |
| 状态守卫 swallow (非 active state) | a3 在窗口内是 BUSY = active state |
| completion::registry defer (log monitor authoritative) | Step 1 reader 同一函数 `mark_agent_idle_matched` 成功转 a3 → a3 不在 registry, defer 不触发 |
| watcher 没 spawn | `orchestrator/mod.rs:38-41` 无条件 spawn |
| watcher task panic | journal 全程无 panic |

→ **逻辑全对, 线上就是没兜起来** = pane_diff watcher 在运行的 daemon 里没有有效地处理到 a3 (没 tick 到 / 没 capture 到 / 没走到 recapture 分支)。**确证差在哪一步需要给 watcher 加 per-tick INFO 日志 + debug 重编 + 监督方 redeploy**, 黑盒探不出来了。

## Step 3 防早转 — 未跑 ⏸️ (被 Step 2 失败阻塞)

## 副作用 / 现状
- a3 当前 **STUCK** (测试杀了它的 reader, 没恢复)。
- a1 新加 2 个**真实字节单测** (src/marker/matcher.rs + src/pane_diff/mod.rs, **未 commit**) — 正好补上 a4 audit 警告的"合成 fixture 假覆盖"缺口 (老测试用 `"...? for shortcuts...\n"` 无尾部空行, 不代表真实 capture)。
- **未 merge, 未动 hook_push_providers, 未报 done。**

## 建议下一步 (需监督方拍 + redeploy)
给 `pane_diff_watcher_tick` 加一行 per-tick `tracing::info` (打 BUSY agent 数 + 每个 UiOnly agent 的 scan 结果 + consecutive_ticks), debug 重编, redeploy, 重跑 Step 2 → 一眼看出线上 watcher 到底 tick 没 tick / 抓 a3 抓到啥。
