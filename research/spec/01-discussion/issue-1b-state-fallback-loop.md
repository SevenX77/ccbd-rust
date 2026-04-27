# 议题 1b：状态识别失败时的反馈-更新-固化闭环

## 第 1 轮 (Round 1) - Master Claude 立场

### 1. 核心背景
虽然议题 1 确立了 A1 决议（L2 必须理解 Agent 协议），但由于第三方 CLI（如 Claude Code, Gemini CLI）频繁更新，纯规则匹配必然存在失效期。用户拒绝让 L3 (Master Claude) 永久承担状态判断职责，要求建立从异常识别到规则固化的闭环。

### 2. 7 步闭环方案架构
1.  **L2 主路径**：vt100 解析 + 预设 Marker 严格匹配。
2.  **L2 异常路径**：匹配失败进入 `Unknown` 状态，并自动 dump 现场 Evidence（屏幕字节流 + 尝试过的规则 + 时间戳）存入 SQLite。
3.  **L3 临时兜底**：Master 订阅到 `Unknown` 事件，启动 LLM 观察 pane 二级判断，通过 `agent.assert_state` RPC 强行告知 L2 当前状态以推进业务。
4.  **反馈分析**：Maintainer 定期 review `pending_review` 队列中的 Evidence。
5.  **测试集 fixture 化**：Reviewed Evidence 自动转为 Rust `tests/fixtures/`，确保后续规则不退化。
6.  **观察期（双跑）**：新规则上线后与旧规则并行，结果一致性达标后转正。
7.  **固化**：验证通过的新规则写入 Rust 内置 fallback config，发版即生效。

---

## 第 2 轮 (Round 2) - Gemini 架构评估

### 1. 核心判断
我完全认同该 7 步闭环。它实现了**「异常即数据，数据即用例」**的工程理想。针对 5 个具体工程关切，我的判定如下：

### 2. 工程评估问题回答

#### Q1：Evidence 数据量与 Retention 策略
*   **判定**：**24h Retention 绝对不足以支持人工 Review。**
*   **实例**：若某个 Provider 在周五晚上更新了提示符（Marker），按照 24h 策略，周一早上 Maintainer 上班时 Evidence 已被清理。
*   **修正建议**：Evidence 应作为独立资源管理。建议 `evidence` 表保留 **30 天** 或 **直至被标记为 `reviewed`**。存储成本（10-50KB/条）在 SQLite WAL 模式下微乎其微（1000 条才 50MB）。

#### Q2：`agent.assert_state` RPC 是否合理
*   **判定**：**这是必要的“Oracle（预言机）”模式，不违反 L2 闭环原则。**
*   **理由**：L2 依然是状态机的**执行者**，L3 只是在 Ambiguous 场景下扮演了**断言者**。
*   **关键约束**：L2 必须在内部状态中标记该状态为 `Idle(Asserted)` 而非 `Idle(Matched)`。这种“带污染标记”的状态流转能确保我们知道该 Agent 当前是靠“临时拐杖”走路的。

#### Q3：Step 5 的 Fixture 自动化代价
*   **判定**：**极低，且极其必要。**
*   **实现建议**：不需要复杂的自动化逻辑，只需要一个 CLI 命令 `ccbd tools export-evidence <id>` 将 DB 中的 BLOB 直接写出为文件。
*   **实例**：`tests/state_recognition_test.rs` 可以通过 `glob` 自动加载 `tests/fixtures/*.txt` 并断言其输出状态与预期一致。

#### Q4：Step 6 的双跑模式代价
*   **判定**：**在 Rust 中几乎为零。**
*   **理由**：vt100 解析的瓶颈在于 PTY I/O，而不是内存中的字符串匹配。使用 Rust 的 `aho-corasick` 或 `regex_set` 同时匹配 10 套规则的耗时在微秒级。
*   **模式建议**：这应作为 L2 的**常驻动态配置能力**（Dynamic Config），而非编译标志。

#### Q5：Step 7 的固化判据
*   **判定**：**“Unknown rate 下降 >50%” 仅作为进度指标，不能作为准入指标。**
*   **修正建议**：准入判据应为：**「在新规则覆盖范围内，L3 断言与 L2 匹配的一致性达到 100%」** 且持续 **72 小时** 无 `Unknown` 触发。

### 3. 新增工程关切：Evidence 隐私脱敏
*   **风险**：Evidence 捕获的是完整 pane，可能包含用户代码、密钥等敏感信息。
*   **对策**：`agent.assert_state` 必须是可选权限。如果 L3 判断 evidence 包含机密，可以调用 `agent.discard_evidence(id)` 禁止该数据进入 review 队列。

---

## 3. 最终收敛方案

**决议结果：通过 7 步闭环方案。**
1.  **L2 职责**：在 `Unknown` 时不仅记录屏幕，还需记录“当时所有匹配失败的规则指纹”。
2.  **L3 权限**：引入 `agent.assert_state(id, state, evidence_ref)` 接口，作为 L2 状态机的外部逃生舱。
3.  **固化路径**：从 `Evidence (DB)` -> `Fixture (File)` -> `Rule (Rust Static Context)` 的链路必须工具化。
4.  **隐私保护**：新增 `discard_evidence` 接口防止敏感信息污染测试集。
