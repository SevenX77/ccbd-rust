# Idea: ah 全流程 Grand Tour PR-6 (ERROR Recovery + Claude Session Resume)

## §A 大方向

PR-6 的核心目标是消除 `session.realign` 在处理 `CRASHED` 节点时的“盲区”，并赋予 Claude Agent 故障后自动续航的能力。本 PR 将范围严格限定在 **Claude Agent Worker** 的恢复与 `--continue` 参数注入，不涉及其他 Provider 或 Master CLI 逻辑。

## §B 核心机制

- **B1 修复 SQL 观测盲区**: 修改 `running_agent_hashes`，使其查询范围包含 `CRASHED` 状态的 DB 行。这确保 `realign` 调谐器能“看见”已崩溃的 Agent，并将其纳入 Hash 对比流，而非误判为 `NEW` 节点，从而绕过 `handle_agent_spawn` 的 DB 主键冲突。
- **B2 显式信号透传**: 在 `realign` 流程识别出 Agent 此前已存在（无论当前是 IDLE/BUSY 还是 CRASHED）后，将 `is_recovery=true` 信号显式沿 `spawn_realign_agent` -> `handle_agent_spawn` -> `wrap_command` 调用链透传，确保行为决策链清晰可追溯。
- **B3 Manifest 动态命令构造**: 扩展 `ProviderManifest` 结构，新增 `resume_args: Vec<String>` 字段。针对 Claude Worker，该字段设为 `["--continue"]`。`wrap_command` 在接收到 `is_recovery=true` 信号时，将 `resume_args` 追加至 Base Command 之后，实现按需注入。
- **B4 测试验证自动化 (Case 11)**: 采用“物理文件对账”方案。扩展 `fake claude` mock 脚本，使其在启动时将全部参数 (`$@`) 写入 `GRAND_TOUR_RESUME_ARG_MARKER` 环境变量指定的文件。集成测试通过读取该文件，断言 `--continue` 标志已真实传达给物理进程。

## §C 核心决策

- **C1 Scope Claude Only**: 鉴于 Codex/Gemini 的 Resume 机制尚未在 Manifest 层面发现对应 flag，本 PR 仅实现 Claude 链路。Codex/Gemini 的适配留待 PR-7+。
- **C2 信号源唯一性**: `is_recovery` 信号严格源自 `realign` 流程对 DB 历史记录的识别，坚决不采用“检查 Sandbox 目录是否已初始化”等不稳定的隐式查询。
- **C3 通用化 Manifest 接口**: `resume_args` 采用 `Vec<String>` 而非单字符串，为未来多参数或不同 Provider 的 Resume 方案提供通用扩展接口。
- **C4 行为兼容性**: 此次变更仅对 `CRASHED` 行或明确触发 REALIGNED 路径的 Agent 追加 Resume 参数。正常的 `NEW` 启动路径（无 DB row）绝不添加该参数，防止 Claude 报错。
- **C5 无 LLM 测试依赖**: 通过 marker 文件物理校验，避免在 E2E 测试中引入复杂的 mock 逻辑解析。

## §D 不在 Scope (Boundary)

- **D1 Codex/Gemini Resume**: 留待 PR-7+ 处理。
- **D2 Master CLI cmd drift**: 与 worker 恢复无关，属 PR-5 范畴。
- **D3 Session 级聚合 Status**: 仅关注 per-agent 状态流转。
- **D4 仓库重命名 (ccbd -> ah)**: 保持命名现状，不增加无关重构。
- **D5 KILLED 状态恢复**: PR-6 仅针对异常 `CRASHED`；`KILLED` 属用户主动终止，默认不执行自动 Resume。

## §E 风险 + 风险缓解

- **风险**: `is_recovery` 信号在冗长的 RPC Handler 调用链中意外丢失。
- **缓解**: 强制修改 `handle_agent_spawn` 接口签名，使其必须接收 `is_recovery` 参数；默认 `agent.spawn` RPC 路径传 `false`，确保只有经过识别的调谐路径能激活恢复逻辑。

## §F LOC 预算 (250-350 LOC 分配)

- **B1 (SQL 观测)**: ~30 LOC (查询语句修正与单元测试)
- **B2 (信号透传)**: ~80 LOC (Handler 签名变更与参数链传)
- **B3 (Manifest 构造)**: ~100 LOC (Manifest 定义扩充、`wrap_command` 逻辑更新)
- **B4 (测试改造)**: ~120 LOC (Case 11 翻转、Mock 脚本增强、Marker 校验逻辑)
