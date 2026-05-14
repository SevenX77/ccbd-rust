# Design: ccbd Prompt-Handler Framework

本文档定义了 `ccbd` 内部通用 Prompt 处理框架（Prompt-Handler）的设计。该框架旨在自主识别并处理 Agent Pane 上的交互式提示（如更新、信任路径、EULA 等），确保自动化流程不被阻塞。

## 1. 架构定位

Prompt-Handler 作为 `ccbd` 的核心基础设施模块，位于物理 IO 层与状态机层之间。

### 1.1 模块位置
*   代码路径：`src/prompt_handler/`
*   集成点：
    *   **Monitor 协同**：作为 `agent_watch` 的补充。`agent_watch` 监控进程生死，`prompt_handler` 监控屏幕内容异常。
    *   **Orchestrator 集成**：在 `WAITING_FOR_ACK` 状态下，与 `spawn_new_capture_seed` 深度融合，识别并消除导致状态机假死的 Prompt。
    *   **Agent IO 集成**：复用 `tmux` 模块的 `capture_pane` 和 `send_keys` 能力。

## 2. 触发与识别机制

### 2.1 触发时机 (Trigger)
1.  **主动探测 (WAITING_FOR_ACK)**：在发送指令后的 ACK 窗口内。如果 `spawn_new_capture_seed` 检测到 `is_meaningful_diff` 但不是预期的 Prompt 变化，则触发 Prompt 扫描。
2.  **周期探测 (IDLE/BUSY)**：后台常驻任务每 2-5 秒执行一次轻量级 `capture-pane`。
3.  **异常触发**：当 `marker_timer` 触发 `STUCK` 超时前，强制进行一次 Prompt 扫描。

### 2.2 识别算法
*   **Hash 去噪**：首先对 Pane 内容进行 Sanitization（移除 ANSI 转义、时间戳等变动部分），计算内容 Hash。
*   **Fingerprint 匹配**：将处理后的内容与预案库中的 Fingerprint（正则表达式或特征子串）进行匹配。
*   **状态分流**：
    *   匹配成功：执行对应 Action。
    *   匹配失败：进入 LLM 识别流程。

## 3. 预案库 (Knowledge Base)

### 3.1 物理存储
*   路径：`~/.ccb/prompt-cases.json` (全局共用)
*   并发安全：采用文件锁（File Lock）确保多项目 `ccbd` 并发读写安全。

### 3.2 Schema 设计
```json
{
  "version": "1",
  "cases": [
    {
      "id": "codex_update_01",
      "provider": "codex",
      "fingerprint": {
        "type": "regex",
        "pattern": "Update available!.*runs `npm install -g @openai/codex`"
      },
      "action": [
        {"type": "key", "value": "2"},
        {"type": "key", "value": "Enter"}
      ],
      "category": "auto-skip",
      "description": "Skip codex global update to avoid EACCES",
      "confidence_threshold": 0.9,
      "used_count": 42
    }
  ]
}
```

## 4. LLM 识别流程 (Self-Learning)

### 4.1 调用链
1.  **Context 组装**：包含 `provider` 类型、`pane_content`（最后 1000 字符）、以及当前的 `agent_state`。
2.  **LLM 选择**：优先调用主控（Master）配置的 Claude API 或内部管理的 a2 (Gemini)。
3.  **识别任务**：判断是否为交互式 Prompt？如果是，选项是什么？推荐的操作按键序列是什么？是否安全？

### 4.2 LLM 输出与落盘
*   如果 LLM 给出的 `category` 为 `auto-skip` 或 `auto-accept` 且 `confidence > 0.9`，则自动执行 Action 并将新 Case 增量写入 `prompt-cases.json`。
*   否则，转入人工裁判路径。

## 5. 阻塞与人工裁判

### 5.1 PROMPT_PENDING 状态
*   新增 Agent 状态：`PROMPT_PENDING`。
*   **行为**：在此状态下，Orchestrator 停止向该 Agent 派发新 Job。

### 5.2 事件广播
*   Emit 事件：`UNKNOWN_PROMPT_DETECTED`。
*   Payload 包含：
    *   `pane_screenshot` (文本版)
    *   `suggested_action` (来自 LLM)
    *   `block_reason` (识别失败的原因)

### 5.3 裁判指令
*   新增 RPC：`agent.resolve_prompt(agent_id, action, save_to_kb: bool)`。
*   主控通过此接口告知 `ccbd` 如何操作。

## 6. 安全策略

