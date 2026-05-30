# audit round 2: a1 (工程) + a3 (PM 替身) 审 idea-round2.md — 判定未收敛, 回 1c round3

> SOP-08 §1.1 1d. 主控已 capture pane 取两份 reply 原文 + fact-check 关键 file:line。
> 结论: a1 与 a3 **强收敛** (无分歧, 不需三轮辩论)。方向对, 但有 1 命门 + 数条 must-fix, 不能进 1e formal design。

## 收敛命门 (a1 + a3 一致, 最高优先级)

**就绪/readiness 检测 (init_probe) 必须纳入"可学习自愈"表面 —— round-2 恰好把它排除了。**

- 两套独立检测路径 (a3 实证): `src/provider/init_probe.rs` = SPAWNING→IDLE 就绪门 (InitGateProbe.detect); `src/marker/matcher.rs` = 稳态 idle/busy marker。
- round-2 假设就绪门是"不会坏的保底种子", 只让到达 IDLE 之后的漂移可学习 (idea-round2 §三.2)。
- 但 dogfooding 真实烂掉的两处都在就绪门: **Gap #1** (antigravity idle `\s*$` 锚定失配 model 后缀, init_probe.rs + matcher.rs 两表面) + **Gap #5** (claude CLI v2.1.158 真到 `❯ Try "fix lint errors"` + "Opus 4.8" idle 但 ClaudeInitProbe / matcher.rs:98 `(?m)^\s*❯\s*$` 失配 → 卡 SPAWNING → 60s 超时 UNKNOWN)。
- 命门逻辑: CLI 更新 → 硬编码就绪门失配 → agent 永远到不了 IDLE → 自动捕获 (只在 IDLE/BUSY 触发) **永远不被触发** → "单凡程序更新就抓瞎" 无解。round-2 把最该自愈的表面设计在自愈范围外。
- **修**: readiness 超时若发生在"稳定但不认识的屏幕"上, 不要 dead-end 进 UNKNOWN (现状 init_probe_task.rs:296 `mark_unknown_after_timeout` 只进 UNKNOWN, 不发 master 可订阅事件), 而是路由进同一个 UNKNOWN_PATTERN_STABLE → master 判定 ("这个稳定屏幕其实是 idle") → learn 新就绪/idle pattern → SPAWNING→IDLE。就绪门成为可学习规则的消费者。

## must-fix (round3 必补)

1. **防假绿验证补"过窄/锚错"方向** (a3, 证据 High): round-2 §F 的正则校验只查"过宽" (master 正则在 test_cases 外空行也匹配 → 拒)。但 Gap #1/#3/#5 真实失败模式是**反向**的 —— 锚定太紧 (`\s*$` / `^...$`) 在带 model 后缀的真实行上永不匹配, 且当时测试假绿 (fixture 用干净行无后缀)。**learn_rule 校验必须要求 master 同时提交真实带噪正例 (如带 model 后缀的真实行), 正则必须命中真实正例才入库** —— 否则重蹈"测试绿、真实瞎"。

2. **删 exit code 真值来源** (a3, 证据 High): 交互式 agent 长驻, 无 per-job shell exit code (exit_code 挂 agents 表 = 进程级); findings 没用到。留着误导实施。完成真值靠光标位置 + FINALIZING。

3. **加 Reply/Extraction 规则类型** (a1): round-2 只覆盖 Prompt/Marker/Cancel。Gap #2 (antigravity first-ask banner / scroll-state reply 噪音) 根因不是继续加 chrome filter, 是"答案区域提取" (最后 prompt-echo 行到下一个 separator 之间)。需 CaptureRule/ExtractionRule 覆盖, 否则一直硬编码追。

4. **6 硬伤要落成可实施契约, 不只点名** (a1, 全 file:line):
   - event bus: round-2 说移除 job_id 强校验 + notify_event, 但现状仍强制 (handlers.rs:1013 + stream :1622)。formal 前需定义 frame schema / filter / backfill / ordering。
   - try_llm_slow_path: 调用树深 (runner.rs:327 + RealHaiku 硬接 integration.rs:289)。需迁移步骤 + 测试退场清单。
   - Category 兼容: 现 category 是自由 String 含 "auto-skip"/"auto-accept"/"manual-resolve" (schema.rs:70)。直接换 enum 语义断裂, `#[serde(other)]` 不够。
   - learn_rule: 必须明确**不复用** resolve_prompt (它绑 PROMPT_PENDING + 发键 + manual-resolve + 存整屏 escaped regex, resolve.rs:52/221)。新 RPC 独立。

## 该砍 (PM 视角, 镀金, 延后 v2/Phase4)

- confidence / fail_count / QUARANTINE / LRU 30天治理: 习得层还没攒规则前治理无对象 = 空转引擎。Phase 3 闭环活了再做。
- ExpectedNextState 多步登录会话追踪: antigravity 已登录 (findings 行 82), 冷启动多步无需求驱动。

## 已 solid 可保留进 formal design

prompt 类复用 (UNKNOWN_PROMPT + PROMPT_PENDING + resolve 经 learn_rule 泛化) / 光标真值 / FINALIZING 防假绿 / event 总线泛化 / master 离线 fallback (escalate→FAILED 可观测) / 4-phase 分期。

## 下一步

回 1c: a2 出 idea-round3, 把命门 (就绪门可学习化) + 4 must-fix 补进, 砍镀金。收敛后进 1e (a1 主笔 formal design.md)。
**Phase 1 仍可立即 unblock antigravity** (已 commit aa7f3ad), 但 Phase 1 的 init_probe 种子从设计起就必须"失配时路由进 learn 回路", 不留纯硬编码, 否则下个 CLI 更新又是 Gap #5。
