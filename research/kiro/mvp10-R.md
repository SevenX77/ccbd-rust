# Kiro Requirements: MVP 10 (进程生命周期 cgroup 治理与资源彻底回收 / Process Lifecycle cgroup Governance & Resource Recovery)

> **文档定位**：本文件是 ccbd-rust MVP 10 阶段的官方 R (Requirements) 规格。本阶段不引入新功能，目标是修补 MVP 6-9 实施过程中暴露的资源治理结构性缺陷——彻底根除 tmux server 孤儿、socket 文件累积、tempdir 残留这三类资源泄漏，把"用 Rust 重写解决 Python ccbd 留孤儿"这条核心承诺真正兑现。

---

## 0. 立项背景与边界共识

### 0.1 为什么必须做这个 MVP（核心驱动）

**用户重写 ccb 的最大动力——彻底解决 Python ccbd 时代的孤儿进程与内存泄漏问题——在 ccbd-rust 当前架构下只兑现了一半。**

MVP 1-9 实施完成后，agent 进程那条线已经通过 `systemd-run --slice=ccbd-agents.slice --property=BindsTo=ccbd-rust.service`（`src/sandbox/systemd.rs:23-25`）实现了 cgroup 强绑定：ccbd-rust 主服务一死，systemd 自动 SIGTERM 全 slice。这是相对 Python ccbd 的最大架构进步。

但 **tmux server 这条线在 ccbd-rust 当前实现里没有同款机制**。tmux server 用 `Command::new("tmux") -d new-session` 直接 spawn 出 daemon，detach 后跟 ccbd-rust 进程无父子关系，且全 src/ 树 grep `kill-server` / `kill_session` 只在 `#[cfg(test)]` 测试模块里出现一次，production 代码没有任何调 tmux kill-server 的退出路径。

**实测后果**（2026-04-30 取证，5 小时窗口）：
- 155 个 PPID=1 的 `tmux -L ccbd-<hex> -s ccbd-agents -c /tmp/.tmpXXXXXX` 孤儿 server
- 累积 RSS 538 MB（不是 KB）
- `/tmp/tmux-1001/` 下 2878 个 stale `ccbd-` 前缀 socket inode（server 死了 socket 没人清）
- 35% 的 tempfile workdir 因为 tmux 持有 pipes/ fd 没法被 OS 回收 inode，残留在 /tmp

这不是 mvp9 收尾时的偶发现象，是只要 ccbd-rust 主进程因任何原因（panic / OOM / SIGKILL / 测试超时强杀 / 用户 Ctrl+C）退出，**就必然**会留下孤儿 tmux server 的结构性缺陷。

### 0.2 本 MVP 的核心边界

- **tmux server cgroup 绑定**：把 tmux server spawn 包进 systemd-run scope，跟 agent 同款 BindsTo 机制
- **Production 优雅关闭**：ccbd-rust 主程序加 SIGTERM/SIGINT handler，shutdown 阶段显式回收 tmux 资源
- **测试 harness 强化**：测试 Harness 不再依赖 Drop 兜底，改用 systemd-run scope 让 cgroup 帮兜底
- **Socket 文件显式回收**：任何 kill-server 之后必须 remove_file(socket_path)
- **一次性现场清理**：杀掉当前 154 个孤儿 + 清 2878 个 stale socket + 清残留 tempdir
- **诊断集成**：`ccb doctor` 增加孤儿 tmux server 检测项

**不在范围内**：
- 不改造 agent 进程的 sandbox 路径（已经是好的）
- 不修改 RPC schema（不引入新 RPC）
- 不改 Tmux 布局算法（mvp9 G9.2 已经定型）
- 不引入 tmux 内置 `exit-empty` / `exit-unattached`（有 race，已被 plan 阶段否决）

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 10 验收必须全部通过：