*   **Action 白名单**：只允许发送基础按键（Esc, Enter, 方向键, 字母/数字）。严禁发送 `&`, `|`, `;` 等可能构造 Shell 注入的特殊字符。
*   **防误杀检查**：如果识别出的 Action 导致进程退出（Watcher 捕获），则标记该 Case 为 `Dangerous/Invalid`，下次不再自动执行。
*   **全局隔离**：虽然预案库共享，但执行 Action 必须在 Agent 独立的 PTY 内，受原有沙盒限制。

## 7. 实施计划

1.  **Phase 1**: 实现基于 `prompt-cases.json` 的静态正则匹配处理（解决当前 Codex Update 问题）。
2.  **Phase 2**: 实现 `PROMPT_PENDING` 状态与人工裁判 RPC 路径。
3.  **Phase 3**: 集成 LLM 识别流程，完成 Self-learning 闭环。

---

## 8. 备注 (Open Questions)

1.  **多层 Prompt 嵌套**：如果处理完一个 Prompt 后紧接着弹出第二个（例：Trust Path 后弹 Update），框架是否需要递归处理？
2.  **TTY 交互复杂度**：部分 Prompt 强依赖光标位置或背景色（如 ncurses 界面），纯文本 capture 识别率是否足够？是否需要引入 `vt100` parser 的属性检查（如 Bold/Color）？
3.  **LLM 成本**：高频 polling + LLM 识别可能带来不必要的 API 消耗。是否需要更强的本地“画面变化”前置过滤？

---

## 9. Round 2 收敛 + Final Plan (主控-a2 辩论收敛后定稿)

### 9.1 Round 2 收敛 8 条答案

| # | 决议 |
|---|---|
| Q1 第一条 case 怎么来 | **内置 bootstrapping**: ccbd 代码内置 default cases (codex update / 通用 trust path), 首次启动如 `~/.ccb/prompt-cases.json` 不存在则 ship 出来。优先级: user/LLM 新增 > 内置 |
| Q2 LLM 调用路径 | **混合 A+C** (主控原话: "Haiku 4.5 主控做 backup, 跑不通用主控就行"): 默认 ccbd 直接 HTTP 调 Anthropic API Haiku 4.5 → 失败 / 低置信度 fallback 到 emit 给主控人工判 → 主控也掉线则 PROMPT_PENDING 阻塞 |
| Q3 多 ccbd 写冲突 | **Union Merge + 冲突阻塞**: fingerprint 相同 + action 不同 → 进 PROMPT_PENDING 等主控裁决唯一"官方"action |
| Q4 Schema 字段 | 全量加 `created_at` / `last_used_at` / `created_by` ("llm-auto"/"master-manual"/"ccbd-default") / `regex_flags` (Multiline/Dotall) / `trigger_state`。Phase 1 先支持存 储, 逻辑后续填充 |
| Q5 白名单 | 基础 Keysym (Esc / Enter / 方向键 / Tab / Space / Ctrl+C/D) + 字母数字 + Literal String (yes/no/agree); Action 执行层禁止非打印特殊字符 (`&`,`;`,`|`,`$`) |
| Q6 多层 prompt 嵌套 | 递归处理 + Max Depth 3, 超过报错给主控 |
| Q7 ncurses 颜色识别 | Phase 1 纯文本够; Phase 3 LLM 路径如发现纯文本识别率不够再加 vt100 parser 属性 |
| Q8 LLM 成本控制 | Hash-Gating 4 级分流: (1) 画面没变跳过 (2) 匹配 IDLE marker 跳过 (3) 不匹配 IDLE 进正则预案匹配 (4) 预案不命中才进 LLM。需要正则过滤动态进度条噪点 |

### 9.2 Final Phase 分期 (覆盖 Section 7 的初版分期)

| Phase | 范围 | 工作量估计 |
|---|---|---|
| **Phase 1 (本轮 MVP)** | (a) 静态正则匹配 + 内置种子预案 (至少 codex update + 通用 trust path 各一条) (b) 多层 prompt 递归 (Max Depth 3) (c) PROMPT_PENDING agent 状态 (d) emit `UNKNOWN_PROMPT_DETECTED` 事件 (e) 新增 RPC `agent.resolve_prompt(agent_id, action, save_to_kb: bool)` 让主控人工裁判 (f) Hash-Gating 4 级分流 (无 LLM 路径) | a1 估 1-2 天 |
| **Phase 2** | (a) Schema 字段完善: created_at / last_used_at / created_by / regex_flags / trigger_state 全用上 (b) 多 ccbd 写冲突阻塞: file lock + fingerprint 模糊匹配 cleanup (c) 防误杀检查 (Action 导致 process 死则标 Dangerous) | a1 估半天 |
| **Phase 3** | (a) LLM 自学闭环 Haiku 4.5 (Anthropic API direct, key 走 env `ANTHROPIC_API_KEY` + `~/.ccb/config.toml` fallback) (b) LLM 调不通 / 低置信度 fallback 主控人工裁判 (c) 高置信度自动入库 (d) 必要时加 vt100 parser 颜色属性 | a1 估 1 天 |

