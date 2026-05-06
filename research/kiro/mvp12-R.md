# Kiro Requirements: MVP 12 (Python ccb 1:1 翻译续作)

## 0. 立项背景
MVP 11 在实施过程中严重偏离了“全量复刻”的初衷。在处理 Provider 复杂的 TUI 交互与生命周期时，设计方案没有尊重 Python 版积累的工程实践，反而凭直觉发明了 `StartupSequenceEngine`、静态 `marker_pattern` 和各种按键拦截器。这些“架构发明”不仅引入了 6 个致命的基础功能 Bug（如回车判断错误、挂起死锁等），还破坏了原有的稳定性。

MVP 12 的核心任务是**纠偏**。本阶段的最高铁律是：**向 Python ccb 的生产级稳定性进行“架构降级”，做到 1:1 翻译，坚决不发明**。Python ccb 在 `claude_code_bridge/lib/` 目录下已经是经过实战检验的 Ground Truth，必须逐行对照其实现机制来替代 Rust 侧的错误抽象。

在重塑 Provider 适配层的同时，MVP 11 中证明正确且属于 Rust 生态红利的底层机制（如基于 Systemd `BindsTo` 的孤儿进程回收、SQLite 事务 CAS 等）予以保留。重点在于彻底铲除 Rust 侧自创的 TUI 扫描和序列伪装逻辑。

## 1. AC 矩阵 (端到端验收)

| AC | 描述 | Python 来源 (file:line) | mvp11 走偏点 (Rust file:line) |
|----|------|------------------------|------------------------------|
| AC1 | 默认 sandbox `ccb-rs start --wait` 三 agent IDLE | `provider_core/init_gate.py:53` | `src/marker/startup_engine.rs:1` (发明的引擎) |
| AC2 | `ccb-rs ask` 真 send + 收 reply | `terminal_runtime/tmux_send.py:137` + `ccbd/services/dispatcher.py:126` | `src/agent_io/writer.rs:12` |
| AC3 | `ccb-rs cancel` 中途任务 | `ccbd/services/dispatcher.py:118` | `src/rpc/handlers.rs:580` |
| AC4 | `ccb-rs kill --session` 内核级回收 | (mvp11 已实施 BindsTo 路径) | N/A |
| AC5 | 跨 daemon 重启 detach 验证 | `workspace/reconcile.py:55` | `src/db/system.rs:200` (TODO) |
| AC6 | grid layout agent→pane 确定性绑定 | `terminal_runtime/layouts_split.py:20` | `src/tmux/layout.rs:37` (只发了 tmux tiled 命令) |

## 2. 6 个 Bug 的 Requirement (R-1 ~ R-6)

### R-1 (Bug 1: writer Enter 判断错)
- **R-1.cur (现状)**: Rust 侧 `src/agent_io/writer.rs:12` 错误地根据 `text.ends_with('\n')` 来决定是否发送 `Enter`，导致 CLI 提交失效。
- **R-1.py (Python 真做法)**: `terminal_runtime/tmux_send.py:137` 明确在 `paste-buffer` 后无条件追加 `send-keys ... Enter`。
- **R-1.fix (修复方向)**: 移除 `ends_with('\n')` 判断，发送文本后无条件触发一次独立的 `Enter` 键发送。
- **R-1.acc (验收)**: 发送不带换行符的 prompt，验证 Codex 确实收到了该命令并开始处理。

### R-2 (Bug 2: reply dispatcher 链路断)
- **R-2.cur (现状)**: Rust 缺乏显式的 reply 处理链路，依赖 `reader.rs` 里的稳定性扫描去触发状态转移，遇到无后续输出的情况卡死。
- **R-2.py (Python 真做法)**: `ccbd/services/dispatcher_runtime/finalization_runtime/service.py:38` 在 `complete_job` 时，会检查 `auto_reply_delivery_on_complete`，通过 `prepare_reply_deliveries` 和 `message_bureau.ack_reply` 主动回收。
- **R-2.fix (修复方向)**: **明确调度边界——禁止底层 IO Reader 直接操作 Job 终端状态，收归 Orchestrator 统一调度**。在 Rust 的 dispatcher (Orchestrator) 完结 job 时，追加对应 `auto_reply_delivery` 语义的 ack 和收尾动作；reader 仅发出 BUSY→IDLE 状态信号，不调 `mark_job_completed`。
- **R-2.acc (验收)**: `ccb-rs ask --wait` 后，任务必须稳定返回 `COMPLETED`，不会无限挂起。

