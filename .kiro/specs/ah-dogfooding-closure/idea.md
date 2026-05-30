# Idea: ah dogfooding closure (自驱闭环思路)

## §A 大方向

PR-6 体量任务在 ah 上无人工介入闭环的关键在于“消除观测盲区”与“建立实时反馈链”。dogfood e2e 核心目标是验证 ah 路径在模拟 PR-6 互动模式下，能否通过 **ah ask --wait** 实现主控端 0 额外介入（0 cancel / 0 capture-pane / 0 poll），并确保 5 项关键指标（Push 延迟、Stuck 时延等）全面达标。

## §B 核心机制

- **B1 完成通知 push 链 (组 A2 + B)**: 将现有的进程内 pubsub 包装为基于 **Unix socket** 的持久事件流。`ah ask --wait` 订阅此流，一旦监听到 Job 到达终态即返回，彻底废弃“主控主动轮询”模式。
- **B2 完成检测真路径 (组 A1)**: 移除 `dispatch_and_complete_job` 伪驱动，由 `agent_io` 实时解析 Tmux Pane 流。配合 per-provider 的 **MarkerMatcher** 识别 IDLE marker，驱动 Job 状态机从 BUSY 自然演进至 IDLE。
- **B3 stuck 主动 escalate (组 C)**: 增强 `pane_diff_watcher` 信号源，整合内容哈希与 mtime。触发 Stuck 后通过 B1 通道推送 STUCK 事件至主控，由主控（而非人工）触发重试或取消。
- **B4 slash command 真投递 (组 D)**: 识别消息首字符为 `/` 时，绕过 paste-buffer，执行 **keystroke direct send**。支持 per-provider 映射（如 `/clear` 对齐各 provider 原生行为）。
- **B5 健康度多层探测 (组 E)**: 完善 `InitProbe` 链路，确保 tmux、provider 协议层及完成检测器三位一体。任一层故障均触发 B3 级联上报，解决“pane alive ≠ provider alive”假象。
- **B6 e2e dogfooding 主测 (组 G)**: 由 a1 主笔 `tests/ah_dogfooding.rs`。使用 **fake provider bash script** 模拟 SOP-08 13 步互动模式，物理对账 IDLE marker 及其与 JobID 的对应关系。

## §C 核心决策

- **C1 传输选型**: Push 通道首选 **Unix socket**。ah 定位于本地 orchestrator，Unix socket 具有最低延迟和最高可靠性，且符合 “No HTTP/SSE” 的 localhost-only 隔离设计。
- **C2 互动模拟协议**: fake provider 发送 `<<ah-idle:job-id=X>>`。ah daemon 必须解析 ID 并在 SQLite 中对账，严防旧 READY 探针误触发新 Job 完成判定（防 §3 旧 READY 误判）。
- **C3 自动度量收集**: e2e 测试内置 instrument 机制。断言主控端介入计数器恒为 0，并利用 histogram 验证 Push 延迟（p95 ≤ 500ms）及 Stuck 响应（≤ 310s）。
- **C4 默认交互模式**: 消息发送默认采用 **keystroke** 模式以规避 bracketed paste 导致的 slash 解析失效及队列堆积问题。大体量内容可选降级为 paste，但需带 marker 头部引导。
- **C5 测试加速机制**: e2e 中允许通过环境变量将 300s 的 Stuck 阈值临时 override 为 30s，在不修改 src 逻辑的前提下加速闭环验证。
- **C6 隔离优先**: 维持 master 与 ah daemon 的 1:1 物理绑定，暂不处理多主控竞争，确保 dogfood 链路的确定性。

## §D 不在 Scope (Boundary)

- **D1 真 LLM 交互**: e2e 仅限于 fake provider 协议层验证，真 LLM 留待外部端到端验收。
- **D2 多主控并发**: 仅支持单 Session 深度自驱。
- **D3 Web UI/Stdout 增强**: 专注于 master client 的 headless 闭环。
- **D4 跨机调度**: 维持 localhost 调度边界。

## §E 风险 + 风险缓解

- **风险**: 复杂的 ANSI 字符干扰 IDLE marker 识别。**缓解**: `agent_io` 内部 `vt100` parser 预处理，去除样式干扰后再进行正则匹配。
- **风险**: 频繁的 Job 变更导致 Unix socket 缓冲区溢出。**缓解**: 采用无锁队列实现 pubsub 到 socket 的分发，并引入客户端流控。