### 9.3 Phase 1 设计意图 (主控拍板, 原话)

> Phase 1 没 LLM, 未匹配预案的 prompt 直接走"主控人工裁判"路径。这样 Phase 1 跑通后: 已知 prompt 自动跳过, 未知 prompt 不会让 agent 死而是阻塞等主控告诉 ccbd 按啥, ccbd 存预案下次自动认 。"先解决 agent 不死, LLM 自学是后续优化"。

## 10. Phase 1 e2e 反馈 + Redesign (主控-a2 三轮辩论收敛后定稿)

### 10.1 Phase 1 e2e 发现的 3 个设计缺陷 (a1 调查 job_c93352e0805c 报告)

| 问题 | 现象 | 归属判定 |
|---|---|---|
| Q1 Hash-Gating Level 1 没 dedup residual banner | gating.rs 只比相邻 capture hash, 不记 "已处理过的 prompt hash"。codex 升级 banner 发完跳过后残留 scrollback, 下次 capture hash 不变 → 走 Level 3 KB → 又 fire action → depth=3 → depth_exceeded → agent 卡 PROMPT_PENDING | 设计缺陷 (§9.2 (f) Hash-Gating 4 级缺 "handled hash 记忆" 层) |
| Q2 capture stability window + scope 不足 | runner.rs 固定 200ms post-action delay; capture-pane -S -200 含 scrollback 历史 banner 永远命中 | 设计缺陷 (§9.1 Q6 + 缺 capture scope 规则) |
| Q3 marker 误判 update/trust 画面 IDLE | codex marker `(?m)^›\s` 过宽; trust dialog / update menu 也有 `› ` 行; design 没规定 marker vs prompt-handler 优先级 | 设计缺陷 (§1.1 / §2.1 接口契约) |

### 10.2 Q1 Hash-Gating 重画 — Handled-Prompt 记忆集

- **Signature 定义**: `signature = SHA256(case_id + matched_substring)`。必须提取正则捕获到的特征文本进行 Hash，确保只锚定弹窗本体，避免 Spinner 抖动导致整屏 Hash 巨变。
- **混合存储方案 (In-memory + DB Persistence)**:
  - **Runtime 视角**: Prompt-Handler Runner 持有一个 in-memory `HashSet<[u8;32]>` (生命周期与单次 scan session 绑定)，用于解决本轮递归内的死循环。
  - **持久化视角**: DB `agents.handled_prompt_signatures` 字段 (JSON Array) 作为跨 scan 和跨重启的持久化层。
  - **读写时机**: scan 入口时从 DB load 已有 signature 到 in-memory set；执行 known action 后在 in-memory 追加 signature；scan 退出前 flush 回 DB。
  - **清空时机**: per-state-transition 状态跃迁钩子，即在 Agent state 明确跃迁至 `IDLE` 那一刻执行 DB 列清空，下次 scan 自然加载到空 set。
  - *性能与理由*: 每次 prompt scan 进出 DB 各一次，不在 capture 轮询的 hot loop 里产生 DB IO，既保证了 Agent 的跨重启幂等性，又控制了锁竞争开销。

### 10.3 Q2 Capture Scope + Post-action Stability

- **Capture Scope**:
  - Prompt-Handler 必须且仅能抓取 **visible-only** (防止 scrollback 中的历史 banner 重复命中)。实施 Contract 为调用 `tmux capture-pane -t <pane> -p` (不带 `-S/-E` 参数)。
  - Marker 则可以抓取包含近期 scrollback 的视图。
- **Post-action Stability**:
  - 发出 action 后，废弃固定的 200ms sleep。使用 "Hash 稳定 N 次或超时截断" 的自适应轮询。
  - *per-provider 调参* (下放到 ProviderManifest):
    - Codex (TUI 渲染慢): 稳定阈值 3 次，间隔 150ms，最大等待 3000ms。
    - Claude / Gemini: 稳定阈值 2 次，间隔 100ms，最大等待 1000ms。

### 10.4 Q3 Marker vs Prompt-Handler 优先级 — 两阶段串行截断

