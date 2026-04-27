# 议题 A-4：沙盒方案 (Bubblewrap vs Custom Syscall)

## 第 1 轮 (Round 1) - Master Claude 立场

### 1. 核心架构选型
- **底层工具**：采用 `Bubblewrap` (`bwrap`) 作为唯一的非特权沙盒后端实现。
- **隔离粒度**：基于 User/Mount/PID/UTS Namespace 提供进程级的物理隔离。

### 2. 辩论：为什么不自研 Syscall 调用？
- **安全风险与审计成本**：沙盒是一个高风险安全组件。直接调用 `unshare(2)` 需要处理极其复杂的 Capability Drop、Mount 顺序和 Seccomp 过滤逻辑，稍有不慎即成漏洞。
- **复用行业红利**：`bwrap` 是 Flatpak 等 Linux 桌面生态的标准组件，经历过海量实战和 CVE 审计。直接调用二进制不仅代码极简，且能白嫖整个开源社区的安全更新。

---

## 第 2 轮 (Round 2) - Gemini 架构评估

### 1. 核心判断
我完全认同使用 `bwrap` 作为唯一方案。在 L2 调度层（资源与进程管理器）的定位下，自己去实现 Linux Namespace 的底层绑定是典型的「重新发明轮子（NIH）」，不仅带来巨大的维护负担，更会破坏系统的安全基线。

### 2. 五项工程决策细化

#### A-4-1：降级模式与可用性
- **判定**：**严禁静默降级（Fail Open），提供环境级显式 Bypass。**
- **机制**：
    1. ccbd 启动时执行 `which bwrap`。若未找到，直接 Panic 退出并提示安装。
    2. 对于特殊的 CI/CD 环境，提供 `CCBD_UNSAFE_NO_SANDBOX=1` 环境变量以允许完全绕过沙盒机制。不提供 CLI Flag，以增加使用者配置时的心理负担。

#### A-4-2：进程调用模型
- **判定**：**Subprocess 模式 (Command::new)。**
- **理由**：`bwrap` CLI 的参数集非常稳定。将其作为普通的子进程拉起，天然适配 A-5 决议中的 `pidfd` 和 A-3 中的 PTY 附加逻辑，无需纠结 C FFI 的静态链接噩梦。

#### A-4-3：Baseline 参数调优
- **判定**：**收紧全局挂载，细化项目级映射。**
- **修正**：废弃全局挂载 `.ccb` 目录的做法。通过动态组装，仅挂载当前 Agent 所需的最低限度文件：
  `--ro-bind <project_dir>/.ccb/agents/<agent_id> /workspace/.ccb_agent`

#### A-4-4：Per-Provider Sandbox Profile
- **判定**：**沙盒配置下沉至 Provider 契约。**
- **机制**：通过读取 `.ccb/providers/<name>.toml` 中的 `[sandbox]` 节，动态注入额外的网络权限（`--share-net`）或凭证挂载（`--ro-bind /host/credentials /sandbox/credentials`）。L2 仅作为这些配置的物理执行器。

#### A-4-5：错误传播
- **判定**：**遵照 A-7 决议，结构化映射 bwrap 错误。**
- **错误码设计**：
    - `SANDBOX_BWRAP_NOT_FOUND`: Daemon 启动前置检查失败。
    - `SANDBOX_USER_NS_DISABLED`: bwrap 退出提示缺少内核特性。
    - `SANDBOX_MOUNT_FAILED`: 特定文件映射失败导致的启动终止。