1. **AC1 [Tmux Server cgroup 绑定]**：tmux server spawn 路径走 `systemd-run --user --scope --slice=ccbd-agents.slice [--property=BindsTo=ccbd-rust.service]`。集成测试模拟 ccbd-rust 主进程被 SIGKILL 后，验证对应 tmux server 在 systemd 时限内（默认 ≤ 5s）被自动清理。**判据**：`tmux -L <socket> ls-sessions` 失败 + 对应 `ccbd-tmux-<hash>.scope` unit 在 `systemctl --user list-units --type=scope` 中已不存在。**不**用 PPID=1 作为判据（daemonized tmux server 在 systemd scope 下也会显示 PPID=1，PPID 不能识别 cgroup 治理状态）。
2. **AC2 [Production Graceful Shutdown]**：ccbd-rust 主程序捕获 SIGTERM/SIGINT，在退出前遍历当前活跃的 TmuxServer 实例，顺序执行 `tmux kill-session -t ccbd-agents` → `kill-server` → 50ms sleep → `remove_file(socket_path)`。集成测试 `kill -TERM <ccbd_pid>` 后断言：(a) 对应 tmux 进程消失；(b) `/tmp/tmux-$UID/<socket>` 文件已删除。
3. **AC3 [测试 Harness 与 wrapper scope 模型]**：AC3 兑现路径是 **专项 SIGKILL 测试**（`tests/mvp10_acceptance.rs::test_main_sigkill_systemd_cleans`）使用 wrapper scope 模型——测试代码用 `systemd-run --user --scope --collect --unit=ccbd-test-victim-<ts>` 创建 wrapper scope 并在其中运行一个 `ccbd_test_helper` child 进程；helper 通过环境变量 `CCBD_TEST_WRAPPER_SCOPE=ccbd-test-victim-<ts>.scope` 拿到 wrapper unit 名，让 tmux scope 通过 `BindsTo=ccbd-test-victim-<ts>.scope` 绑定。测试通过 `systemctl --user stop ccbd-test-victim-<ts>.scope` 触发 systemd 杀 wrapper scope（unit-to-unit 依赖）；5s 内断言 tmux scope unit 在 `systemctl --user list-units` 中消失 + ls-sessions 探测失败 + cgroup 内 tmux 进程消失。**普通 cargo test 路径**（`tests/mvp[6-9]_acceptance.rs`）只要求 Drop + cleanup script sweep 兜底，不宣称 PID 级 BindsTo（systemd BindsTo 是 unit-to-unit，不是 unit-to-PID）。**判据同 AC1**：用 cgroup unit 状态而非 PPID 判定。
4. **AC4 [Socket 文件回收]**：任何代码路径（production / 测试 / doctor）执行 `tmux kill-server` 之后，必须显式 `std::fs::remove_file(socket_path)`，error 容忍 `NotFound`。单测覆盖：直接构造 socket 文件、调用 cleanup helper、断言文件不存在。
5. **AC5 [现场清理脚本]**：`scripts/cleanup_orphan_tmux.sh` 提供一次性清理。**孤儿判据**：对每个 `/tmp/tmux-$UID/ccbd-*` socket 跑 `tmux -L <name> ls-sessions`——失败即孤儿。活 server 一律不动（无论 PPID 是什么）。脚本附带清理无主 `/tmp/.tmp??????` workdir（无任何 live process 引用的）。脚本必须 idempotent，多次运行无副作用。
6. **AC6 [Doctor 集成]**：`ccb doctor` 增加检查项 "tmux server orphans"。扫描 `/tmp/tmux-$UID/` 下所有 `ccbd-` 前缀 socket，对每个 probe `ls-sessions`，统计活/孤儿数，输出建议（指向 cleanup_orphan_tmux.sh）。
7. **AC7 [回归测试零孤儿]**：MVP 6-9 的 acceptance 测试套件全部 cargo test 跑完后，`/tmp/tmux-$UID/ccbd-*` socket 中通过 `ls-sessions` 探测失败的孤儿数为 0。CI 流程加入这条断言。**不**用 PPID 判据。

---

## 2. 状态机激活范围 (Delta)

本 MVP **不变更**任何 agent / job / session 状态机定义。tmux server 的生命周期外部化到 systemd cgroup，不进入 ccbd-rust 自己的状态机。

