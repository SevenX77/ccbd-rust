# 议题 5：资源与沙盒的所有权边界

## 第 1 轮 (Round 1) - Master Claude 立场

本议题旨在定义 ccbd-rust (L2) 与 Master Claude (L3) 及其用户之间的物理资源切线，确保资源生命周期清晰且无冲突。

### 1. 所有权矩阵 (Ownership Matrix)

| 资源类别 | 所有者 (Owner) | 读权限 | 写权限 | 清理责任方 |
|:---|:---|:---|:---|:---|
| **Sandbox 内部文件** | **L2** | L2 + L3 (仅通过 RPC) | L2 | L2 (Janitor 自动清理) |
| **Sandbox 外部工作区 (Git Repo)** | **用户 / L3** | L2 + L3 | L2 (挂载) + L3 | L3 / 用户 (L2 不动) |
| **ccbd-rust SQLite DB** | **L2 独占** | 仅 L2 (禁止 L3 直读) | L2 | L2 |
| **ccbd-rust UDS Socket** | **L2 独占** | L3 (连接请求) | L2 | L2 (启停管理) |
| **ccbd-rust 运行日志** | **L2 独占** | 用户 (Debug 用) | L2 | L2 (自动滚动) |
| **Agent stdout/stderr 缓存** | **L2 临时持有** | L3 (RPC 订阅) | L2 | L2 (按 Retention 策略) |
| **Provider Session 文件** | **Provider CLI** | CLI + L3 | CLI | Provider CLI 自己管 |

### 2. 关键判断 (Core Judgments)

1.  **判断 1：L3 不直接读 SQLite 文件**
    *   *实例*：L3 严禁直接 `sqlite3 ~/.local/state/ccbd/ccbd.sqlite` 查询 Agent 状态，必须调用 `agent.status` RPC。因为 L2 内部可能处于事务中间态，直读会导致观测到不一致的“脏数据”。
2.  **判断 2：外部工作区状态 L2 不负责“回滚”**
    *   *实例*：Agent 在沙盒内修改了宿主机的 `main.rs`。即使 L2 崩溃并重启，`main.rs` 的已修改内容必须保留，由用户或 L3 通过 Git 回滚。L2 重启不应重置用户工作区。
3.  **判断 3：跨重启清理采取“双重路径”**
    *   *实例*：L2 崩溃重连。主路径：L2 启动时执行 Startup Reconcile，扫描 SQLite 记录，发现 PID 已消失的 Agent，立即强制清理关联的 `sandbox/` 目录。兜底路径：在 Linux 下利用 Systemd `BindsTo` 自动级联清理。
4.  **判断 4：Retention Policy 强制约束**
    *   *实例*：事件表必须有 retention（24h 或 1000 条）。防止用户长期运行导致 SQLite 膨胀到 GB 级别，确保 WAL 模式下的查询始终处于毫秒级响应。
5.  **判断 5：日志隐私红线 (Privacy Boundary)**
    *   *实例*：`ccbd.stderr.log` 只能包含 `[INFO] Agent Spawned` 类的元数据。严禁记录任何 Prompt 或 Agent 吐出的业务代码，防止敏感信息泄漏给 Bug 审计者。

## 第 2 轮 (Round 2) - Gemini 评估与收敛

我完全认同 Master Claude 的所有权划分。这一矩阵确保了 ccbd-rust 作为一个“Dispatcher”的纯粹性。针对决策点，我补充如下评估意见：

### 1. 补强：配置文件的所有权 (R-CONFIG-OWNER)
*   **决策**：`.ccb/ccb.config` 和 `.ccb/providers/*.toml` 属于 **「用户/L3 所有，L2 只读」**。
*   **理由**：这是用户的意图文件。L2 应该将其视为“不可变输入（Immutable Input）”。如果格式错误，L2 应通过 RPC 返回结构化错误码（R-ERROR-CODES-1），绝对不能尝试“自动修复并写回”配置文件。
*   **实例**：用户在 `.toml` 里写错了一个 marker 正则。ccbd-rust 启动时解析失败并报错，要求用户修改；L2 绝不尝试自动修正并覆盖该文件。

### 2. 关于 SQLite 直读的禁令
*   **结论**：同意“软禁令（Soft-ban）+ 文档警告”。
*   **理由**：物理上无法阻止同 UID 进程读取文件，但必须在设计文档中明确：**“绕过 RPC 直接读取 SQLite 产生的一切状态撕裂，ccbd-rust 不予负责”**。

### 3. 非 Systemd 环境下的孤儿清理
*   **方案**：在 macOS (launchd) 或 Docker 容器中，我们无法依赖 `BindsTo`。
*   **对策**：**Startup Reconcile 必须是幂等的且具有最高优先级**。L2 启动的第一件事就是遍历数据库记录与沙盒目录，凡是“无主”或“已故”进程关联的资源一律清除。

### 4. 日志隐私与 Retention 的合理性
*   **结论**：完全认同。
*   **理由**：24h/1000 条的保留量足以支撑开发者在一次严重的崩溃后重启并找回上下文。

### 最终共识矩阵 (Consolidated Matrix)

| 资源 | 所有权 | 读写约束 |
|:---|:---|:---|
| **配置文件 (.config, .toml)** | 用户 / L3 | **L2 只读** |
| **SQLite DB / Socket** | L2 | L3 仅能通过 RPC 交互 |
| **Sandbox 临时目录** | L2 | L2 自动清理，L3 通过 RPC 读取 |
| **用户工作区 (Git Repo)** | 用户 | L2 仅通过只读/读写挂载映射，不管理其生命周期 |
| **stderr 日志** | L2 | 严禁包含对话内容，仅包含系统审计事件 |

**决议结果：阶段一 议题 5 收敛。**
ccbd-rust 正式确立为：一个对自己内部资源（SQLite, Socket, Sandbox Dir）拥有绝对生命周期控制权，但对外部资源（Git Repo, Config Files）保持敬畏、仅做只读或受限映射的系统。
