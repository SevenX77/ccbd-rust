# Design: ah 全流程 E2E Grand Tour 测试设计

## §1 第一性原理 + 目标

- **为什么要进行 Grand Tour**: 现有的单元测试和子系统 E2E 测试虽然达到了 95%+ 的模块覆盖率，但它们都是"模块对"（Component-level verification），无法保证产品作为一个整体能顺利交付给最终用户。Grand Tour 是"产品对"（Product-level verification），它代表了从最终用户视角出发的完整、连续、不间断的宏观业务流程。
- **与现有测试的互补性**: 子系统测试验证了状态机的细节逻辑（如某个特定 RPC 的原子性或某个钩子的物理拦截），而 Grand Tour 验证的是多个连续的状态演变在持久化、进程生命周期 and 物理副作用（文件系统）中累积时的宏观正确性，确保不发生由于长链路累积产生的非预期脑裂或资源泄漏。

## §2 核心机制思路

- **单 Happy Path (Rust Integration)**: 在 Rust 集成测试中实现一个长生命周期的 `#[test]`。该测试从完全干净的临时环境启动，依次通过多次状态调谐和 RPC 驱动，串联起从环境初始化、服务拉起、任务下发、配置变更、指纹对齐到最终安全关闭的完整生命周期，中途不重置数据库或状态目录。
- **多场景 Path (Rust Integration)**: 针对主线旅程中发生的分支行为（如指纹漂移、孤儿进程清除、并发阻塞、状态崩溃等），利用相同的集成测试脚手架构建独立的、次要的 Happy Path，模拟用户在特定产品阶段遭遇的典型异常并验证系统的鲁棒性。
- **Bash Walkthrough**: 从纯黑盒 and 真实用户的命令行视角出发。不直接通过 Rust 的 `dispatch` 模块或 RPC 客户端调用，而是通过调用编译出的 `ah` CLI 二进制执行命令。该脚本旨在提供给人类进行黑盒审查或在 CI 独立流水线中快速验证 CLI 入口的一致性。

## §3 关键决策

- **真实 LLM 调用决策 (Mock vs Real)**:
  - *决策*: 在 Grand Tour 全流程测试中**不引入真实 LLM 调用**（如不使用真实的 Claude/Gemini API），全部采用 **mock provider**（如自定义的短命命令、返回固定 Echo 的 Bash 进程）。
  - *理由*: 真实 LLM 会带来高昂的 Token 成本、显著的网络 Flakiniss，且无法在没有预设环境变量凭证的干净 CI 环境中稳定运行。真实的 LLM 校验和接口连通性应严格限制在 `mvp11_real_*` 系列等专项测试中，Grand Tour 聚焦于产品“功能链路”与“状态机制”的连通。真 LLM 链路由 mvp7/mvp9/mvp11_real_* 系列独立覆盖，本 Grand Tour 用 mock 聚焦状态机制链路与功能闭环。
- **入口选择 (ah CLI vs RPC)**:
  - *决策*: 核心的 Rust 串联测试（Happy Path & 多场景 Path）采用 **RPC/内部核心组件入口**（复用现有测试的进程管理与数据层结构）；而 Bash Walkthrough 采用 **ah CLI 入口**。
  - *理由*: 如果在 Rust 长测试中全部使用 `Command::new("ah")`，会导致测试代码充斥着大量的进程等待、标准输出解析，不仅难以捕获精细的 DB 状态进行断言，还会由于大量的 TTY 嵌套导致测试脆弱。用内部核心入口保证断言的深度和物理透明度，用 Bash 脚本保证最外层 CLI 的交付质量。

## §4 用户旅程矩阵 (主线 + 分支)

### 主线 Happy Path 流程 (覆盖所有 Runtime 命令)
1. **ah start**: 初始化全新的项目环境，触发物理沙盒、配置精准重定向以及 Git 插件的自动物化 (init + spawn)。
2. **ah ping**: 执行健康检查，验证 Daemon 及 RPC 通道就绪。
3. **ah ask**: 提交一个模拟的异步任务，验证状态转为 `BUSY` 并生成物理证据 (异步下发)。
4. **ah ps**: 取证验证，检查 Session/Agent 状态是否符合预期（验证 BUSY + agent 状态）。
5. **ah pend**: 等待异步任务完成，并进行收尾阶段的物理断言。
6. **ah logs**: 获取任务执行后的存储输出，验证取证链路完整性。
7. **修改配置**: 动态改写项目中的 `ah.toml`（如修改 hooks 或新增 agent），制造指纹漂移 (drift 触发)。
8. **ah up**: 触发指纹漂移计算，执行 `session.realign` 调谐流程使物理环境对齐 SoT。
9. **ah ask**: 再次提交任务，验证新配置/新钩子在调谐后已生效。
10. **ah prompt resolve**: 模拟交互式 Prompt 响应闭环，验证交互链路（复用 prompt_handler_e2e 逻辑）。
11. **ah cancel**: 对后续任务执行取消操作，验证任务取消的幂等性与状态机正确性。
12. **ah watch**: 建立短时间观察连接，验证流式输出 (Events) 链路。
13. **ah kill**: 执行单 Agent 的软杀掉，验证局部状态清理与级联影响。
14. **ah stop**: 安全关闭全局 Daemon 进程，级联强制收割所有相关子孙进程，清理临时沙盒。

