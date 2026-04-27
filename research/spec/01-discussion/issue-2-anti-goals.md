# 议题 2：ccbd-rust 「绝对不做之事」清单 (Anti-Goals)

## 第 1 轮 (Round 1) - Master Claude 提议

Master Claude 完全认同初始的 5 条 Anti-Goals，并提议扩展到 9 条。以下为 9 条候选清单及实例背书：

1.  **不解析 Prompt (No Prompt Parsing)**
    *   *说明*：绝对不感知用户发送给 Agent 的自然语言内容。
    *   *实例*：L3 发送 `{"text": "/clear\n你好，请帮我写代码"}`，ccbd-rust 仅把它视为一串 UTF-8 字节，完全不会去理解“你好”是什么意思。
2.  **不管理多轮对话上下文 (No Context Window Management)**
    *   *说明*：不负责存储、拼接或截断对话历史以适应 LLM 的上下文窗口。
    *   *实例*：当对话过长导致 LLM 报错 `Context Length Exceeded` 时，ccbd-rust 只负责将这个错误字符串透传，缩减上下文重发的决策在 L3 或 Agent CLI。
3.  **不执行 LLM 推理 (No LLM API Calls)**
    *   *说明*：本身不直接建立任何与 OpenAI、Anthropic 或 Google API 的 HTTPS 连接。
    *   *实例*：如果本地网络断开，导致 Agent CLI 无法连接大模型，ccbd-rust 不会发起重连尝试，也不会记录 API 延迟，网络状态管理由 Agent CLI 负责。
4.  **不进行业务级重试 (No Business Logic Retries)**
    *   *说明*：当 Agent 逻辑报错或输出幻觉时，L2 只汇报最终输出和状态，不自动重试。
    *   *实例*：Codex 输出了一段连编译都过不了的 Rust 代码，即便报错很明显，ccbd-rust 也会如实记录为完成（DeliveryAck），是否要重试让 Codex 再写一遍是 L3 要决策的事。
5.  **不管理 API Keys (No Credential Management)**
    *   *说明*：LLM 访问凭证由 Agent CLI 的本地配置或 L3 传入环境变量管理，L2 本身不负责存储密钥。
    *   *实例*：L2 不提供 `set_api_key` 这种 RPC 方法，环境构建时 `~/.claude/` 这种凭证目录是直接从宿主机 bind-mount 到沙盒中的。
6.  **不做编排逻辑 (No Workflow Orchestration)**
    *   *说明*：不感知任务依赖图（task DAG）、并行/串行编排等业务逻辑。
    *   *实例*：L3 想跑一个流程“先让 Gemini 设计，再让 Codex 写代码”。ccbd-rust 只看到先来了一个 `agent.send(gemini-1)`，过了一会又来了一个 `agent.send(codex-1)`。它根本不知道这两个行为存在时序依赖。
7.  **不做权限鉴权决策 (No Auth/Permission System)**
    *   *说明*：完全依赖 OS 文件权限，不引入 user / role 体系。
    *   *实例*：如果某个本地恶意脚本获得了读写 `~/.local/state/ccbd/ccbd.sock` 的权限并发送了删库指令，ccbd-rust 会无条件执行。权限拦截必须在 L3 或者操作系统的 `chmod 0600` 层面解决。
8.  **不做 transcript 长期归档 (No Long-term Event Retention)**
    *   *说明*：数据库仅为了应对重连断点续传（如 R-DISPATCH-2 需求）保留短期事件缓存，不承担永久日志查询功能。
    *   *实例*：用户想查询 7 天前的对话内容，这绝不能通过 L2 的 `agent.read` 查询历史事件。L3 应该直接读取如 `~/.codex/sessions/*.json` 这种由 Agent CLI 自身持久化的文件。
9.  **不做内容审查 / 安全过滤 (No Content Moderation)**
    *   *说明*：不审查 L3 投递给 Agent 的内容是否含有注入攻击或泄露 API Key。
    *   *实例*：L3 发送了含有 AWS Access Key 的 Prompt 给 Codex，ccbd-rust 只管发。如果出了安全事故，责任归属在 L3 或 Agent CLI 内置的安全策略。

## 第 2 轮 (Round 2) - Gemini 评估与补强

我完全赞同 Master Claude 补充的 6-9 条。这不仅澄清了 L2 的责任底线，更使得“调度层”的定义无懈可击。
特别是第 8 条（不长期归档），这直接决定了 SQLite 数据库不会无限膨胀，保障了长期运行的稳定性。

基于架构师的严谨，我提议增加第 10 条，以应对后期极其容易“扯皮”的功能：

10. **不做成本/计费追踪 (No Cost/Billing Tracking)**
    *   *说明*：ccbd-rust 只管理 PTY 字节流生命周期，不负责统计消耗的 Token 数或预估 API 消费金额。
    *   *实例*：用户希望仪表盘显示当前任务消耗了多少美元。由于 ccbd-rust 只是把终端屏幕的输出存下来，它不知道 1000 个英文字符到底折算成多少 Token，更不知道不同模型的单价。这类 Metric 聚合必须由 Agent CLI （它们自己解析 LLM 的 API Response 中的 Usage 字段）或者 L3 负责。

### 最终共识结论

**决议结果：通过完整的 10 条 Anti-Goals 清单。**
双方一致认定，ccbd-rust 只有恪守这 10 条铁律，将编排、鉴权、归档、计费等智能和业务属性坚决阻挡在边界之外，才能成为一个不死不休、极度可靠的物理 I/O 调度中枢。
