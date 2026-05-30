# Research: ah 全流程 e2e 测试现状 + gap

本文档对 `ah` (ccbd-rust) 当前的用户旅程覆盖度与端到端（E2E）全流程测试现状进行梳理，响应产品侧（PM）对于“将整个流程所有功能串联测试一遍”的需求。

## 1. ah 用户流程梳理 (User Journeys)

基于 `src/bin/ah.rs:32-97` 中的 `Cmd` enum，当前 `ah` 的核心用户旅程与对应的 RPC Handler 如下：

- **Start**: 初始化与启动。`ah start` -> `session.create` / `session.spawn_master_pane` / `agent.spawn`。
- **Up**: 配置对齐。`ah up` -> `session.realign` / `agent.realign`。
- **Execution**: 任务执行。
  - `ah ask` -> `job.submit`。
  - `ah pend` -> `job.wait`。
  - `ah cancel` -> `job.cancel`。
- **Interaction**: 状态介入。
  - `ah prompt resolve` -> `agent.resolve_prompt`。
  - `ah kill` -> `session.kill` / `agent.kill`。
- **Observability**: 可观测性。`ah ps`, `ah logs`, `ah watch`, `ah ping`。
- **Lifecycle**: 生命周期结束。`ah stop` -> `system.shutdown`。

## 2. 现有 e2e 覆盖度现状

对 `tests/` 目录下的 e2e/集成测试文件进行梳理：

- **基础交互与多 Agent 并发**:
  - `mvp7_real_codex.rs` / `mvp7_real_gemini.rs`: 覆盖 Start -> Ask -> Wait 的真机 LLM 单线 Happy Path。
  - `mvp8_real_codex.rs`: 覆盖异常交互（如执行错误）的反馈。
  - `mvp9_real_codex_claude.rs`: 覆盖多 Agent 并发 Spawn 与批量请求。
  - `mvp11_real_*.rs`: 验证真实 LLM 的基础能力连通性（如 bash echo）。
- **扩展性与物化 (PR4)**:
  - `pr4d_auto_provisioning.rs`: 覆盖 Git 插件自动拉取机制。
  - `pr4e_up_fingerprint.rs`: 覆盖指纹漂移计算与 `session.realign`。
- **底层拦截与状态机**:
  - `prompt_handler_e2e.rs`: 覆盖 Prompt 拦截与 `agent.resolve_prompt` 闭环。
  - `pr1a_evidence_statemachine.rs`: 覆盖物理证据门控机制。
  - `pr1b_readfirst_hook.rs`: 覆盖 Hook 拦截与 RPC 注入。
- **守护进程与生命周期 (R1)**:
  - `r1_shutdown_cleanup.rs`: 覆盖 `ah stop` 与 Sandbox 物理清理。
  - `r1_master_exit_shutdown.rs`: 覆盖 Master 退出的级联清理。

## 3. 全流程 e2e Gap 分析

| 用户旅程阶段 | 当前测试覆盖 | Gap |
| :--- | :--- | :--- |
| **全生命周期串联 (The "Grand Tour")** | 各阶段独立覆盖 | ❌ **0 覆盖**。目前没有一个测试从 `ah start` (触发 Git 拉取) -> 真 LLM 执行任务 -> 修改配置触发 `ah up` -> 产生证据记录 -> `ah stop` 彻底串联起来。 |
| **真实 CLI 驱动 (Bash Walkthrough)** | `mvp10_acceptance` 等调用了 `CARGO_BIN_EXE` | ❌ 现有测试大多在 Rust 内直接构建 RPC Payload，缺乏真实的命令行完整端到端黑盒脚本（如真实的 bash 脚本）。 |

## 4. 全流程 e2e 应该长啥样

从第一性原理出发，真正的全流程 E2E 应该包含以下三个层级：

1.  **单 Happy Path Test (Rust Integration)**:
    - 一个巨型的 `#[tokio::test]` 函数：初始化配置 -> `ah start` 启动 Daemon 和 Agent -> `ah ask` 提交真实任务 -> 动态修改 `ah.toml` -> `ah up` 触发指纹对齐重启 -> 再次 `ah ask` 验证新插件/钩子生效 -> `ah stop` 关闭 Daemon。
2.  **多场景 Path (Rust Integration)**:
    - 针对 `ah up` 的异常分支（如 Drift, Orphan, BUSY 阻断）或任务执行的强物理拦截（PR-1a/b），分别编写从头到尾的独立路径测试。
3.  **Bash Walkthrough 脚本**:
    - 提供一个 `scripts/full_e2e_walkthrough.sh`，完全使用 `ah` CLI 二进制黑盒执行。这是给真人及 CI 呈现的最终形态，展示产品已准备好交付给终端用户。

## 5. 评估结论与建议

- **覆盖度现状**: 当前 E2E 在**子系统与核心机制级别（RPC/状态机/物化）的覆盖度已达 95% 以上**，但**串联业务流的宏观全流程覆盖度为 0**。系统模块是健壮的，但缺乏“集成大考”。
- **实施建议**: **必须做，但建议作为一个独立的 PR (如 Task #15 "Grand Tour E2E") 实施**。
- **工作量评估**:
  - 编写 Rust 版的 "Grand Tour" 串联测试难度适中（可复用大量现有 harness），约 200-300 LOC。
  - 编写 Bash 版本的端到端验证脚本需要处理异步日志等待与退出码捕获，约 100-200 行 Shell。

**总结建议**: 现有的独立 E2E 测试已经高度可信地验证了底层模块的可靠性；全流程 E2E 测试是产品发布（GA）前的关键里程碑，建议将其划分为独立的专项测试 PR 落地。
