# Design: prompt_handler 陈旧 scrollback churn 死锁修复 (真 dogfooding C 维 blocker)

状态: 设计已收敛 (a2 架构溯源 2 轮 + 主控反论 + a2 接受修正)。待 a3 PM-proxy audit → a1 test-first 实施。

## 1. 物理实证的死锁 (真 codex dogfooding)

`ah start` 起真 codex → `ah ask a1 "Reply with exactly one word: PONG"`。pane 实证 codex **答对** (`› ...PONG` → `• PONG`), 但 `ask --wait` 永不返回, agent 永卡 BUSY。codex pane 无限自注入 `2`+Enter (每轮一次真 codex API turn, 烧配额)。

## 2. 根因定性: 设计缺陷 (时空错位)

matcher 用**过去的空间证据 (200 行 scrollback)** 指导**当前的交互行为 (current turn)**。三个共谋因子 (全部 file:line 实证):

1. **空间 (陈旧 haystack)**: `src/tmux/session.rs:421-437` `capture_pane_sync` 用 `tmux capture-pane -p -S -200`, 抓 200 行历史。codex 启动横幅 `✨ Update available! ... npm install -g @openai/codex` 整个会话期留在历史里。
2. **匹配无位置感知**: `src/prompt_handler/matcher.rs` `match_prompt` 在整 200 行 sanitized 文本跑正则, 不管命中位置是否紧邻活跃输入光标。命中陈旧横幅 → `KnownAction`。
3. **ACK 循环每 diff 从 depth 0 重扫**: `src/rpc/handlers.rs:2173-2201`。agent 答完 user prompt 后状态 BUSY/WAITING_FOR_ACK, 每次 "first meaningful diff" 调 `scan_prompt_and_apply_outcome`。命中 Handled 后 (2199-2200) 重置 `processed_len=0`+`last_meaningful_diff_at=None` → codex 回显的 `2` 成为下一次 "first diff" → 再扫 (又 depth 0) → 再命中陈旧横幅 → 无限。depth-cap / same-hash 保护是**单次 scan 内**的, ACK 循环每轮重启全新 scan, 保护跨 churn 不累积。

## 3. 否决的修补: trigger_state = Some("SPAWNING")

砸了**故意契约** `tests/prompt_handler_e2e.rs:379` (IDLE 也要 auto-skip); 且 `trigger_state` 单值无法表达 "SPAWNING 或 IDLE 但不含 BUSY/WAITING_FOR_ACK"。是按症状位置打补丁。

## 4. 否决的不充分方案: confirm_can_input 前置守卫 (单独)

`confirm_can_input` (runner.rs:538-595) → `is_input_candidate` (runner.rs:611-623) 对 codex 判据是 "**任意一行**以 `›` 开头" (存在量词, 非位置锚定)。真场景: codex 答完 PONG **回到底部活跃空闲 `›`**, 与上方陈旧横幅**共存于同一 200 行 capture**。探针 `x` 在底部活跃 `›` 回显 → Confirmed → 仍放行注入 `2` 到活跃输入行 → codex 当新 turn 处理 → churn 不消失还灌垃圾。故 can_input 是**必要不充分** (验"pane 能输入", 非"命中 prompt 是活跃前台")。

## 5. 收敛设计: 热区匹配 + ACK 期语义隔离 (修正哲学: 全量匹配 → 热区匹配)

> a3 PM-proxy audit 已收紧: **B = 必选根因修 (杀 churn 整类), C = 语义补强 (可选), A = 辅助**。不是 B/C 二选一。