### R-3 (Bug 3: grid layout)
- **R-3.cur (现状)**: `src/tmux/layout.rs:37` 中 `apply_layout` 仅粗暴发送 `select-layout tiled`，导致 Pane 顺序失控。
- **R-3.py (Python 真做法)**: `terminal_runtime/layouts_split.py:20` 的 `build_split_layout` 实现了一套精确的树形 Pane 切分逻辑 (right, bottom)，并显式给 provider 分配。
- **R-3.fix (修复方向)**: 1:1 翻译 `build_split_layout` 算法，在 Rust 侧通过一连串具体的 `split-pane` 和 `select-pane` 实现精确布局。
- **R-3.acc (验收)**: 多 Agent 模式下，左大、右上、右下的 Pane 位置与 Python 版在相同显示器尺寸下 100% 一致。

### R-4 (Bug 4: init_probe S1-S3)
- **R-4.cur (现状)**: Rust 用静态 Regex 甚至瞎猜的字面值作为 readiness 依据 (`src/provider/manifest.rs:168`)。
- **R-4.py (Python 真做法)**: `provider_backends/claude/init_probe.py:100` 通过 `_banner_gone`, `_prompt_present`, `_steady_marker_present` 三段式检测完成 readiness 判定。
- **R-4.fix (修复方向)**: 废弃 Manifest 里的静态 regex，为各 Provider 实现类似 Python 的分步屏幕内容侦测 Probe。
- **R-4.acc (验收)**: 在含大段干扰信息的冷启动过程中，各 Provider 均能准确识别真正的 IDLE 时刻。

### R-5 (Bug 5: home_layout materialize)
- **R-5.cur (现状)**: `src/sandbox/bwrap.rs` 仅做了宿主挂载，忽略了 Provider 配置文件内容的沙盒适配。
- **R-5.py (Python 真做法)**: `provider_backends/gemini/launcher_runtime/home.py:109` 的 `_materialize_trusted_folders` 会合并宿主的配置并写入新的沙盒版配置。
- **R-5.fix (修复方向)**: 增加在拉起 `bwrap` 之前的准备阶段，执行针对 `trustedFolders.json` (Gemini) 等文件的 materialize 逻辑。
- **R-5.acc (验收)**: 在沙盒内启动的 Agent，能够无障碍地读取项目文件，不弹出 Trust 提示。

### R-6 (Bug 6: reconcile SessionWatch)
- **R-6.cur (现状)**: `src/db/system.rs:200` 留了个 `TODO(G11.0 follow-up): reattach SessionWatch tasks`，daemon 重启后无法监听已存在的 anchor unit 死亡。
- **R-6.py (Python 真做法 / 部分等价)**: ⚠️ Rust 的 systemd anchor + agent.scope `BindsTo=anchor` 模式是 mvp11 引入的 Rust-only 机制，Python ccb 无完整等价物（Python 用 `app._terminate_pid_tree` 应用层 SIGKILL 树）。可参考 `workspace/reconcile.py:55` `reconcile_start_workspaces` 的"扫 persisted specs 与 desired specs 对齐"思路，但 SessionWatch reattach 本身是 Rust 特有需求。
- **R-6.fix (修复方向)**: daemon 启动 reconcile 阶段，通过 DB 查 ACTIVE session，对每个 session 的 `ccbd-session-<id>.service` 重新挂 `systemctl is-active` 轮询 task。注意：anchor unit 已存在的话不能 `systemd-run` 重建，只能 attach 监听。
- **R-6.acc (验收)**: `ccb-rs start` → `kill -9 daemon` → daemon auto-restart → `systemctl --user stop ccbd-session-<id>.service` → daemon 同步 DB 标记 `KILLED` + 清 tmux pane。

## 3. 保留的 mvp11 资产 (continuity)
- **(R-1 决断的) Systemd Anchor & `BindsTo`**: 将 `agent.scope` 绑定到孤立的 `ccbd-session-<id>.service`，解决了 Python 中原有的复杂清理问题（依赖 `app._terminate_pid_tree`）。这是利用 Rust/Linux 系统调用的合理降维打击。
- **(R-2) `env_passthrough` & `injected_env_vars`**: 这两个字段对应了 `runtime_env/control_plane.py` 的行为，且在 bwrap 隔离下是必需的，设计方向正确。
- **(R-5) 8 维度 Rubrics**: 强制的“生产对齐”考核标准非常关键，是保证本次和未来开发不走样的基石。
- **DB 级 CAS 短路保护**: `UPDATE sessions SET status='KILLED'` 的原子锁消除了 TOCTOU 漏洞，优于简单的状态同步。