两阶段流水线的提供商适用性区分：
- **对于 prompt-scan applicable provider (codex/gemini/claude)**: 必须实施两阶段截断。Prompt-Handler 优先判定，**仅当**其返回 `NoActionNeeded` 时，才允许 Marker 介入。
  - **ACK path (`handle_ack:1144`)**: **每次**尝试将 Agent 转入 `IDLE` 或 `BUSY` 前，都必须先跑 Prompt-Handler 优先截断。
  - **BUSY timeout path (`timer.rs:174`)**: 仅在 **STUCK timeout 兜底前**扫描，不影响日常的 marker IDLE 判定路径。本轮 redesign 不修改周期性 marker poll 路径，避免引入新的状态机分叉。
- **对于 bash provider**: 明确跳过 Prompt-Handler 的扫描，直接交由 Marker 进行 `IDLE` 判定（保留 Phase 0 行为）。
- **防误伤深度防御**: 在 ProviderManifest 的 `idle_anti_pattern` 属性中补齐 `(?m)^›\s[\d]\.`。Marker 在匹配主 regex 后若命中 anti-pattern，则产生 AND-NOT 否决（判定为非 IDLE）。**注意：anti-pattern 检测必须复用 Marker 自己的 capture（包含 scrollback，这是合理的），但 anti-pattern regex 本身应当锚定当前可见区域的特征**，避免历史菜单行引起永久性否决误伤。或者实施时可确保 Marker 仅在 visible-only 区域执行 anti-pattern 的匹配。
- **Phase 3 LLM Forward-compat**: 两阶段流水线保证接入 LLM 后也无需改动核心截断逻辑，LLM 判断出的 Interactive Prompt 将天然截断底层的 Marker 误判。

### 10.5 实施集成点 (现网代码 file:line)

| 改动 | 现网集成点 | 估计行数 |
|---|---|---|
| Hash-Gating 记忆集 | src/prompt_handler/gating.rs (新增 HandledSet 参数) | ~50 |
| Post-action stability + capture scope | src/prompt_handler/runner.rs (替换 DEFAULT_ACTION_SETTLE_DELAY) | ~70 |
| matcher 返回 matched_substring | src/prompt_handler/matcher.rs (MatchOutcome 结构体扩展) | ~20 |
| 两阶段截断 (ACK path) | src/rpc/handlers.rs:1144 (handle_ack) | ~10 |
| 两阶段截断 (BUSY timeout path) | src/marker/timer.rs:179 (scan_prompt_before_busy_timeout) | ~10 |
| bash provider 跳过 prompt-handler | src/prompt_handler/integration.rs:20 (`run_prompt_scan` 入口 if provider == "bash" -> return NoActionNeeded) | ~5 |
| per-provider 调参 | src/provider/manifest.rs (新增 stability_threshold / poll_interval_ms / max_wait_timeout_ms) | ~30 |
| visible-only capture method | src/tmux/session.rs (新增 capture_visible_pane_sync) | ~15 |
| anti_pattern AND-NOT 落地 | src/marker/matcher.rs (按现有 anti_regex 字段补 codex 默认 entry) | ~5 |
| HashSet 持久化位置 | src/db/mod.rs:28-49, src/db/schema.rs:17/96, src/db/agents.rs:44, src/db/state_machine.rs:258 | ~100 |

总计估计: ~315 行 + tests

### 10.6 一周后不冒同源 bug 自检 (按 L1 SOP §3.3.6)

- **Scrollback 残留** → visible-only 根除。
- **TUI 重绘慢** → hash 稳定协议替代死 sleep。
- **Spinner 抖动导致整屏 hash 巨变** → signature 精度收窄到 case_id + matched_substring。
- **marker 盲正则误伤业务画面** → 两阶段串行截断 + AND-NOT anti_pattern 双重防御。

### 10.7 实施优先级 (推荐 a1 顺序)

1. T11: matcher.rs MatchOutcome 加 matched_substring (无依赖，最底层基础)
2. T13: tmux capture_visible_pane_sync (无依赖，底层基础)
3. T15: manifest.rs per-provider 调参字段 (含 stability_threshold，无依赖)
4. T18: handled signature storage helper + IDLE clear hook (核心依赖，必须在 T12 前完成 DB 和 in-memory trait)
5. T12: gating.rs HandledSet + signature 逻辑 (依赖 T11 的提取结果与 T18 的存储接口)
6. T14: runner.rs visible capture 调用 + post-action stability 轮询 + handled write (依赖 T13, T15, T12, T18)
7. T16: rpc/handlers.rs + marker/timer.rs 两阶段截断集成 (依赖 T11-T15 整体 scan 的完备性)
8. T17: codex idle_anti_pattern AND-NOT 补齐 (深度防御，独立)
9. T19: 单元测试 + e2e mock fixture 补充 (参考 M10 mock fixture: residual / update_menu / bash_echo_chevron)
10. T20: 真实 playground e2e 验证 (a3 主力, a1 fallback)