### B (必选根因修, 空间锚定) — 动作触发的匹配只看活跃前台区, 不看全 200 行 scrollback
- 原则: 需要自动处理的交互式 prompt, 其触发特征**必然紧邻当前输入点**; 滚出当前可见屏的 prompt 不再是"活跃前台"。
- **为什么必选 (a3 MUST-FIX #2)**: C 只能压住"启动通知类" churn; 但**任何已处理过的 mid-task prompt 滚进历史** (如 `Confirm delete? y/n`) 再被 ACK depth-0 重扫命中 = 同源 churn, **只有 B 能杀这一整类**。只落 C / 留 trigger_state = 修症状不修根因 (违反反补丁纪律, 这是设计缺陷必须 `feat/refactor` 重画接口)。
- **必须覆盖两条动作产出路径 (a3 MUST-VERIFY #1)**: 不仅 `matcher.rs match_prompt` (KB cases), 还有 `gating.rs:114-134 lookup_prompt_experience` (学习层) —— 它也在**全量 sanitized_text** 上查、命中也产出 `KnownAction{actions}`。B 的热区收缩若漏了学习层, 一个**学过的陈旧 prompt**仍能从历史被重新触发 = churn 后门。
- 落地形态 (a1 工程定最鲁棒, 见 §6): 候选 (a) 动作匹配改用**可见屏区** (tmux 不带 -S / 按 pane 高度) 而非 -200; (b) tail-window N 行; (c) `MatchOutcome` 加 `match_offset`, 要求命中位置在 capture 末尾活跃区。权衡: 多行 prompt 题干可能略超可见屏 (别因热区太窄漏匹配真前台 prompt)。
- **守住两契约**: IDLE@379 / SPAWNING@406 场景里横幅**就是**活跃前台 (在屏内), 仍命中; 真 dogfooding 里横幅已滚进历史 (屏外), 不命中。

### C (语义补强, 可选) — ACK 循环只处理"阻碍当前任务完成"的 prompt, 不处理背景启动通知
- ACK 期 (`handlers.rs:2173` ACK 循环) 的 scan 应携带"这是 ACK 期扫描"语义; 此期 matcher 忽略**纯启动期通知类** case (update 横幅 / trust 目录)。因为 ACK 期只关心 mid-task 阻塞 prompt, 不关心背景启动通知。
- **关键 (a3 contract 洞察)**: 这是按 **scan 用途 (purpose)** 隔离, **不是按 agent state**。flag 的来源 = **ACK 循环那一层 (handlers.rs:2173) 往下穿**; `scan_codex_prompt` 直调路径 (@379 走的) **默认无 flag** → 启动 case 在 IDLE 直扫仍被处理 → @379 auto-skip 保住。**绝不能**把 flag 错绑到 agent state 上 (那是退回坏掉的 trigger_state 老路)。
- **它是运行期 scan 入参 (函数签名), 不是持久化到 KB 的 schema 字段 (a3 nit)** —— 不要去改 `PromptCase` 结构。

### A (辅助) — 保留 confirm_can_input 作注入前最后物理防线 (防 UI 闪烁/极端卡顿误触), 不作核心。

## 6. 实施待定 (a1 工程 audit 后定, test-first)

- **B 必做** (非可选); a1 定最鲁棒落地形态 (可见屏 vs tail-window vs offset), 权衡多行 prompt 题干超屏 vs 陈旧隔离的 trade-off; **明确覆盖 KB + 学习层两条路径**。
- C **可选补强**; 若做, 用 scan-context flag (运行期入参, 从 ACK 循环穿下), 不改 PromptCase schema, 不绑 state。
- A 保留现状, 仅作辅助防线。

## 7. test-first 必须覆盖 (红灯先行)
1. **守 IDLE 契约**: `known_codex_update_prompt_is_auto_skipped_in_tmux`@379 仍绿 (横幅在屏内 → 命中 auto-skip)。
2. **守 SPAWNING 契约**: `known_startup_prompt_is_auto_handled_before_idle`@406 仍绿。
3. **NEW churn-killer (startup, 端到端)**: 陈旧 update 横幅在 scrollback (屏外) + 底部活跃空闲 `›` 共存, 在 BUSY/WAITING_FOR_ACK 下 scan → **不触发** auto-skip 动作 (不注入 `2`)。真 dogfooding 回归守卫。
4. **NEW churn-killer (通用 mid-task) — 必做 (非条件性)**: 一个已处理过的 mid-task prompt 滚进历史后, 再扫**不**重新触发动作。守 B 杀整类 churn。
5. **NEW B-only 纯空间测试 (a3 MUST-VERIFY #2, 防假绿)**: 陈旧横幅**滚出热区**, 状态在 **IDLE / 非 ACK 直扫路径** (不靠 C 兜底) → 断言**仍不触发**动作。⚠️ 不能只写测试 #3 —— #3 在 BUSY 态, C 没实现也能靠 C 变绿, B 整段可能没写却假绿 (踩 §3.1.5 反凑绿)。本测试必须在 C 不生效的路径逼出 B 的红灯。
6. **NEW 学习层热区测试 (a3 MUST-VERIFY #1)**: 一个**学过的** prompt 滚出热区后再扫 → 不重新触发动作 (守 `lookup_prompt_experience` 路径也被热区覆盖)。
7. 现有 unit/integration (seeds/matcher/runner/gating/pr4a/e2e) 全绿 (clean baseline 已 revert, 红灯从干净基线起)。

## 8. 边界 (不做)
- 不碰 Phase 3 (RuntimeMarker / ReplyExtraction / try_llm_slow_path / api-key / FINALIZING)。
- 不引入新外部依赖。

## 9. 实施后 a3 src audit 结论 (2026-06-02) + 真 repro 收敛门

a1 实施 B (固定 40 行 tail-window, 覆盖 KB `matcher.rs match_prompt` + 学习层 `gating.rs lookup_prompt_experience`), 未做 C。a3 PM-proxy audit (实跑契约 e2e + 73 lib 单测) 结论:

- **闭合**: 学习层热区覆盖 (MUST-VERIFY #1) ✓; 无 state-触发漂移 (纯空间尾窗, seeds.rs revert 干净) ✓; @379/@406 契约语义未破 (物理实证 2 passed + 73 lib passed) ✓。
- **🔴 MUST-FIX (本修复收敛门)**: 所有"杀 churn"单测都用**合成 45 行 filler** 把横幅挤出 40 行窗。真 repro (`ah ask a1 PONG` 单字回合) 极短, **启动横幅与底部活跃 `›` 可能 < 40 行** → 横幅仍落活跃尾区 → B 不排除 → **churn 原样复活, 无 C 兜底**。合成单测绿 ≠ churn 已杀 (踩 §3.1.5 假绿)。
- **唯一真验证 = SOP-08 step 9 真 dogfooding**: 重跑 `ah start` + `ah ask a1 "Reply ... PONG"`, 物理确认 (a) codex pane 无 `2`+Enter 自注入 (b) `ask --wait` 返回不再永卡 BUSY + agent→IDLE。
- **若真横幅在 40 行内仍命中** → B 单独不足 → 必补 C (ACK 期忽略启动通知类) 或把 B 改 pane-height/可见屏锚定 (设计 §5-B 候选 a)。
- **must-verify (不阻塞)**: 题干本身 >40 行的长 prompt 锚点会被挤出尾窗 → 漏匹配真前台 (codex/gemini/claude 当前 builtin 不触发, 学习层长题干有此风险); 复盘 pane-height vs 固定 N=40。
- **nit**: `lookup_prompt_experience_sync` hash 路径仍按全量 hash 查 (lookup 传 active_hash, 记录侧全量 hash 不对称) — 生产只记 regex-type 经验 (hash-type 仅 `#[cfg(test)]`), 当前无实际影响; 将来加 hash 学习前需补注释/对齐。

**§7 测试矩阵补充 (a3 DESIGN SIGNAL)**: #3/#4 全是合成 scrollback, 无法替代真 dogfood; 本修复的**收敛标准 = 真 codex repro 物理复验** (synthetic 单测必要不充分)。

## 10. 真 dogfood 物理复验结论 (2026-06-02) — B 不足铁证 + C 转必选

`ah start` 真 codex (v0.135.0) + `ah ask a1 PONG`:
- **happy path PASS**: codex 答 PONG, `ask --wait` 返回 (exit 0), agent→IDLE, 无 `2` churn (当前 codex 无 update 横幅, churn 源不存在)。
- **codex TUI 实测仅 15 行** (welcome box 行 2-8, 活跃 `›` 行 13)。真横幅若在, 距底部活跃 `›` 仅 ~10 行 → **落在 40 行热区内** → B 不排除。
- **B 不足铁证 (受控复验)**: 把 `codex_update_01` regex 临时指向常驻版本行 `OpenAI Codex (v` (行 3, 热区内), ask PONG → ACK 循环命中该行 → **无限注入 `2`+Enter, agent 卡 BUSY, ask --wait 死锁** (pane 实证: `› 2` 重复 15+ 次 + codex 真消费 `• 2` 烧配额)。codex 答对 PONG 后才 churn, 完全复刻原 bug。
- **结论**: B (40 行尾窗) 对 codex 紧凑 TUI **无法空间隔离**陈旧 startup-notification (与活跃输入共屏)。**这正是设计 §5-C / a1 实施 brief step19-21 预设的 "启动横幅在很短会话里仍落在热区内、ACK 期却不该触发, B 兜不住才加 C" 场景 —— 已被真 dogfood 确认是真实场景, 非假设**。
- **行动**: C (ACK 期语义隔离) 由"可选补强"**转为必选**, 按设计 §5-C 实施 (scan-purpose flag, 从 ACK 循环穿下; 直调路径默认无 flag 保 @379/@406; 不绑 state)。B 保留作长会话纵深防御。
- **cleanup 观察 (B 维)**: `ah stop` 后 codex 子进程短暂残留 (延迟 reap, 非硬泄漏, ~数秒后被 cascade 收掉); stale ahd-* tmux socket 在 stop 时被清。