## 4. 废弃的 mvp11 走偏 (要清掉)
- **`src/marker/startup_engine.rs` & `StartupSequenceEngine`**: 完全是“过度发明”，试图用一套硬编码的按键时序来伪装真正的启动交互，而 Python 中对应的 `InitGate` 和 `InitGateProbe` (`init_gate.py:53`) 是基于真实屏幕内容的侦测。必须彻底废弃。
- **`SendKeysVerified`**: Python 版的 `_verify_delivery` (`tmux_send.py:260`) 是一个可选特性，并且有专门的捕获与重试机制。Rust 侧抽象成了核心 `StartupStep` 带来了大量的不确定性。
- **静态 `marker_pattern`**: 废弃硬编码的 `type your message` 等字符串（`manifest.rs`）。它们不能应对 Python 侧复杂的 S1-S3 状态机判定（见 `init_probe.py`）。
- **`src/agent_io/writer.rs:12` 的 Enter 判断**: 违反了 `tmux_send.py:137` 的设计意图，需被修正。
- **`reader.rs` 里的“猜”稳定性逻辑**: Python 端依赖独立的 Timer 和精确的 Dispatcher 轮转，而非 Reader 过程中的粗暴倒计时拦截。

## 5. 6 stage 拆分 (M12.0 ~ M12.6)

> 拆分原则：1 bug = 1 stage 隔离粒度（按 followup-prompt + TaskList 对齐）。R.md + D.md 一起作为 M12.0 设计阶段产出。M12.1-M12.5 各自独立修一个 bug，可并行验证。M12.6 整合验收。

- **M12.0**: 行为映射文档（R.md + D.md）。Gemini 主导 read Python ccb 关键路径，每条声明必须能映射回 Python file:line。当前阶段。
- **M12.1**: home_layout materialize（R-5 / Bug 5）。Codex 主导，新建 `src/provider/home_layout.rs`，对照 `provider_backends/{claude,gemini}/launcher_runtime/home.py` 翻译 `_materialize_trusted_folders` / `prepare_*_home_overrides` / `resolve_*_home_layout`。Blocked by M12.0。
- **M12.2**: Send/Reply 路径翻译（R-1 + R-2 / Bug 1+2）。Codex 主导，重写 `src/agent_io/writer.rs` send 路径（无条件 Enter）+ `src/agent_io/reader.rs` reply 路径（dispatcher 完成信号模式）。对照 `terminal_runtime/tmux_send.py` + `ccbd/services/dispatcher_runtime/finalization_runtime/service.py` + `ccbd/keeper_runtime/loop.py`。Blocked by M12.0。
- **M12.3**: Grid layout 序号绑定（R-3 / Bug 3）。Codex 主导，改造 `src/tmux/layout.rs::apply_layout` 为 Agent→Pane 确定性映射。对照 `terminal_runtime/layouts_split.py::build_split_layout` + `cli/services/runtime_launch_runtime/tmux_panes.py`。Blocked by M12.0。
- **M12.4**: init_probe S1-S3 状态机（R-4 / Bug 4）。Codex 主导，写新模块 `src/provider/init_probe.rs` 替代 `src/marker/startup_engine.rs`（654 行废弃）。对照 `provider_backends/{codex,claude,gemini}/init_probe.py` + `provider_core/init_gate.py`。同步废 Manifest 里 `startup_sequence` / `interactive_prompt_handlers` / `marker_pattern` 字面值字段。Blocked by M12.0。
- **M12.5**: reconcile 重装 SessionWatch（R-6 / Bug 6）。Codex 主导，补回 `src/db/system.rs:200` 的 `TODO(G11.0 follow-up)`。Rust 特有需求（systemd anchor 模式无 Python 1:1 等价），思路参考 Python `workspace/reconcile.py`。Blocked by M12.0。
- **M12.6**: 端到端真实测验收。master Claude 主导，跑 followup-prompt 6 步实测清单（start / ps / ask×3 / cancel / kill --session / 跨 daemon 重启 detach）。Blocked by M12.1-M12.5。

## 6. 风险与已知遗留
1. **Grid Layout 版本兼容性风险**: 不同的 tmux 版本（如 3.2 vs 3.3a）对于 `split-pane -p`（百分比）的取整处理有细微差异。Python 版的布局算法在极小终端下可能会触发 `no space for new pane` 异常（参考 `tmux_panes.py:179` 的 Fallback 逻辑）。在 Rust 中硬翻译可能需要复刻类似的兜底机制。
2. **bwrap 路径伪装与 Host Overlay 冲突**: 虽然实现了 `materialize_trusted_folders`，但在涉及极深层次的项目软链接时，bwrap 沙盒内的 namespace 可能与宿主的绝对路径产生漂移。
3. **Regex 引擎差异风险**: Python 原版的 TUI 探测有时依赖于特定的空白字符处理或 unicode 特性。翻译为 Rust 的 `regex` crate 时，需要极其谨慎地对待多行（`(?m)`）匹配和不可见字符。
4. **`InitGate` 的时序竞争**: Python 的 `InitGate` 在单线程事件循环中运行良好，但 Rust 的多线程 Tokio 模型可能会在快速的 PTY 输出中造成 `capture_visible` 数据落后的问题，这需要在实现时引入合理的重试机制（参考 `init_gate.py:300`）。