唯一新增的语义是 **tmux server scope unit naming**：
- scope unit name: `ccbd-tmux-<socket_short>.scope`（其中 socket_short = socket_name 前 8 字符）
- 由 systemd 管理生命周期，ccbd-rust 不在 DB 跟踪 scope 状态

---

## 3. R-* 需求切割矩阵 (Scope Definitions)

| Req ID | Description | MVP 1-9 状态 | MVP 10 更新状态 | 备注 |
|---|---|---|---|---|
| **R-CGROUP-1** | Agent 进程 cgroup 绑定 | 🟢 Full | 🟢 Full | 无变更（mvp1 已落地）|
| **R-CGROUP-2** | tmux server cgroup 绑定 | 🔴 Missing | 🟢 **Full** | 本 MVP 主要工作 |
| **R-SHUTDOWN-1** | Production 优雅关闭 | 🔴 Missing | 🟢 **Full** | tokio::signal handler |
| **R-CLEANUP-1** | Socket 文件显式回收 | 🔴 Missing | 🟢 **Full** | kill-server 后 remove_file |
| **R-TEST-HARNESS-1** | 测试 harness 不依赖 Drop | 🔴 Brittle | 🟢 **Full** | systemd-run scope 兜底 |
| **R-DIAG-2** | Doctor 检测孤儿 tmux | ⚪ N/A | 🟢 **Full** | 新增 doctor 检查项 |
| **R-OPS-1** | 现场清理脚本 | ⚪ N/A | 🟢 **Full** | scripts/cleanup_orphan_tmux.sh |

---

## 4. 范围分阶段（实施视角）

### G10.0：Tmux Server cgroup 绑定（核心架构修复）
- 改造 `src/tmux/session.rs` 的 server spawn 路径，包一层 `systemd-run --user --scope`
- 处理 `systemd-run` 不可用时的 fallback（与 sandbox.systemd_run_available 字段对齐）
- 单测覆盖 argv 构造正确性
- **Checkpoint**：spawn 出来的 tmux server 归属于 ccbd-agents.slice 下的独立 scope

### G10.1：Production Graceful Shutdown
- ccbd-rust 主程序 main loop 拦 SIGTERM/SIGINT
- 退出前直接执行 `tmux kill-session -t ccbd-agents` → `tmux kill-server` → 50ms sleep → `remove_file(socket_path)`（不调 TmuxServer::kill_session_window，那是 window 级 API）
- **Checkpoint**：`kill -TERM <ccbd_pid>` 后无残留 tmux 进程（ls-sessions 探测失败）+ 无残留 socket 文件

### G10.2：测试 Harness 强化 + Socket 显式回收
- `tests/mvp*_acceptance.rs` 的 Harness::new 用 systemd-run scope 包 tmux 启动（`scope_policy_for_test`）
- Harness::Drop 内增加 socket file remove
- 添加专项集成测试 `tests/mvp10_acceptance.rs` 验证 SIGKILL 场景（wrapper scope 模型）下零残留
- **Checkpoint**：cargo test 全套跑完后，`/tmp/tmux-$UID/ccbd-*` 中 ls-sessions probe 失败的孤儿数为 0（活 socket 允许存在；不再用 PPID=1 或全量 socket 归零作为判据）

### G10.3：诊断 + 一次性清理 + CI 守门
- `scripts/cleanup_orphan_tmux.sh` 一次性清理脚本
- `ccb doctor` 增加孤儿检测项
- CI 加入 `tests/mvp10_acceptance.rs::test_no_orphan_tmux_after_test_suite` 守门断言
- **Checkpoint**：回归测试零孤儿；开发者执行 `ccb doctor` 即可看到本地是否有遗留

---

## 5. 跟前后 MVP 的接口约束

- **Agent spawn 路径不变**：`src/sandbox/systemd.rs` 维持原状
- **TmuxServer 公开 API 不变**：`new_session` / `ensure_session` / `kill_session_window` 签名保持
- **新增内部抽象**：`src/tmux/scope.rs` 模块封装 systemd-run scope wrapping，不暴露到 RPC 层
- **Doctor RPC**：复用 mvp9 的 `ccb doctor` CLI 命令，仅增加输出项，不新增 RPC 方法
- **CI 兼容**：`tests/mvp10_acceptance.rs::test_no_orphan_tmux_after_test_suite` 在 CI 末尾运行，要求当前 user 必须能用 systemd user session（CI 环境必须支持 `systemd-run --user`）

