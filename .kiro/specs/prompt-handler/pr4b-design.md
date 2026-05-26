# Design: PR4b - DB 自学习 (Learning Layer) + LLM 慢路径 (Slow Path)

本文档定义了 `ccbd` Prompt-Handler 的扩展设计，引入基于数据库的“自学习层”和基于 LLM (Claude Haiku 4.5) 的“慢路径识别”，完成 Prompt 处理的闭环。

## 1. 继承与现状 (Inherited Context)

### 1.1 继承字段表 (PR4b 不动)
以下字段继承自 `prompt-cases.json` (§3.2) 及 Phase 2 扩展，PR4b 在查找逻辑中需完全兼容。

| 字段 | 类型 | 说明 | 来源 |
|---|---|---|---|
| `id` | String | Case 唯一标识 | §3.2 |
| `provider` | Option<String> | 适用 Provider (codex/gemini/claude/...) | §3.2 |
| `fingerprint` | JSON Object | 匹配特征 (目前仅支持 `type: "regex"`) | §3.2 |
| `action` | Vec<Action> | 执行动作序列 (Key/Literal) | §3.2 |
| `category` | String | 分类 (auto-skip/auto-accept/manual) | §3.2 |
| `confidence_threshold`| Option<f64> | 置信度阈值 (默认 0.9) | §3.2 |
| `regex_flags` | Vec<String> | Regex 编译标志 (Multiline/Dotall/...) | §9.1 Q4 |
| `trigger_state` | Option<String> | 触发状态 (IDLE/BUSY/WAITING_FOR_ACK) | §9.1 Q4 |

### 1.2 现有生命周期契约 (PR6b 落地)
*   **Steady 态 (IDLE/BUSY)**: 允许触发 LLM 识别。
*   **Transient 态 (SPAWNING/WAITING_FOR_ACK)**: 即使识别为未知 Prompt，也必须 `Defer`（延迟处理），不触发 LLM，避免在进程启动或指令确认的瞬态产生误判。
*   **Max Depth**: 递归处理封顶为 3 层。

---

## 2. 学习层：`prompt_experience` 数据库表 [NEW]

为了实现高效的自学习，系统不再将 LLM 学习到的 Case 写回静态的 `prompt-cases.json`，而是存入数据库。

### 2.1 Schema 设计 (SQLite STRICT)
```sql
CREATE TABLE IF NOT EXISTS prompt_experience (
    id TEXT PRIMARY KEY,
    provider TEXT,                   -- 适用 Provider，NULL 表示通用
    fingerprint_type TEXT NOT NULL,  -- 'hash' (全屏哈希) 或 'regex' (正则表达式)
    fingerprint_value TEXT NOT NULL, -- 哈希值字符串或正则模式
    action_json TEXT NOT NULL,       -- PromptAction 的 JSON 序列化数组
    category TEXT NOT NULL,          -- 分类
    confidence REAL NOT NULL,        -- LLM 产生的置信度
    source TEXT NOT NULL,            -- 来源: 'llm-haiku-4.5', 'master-manual'
    used_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    last_used_at INTEGER NOT NULL DEFAULT (unixepoch()),
    trigger_state TEXT,              -- 触发时的 Agent 状态
    UNIQUE(provider, fingerprint_type, fingerprint_value)
) STRICT;
```

### 2.2 匹配策略
查找顺序调整为：
1.  **快路径 (L1: JSON Regex)**: 匹配 `prompt-cases.json` 中的内置/用户自定义正则。
2.  **学习层 (L2: DB Experience)**: 匹配 `prompt_experience` 表。优先匹配 `fingerprint_type = 'regex'`，其次匹配 `fingerprint_type = 'hash'`。
3.  **慢路径 (L3: LLM)**: 前两者均不命中且 Agent 处于 Steady 态时触发。

---

## 3. 慢路径：LLM 识别 (Claude Haiku 4.5) [NEW]

### 3.1 LLM 调用契约
*   **模型**: `claude-haiku-4-5-20251001` (Anthropic API)。
*   **Context 组装**:
    *   `provider`: 当前 Agent 的 Provider。
    *   `agent_state`: 当前 Agent 状态 (IDLE/BUSY)。
    *   `pane_snapshot`: 屏幕清洗后的文本 (`sanitize_pane_text` 后的内容，取末尾 2000 字符)。
*   **Prompt 策略**:
    *   系统 Prompt 定义 Prompt-Handler 的职责：识别交互式弹窗、更新提示、法律条款，并给出安全的按键序列。
    *   要求输出结构化 JSON。
