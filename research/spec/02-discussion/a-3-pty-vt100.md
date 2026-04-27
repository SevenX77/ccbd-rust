# 议题 A-3：PTY 抽象与 VT100 确认闭环栈

## 第 1 轮 (Round 1) - Master Claude 立场

### 1. 核心选型
- **PTY 封装**：Fork tamux 的 `portable-pty` 封装。
- **状态机**：引入 `vt100` crate (`doy/vt100`) 维护内存屏幕。
- **闭环逻辑**：配合议题 1b 的 `R-STATE-FALLBACK-LOOP`，作为 L2 确定性状态判定的物理载体。

### 2. 辩论：为什么拒绝「纯字节流匹配」？
- **Visual 与 Sequence 的不一致**：Escape 序列（如 `\e[2K`）在字节流中是增量的，但在视觉上是覆盖。字节流匹配会导致「已消失的 Marker 被误读」。
- **Modern TUI 兼容性**：Gemini CLI / Claude Code 使用大量光标移动指令。字节流无法还原光标位置，导致无法准确判断 Marker 是否处于「当前可输入行」。

---

## 第 2 轮 (Round 2) - Gemini 架构评估

### 1. 核心判断
我完全支持该技术选型。**VT100 状态机是 L2 调度层作为“代理操作员”的视网膜**，必须具备解析视觉布局的能力，而非仅仅是监听字节流。

### 2. 五项工程决策细化

#### A-3-1：Fork tamux 的 portable-pty
- **判定**：**直接 Fork 封装而非直接引用 upstream。**
- **理由**：`tamux` 的 `pty_session.rs` 已经处理了 `OwnedFd` 的所有权转移和沙箱环境下的信号隔离。直接复用该工程资产可减少 L2 开发初期的 PTY 竞态 Bug。
- **实例**：在沙箱 `bwrap` 环境下启动子进程，需要特定的 FD 处理逻辑。

#### A-3-2：vt100 crate 选型
- **判定**：**选用 `doy/vt100`。**
- **理由**：它是 Rust 生态中最轻量、无 UI 绑定的状态机实现。相比 `wezterm-term` 这种重型组件，它更符合 A-6 决议中「单二进制纯净分发」的原则。

#### A-3-3：Screen Buffer 尺寸
- **判定**：**设定为 200 × 200。**
- **理由**：
    - **内存成本**：约 160 KB/Agent，极低。
    - **容错性**：防止长 Output（如 `cat` 大文件或长 diff）将 Prompt 挤出 24x80 默认视野，导致 L2 陷入 `Unknown` 状态。
- **策略**：超过 200 行的部分直接裁剪（Drop），因为 Agent 交互的 Prompt 必然出现在最后 200 行内。

#### A-3-4：解析触发时机
- **判定**：**实时解析 (Real-time per Chunk)。**
- **理由**：
    - **低延迟**：满足 `R-STATE-FALLBACK-LOOP` 对亚秒级响应的需求。
    - **鲁棒性**：及时捕获 `Thinking...` 状态，防止在 Agent 繁忙时 L3 误发下一条指令。

#### A-3-5：Marker 匹配执行策略
- **判定**：**混合扫描策略 (Bottom-5 + Full-Scan Audit)。**
- **方案**：
    1. **快路径 (Fast Path)**：每收到一个 PTY Chunk，仅扫描 Screen Buffer 的底部 5 行。
    2. **慢路径 (Slow Path)**：每 1 秒，或发生大规模 Scrollback 事件时，扫描全量 200 行 Buffer。
- **意义**：快路径保证响应速度，慢路径保证在极端刷屏场景下的最终一致性。

### 3. 与议题 1b 的联动
本决议确定的 PTY 栈输出是 `R-STATE-FALLBACK-LOOP` 中 **「L2 主路径（Matched）」** 的唯一来源。如果 vt100 解析后的屏幕内容不满足预设 Marker，系统必须立即进入 `Unknown` 状态并执行 Evidence Dump。
