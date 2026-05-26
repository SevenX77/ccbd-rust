# Phase 1 诚实签收文档 (Honest Sign-off)

本文档根据 **“冷酷区分已交付 vs 已延后”** 的原则，记录 ccbd-rust Prompt-Handler Phase 1 的真实交付状态。

## 1. 已交付 (Delivered)
基于文件系统、CI 日志及三方审计实证：

- **物理生命周期主干 (PR4a)**：
    - 落地了基于第一性原理的 **can-input 探针** 就绪门。
    - 实现了 `SPAWNING → IDLE`（主动探针确认）与 `BUSY → IDLE`（被动静默信号）的职责分离。
    - 参考：`.kiro/specs/prompt-handler/lifecycle-spine.md`。
- **CI 债务清偿 (Infrastructure)**：
    - 彻底清理了 main 分支长期堆积的 6 层 masked CI 失败链（包括 userns 权限、pidfd 竞争、环境变量竞态等）。
    - 实现分支 CI 端到端全绿（实证 run：`26429294308`）。
- **项目级状态隔离 (PR1)**：
    - 废弃了污染 Repo 的 `.ccb-rs` 目录，实现了基于 `ccb.toml` 路径哈希的 XDG 状态隔离。
- **配置定义清理 (PR6a)**：
    - 删除了 ccb.toml 顶层已废弃的 `layout = "grid"` 字段，拓扑逻辑回归至 master/agents 段驱动。
- **测试卫生 (Infrastructure)**：
    - 全局环境变量竞态已通过 `serial_test` 串行化（commit c3c8b29），消除了并发测试下的 Flaky 隐患。

## 2. 已延后 (Deferred)
出于环境约束或风险控制，以下项明确延后至后续阶段：

- **自学习层与 LLM 慢路径 (PR4b)**：
    - **现状**：`prompt_experience` 自学习数据库及 Haiku 4.5 HTTP 调用链路尚未开发。
    - **理由**：受限于 VPS 真实 LLM 调用环境的饱和与物理接入限制。
- **真机 Parity E2E 验证 (PR6b)**：
    - **现状**：CI 目前仅能运行 Mock 管道，尚未在真实 LLM Provider 环境下跑通 full-suite。
    - **理由**：同样需要稳定且真实的 Provider 凭证与运行资源。
- **PATH-shadow 生产替换 (PR6a)**：
    - **现状**：未强制让 `ccb-rust` 注入 PATH 拦截原 Python `ccb` 命令。
    - **理由**：为规避对当前正在运行的生产环境产生不可逆干扰，需用户手动拍板执行迁移。

## 3. 当前能力边界
Phase 1 交付的是一个 **“稳健的物理底座 + 确定性的开发环境”**。
- **能做到的**：在不依赖外部 LLM 的情况下，物理确认 Agent 的输入链路是否通达，并能利用内置正则（KB）自动点掉已知弹窗，同时保证 CI 对当前确定性回归底座的全面覆盖。
- **暂不能做到的**：自主识别并学习“从未见过”的新弹窗。此类情况目前会准确停留在 `PROMPT_PENDING` 状态等待人工介入。

## 4. 剩余未决项 (Open Items)
- **PR8 Migration 框架**：尚未建立通用的数据库版本迁移管理机制。
