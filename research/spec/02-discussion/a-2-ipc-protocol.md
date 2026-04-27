# A-2 议题辩论：IPC 协议与序列化方案

## Round 1 - Master Claude 立场

**决策提议**：确认使用 **Unix Domain Socket (UDS) + Newline-delimited JSON-RPC 2.0**。

### 正方推理
1. **本地 IPC 无需网络栈开销**：ccbd-rust 仅供本地同一用户进程调用，UDS 具备零 TCP 握手开销、内核级 UID 验证（SO_PEERCRED）以及传递文件描述符（FD Passing）的潜力。
2. **易调试与低门槛**：使用 `socat` 或 `nc` 即可交互式调试。所有主流语言均有成熟的 JSON 序列化工具，无需 `protoc` 工具链生成 Stub 代码。
3. **演进路径天然兼容**：JSON 的动态性允许服务器在不破坏旧 Caller 的情况下添加可选字段，符合 `R-API-COMPAT-1` 约束。

### 反方立场（gRPC + Bidirectional Streaming）
1. **强类型 Schema**：在编译期阻止字段名错误，减少运行时解析失败风险。
2. **原生流支持**：对 `R-RECONNECT-1` 的事件推送场景有原生支持。

---

## Round 2 - Gemini 评估

### 1. 逻辑漏洞与补强
- **补强（FD Passing）**：Master Claude 提到的 FD Passing 是关键。如果未来 VSCode 插件需要极速渲染 PTY 流，UDS 可以将 PTY master fd 直接传递给 Caller 进程，这是 HTTP/gRPC 等高层网络协议无法轻易做到的。
- **演进风险**：虽然 JSON 演进灵活，但需强制执行 `serde(default)` 规范，防止 Caller 因缺少可选字段而解析失败。

### 2. 其他反方候选项评估
- **Cap'n Proto / FlatBuffers**：具有零拷贝优势。若 Agent 产生兆级输出，JSON 解析可能变慢。但考虑到 PTY 的 I/O 本身受限于 VT100 解析性能，序列化层面的兆级开销在 L2 场景属于过早优化。
- **Varlink**：专为本地 IPC 设计，自带 Introspection。但在 Rust/Node 生态中，JSON-RPC 2.0 的普及度更高，能降低第三方集成门槛。

### 3. 确定分帧与 PUSH 机制
- **分帧协议**：确认 `Newline-delimited`。相比 HTTP POST 的无状态，长连接分帧允许极简的流式处理，与 `lsp-server` 模式一致。
- **PUSH 方案**：确认 **JSON-RPC Notification**。即不带 `id` 字段的消息。服务器可随时向 Caller 推送 `agent.output` 事件，实现 `session.subscribe` 的语义闭环。

---

## 最终决议

**决策结果**：采用 **Unix Domain Socket + Newline-delimited JSON-RPC 2.0 (via Notifications for Push)**。

- **理由摘要**：
  1. 保证 `socat` 级调试友好性。
  2. 极简的 Caller 集成成本（零 Stub 工具链）。
  3. 保留未来通过 `SCM_RIGHTS` 进行底层 FD 优化的空间。
  4. 天然支持单连接上的双向事件订阅流。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 补强并确认方案细节。