---

## 6. 核心架构决断 (Architectural Decisions)

### 决断 1：tmux server 是否应该走 systemd-run scope
**推荐：是，且必须**

- 与 agent 进程现有的 `BindsTo=ccbd-rust.service` 机制完全对称
- ccbd-rust 重写 ccb 的核心承诺就是让 OS cgroup 接管资源治理，而不是依赖应用层 best-effort
- 替代方案"tmux 内置 exit-empty/exit-unattached"被否决——存在 race condition（agent spawn 前 server 可能因空闲自杀），且把状态外部化到 tmux 配置不利于排查

### 决断 2：Graceful shutdown 与 systemd-run scope 是否冗余
**推荐：互补，两条都要**

- systemd-run scope 处理 SIGKILL / OOM / panic / abort 等"主进程死时"路径
- Graceful shutdown 处理"主进程正常 SIGTERM 退出"路径——此时 systemd 不会马上 SIGTERM scope（因为 scope 跟 service unit 绑定，service 还没 stop）
- 两者覆盖正交场景，缺一不可

### 决断 3：测试 Harness 改造是否要保留 Drop
**推荐：保留 Drop 作为应用层兜底，但不是唯一保障**

- Drop 在正常 panic-unwind 场景下有效，先跑 Drop 再让 scope 兜底
- 单一依赖任何一层都不够鲁棒，两层 belt-and-suspenders

### 决断 4：socket 文件孤儿是否需要主动清理
**推荐：必须主动清理**

- tmux server 自己**不会**在 kill-server 时删 socket 文件——这是 tmux 设计如此（保留 socket 用于其他客户端 attach 检测）
- 当前观察到 2878 个 stale socket 印证了"不主动删 = 必然累积"
- 实施成本极小：每次 kill-server 后一行 `fs::remove_file(socket_path).ok()`

### 决断 5：scope unit naming 策略
**推荐：`ccbd-tmux-<socket_short>.scope`**

- 用 socket_name 前 8 字符（来自 sha256 state_dir）保证 deterministic
- 与 agent 的 `ccbd-agent-<agent_id>` naming 对称，便于 `systemctl --user list-units` 排查
- 不带时间戳（避免重启后 unit 名漂移影响 reconcile）

### 决断 6：CI 守门断言要不要 hard fail
**推荐：hard fail**

- 验收的硬指标就是"零孤儿"。任何一个孤儿出现都说明本 MVP 的核心承诺被打破
- 软警告会随时间被忽略，导致回归

### 决断 7：BindsTo unit 不存在场景的 fallback 策略（plan review Round 1 修订）
**推荐：分层 fallback——Systemd 包裹仍然包，但 BindsTo 可选**

- **生产模式**（systemctl --user start ccbd-rust.service 启动）：scope 包 + BindsTo=ccbd-rust.service —— 完整 cgroup 生命周期绑定
- **开发模式**（cargo run / nohup 启动）：scope 包 + binds_to=None —— 仍享受 cgroup 隔离与 `--collect` 自动 unit 清理，但孤儿兜底由 G10.1 graceful shutdown + startup reconcile sweep 联合负责
- **受限环境**（CI 容器无 user systemd）：ScopePolicy::None —— 完全 fallback 到原始 spawn，仅靠 graceful shutdown + Drop 兜底

**理由**：Gemini Round 1 plan review 指出，单一 BindsTo=ccbd-rust.service 假设 ccbd-rust 总是跑在该 service unit 下，但 `cargo run --bin ccbd` / `nohup ccbd` 等手动启动场景下该 unit 不存在，systemd-run 会因依赖缺失而启动失败 → ccbd-rust 完全起不来。`detect_scope_policy` 探测 /proc/self/cgroup 决定运行模式，分层降级保兼容。