*注: 主线显式排除以下命令: (a) ah attach (TTY 交互式 attach 到 agent pane, 自动化断言价值低, 在 §6 M1 Bash walkthrough 中提及覆盖); (b) Doctor/Config validate/Migrate/Version 非 Runtime 命令, 在 §6 M1 Bash walkthrough 中覆盖。*

### 分支 Path 矩阵
- **DRIFT**: 变更物理环境中的 hooks / plugins / cmd，调用调谐流验证指纹变动并正确触发自动 realign。
- **ORPHAN**: 在 `ah.toml` 中显式删除某 agent 块，执行调谐，验证系统在审计模式或强制模式下能安全收割被遗弃的孤儿 Agent。
- **NEW**: 在 `ah.toml` 中追加未声明的 agent 块，执行调谐流，验证能无缝 spawn 并物化该新 Agent 而不干扰现有活跃 Session。
- **BUSY**: 当 Agent 处于 `BUSY` 状态（任务正在执行）时触发配置更新，验证系统能根据策略选择跳过 (skip) 或在加了强制参数 (--force) 时能安全收割重来。
- **ERROR 恢复**: 制造 provider 启动失败（如返回非零退出码），验证 Agent 状态机转为 `CRASHED`；随后清理物理环境并重新进行调谐，验证系统能从崩溃状态平滑恢复到就绪。

## §5 物理断言风格

- **深度复用机制**: 坚决摒弃“只验证 RPC 返回值”的表面绿灯，深度复用 `PR-1a evidence statemachine` 建立的物理门控断言以及 `PR4e config_hash` 的指纹比对技术。
- **四维联合断言**: 每一个状态节点必须通过以下四个维度的联合交叉检验才算通过：
  1. **OS 进程树结构**: 通过 `/proc/<pid>` 或物理进程状态检查，确认真实子进程（如 tmux, managed agent）数量和连坐关系的物理准确性。
  2. **SQLite SoT 数据记录**: 检查 `agents`、`sessions` 及 `events` 表中的状态、版本号和事件流是否按事务正确硬化。
  3. **文件系统物理副作用**: 检查沙盒 HOME 目录中的 Auth 凭证 Symlink、Per-Provider Rules 文件物化（ copy 状态）是否精准，以及证据链文件是否正确落盘。
  4. **外层响应值**: 验证 RPC 的结果或 CLI 的退出码与上述三层物理事实绝对一致。

## §6 实施切片大方向

- **M1: Bash Walkthrough 验证脚本** (工作量: 约 100-200 行 Shell)
  - 编写独立的演示 and 验收脚本，完全依赖编译出的 `ah` 裸二进制进行黑盒串联。用于给最终用户演示或进行 CI 最外层冒烟，不塞入 `cargo test` 阻塞主编译。
- **M2: Rust 宏观单 Happy Path 集成测试** (工作量: 约 500-700 LOC)
  - 编写巨型 Rust 集成测试，复用现有的 `TmuxServerGuard` 和 SQLite 初始化，完整串联“主线旅程”。
- **M3: Rust 多分支扩展路径测试** (工作量: 约 150-250 LOC / 每个主要分支)
  - 针对 §4 矩阵中的 DRIFT, ORPHAN, NEW, BUSY 和 ERROR 场景，沉淀出独立的变种测试路径。

## §7 决议汇总

- **§7.1 CI lane**: Grand Tour 测试用 `#[ignore]` 标记，默认 `cargo test` 跳过，CI Nightly Lane + 本地 `cargo test --include-ignored` 执行。决议: 主控+a1 自决，不抛 PM。理由: 主控 vibecoding 反馈循环不被 Grand Tour 拖累。
- **§7.2 分支覆盖范围**: 按 PM "所有功能都测试一遍" 诉求，5 分支 (DRIFT/ORPHAN/NEW/BUSY/ERROR) 必须在 PR 系列内全覆盖 (M3 分批落实)。决议: 主控自驱分批策略 (建议 PR-1 M1+M2 主线; PR-2 M3 DRIFT+NEW; PR-3 M3 ORPHAN+BUSY+ERROR)，不抛 PM。
- **§7.3 工作量估算**: 按 §4 主线扩充到 14 步后，M2 单 Happy Path 实际 500-700 LOC (含 Harness ~100 + RPC 调用 ~200 + 四维断言 ~300)。M3 每分支 150-250 LOC。M1 Bash walkthrough 100-200 行。决议: 主控自决估算，不抛 PM。
