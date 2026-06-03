# 思路 round 2 (a2 主笔, 基于 audit-round1 重出) — 主控已 fact-check

> SOP-08 §1.1 sub-step 1c redo. 下一步: a1+a3 收敛 audit (1d).

## 一、两个第一性问题

### 1. Marker 完成检测真值来源 ("屏幕稳定 = 完成 还是 卡住?")
屏幕稳定只是**必要条件非充分条件**, 必须物理交叉校验:
- **真值来源 A (光标)**: CLI provider 任务完成通常意味光标回到 prompt 行 (屏幕底部特定正则位置)。屏幕稳定但光标悬空 → 倾向判 Thinking/Stuck 而非 Done。
- **真值来源 B (exit code)**: master 介入判断时, 若是 ah 派发的 job, 尝试读对应 shell exit code。
- **真值来源 C (交互实证锚点)**: master 沉淀规则时不仅存正则, 还存"光标相对位置" (例: antigravity 完工屏幕必现 ✦ 且光标在其后一空格)。
- **防假绿机制**: 引入 FINALIZING 状态 — 判定"完成"后不立即结算 job, 额外观察 500ms; 期间产生任何输出 → 立即撤回完成判定。

### 2. KB 生命周期治理 (纠偏 / 淘汰 / 冲突仲裁)
- **分层 (Layered KB)**: Seed 层 (只读, 程序提供) + Learned 层 (master 沉淀, 每条带 confidence + last_success_ts)。
- **纠偏/回滚**: 沉淀规则命中后导致 STUCK 或用户手动 cancel → fail_count+1; fail_count>N → QUARANTINE 隔离 (不再生效) + 触发 UNKNOWN_PATTERN 重发 master 重新学习/纠偏。
- **淘汰**: LRU — 沉淀规则 last_used_at 超 30 天且 provider 版本已升级 → 自动归档。
- **冲突仲裁**: 优先级 Prompt (阻塞类) > Marker (结束类) > Cancel; 同类冲突精确匹配 (Anchor/Regex length) 胜出, 习得层冲突最近使用 (Recent) 胜出。

## 二、其余 Must-fix 思路
- **A. 泛化 event.subscribe**: 移除 handlers.rs:1019 对 job_id 强校验 (改 Option); db/events.rs insert_event 成功后同步调 orchestrator::pubsub::notify_event; master 订阅用 `{"kind":"unknown_pattern"}` 过滤器实时被动触发。
- **B. master 不在线 Fallback**: PROMPT_PENDING 记 entered_at; 引入 escalate_watcher_loop — 超 MAX_MASTER_WAIT_S (如 60s) → Agent 转 FAILED + Job 报错 MASTER_OFFLINE_INTERVENTION_TIMEOUT (可观测, 非静默)。
- **D. 通用沉淀 RPC**: 新增 `agent.learn_rule` RPC, 参数 `{category: "Marker"|"Prompt"|"Cancel", fingerprint, action, test_cases}`; schema.rs 用 `#[serde(other)]` 确保旧数据解析不崩。
- **E. antigravity 多步登录**: "会话式 prompt 解决" — master 处理 UNKNOWN 不只回一个 action, 而是回 ExpectedNextState; 下一步仍未知则 master 关联上下文 (经 agent_id 追踪同一登录流第几步)。
- **F. 验收点**: UNKNOWN 触发 = 屏幕 hash 稳定 3 次扫描 (每 200ms) 且无 KB 命中; 正则拒绝 = master 提交正则若能在 test_cases 之外空行匹配则 RPC 拒绝; 负向测试 = 人为在 codex 输出注入假完成符, 验证 ah 因"光标位置不对"拒绝假绿。

## 三、"种子 + 自动兜未知" framing 判断 (a2 推荐)
**该 framing 合理, 是目前唯一工程可行路径。** 理由:
1. 确定性启动 vs 灵活性漂移: antigravity 登录框/OAuth URL 是必经之路, 纯冷启动依赖自动抓取风险高 (master 卡一秒用户就觉得"ah 坏了连登录都过不去")。种子保证基本盘。
2. 降低 master 熵值: 种子把环境初始化到 IDLE, 自动机制在 IDLE/BUSY 下处理漂移 (CLI 升级提示/新 quota 警告), 场景聚焦判准率高。
3. antigravity 现状: 用户已装好登录好, 种子只需含"进入后 Marker"+"可能二次授权"。

**推荐 sequencing**:
- Phase 1 (Boot): 手工为 antigravity 写极简 manifest (最基本完成符 + cancel 键)。
- Phase 2 (Infrastructure): 打通通用 event.subscribe + agent.learn_rule RPC。
- Phase 3 (Feedback Loop): 移除 try_llm_slow_path, 实现"发现未知→master 学习→规则回填"闭环。
- Phase 4 (Governance): 加规则纠偏 + 超时报警。

**结论**: 不建议自动机制直接处理 antigravity 冷启动登录, 风险太高。采用"种子保底启动, 自动机制接管后续漂移"。

## a2 read/grep: handlers.rs (订阅泛化) / schema.rs (Category 兼容) / manifest.rs (Seed 结构) / audit-round1.md (must-fix 对照)
