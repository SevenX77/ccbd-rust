# ah 通用"输出模式自动抓取+沉淀"机制 — research + 思路 (a2 主笔, 主控 fact-check 通过)

> SOP-08 §1.1 sub-step 1a (research) + 1c (思路). 下一步: a1+a3 audit (1b/1d).
> 主控已 grep 验证 a2 所有 file:line 引用 (除 `CCB_PER_AGENT_SUBCGROUP` 确切 env 名未命中, cgroup 检测实际在 src/systemd_unit.rs, 方向不变).

## 用户根本诉求 (原话)

> "信任和登陆这类卡点不是应该有一套机制去自动解决和自动沉淀的吗? ... 这套流程不单单用在启动时卡登录框这些, 任务完成抓标识等等都要用这套机制, 否则单凡程序有点更新有点调整就抓瞎"

> "Gemini cli马上不能用了, 要换成antigravity cli ... 之前的Gemini 登陆框, pane标识等等都需要重新做, 正好用来测试自动抓取和沉淀的流程"

> "不要api key了, 直接让master干这个活儿" (master 订阅付费, 不增成本, 省 api key 不稳定因素, 链路更简单)

---

## 1. 核心机制: 通用"输出模式自动抓取+沉淀" (antigravity 为首用例)

### 现状 (实证 file:line)
- 提示框知识库 (KB): 写死在 `src/prompt_handler/seeds.rs:5` (`default_cases()`) + `src/prompt_handler/kb.rs`。
- 完成/空闲标记 (Marker): `src/provider/manifest.rs` 每 provider 写死 `idle_detection_mode` (bash/unknown=LineEndRegex, codex/gemini/claude=ObservedStability) + 静态正则; `src/marker/matcher.rs` 用正则扫 vt100 渲染后屏幕。
- 稳定性判定: `runner.rs` + `integration.rs` 已有 SPAWNING 期特征哈希一致性判定雏形 (`AH_PROMPT_TRANSIENT_UNKNOWN_STABLE_SCANS`)。

### 设计思路
1. **统一入口 + 推给 Master**: 当 runner/matcher 检测到屏幕输出"已稳定"(复用连续 N 次扫描哈希一致, 滤掉瞬时刷屏垃圾), 但**既不匹配提示框 KB, 也不匹配当前任务 Marker** → 触发并持久化一个 `UNKNOWN_PATTERN_STABLE` 事件 (入 event 表)。订阅了 `event.subscribe` 的 Master (Claude) 捕捉到 → 主动读 pane 截图/文本流。
2. **Master 决策 + 沉淀**: Master 判断画面是"新登录框"/"信任工作区"/"任务完成标识"/"取消确认"等 → 经类似 `ah prompt resolve --save-to-kb` (背后 `src/cli/prompt.rs` 或新 RPC `agent.save_kb_rule`) 把模式回填 DB。
3. **KB 结构统一扩容**: 重构 `PromptCase` (`src/prompt_handler/schema.rs:73` 已有 `provider: Option<String>`), 加 `Category` 枚举 (Prompt / Marker / Cancel) 统管所有交互特征。沉淀模式优先用 Regex (兼容现有 `PromptFingerprint::Regex` @ `schema.rs:111`), Master 生成正则须限定足够紧的边界 + 辅以锚点文本 Hash。

### 关键决策点
- **KB 隔离级别**: 用 `PromptCase.provider: Option<String>` 字段, Master 沉淀时打 `antigravity` 标签, 避免误杀别 agent。
- **瞬时 vs 稳定**: 泛化 `runner.rs:317` 的 `action_settle_delay` + Hash 对比, 抽象成独立 `StableScreenDetector` 供 Marker 和 Prompt 共用。

### 风险 (三轴)
- [证据 High × 影响 High × 置信 High] Master 靠 LLM 自动写正则: 过宽→提前误判完成, 过窄→一直卡死。**对策**: API 强制 Master 返回校验用例 (test cases), Rust 端二次正则验证后才入库。

---

## 2. 去掉 api-key LLM 兜底 (跟 1 连体)

### 现状
- API Key 挂载: `src/provider/manifest.rs:52` 的 `ENV_PASSTHROUGH` 含 5 个凭证 (ANTHROPIC_API_KEY:53 / ANTHROPIC_AUTH_TOKEN:54 / GEMINI_API_KEY:79 / GOOGLE_API_KEY:81 / OPENAI_API_KEY:89)。
- 慢路径: `src/prompt_handler/runner.rs:327` 调 `try_llm_slow_path` (定义在 :420)。

