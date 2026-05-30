# ah 通用抓取机制思路 round 3 (解命门 + 4 must-fix + 砍镀金)

> SOP-08 §1.1 1d 后重回 1c。基于 a1+a3 的 audit-round2 审计结论重出。
> 目标: 彻底收敛设计思路, 进入 1e formal design 落地。

## 一、收敛命门: 就绪门 (Readiness) 可学习化

**核心痛察**: 目前 `src/provider/init_probe.rs` 及其关联的 `InitGateProbe` 是硬编码的"保底种子"。如果 CLI 更新导致提示符或状态行微调 (如 Gap #5 Claude CLI 升级), agent 会卡在 SPAWNING 状态 60s 后进入 UNKNOWN 死亡终局, 因为自动学习机制只在 IDLE/BUSY 稳态下触发。

### 1. 路由改造 (init_probe_task.rs:296)
- **现状**: 超时即 `mark_unknown_after_timeout`, 产生 UNKNOWN 状态但无事件驱动 Master 介入。
- **改造**: 若 SPAWNING 期间屏幕哈希连续 3 次稳定 (500ms 间隔) 且不匹配现有种子 → 触发 `UNKNOWN_PATTERN_STABLE` 事件, 状态保持在 SPAWNING_INTERVENTION。
- **Master 介入**: Master 识别屏幕为"已就绪的 IDLE 态", 通过 `agent.learn_rule` 提交新规则。
- **状态跃迁**: 消费点在 `init_probe_task` 轮询中增加 `LearnedRule` 匹配。一旦命中 → SPAWNING → IDLE。

### 2. 三类规则统一模型 (Category 泛化)
在 `src/prompt_handler/schema.rs:75` 的 `category` 字段明确以下枚举值:
- `StartupReadiness`: 用于 SPAWNING 期的就绪检测 (InitGateProbe 的习得替代品)。
- `RuntimeMarker`: 用于稳态下的 IDLE/BUSY 切换 (matcher.rs 的习得替代品)。
- `ReplyExtraction`: 用于从屏幕流中提取纯净答案区域。

---

## 二、4 Must-fix 落地契约

### 1. 防假绿: "带噪正例"强制校验
- **RPC 变更**: `agent.learn_rule` 参数必须包含 `positive_examples: Vec<String>` (包含真实 model 后缀、乱码、前缀等带噪行)。
- **校验逻辑**: Rust 端入库前验证 Master 提交的 Regex 必须能命中 `positive_examples`。
- **意义**: 彻底解决 Gap #1/#3/#5 这种"锚定太紧 (锚死行尾) 导致真实带后缀行匹配不上"且测试假绿的问题。

### 2. 删 `exit_code` 真值来源
- **清理**: 从 `agents` 表和 `Job` 结构中移除 `exit_code` 预期。交互式 Agent 长驻, 进程不退出则无 Shell exit code。
- **新真值**: 依靠"光标位置" (Cursor Invariant) + 屏幕稳态 + `FINALIZING` (500ms 观察期无新增输出) 作为完成判据。

### 3. 加 `ReplyExtraction` 规则
- **机制**: 习得规则定义起始锚点 (Last Prompt-echo) 和结束锚点 (Next Separator/Status Line)。
- **作用**: 彻底解决 Gap #2 (antigravity 答案中夹杂 TUI 框线、Thought 行、Banner 噪音)。不再硬编码 filter 追杀 chrome, 而是划定答案提取区间。

### 4. 6 硬伤落成可实施契约
- **Event Bus 泛化**: 移除 `handlers.rs:1013` (及 1622) 对 `job_id` 的强校验, 允许 `job_id` 为空以订阅全局事件 (`UNKNOWN_PATTERN_STABLE`)。
- **try_llm_slow_path 迁移**: 标记 `runner.rs:327` 和 `integration.rs:289` 为 DEPRECATED。新逻辑直接抛出 `InterventionRequired` 错误, 触发 master 学习流。
- **Category 兼容**: `schema.rs:75` 的 `category` 保持为 String 但在验证层使用 `enum Category` 校验。新增 `manual-resolve` 作为 legacy 兼容项。
- **learn_rule 独立性**: 在 `src/rpc/handlers.rs` 新增 `handle_agent_learn_rule` RPC 处理器, 彻底剥离 `resolve.rs:221` 强绑 `STATE_PROMPT_PENDING` 的逻辑。

---

## 三、砍掉镀金 (延后 Phase 4+)

1. **治理逻辑 (Governance)**: `confidence` / `fail_count` / `QUARANTINE` 隔离区 / `LRU` 30天淘汰。
   - **理由**: 习得层初期规则少, 治理逻辑会导致空转和过早优化。优先跑通"抓取-学习-回填"闭环。
2. **多步登录状态追踪 (ExpectedNextState)**:
   - **理由**: Antigravity 现状已登录。多步交互可由 Master 侧记录会话上下文, 无需在 ah 核心协议层引入复杂状态机。

---

## 四、实证 file:line 归档 (a2 验证)
- `src/rpc/handlers.rs:1013/1622`: `job_id` 强校验点 (待松绑)。
- `src/prompt_handler/runner.rs:327`: `try_llm_slow_path` (待删除)。
- `src/prompt_handler/schema.rs:75`: `pub category: String` (待扩容枚举)。
- `src/prompt_handler/resolve.rs:52/221`: `resolve_prompt` 与 `STATE_PROMPT_PENDING` 强绑定点 (新 RPC 须独立)。
- `src/provider/init_probe.rs`: 硬编码就绪门 (待引入习得规则消费者)。
- `init_probe_task.rs:296`: `mark_unknown_after_timeout` (待路由至学习回路)。

---
**结论**: 本 round 3 思路已覆盖 audit-round2 所有命门与必修项。收敛后推荐进入 1e formal design。