*   **输出 Schema**:
    ```json
    {
      "is_interactive_prompt": bool,
      "category": "auto-skip" | "auto-accept" | "manual",
      "action": [{"type": "key", "value": "Enter"}, ...],
      "confidence": float,  // 0.0 - 1.0
      "safe": bool,         // 是否确认不含危险指令
      "suggested_regex": string // (可选) 用于存入 DB 的正则特征
    }
    ```

### 3.2 置信度与执行
*   **Confidence ≥ 0.8 且 safe = true**: 自动执行动作，并异步存入 `prompt_experience` (source='llm-haiku-4.5')。
*   **Confidence < 0.8 或 safe = false**: 标记为 `PROMPT_PENDING`，进入人工裁判路径。
*   **网络/超时错误**: 降级为 `PROMPT_PENDING`，报错原因为 `llm_error` 或 `llm_timeout`。

---

## 4. 技术栈选型 [NEW]

### 4.1 HTTP Client: `reqwest`
*   **理由**: 项目已深度依赖 `tokio` 异步运行时。`reqwest` 是 Rust 生态中与 `tokio` 集成度最高、最成熟的异步 HTTP 客户端。
*   **模块位置**: `src/prompt_handler/llm_client.rs`。

### 4.2 API Key 管理
*   **来源 1 (环境变量)**: `ANTHROPIC_API_KEY` (最高优先级)。
*   **来源 2 (配置文件)**: `ccb.toml` 或 `~/.ccb/config.toml` 中的 `[auth.anthropic] api_key` 字段。
*   **缺失处理**: 若均缺失，跳过 LLM 调用，直接返回 `Pending (reason="missing_api_key")`。

---

## 5. 逻辑流与集成点

### 5.1 递归 Runner 扩展 (`src/prompt_handler/runner.rs`)
`handle_prompt_chain` 循环中引入三层级联：
```rust
// 逻辑伪代码 (非代码修改)
loop {
    let capture = io.capture_pane();
    let decision = classify_capture_with_layers(ctx, capture); // 内部按 JSON -> DB -> LLM 顺序查找
    match decision {
        KnownAction => execute_and_continue(),
        Unknown => {
            if is_steady(state) && has_key {
                let llm_outcome = call_llm(capture).await;
                if llm_outcome.confidence >= 0.8 {
                    save_experience(llm_outcome);
                    execute_and_continue();
                } else {
                    return Pending;
                }
            } else {
                return Pending;
            }
        }
        Skip => return NoActionNeeded,
    }
}
```

### 5.2 集成点实证 (File:Line)
*   **Gating 逻辑**: `src/prompt_handler/gating.rs` 需扩展 `classify_capture` 以支持数据库查询。
*   **生命周期检查**: `src/prompt_handler/integration.rs:141` 的 `is_prompt_demote_deferred_state` 必须在触发 LLM 前再次校验。
*   **DB 接入**: `src/db/mod.rs` 需增加 `prompt_experience` 相关查询与写入接口。

---

## 6. 验证与 VPS 测试

### 6.1 VPS 真 Haiku API 验证
*   **场景**: 在真实 VPS 上，使用真实的 Anthropic API Key，针对常见的未知 EULA (如首次启动的 git/npm 提示) 进行端到端验证。
*   **目标**: 确认 LLM 能够正确识别屏幕内容并给出有效的 `PromptAction`。

### 6.2 VPS 本地 Mock 验证
*   **场景**: 即使在 VPS 上，由于 LLM 成本和非确定性，CI/CD 应主要依赖 Mock HTTP 服务。
*   **实施**: 使用 `mockito` 或类似的 mock server 模拟 Anthropic API 的 JSON 返回，覆盖 `Confidence < 0.8`、`Timeout`、`Invalid JSON` 等异常分支。

---

## 7. 关键决策清单 (Decision Log)

1.  **[DECISION] 学习层不合并回 JSON**: 为了保持 `prompt-cases.json` 的整洁和“分发性”，LLM 学习到的成果仅留存在本地数据库。
2.  **[DECISION] 选用 `reqwest` (Async)**: 尽管现有 `runner.rs` 处于 `spawn_blocking` 中，但长远看 `prompt_handler` 应当全面异步化。
3.  **[DECISION] 严格 0.8 阈值**: 优先保证安全性，不确定的情况宁可阻塞等待人工介入。
4.  **[DECISION] 状态敏感性**: 尊重 PR6b 的 `transient` 状态保护，不在 `SPAWNING` 阶段调 LLM，防止浪费 Token。