### 设计思路
- 删 `manifest.rs` 5 个凭证条目, 落实 SOP-04 (OAuth-only)。
- 删 `runner.rs` 的 `try_llm_slow_path` 整个调用树 + `LlmSlowPathDecision` 枚举。
- **断点改造**: 原本 :327 依赖 LLM 决断的地方 → 直接转 `PROMPT_PENDING` (挂起) + 发射 `UNKNOWN_PATTERN_STABLE` 让 Master 介入。从"内部 api-key 兜底"变成"主控调度"标准流程。

### 关键决策点
- 失败态归属: 不再内部抛错重试, 抛明确 `InterventionRequired` 状态供外层监控消费。

### 风险 (三轴)
- [证据 High × 影响 High × 置信 High] 移除后原本可能慢速通过的 case 现在必挂起 → 任务停滞时间增加, 强依赖 Master 在线。**这是设计取舍, 用户已确认接受 (master 订阅付费常在线)。**

---

## 3. Cancel: ESC + per-provider 差异化

### 现状
- `src/rpc/handlers.rs:1046` `handle_job_cancel` 硬编码 `ctx.tmux_server.send_ctrl_c(pane_id)` (Ctrl-C/SIGINT)。
- 用户实证: 人工取消按 ESC; Ctrl-C 按两下 = 退出程序 (危险)。

### 设计思路
- cancel 动作变 provider 级清单配置: `Manifest` 加 `cancel_sequence: Vec<String>` (codex 可能 `["Ctrl-C"]`, antigravity 可能 `["Escape"]`)。
- `handlers.rs:1046` 执行时查该 agent 的 `cancel_sequence` → 转 tmux send-keys 下发, 代替暴力 send_ctrl_c。
- 长远: Master 经沉淀机制发现某取消键无效时可动态覆写。

### 关键决策点
- Cancel 是否改成"杀子进程"? 暂不建议, send-keys 更平滑。

### 风险 (三轴)
- [证据 Medium × 影响 Medium × 置信 Medium] ESC 在某些 TTY 仅退入 Normal mode → job 标 CANCEL_REQUESTED 但 AI 进程仍吐字。**对策**: 结合 Marker 检测是否真回 Idle。

---

## 4. 健康检查: 加内存维度 + 覆盖 Master (进 ahd)

### 现状
- `src/provider/health_check.rs:122` `health_check_watcher_loop` 仅遍历 agent (query_agents_by_state) + 单纯比对 `last_progress_ts`。
- `src/orchestrator/mod.rs:32` 在 ahd 启动时 spawn 它 (同 :27 spawn pane_diff_watcher_loop)。
- **是 ahd 守护进程 loop, 不是 master loop** (回答用户 Q5)。

### 设计思路
- **加内存维度**: 在 `health_check_watcher_tick` 内, 读 agent systemd scope 对应 cgroup 的 `memory.current` (而非单 PID 的 /proc/<pid>/statm — 因 agent 是进程树, 单 PID 测不到 npm/node 子进程泄漏)。内存增长异常/到硬限 → 标 `STUCK (OOM Risk)`。
- **覆盖 Master**: Master 不在 agent 状态表 (经 `CCB_MASTER_CLAUDE_PID` @ manifest.rs:64 存在, 现仅 pidfd 死亡检测 @ master_watch.rs:12)。tick 开头单独查该 PID 存活+内存。Master 僵死 → 高级别告警/全局降级。

### 关键决策点
- 跨平台: 读 cgroup `memory.current` 比 /proc 稳妥准确 (systemd scope 已建 per-agent cgroup)。

### 风险 (三轴)
- [证据 High × 影响 High × 置信 High] agent 是进程树, 查单 cli PID 无法反映子进程 (npm/node) 真实内存泄漏 → 必须靠 cgroup 全量统计。

---

## a2 建议优先级
1. 去 LLM 兜底 (最易, 阻断坏味道): 清 manifest.rs api key + runner.rs 兜底。
2. Cancel 解耦 (工程量极小, 立刻缓解"按两下退出"): 改 handlers.rs:1046 引入可配键序列。
3. 跑通 antigravity + 核心机制 (最高价值): **应先把 antigravity 跑通** — 全新 CLI 各类框/Marker 全未知, 直接用它强迫系统进 PROMPT_PENDING, 以此驱动打通 Master 订阅→分析→save-to-kb 回填闭环 (dogfooding)。
4. 健康检查加内存 (防御工程): 最后稳定性保障。

## a2 read/grep 记录
manifest.rs (idle_detection_mode + ENV_PASSTHROUGH) / matcher.rs (Regex::new) / kb.rs + seeds.rs (default_cases + PromptCase) / runner.rs (L327 try_llm_slow_path) / handlers.rs (L1046 send_ctrl_c) / health_check.rs + orchestrator/mod.rs (进度时间监控)。
