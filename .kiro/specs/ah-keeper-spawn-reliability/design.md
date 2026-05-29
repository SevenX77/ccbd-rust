# Design Idea: ah Keeper/Spawn Reliability E2E Tests (Task #14)

| 状态 | 1c 思路 (Idea) |
| :--- | :--- |
| **日期** | 2026-05-29 |
| **范围** | 守护进程启动可靠性、单例守卫与风暴阻断的 e2e 复现与验收 |

## 1. 痛点 + 根本动机 (Motivation)

根据 2026-05-26 事故分析，`ccbd` 陷入 keeper spawn 死循环会导致系统 PSI 内存压力飙升（达 81%），进而触发 `systemd-oomd` 级联杀除正常业务进程。

- **设计目标**：构建一套基于真实物理环境的 e2e 测试集，证明 `ccbd-rust` 当前存在的可靠性漏洞（如多 Daemon 并存、无退避风暴、超时进程泄漏），并作为后续 Must-Fix PR 的准入验收标准。
- **差异化定位**：现有 `mvp*` 和 `pr*_*.rs` 测试侧重于“功能正确性”，而本测试集侧重于“**异常路径下的物理稳定性**”。

---

## 2. E2E 测试设计原则

- **第一性原理：物理实证**：测试不依赖 mock，必须拉起真实的 `ccbd` 二进制进程，使用真实的 Unix Domain Socket 和 Tmux 资源。
- **测试边界：跨进程验证**：由于单例守卫和进程泄漏涉及进程间交互，单元测试（Unit Test）无法覆盖。必须使用 `std::process::Command` 驱动子进程。
- **物理断言 (Physical Assertions)**：
    - `ps -ef | grep ccbd`：统计进程实例数。
    - `ls <socket_path>`：验证 Socket 文件的所有权抢占。
    - `event_count`：从 SQLite 审计表中提取 `state_change`、`retry_backoff` 时序及 `circuit_open` 事件。
    - `stderr grep`：捕获 `tracing` 日志中的关键决策点（Decision Points）。
- **TDD 开发模式**：合并复现与验收场景。测试在当前代码下表现为红灯（FAIL），修复后转为绿灯（PASS）。此模式支持长期回归，防止代码退化。

---

## 3. E2E 测试点矩阵 (Test Matrix)

| 优先级 | 需求 ID | 漏洞描述 (Gap) | 测试场景 (red-to-green) | 物理断言依据 |
| :--- | :--- | :--- | :--- | :--- |
| **P0** | **R1** | `unlink-before-bind` 导致多 Daemon 并存 | 尝试启动第二个 `ccbd` 实例。 | `ps` 实例数恒 ≤ 1；抢占者应报 `AddrInUse` 退出。 |
| **P0** | **R2** | 无退避导致的 Spawn 风暴 (Thrashing) | 循环调用 RPC `agent.spawn` 且注入失败。 | 记录 10s 内 `state_change` 事件间隔，验证呈指数增长。 |
| **P0** | **R3** | 无熔断导致的无限空转 | 持续注入启动失败，触发高频重试。 | 验证 N 次失败后，`agent.spawn` 报 `SPAWN_CIRCUIT_OPEN` 且停止重试。 |
| **P0** | **R4** | 超时子进程不杀除 (Zombie) | 注入短命/不合法 provider 命令致使启动失败。 | `readiness` 超时后，原 PID 必须在物理进程表中消失。 |
| **P1** | **R5** | 冲突被误判为错误重试 | 模拟 `AgentAlreadyExists` 或 Socket 冲突。 | 验证日志记录决策为 "Healthy Skip" 而非触发重试流。 |
| **P1** | **R6** | 决策逻辑不可见 | 审计全生命周期日志输出。 | `stderr` 必须包含熔断、退避、硬杀除的结构化 Decision Log。 |

### 3.1 AC5 Bounded Soak (稳定态验收)
- **场景**: 模拟极端不稳定的 Provider（秒崩），持续压测 60s。
- **断言**:
    1. `ccbd` 进程数恒为 1。
    2. 产生 `circuit_open` 审计事件。
    3. `stderr` 不会出现由于未捕获异常导致的 Traceback 刷屏（针对 Rust Panic 的等价检查）。

---

## 4. 实施切片与复用框架

- **复用 Harness**: 深度复用 `tests/r1_master_exit_shutdown.rs` 及 `tests/mvp10_acceptance.rs` 的跨进程 Harness。
    - 使用 `env!("CARGO_BIN_EXE_ccbd")` 定位真实二进制。
    - 自动管理临时 `STATE_DIR`、Socket 等待及残留进程清理。
- **M1: 物理基建测试 (R1, R4)**：基于进程表与文件系统的硬核断言。
- **M2: 策略时序测试 (R2, R3, AC5)**：带 60s 时间窗口的“退避/熔断”测算。
- **M3: 审计与观测性测试 (R5, R6)**：验证日志决策流与状态机闭环。

---

## 5. 风险与执行注意

- **CI Slow Lane**: 跨进程测试与 Bounded Soak 耗时较长（预计总时长 2-3min）。建议标为 `#[ignore]` 默认手动跑，或在 CI 中设立专门的 `reliability-tests` Stage。
- **Flaky 防御**: 时间窗口测试易受 CPU 负载影响，退避断言应预留 20% 的抖动容忍度。

    - **推荐方案**：测试框架应支持 `unsafe_no_sandbox` 降级模式，仅测试 `ccbd` 自身的行为。
- **议题 5.2：对齐 systemd 重启策略**
    - **分析**：如果 `ccbd` 由 systemd 托管并配置了 `Restart=always`，Keeper 的应用层退避必须与 systemd 的 `RestartSec` 协同工作，避免冲突。推荐在设计阶段明确此边界。
