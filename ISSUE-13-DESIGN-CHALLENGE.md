# Issue #13 — Design Challenge: config-fingerprint normalization (Adversarial Review)

**Reviewer:** o1 (设计辩论席)  
**Target Path:** [ISSUE-13-DESIGN-CHALLENGE.md](file:///home/sevenx/coding/ccbd-rust-wt-issue13/ISSUE-13-DESIGN-CHALLENGE.md)  
**Date:** 2026-07-10  

---

针对 `g1` 提交的 `ISSUE-13-DESIGN-BOUNDARY.md` 修订设计草案，以下是 adversarial 设计挑战与质疑意见。本报告针对第 5 节点名的三个靶心给出了明确的判定，并额外指出了设计笔记中未被 g1 标记的另外两项关键安全与架构缺陷。

---

## 1. 质疑靶心 1：`AH_AGENT_ID` 排除出 hash，算不算配置？

- **判定**：**同意 g1 排除该项，但认为其论证的“冗余性”理由需要修正**。
- **支撑证据**：
  - [src/process_identity.rs:16](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/process_identity.rs#L16) - `env.insert(AH_AGENT_ID.to_string(), agent_id.to_string());`（`AH_AGENT_ID` 作为运行时身份环境变量被注入）。
  - [src/process_identity.rs:31-44](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/process_identity.rs#L31-L44) - 单元测试 `process_identity_vars_are_not_daemon_identity_vars` 明确断言 `AH_AGENT_ID` 是 `per-process identity`，而非 daemon/配置级的 identity。
  - [src/rpc/handlers/realign.rs:224](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/rpc/handlers/realign.rs#L224) - `running.id == agent.agent_id`。
- **辩论与逻辑支撑**：
  - `AH_AGENT_ID` 的本质是系统分配给当前运行实例的 **运行时进程身份（Process Identity）**，属于状态（State）而非配置声明（User-declared Configuration）。
  - 配置哈希（`config_hash`）的目的应当是描述“这台 Agent 的行为配置是否发生变更”（例如修改了 skills，挂载了不同的 hooks，或者改变了 `agent.env` 等），从而决定该 Agent 是否需要 Realignment。
  - 如果把 `AH_AGENT_ID` 包含在哈希中，会导致两个配置完全相同的 Agent（例如 `a1` 和 `a2`）由于 `AH_AGENT_ID` 这一物理标识差异而算得不同的哈希。虽然 realign 目前是按 `agent_id` 过滤单实例进行比对，但将身份信息折叠进配置哈希是概念污染，并且可能破坏将来全局配置审计的可复现性。
- **改进建议**：
  - 维持 `EXCLUDE` 决定。在 `design-rev` 中，应明确区分 **配置哈希 (Configuration Fingerprint)** 和 **身份信息 (Identity Metadata)** 的边界，并阐明配置哈希必须只对 `declared config` 具备决定性。

---

## 2. 质疑靶心 2：`IS_SANDBOX` 排除出 hash，算不算配置？

- **判定**：**不同意 g1 排除该项，此举在安全边界上存在隐患**。
- **支撑证据**：
  - [src/rpc/handlers/agent.rs:459-468](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/rpc/handlers/agent.rs#L459-L468) - `should_inject_is_sandbox` 逻辑：
    ```rust
    pub(crate) fn should_inject_is_sandbox(
        provider_name: &str,
        command: &[&str],
        home_root: &Path,
    ) -> bool {
        provider_name == "claude"
            && command.iter().any(|arg| *arg == "--dangerously-skip-permissions")
            && is_ccb_sandbox_home(home_root)
    }
    ```
- **辩论与逻辑支撑**：
  - **沙箱状态并非单纯由配置静态决定**：g1 认为 `provider` 和 declared config 已在 hash 中，所以 `IS_SANDBOX` 是多余的。但 `should_inject_is_sandbox` 的第三个因子是 `is_ccb_sandbox_home(home_root)`。
  - `home_root` 是宿主机运行时派生并控制的（[src/provider/home_layout.rs](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/provider/home_layout.rs)）。如果在 reconnect 或者 daemon 升级时，宿主机路径属性或权限判断逻辑（`is_ccb_sandbox_home` 的判断标准）发生了变更，或者管理员修改了系统的宿主机权限绕过状态，`IS_SANDBOX` 的计算值就会漂移。
  - **安全边界的承重设计**：一个 Agent 实例究竟在不在沙箱内运行，是本系统的 **关键安全边界属性 (Security Boundary)**。如果在运行期间，由于宿主机环境或判定条件的变化导致其 `IS_SANDBOX` 降级（例如本该在沙箱跑的却在宿主机跑），而 `config_hash` 没有捕捉到这一安全属性漂移，`realign` 就会认为没有变化（`NO_CHANGE`）而不执行重建。这是极高危的安全隐患。
- **改进建议**：
  - **将“沙箱化（Sandbox Isolation）的启用状态”作为独立的显式字段包含在 `ConfigFingerprintInput` 中**（例如，在 `ConfigFingerprintInput` 结构体中新增 `is_sandbox: bool`，在 spawn 和 realign 两端均通过 `should_inject_is_sandbox` 计算出布尔值传入）。
  - 这样，即使运行时参数发生了波动，只要沙箱启用状态本身不改变，哈希值就保持稳定；一旦沙箱模式因环境改变或配置变更出现偏移，将立即被 `realign` 检测为 `DRIFT` 并强制重建。

---

## 3. 质疑靶心 3：`config.env` 全局 `[env]` 导致全员同时 drift

- **判定**：**不可接受。此设计在未做限流的情形下会复现“Respawn Storm”**。
- **支撑证据**：
  - [src/rpc/handlers/realign.rs:210-221](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/rpc/handlers/realign.rs#L210-L221) - `for agent in &agents` 的 realign 循环中没有任何限流、分批或错峰机制。
  - [src/provider/init_probe_task.rs:26-28](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/provider/init_probe_task.rs#L26-L28) - 每个启动的 agent 会立即发起 200 ms 的 `capture-pane` 高频 tmux 轮询。
- **辩论与逻辑支撑**：
  - g1 认为“有 SIGKILL 修复去除了孤儿 pane 就可以接受全员同时 drift”，这严重低估了单线程 tmux 服务器的承载瓶颈。即使旧的进程被杀掉了，全局 env 改变的一瞬间仍会导致 N 个 agent 同时向 `ahd` 发送 spawn 请求，拉起 N 个新的 tmux pane，并带来 $N \times 5$ 次/秒的 `capture-pane` 密集轮询。
  - 这依然是一场在极短时间内爆发的“Respawn Storm”，依然具有瞬间拉垮 tmux 服务器的极高风险。
- **改进建议**：
  - 必须实施 **错峰与速率限制 (Rate-limiting & Staggered Respawn)**：在 `realign.rs` 中引入全局并发限制（例如限制同时处于 spawning 状态的 agent 个数，或使用基于 tokio 信号量的 semaphore 进行并发管理）；
  - 在每个 agent 重建时，插入随机的时间延迟（Jittered interval，如 stagger 500ms），将瞬时的并发洪峰平滑化为时间维度上的平滑波段，彻底消灭 storm 发生机制。

---

## 4. 额外发现的漏洞与设计缺陷

### 4.1 职责泄漏：Client-Server 逻辑解耦与可信漏洞 (Client-Side Merge Vulnerability)

- **判定**：**不同意 g1 建议在 client 侧进行 `config.env` 合并的方案，这引入了严重的架构解耦漏洞。**
- **支撑证据**：
  - [src/cli/start.rs:156-157](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/cli/start.rs#L156-L157) - `merged_env = config.env + agent.env`。
  - [src/cli/up.rs:49](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/cli/up.rs#L49) - 原文仅发送 `"env": agent.env`。
  - [src/rpc/handlers/realign.rs:106-112](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/rpc/handlers/realign.rs#L106-L112) - Server 侧直接从 RPC 参数 `params` 中反序列化 `agents` 信息。
- **辩论与逻辑支撑**：
  - 将 `config.env` 合并的计算逻辑放置在 Client 侧（`start.rs` / `up.rs` 等各调用端），意味着 Server 侧（`agent.rs` / `realign.rs`）在算哈希时，完全依赖并假定 Client 传过来的 `env` map 已经正确做好了 merge。
  - 这违反了“Server 端作为唯一可信数据源（Single Source of Truth）”的安全设计原则。如果未来新增了其他的 Cli 入口，或者有第三方 RPC 自动化脚本不通过当前的 `up.rs` 直接调用 `session.realign` 接口，或者遗漏了 client 端的 merge 逻辑，将会引发致命的配置哈希不一致，再次陷入 phantom drift 的死循环。
- **改进建议**：
  - **完全在 Server 端进行全局配置与 Agent 配置的合并**。
  - RPC 接口中，Client 只传递原始无污染的 TOML 数据（`agents` 的原始 `agent.env`，以及最外层的全局 `config_env`）。
  - 在 Server 端处理请求的入口函数（如 `handle_session_realign` 和 `handle_agent_spawn_with_db_action`）处，统一根据传入的全局 `config_env` 与单个 `agent.env` 产生有效的合并 map，然后传递给哈希计算。Client 应当保持无状态与无逻辑。

### 4.2 状态决断权分裂：`spawn_realign_agent` 的哈希覆盖冲突 (Hash Resolution Race)

- **判定**：**严重缺陷。`realign.rs` 中直接更新数据库哈希的行为与 `agent.rs` 内部的自动更新存在竞争冲突。**
- **支撑证据**：
  - [src/rpc/handlers/realign.rs:413-419](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/rpc/handlers/realign.rs#L413-L419)：
    ```rust
    if !uses_atomic_replacement {
        update_agent_config_hash(ctx.db.clone(), agent.agent_id.clone(), expected_hash.to_string()).await?;
    }
    ```
  - [src/rpc/handlers/agent.rs:353-354](file:///home/sevenx/coding/ccbd-rust-wt-issue13/src/rpc/handlers/agent.rs#L353-L354)：正常 spawn 时，`agent.rs` 会自行往数据库插入最新算得的哈希。
- **辩论与逻辑支撑**：
  - 为什么 `uses_atomic_replacement` 为 `false` 时要由 `realign.rs` 去强行更新数据库的 config_hash？而为 `true` 时却不更新（任由 `agent.rs` 内部去写哈希）？
  - 这导致哈希的写入职责分裂在了两个不同的 RPC 处理器中。一处在正常 spawn 的 `agent.rs` 内部；另一处在 realign 的 `spawn_realign_agent` 外部。一旦两边在处理 `extra_env_vars` 的合并细节或过滤因子上由于后续版本迭代产生了极微小的行为差异，就会导致在不同 `uses_atomic_replacement` 路径下，数据库写入的哈希与最终运行的 agent 实际哈希不一致，从而导致第二次运行 `ah up` 时再次判定为 drift，形成无法跳出的 respawn 死循环。
- **改进建议**：
  - **收拢哈希写入职责**：彻底去除 `realign.rs` 中任何对 `update_agent_config_hash` 的直接调用。
  - 数据库中 `config_hash` 的唯一写入权必须内聚在 `agent.rs` 内部成功完成 spawn 动作的末尾。`realign.rs` 在判定为 drift 后，只需将新 agent 拉起，拉起过程会按照标准流程将最新 Bare Config 计算出的哈希持久化到数据库中。这确保了单一职责原则。
