# Kiro Tasks: MVP 10 (进程生命周期 cgroup 治理与资源彻底回收)

> 文档定位：MVP 10 由 Codex 逐项实施的原子任务清单。本文基于 mvp10-R.md / mvp10-D.md，将 tmux server cgroup 绑定、production graceful shutdown、测试 harness 强化、socket 回收、doctor/cleanup script/CI 守门拆分成可独立验证的任务。
>
> **Plan-Review 修订记录（2026-05-01 Gemini Round 1）**：
> - 🔴 T0.1 把 `ScopeConfig + use_systemd_scope: bool` 升级为 `enum ScopePolicy { Systemd(UnitConfig), None }` + `binds_to: Option<String>`，解决 cargo run / nohup 模式下 BindsTo unit 不存在导致 systemd-run 启动失败的关键漏点
> - 🔴 T0.1 新增 `detect_scope_policy()`：基于 /proc/self/cgroup 检测 ccbd-rust 是否在 service unit 下，决定 binds_to 是 Some 还是 None
> - 🟡 T1.2 cleanup_tmux_resources 在 kill-server 与 remove_file 之间加 50ms sleep，避免 fd 释放 race
> - 🟡 实施顺序调整：T3.2 cleanup_orphan_tmux.sh 提前到 G10.-1（Stage Pre-Cleanup），先清现场再修架构
>
> **Plan-Review 修订记录（2026-05-01 Codex Round 2）**：原 verdict FAIL（overall 6.0），5 个 blocking 已修：
> - 🔴 R AC1/AC3/AC7、D §4.3、T2.4：放弃 PPID=1 作为孤儿判据（daemonized tmux server 在 systemd scope 下也 PPID=1，按 PPID 杀会误判）。改为 ls-sessions probe + cgroup unit 状态对账
> - 🔴 R AC3、D §4.1、T2.4：测试 harness 与 cargo test 生命周期绑定通过 wrapper scope 模型——专项 SIGKILL 测试用 systemd-run 包 helper child + 注入 CCBD_TEST_WRAPPER_SCOPE env var；普通 acceptance 测试 binds_to=None 走 fallback
> - 🔴 D §5.2、T(-1).1：cleanup script 判据从 PPID 改为 ls-sessions probe，不再误杀活 server
> - 🔴 D §2.2、T0.2、T2.2：ScopePolicy 修订贯穿全部文档——彻底删除残留的 ScopeConfig/use_systemd_scope bool 引用，统一 new_with_policy 接口
> - 🔴 D §3.2：cleanup_tmux_resources 改用 `tmux kill-session -t ccbd-agents` 替代 kill_session_window（后者是 window 级 API）
> - 🟡 ScopePolicy/UnitConfig 增加 derive Clone, Debug, PartialEq, Eq
>
> **Plan-Review 修订记录（2026-05-01 Codex Round 3）**：Round 2 verdict FAIL（overall 6.9），3 个 blocking 残留全部清除：
> - 🔴 T1.2 步骤明确禁止 ctx.tmux_server.kill_session_window 调用，改为直接 tmux kill-session
> - 🔴 R §4 G10.1/G10.2 checkpoint、T §0.1 G10.2 checkpoint、T(-1).2 验收：从"PPID=1 ccbd- 数为 0 / 全量 socket 归零"改为"ls-sessions probe 失败的孤儿数为 0；活 socket 允许存在"
> - 🔴 R AC3 文本修正：BindsTo 是 unit-to-unit 不是 unit-to-PID；wrapper scope 通过 systemd-run 创建独立 unit，helper child 跑在该 unit 内
> - 🟡 D §4.3 systemd-run helper 改用 --setenv 显式注入 CCBD_TEST_WRAPPER_SCOPE
> - 🟡 D §5.2 cleanup script pgrep 改用 ps + awk 字段匹配，避免 socket name 特殊字符误匹配
> - 🟡 D §1 架构图 G10.1 节去掉 kill_session_window 字样
> - 🟡 T3.5 commit list 去掉 cleanup_orphan_tmux.sh（已在 G10.-1 commit）
> - 🟡 D §7 AC 测试映射加强：AC4 增加 cleanup helper 单测；AC5 增加 idempotent 测试 + 活 server 保留断言
>
> 范围约束：严格按 MVP10 R/D 落地，不引入新 RPC，不修改 agent sandbox 路径，不改 mvp9 layout 算法。
>
> 实施顺序：本文按 4 个物理 stage 拆分（外加 G10.-1 现场预清理）：G10.-1 Pre-Cleanup、G10.0 Tmux cgroup 绑定、G10.1 Production Graceful Shutdown、G10.2 测试 Harness + Socket 回收、G10.3 Doctor + CI 守门。每个 stage 末尾都有 commit checkpoint。
>
> 实施纪律：本 MVP 是结构性资源治理修复，**禁止用 if-else 补丁式修法**——所有修改必须落在"重画接口契约"或"补全应用层缺失路径"的语义范围内，不允许在错位的位置加 `if orphan { kill_it }` 的 ad-hoc 逻辑。

---

## 0. 总览

### 0.1 Stage 划分

| Stage | 主题 | 目标 | Checkpoint |
|---|---|---|---|
| **G10.-1** | **Pre-Cleanup（现场预清理）** | **scripts/cleanup_orphan_tmux.sh + 一次性清现场** | **当前机器孤儿数为 0；旧 socket 全清；干净基线进入 G10.0** |
| G10.0 | Tmux cgroup 绑定 | systemd-run scope 包装 tmux server spawn | tmux 进程归属于 ccbd-agents.slice/ccbd-tmux-*.scope |
| G10.1 | Production Graceful Shutdown | SIGTERM/SIGINT handler + reconcile 清 stale socket | `kill -TERM <ccbd>` 后 ls-sessions probe 失败的孤儿数为 0 + 对应 socket 文件已删 |
| G10.2 | 测试 Harness + Socket 回收 | 测试 harness 走 scope + Drop 增加 socket remove | cargo test 全套跑完后 ls-sessions probe 失败的 ccbd- 孤儿数为 0（活 socket 允许存在）|
| G10.3 | Doctor + CI | doctor 检查 / CI 守门 | `ccb doctor` 报告孤儿数 + CI hard fail 守门 |

### 0.2 任务依赖图

```
graph TD
  subgraph GM1[G10.-1 Pre-Cleanup]
    Tm1[T(-1).1 cleanup_orphan_tmux.sh]
    Tm2[T(-1).2 一次性清理]
    Tm3[T(-1).3 G10.-1 commit]
  end

  subgraph G100[G10.0 Tmux cgroup 绑定]
    T01[T0.1 新建 src/tmux/scope.rs]
    T02[T0.2 改造 TmuxServer 结构]
    T03[T0.3 ensure_session 走 scope wrapper]
    T04[T0.4 单测覆盖 argv 构造]
    T05[T0.5 G10.0 commit]
  end

  subgraph G101[G10.1 Production Graceful Shutdown]
    T11[T1.1 main loop tokio::signal handler]
    T12[T1.2 cleanup_tmux_resources 实现]
    T13[T1.3 startup reconcile stale socket sweep]
    T14[T1.4 集成测试 SIGTERM 路径]
    T15[T1.5 G10.1 commit]
  end

  subgraph G102[G10.2 测试 Harness + Socket 回收]
    T21[T2.1 抽 Harness common util]
    T22[T2.2 改造 mvp*_acceptance.rs Harness::new]
    T23[T2.3 Drop 增加 socket remove + cleanup_server 增强]
    T24[T2.4 新增 tests/mvp10_acceptance.rs 5 个核心场景]
    T25[T2.5 G10.2 commit]
  end

  subgraph G103[G10.3 Doctor + Cleanup + CI]
    T31[T3.1 src/cli/doctor.rs 加 check_tmux_orphans]
    T32[T3.2 scripts/cleanup_orphan_tmux.sh]
    T33[T3.3 CI 集成 test_no_orphan_tmux_after_test_suite]
    T34[T3.4 一次性清理当前现场（运维任务）]
    T35[T3.5 G10.3 commit + MVP10 收尾]
  end

  Tm1 --> Tm2 --> Tm3
  Tm3 --> T01 --> T02 --> T03 --> T04 --> T05
  T05 --> T11 --> T12 --> T13 --> T14 --> T15
  T15 --> T21 --> T22 --> T23 --> T24 --> T25
  T25 --> T31 --> T33 --> T35
```

注：T32（cleanup_orphan_tmux.sh）和 T34（一次性清理）已迁移到 G10.-1 Pre-Cleanup 阶段（T(-1).1 / T(-1).2），原编号在 §4 G10.3 节保留作 cross-reference。

### 0.3 风险说明

MVP 10 的最大风险是 systemd-run scope 在某些环境（CI 容器、受限沙箱）不可用时的 fallback 路径——必须确保 fallback 路径不破坏 mvp1-9 的现有测试。

实施纪律：
- **禁止补丁式 if-else**。如果实施过程中冒出"在 X 处加 if 判断"的冲动，停下来追根因——通常意味着接口契约错位，应该重画接口而不是堆 if
- 每个 task 完成必须立即编译 + 单测通过才能动下一个
- 任何 stage 出现"修一个 bug 紧接着冒下一个相关 bug"的模式立刻停下来报告

---

## 0.5. 原子任务定义（G10.-1 Pre-Cleanup 现场预清理）

> **修订说明**：此 Stage 由 Round 1 plan review 调整顺序而新增——在做架构修复前先清掉历史孤儿，避免新代码验证被旧污染干扰。原 T3.2 / T3.4 内容前置到这里（编号保留 cross-reference 用）。

### T(-1).1: scripts/cleanup_orphan_tmux.sh（前置）

- 文件路径:
    - 新建: scripts/cleanup_orphan_tmux.sh
- 输入:
    - mvp10-D.md §5.2（Round 2 修订版）
- 输出:
    - bash 脚本，幂等清理基于 ls-sessions probe 判据的孤儿 socket + 残留 workdir
    - **判据**：对每个 `/tmp/tmux-$UID/ccbd-*` socket 跑 `tmux -L <name> ls-sessions`，失败的才是真孤儿。**不**用 PPID。活 server 一律不动（无论 PPID）。
    - chmod +x
- 依赖: 无（纯运维脚本）
- 执行步骤:
    1. 按 D §5.2 写脚本（注意是 Round 2 修订版的 ls-sessions probe 判据，不是初版的 PPID 判据）
    2. shellcheck 通过
    3. chmod +x
    4. 添加文件头 license 与用法说明注释，说明判据原理
- 验收:
    - shellcheck scripts/cleanup_orphan_tmux.sh 无报错
    - 跑一次后：(a) 孤儿 socket 数为 0；(b) 用户其他普通 tmux session（非 ccbd- 前缀）完全不受影响；(c) 当前活 ccbd-rust 实例（如果有）的 socket 仍在

### T(-1).2: 一次性清理当前现场

- 文件路径:
    - N/A（运维操作）
- 输入:
    - T(-1).1 输出的 cleanup_orphan_tmux.sh
- 输出:
    - **stale socket 数归零**（活 ccbd-rust 实例的 socket 不算 stale，允许保留）
    - 无主 /tmp/.tmp??????/pipes 残留显著下降（被活 process 引用的不动）
- 依赖: T(-1).1
- 执行步骤:
    1. 执行 `bash scripts/cleanup_orphan_tmux.sh`
    2. 验证孤儿指标归零（活 socket 仍允许存在）
    3. （可选）记录清理前后的 RSS 节省量到 mvp10-T 末尾"实施收尾"章节
- 验收（Round 2 修订：不再用 PPID 或全量归零）:
    - 对 `/tmp/tmux-$(id -u)/ccbd-*` 中每个 socket 跑 `tmux -L <name> ls-sessions`，失败的孤儿数为 0
    - 当前活的 ccbd-rust 实例（如有）的 socket 仍存在且 ls-sessions 成功
    - `find /tmp -maxdepth 1 -type d -name '.tmp??????' | wc -l` 显著下降（无主 workdir 被清；活的 ccbd state_dir 不动）

### T(-1).3: G10.-1 commit

- 输入:
    - T(-1).1, T(-1).2 完成
- 输出:
    - 一个 commit
- 执行步骤:
    1. shellcheck 通过
    2. commit: `chore(mvp10): G10.-1 add cleanup_orphan_tmux.sh script`
- 验收:
    - 该 commit 仅含 scripts/cleanup_orphan_tmux.sh 一个文件

---

## 1. 原子任务定义（G10.0 Tmux cgroup 绑定）

### T0.1: 新建 src/tmux/scope.rs（systemd-run wrapper）

- 文件路径:
    - 新建: src/tmux/scope.rs
    - 修改: src/tmux/mod.rs（pub mod scope）
- 输入:
    - mvp10-D.md §2.1（Round 1 plan review 修订版）
- 输出:
    - `pub struct UnitConfig` (derive Clone, Debug, PartialEq, Eq): unit_name / slice / binds_to: Option<String>
    - `pub enum ScopePolicy { Systemd(UnitConfig), None }` (derive Clone, Debug, PartialEq, Eq)
    - `pub fn wrap_in_scope(base_cmd: &str, base_args: &[&str], policy: &ScopePolicy) -> Command`
    - `pub fn unit_name_for_socket(socket_name: &str) -> String`
    - `pub fn detect_scope_policy(socket_name: &str) -> ScopePolicy`
- 依赖: 无
- 执行步骤:
    1. 新建 src/tmux/scope.rs，按 D §2.1 实现 UnitConfig + ScopePolicy enum + wrap_in_scope + unit_name_for_socket + detect_scope_policy
    2. wrap_in_scope 在 ScopePolicy::Systemd 时返回 `systemd-run --user --scope --collect --unit=<u> --slice=<s> [--property=BindsTo=<b>] -- <base_cmd> <base_args...>` 的 Command；BindsTo 仅在 binds_to=Some 时附加
    3. ScopePolicy::None 直接返回原始 Command（fallback）
    4. unit_name_for_socket 取 socket_name 去掉 "ccbd-" 前缀后的前 8 字符，前缀加 "ccbd-tmux-"
    5. detect_scope_policy 决策树：
        - systemd_run_available()=false → ScopePolicy::None
        - systemd_run_available()=true + detect_self_in_service()=true → Systemd { binds_to: Some("ccbd-rust.service".into()) }
        - systemd_run_available()=true + detect_self_in_service()=false → Systemd { binds_to: None }
    6. systemd_run_available 用 Command::new("systemd-run").args(["--user","--scope","--","true"]).output() 探测
    7. detect_self_in_service 读 /proc/self/cgroup 看路径是否包含 "ccbd-rust.service"
    8. src/tmux/mod.rs 增加 `pub mod scope;` 导出
- 验收:
    - cargo build 通过
    - 单测验证三个分支：Systemd+BindsTo / Systemd+无 BindsTo / None
    - 调用 wrap_in_scope("tmux", &["-L", "test"], &Systemd(...full...)) 返回 Command 的 args 包含 "systemd-run", "--user", "--scope", "--collect", "--property=BindsTo=ccbd-rust.service", "tmux", "-L", "test"
    - Systemd { binds_to: None } 时不出现 --property=BindsTo
    - ScopePolicy::None 返回 args 仅 "-L", "test"

---

### T0.2: 改造 TmuxServer 结构

- 文件路径:
    - 修改: src/tmux/session.rs
- 输入:
    - mvp10-D.md §2.2（Round 1 plan review 修订版）
    - T0.1 输出的 scope.rs API（ScopePolicy enum）
- 输出:
    - TmuxServer 结构增加 `scope_policy: ScopePolicy` 字段（替代之前设计的 `use_systemd_scope: bool`）
    - 新增 `pub fn new_with_policy(state_dir: &Path, policy: ScopePolicy) -> Self`
    - 现有 `pub fn new(state_dir: &Path) -> Self` 改为：先 compute socket_name，再调用 detect_scope_policy(socket_name)，再 new_with_policy
- 依赖: T0.1
- 执行步骤:
    1. TmuxServer struct 增加 scope_policy: ScopePolicy 字段
    2. 新增 new_with_policy 构造函数
    3. 把 new 改为：
        ```rust
        pub fn new(state_dir: &Path) -> Self {
            let socket_name = compute_socket_name(state_dir);
            let policy = scope::detect_scope_policy(&socket_name);
            Self::new_with_policy(state_dir, policy)
        }
        ```
    4. 暂不改 ensure_session 实现（留给 T0.3）
- 验收:
    - cargo build 通过
    - mvp1-9 acceptance tests 全部 cargo test 通过（行为 identity 保持）
    - 测试 harness 可以通过 new_with_policy 显式注入 ScopePolicy::None 走 fallback

---

### T0.3: ensure_session 走 scope wrapper

- 文件路径:
    - 修改: src/tmux/session.rs（ensure_session 实现）
- 输入:
    - mvp10-D.md §2.2（Round 1 plan review 修订版）
- 输出:
    - ensure_session 内部 spawn tmux new-session 时通过 scope::wrap_in_scope 构造 Command
    - 仅 `new-session -d` 调用包 scope，其他 tmux 子命令（send-keys / kill-pane / capture-pane 等）保持原状
- 依赖: T0.1, T0.2
- 执行步骤:
    1. 找到 ensure_session 内 `Command::new("tmux") -L ... new-session -d` 的构造点
    2. 替换为：
        ```rust
        let mut cmd = scope::wrap_in_scope(
            "tmux",
            &["-L", &self.socket_name, "new-session", "-d", "-s", session_name, "-c", workdir, "-x", "200", "-y", "60"],
            &self.scope_policy,
        );
        ```
    3. 其他 tmux 控制命令不修改（attach 到现有 server，不 spawn 新进程）
- 验收:
    - cargo build 通过
    - 手动跑（systemctl 启动模式）：起一个 ccbd-rust service 实例，spawn agent，然后 `systemctl --user list-units --type=scope | grep ccbd-tmux-` 应能看到对应 scope unit；`cat /proc/<tmux_pid>/cgroup` 应包含 `/user.slice/.../ccbd-agents.slice/ccbd-tmux-XXXXXXXX.scope`
    - 手动跑（cargo run 模式）：`systemctl --user list-units --type=scope | grep ccbd-tmux-` 应能看到 scope，但 `systemctl --user show ccbd-tmux-XXXXXXXX.scope -p BindsTo` 应输出空（即没绑 BindsTo）

---

### T0.4: 单测覆盖 argv 构造

- 文件路径:
    - 修改: src/tmux/scope.rs（#[cfg(test)] 模块）
- 输入:
    - T0.1 输出
- 输出:
    - test_wrap_in_scope_with_systemd: 验证完整 argv 序列
    - test_wrap_in_scope_fallback: 验证 systemd_run_available=false 时返回原始 cmd
    - test_unit_name_for_socket: 验证 ccbd-abc123def456789a → ccbd-tmux-abc123de
- 依赖: T0.1
- 执行步骤:
    1. 在 src/tmux/scope.rs 末尾添加 `#[cfg(test)] mod tests`
    2. 实现三个单测，断言 Command::get_program() 和 get_args() 序列
- 验收:
    - cargo test --lib tmux::scope 通过

---

### T0.5: G10.0 commit

- 输入:
    - T0.1 ~ T0.4 完成
- 输出:
    - 一个原子 commit
- 执行步骤:
    1. cargo fmt + cargo clippy --all-targets --all-features -- -D warnings
    2. cargo test 全套跑通
    3. git add 仅本 stage 涉及文件
    4. commit message: `feat(mvp10): G10.0 wrap tmux server spawn in systemd-run cgroup scope`
- 验收:
    - mvp1-9 + 新增的 scope 单测全部 cargo test --all-targets 通过
    - 该 commit 不含本 stage 之外的修改

---

## 2. 原子任务定义（G10.1 Production Graceful Shutdown）

### T1.1: main loop 注册 tokio::signal handler

- 文件路径:
    - 修改: src/bin/ccbd.rs（如不存在，定位到 ccbd-rust daemon 主程序入口）
- 输入:
    - mvp10-D.md §3.1
- 输出:
    - main 函数内 tokio::select! 分支：accept loop vs shutdown signal
    - 收到 SIGTERM/SIGINT 时调用 cleanup_tmux_resources(&ctx).await 后才退出
- 依赖: G10.0 完成
- 执行步骤:
    1. 定位 daemon 主程序文件（git log / src/bin/ 内查找）
    2. 在 tokio::main 内、accept loop 启动前注册 SIGTERM 和 SIGINT signal stream
    3. 用 tokio::select! 包住 main_serve_loop 和 shutdown_signal
    4. shutdown 分支调用 cleanup_tmux_resources（T1.2 提供）
- 验收:
    - cargo build 通过
    - 不破坏现有 mvp1-9 测试

---

### T1.2: cleanup_tmux_resources 实现

- 文件路径:
    - 修改: src/bin/ccbd.rs 或 src/orchestrator/shutdown.rs（按 ccbd-rust 现有结构选定）
- 输入:
    - mvp10-D.md §3.2（Round 2 plan review 修订版）
- 输出:
    - `async fn cleanup_tmux_resources(ctx: &Ctx)`，五步：tmux kill-session → tmux kill-server → 50ms sleep → remove_file(socket_path)
- 依赖: T1.1
- 执行步骤:
    1. 实现 cleanup_tmux_resources 函数
    2. **第一步**（Round 2 修订）：直接调 `Command::new("tmux").args(["-L", &socket_name, "kill-session", "-t", SESSION_NAME])` —— **不**调 ctx.tmux_server.kill_session_window，那是 window 级 API，session 没关 server 不会退出
    3. 第二步：`Command::new("tmux").args(["-L", &socket_name, "kill-server"])`，错误仅 warn
    4. 第三步：`tokio::time::sleep(Duration::from_millis(50)).await` —— Round 1 修订点：避免 kill-server 与 remove_file 之间 fd 释放 race
    5. 第四步：构造 socket_path = format!("/tmp/tmux-{}/{}", geteuid(), socket_name)，std::fs::remove_file，NotFound 容忍
    6. 所有 warn 走 tracing::warn!
- 验收:
    - cargo build 通过
    - 单测：mock ctx 调用 cleanup_tmux_resources 不 panic
    - 集成测试覆盖：调用后 50ms+ `tmux -L <socket> ls-sessions` 探测失败 + socket 文件不存在
    - 不允许出现 ctx.tmux_server.kill_session_window(SESSION_NAME) 这种调用（语义错误）

---

### T1.3: startup reconcile stale socket sweep

- 文件路径:
    - 修改: src/orchestrator/mod.rs 或 src/main.rs reconcile 函数（mvp9 G9.1 已有 reconcile_all_sessions）
- 输入:
    - mvp10-D.md §3.4 方案 A
- 输出:
    - 在 reconcile 函数末尾增加 sweep_stale_sockets 调用
    - sweep_stale_sockets 扫描 /tmp/tmux-$UID/ 下 ccbd-* 文件，对每个尝试 `tmux -L <name> ls-sessions`，失败即 remove_file
- 依赖: T1.2
- 执行步骤:
    1. 在现有 reconcile 路径找到合适插入点（已有 reconcile_all_sessions 的同一函数末尾）
    2. 实现 fn sweep_stale_sockets()，跳过当前活 ccbd-rust 拥有的 socket（基于 ctx.tmux_server.socket_name）
    3. 用 std::process::Command::new("tmux") -L <name> ls-sessions，stdout/stderr 都丢到 /dev/null
    4. 失败的 socket 调用 std::fs::remove_file
- 验收:
    - 单测覆盖：在临时目录注入一个 fake socket（touch /tmp/tmux-$UID/ccbd-fakedeadbeef0000），跑 sweep，断言文件被删

---

### T1.4: 集成测试 SIGTERM 路径

- 文件路径:
    - 新建: tests/mvp10_acceptance.rs（仅 SIGTERM + reconcile 场景；SIGKILL wrapper scope 模型留给 T2.4 实现）
- 输入:
    - mvp10-D.md §4.3 测试 #2 #4
- 输出:
    - test_main_sigterm_cleans_resources
    - test_startup_reconcile_cleans_stale_sockets
    - test_main_sigkill_systemd_cleans **占位**（仅写 placeholder + `unimplemented!()` 或 `#[ignore]`，最终由 T2.4 wrapper scope 模型实现）
- 依赖: T1.1, T1.2, T1.3
- 执行步骤:
    1. test_main_sigterm_cleans_resources:
       - spawn 一个 ccbd-rust 子进程（用 ccbd::start_project 等内部 API 或 fork 真 daemon）
       - 等其起 tmux server
       - 向 daemon PID 发 SIGTERM
       - 5s 超时内轮询：`tmux -L <socket> ls-sessions` 探测失败 + socket 文件不存在（不用 PPID 判定）
    2. test_main_sigkill_systemd_cleans 占位（T2.4 重写为 wrapper scope 模型）
    3. test_startup_reconcile_cleans_stale_sockets:
       - 在 /tmp/tmux-$UID/ 注入 ccbd-fakedeadbeef0000 空文件
       - 跑一次 reconcile_all_sessions（直接调函数，不走 daemon spawn）
       - 断言文件被删
- 验收:
    - cargo test --test mvp10_acceptance 中 SIGTERM + reconcile 两个测试通过；SIGKILL 测试占位不阻塞

---

### T1.5: G10.1 commit

- 输入:
    - T1.1 ~ T1.4 完成
- 输出:
    - 一个原子 commit
- 执行步骤:
    1. cargo fmt + cargo clippy 干净
    2. cargo test 全套通过
    3. commit: `feat(mvp10): G10.1 production graceful shutdown + reconcile stale socket sweep`
- 验收:
    - mvp1-9 全套 + mvp10 当前已实现的测试都通过

---

## 3. 原子任务定义（G10.2 测试 Harness + Socket 回收）

### T2.1: 抽 Harness common util

- 文件路径:
    - 新建: tests/common/mod.rs 或 tests/_harness.rs（避免每个 mvp*_acceptance 各写一份）
- 输入:
    - mvp10-D.md §4.1（Round 2 修订版）
    - 现有 tests/mvp6/7/8/9_acceptance.rs 的 Harness 实现
- 输出:
    - 公共 Harness 类型（或一个工厂函数）
    - 公共 `scope_policy_for_test(socket_name: &str) -> ScopePolicy` 探测函数（替代之前设计的 can_use_systemd_run() bool）
    - 公共 `can_use_systemd_run() -> bool` 工具函数
- 依赖: G10.0, G10.1 完成
- 执行步骤:
    1. 评估当前各 mvp*_acceptance.rs 的 Harness 共性
    2. 抽公共代码到 tests/common/mod.rs，按 cargo 测试模块约定
    3. 实现 `scope_policy_for_test(socket_name)`：
        - 若 can_use_systemd_run()=false → ScopePolicy::None
        - 否则 ScopePolicy::Systemd(UnitConfig { binds_to: env::var("CCBD_TEST_WRAPPER_SCOPE").ok(), ... })
        - CCBD_TEST_WRAPPER_SCOPE 环境变量由 CI / SIGKILL 专项测试通过 systemd-run wrapper 设置；不存在时 binds_to=None（开发模式 fallback）
    4. 不再保留 new_with_scope(bool) helper——统一走 new_with_policy
- 验收:
    - cargo build --tests 通过
    - 不引入新 panic
    - 单测覆盖 scope_policy_for_test 三种返回情况：None / Systemd+binds_to=None / Systemd+binds_to=Some

---

### T2.2: 改造 mvp*_acceptance.rs Harness::new

- 文件路径:
    - 修改: tests/mvp6_acceptance.rs
    - 修改: tests/mvp7_acceptance.rs
    - 修改: tests/mvp8_acceptance.rs
    - 修改: tests/mvp9_acceptance.rs
- 输入:
    - T2.1 抽出的 common util（scope_policy_for_test）
    - mvp10-D.md §4.1（Round 2 修订版）
- 输出:
    - 各 acceptance 文件的 Harness::new 改用 `TmuxServer::new_with_policy(state_dir, scope_policy_for_test(&socket_name))`
    - 不破坏现有断言
- 依赖: T2.1
- 执行步骤:
    1. 每个 mvp*_acceptance.rs 把 Arc::new(TmuxServer::new(&path)) 替换为：
        ```rust
        let socket_name = compute_socket_name(state_dir.path());
        let policy = scope_policy_for_test(&socket_name);
        Arc::new(TmuxServer::new_with_policy(state_dir.path(), policy))
        ```
    2. 不改测试断言
    3. 完整跑 cargo test --test mvp6_acceptance 等，逐个验证
- 验收:
    - mvp6/7/8/9 全部 cargo test 通过
    - 测试在 CCBD_TEST_WRAPPER_SCOPE 未设置时 binds_to=None（开发模式正常工作）

---

### T2.3: Drop 增加 socket remove + cleanup_server helper 增强

- 文件路径:
    - 修改: tests/mvp6_acceptance.rs / mvp7 / mvp8 / mvp9 / mvp10 的 Drop impl
    - 修改: src/tmux/mod.rs 测试 helper cleanup_server
- 输入:
    - mvp10-D.md §4.1, §4.4
- 输出:
    - 所有 Harness Drop 在 kill-server 后增加 fs::remove_file(socket_path)
    - src/tmux/mod.rs 测试模块的 cleanup_server 同样增强
- 依赖: T2.2
- 执行步骤:
    1. Harness struct 加 socket_name 字段（或直接通过 ctx.tmux_server.socket_name() 取）
    2. Drop 内 kill-server 之后追加 remove_file，错误吞掉
    3. src/tmux/mod.rs:42 cleanup_server helper 同步增强
- 验收:
    - cargo test 全套通过
    - 跑完测试后 ls /tmp/tmux-$UID/ccbd-* 应不再出现属于本次测试的 socket 文件残留

---

### T2.4: 新增 tests/mvp10_acceptance.rs 5 个核心场景（补完）

- 文件路径:
    - 修改: tests/mvp10_acceptance.rs（T1.4 已建文件）
- 输入:
    - mvp10-D.md §4.3（Round 2 修订版，全部判据从 PPID 改为 cgroup unit / ls-sessions probe）
- 输出:
    - 5 个测试函数全部实现：
        - test_tmux_server_in_scope（cgroup 路径断言）
        - test_main_sigterm_cleans_resources（T1.4 已实现，判据修订）
        - test_main_sigkill_systemd_cleans（T1.4 已实现，**用 wrapper scope 模型重写**）
        - test_startup_reconcile_cleans_stale_sockets（T1.4 已实现）
        - test_no_orphan_tmux_after_test_suite（标记 #[ignore]，CI 单独跑）
- 依赖: T2.3
- 执行步骤:
    1. test_tmux_server_in_scope：起 TmuxServer，从 server.ensure_session 后的 pane PID 读 /proc/<pid>/cgroup，断言路径包含 "ccbd-agents.slice/ccbd-tmux-"
    2. test_main_sigkill_systemd_cleans 重写为 wrapper scope 模型：
        ```rust
        // 用 systemd-run 包一个 helper child 进程，通过 --setenv 显式注入 wrapper scope env var
        // （Round 3 修订点：用 systemd-run --setenv，不要用 Command.env，避免不同 systemd-run 模式下环境继承差异）
        let unit = format!("ccbd-test-victim-{}", std::process::id());
        let scope_unit = format!("{}.scope", unit);
        let child = Command::new("systemd-run")
            .args([
                "--user", "--scope", "--collect",
                &format!("--unit={}", unit),
                &format!("--setenv=CCBD_TEST_WRAPPER_SCOPE={}", scope_unit),
                "--",
                env!("CARGO_BIN_EXE_ccbd_test_helper"), "--hold-tmux",
            ])
            .spawn()?;
        // 等 child 起 tmux server (pane probe)
        wait_until(|| socket_alive(&expected_socket), 5_secs);
        // 杀 wrapper scope —— systemd 会 SIGTERM 整个 cgroup，包括 child 和 tmux scope
        Command::new("systemctl").args(["--user", "stop", &format!("{}.scope", unit)]).output()?;
        // 验证 5s 内 ccbd-tmux scope 不再存在 + ls-sessions 失败
        wait_until(|| !scope_unit_exists("ccbd-tmux-...") && !socket_alive(&expected_socket), 5_secs);
        ```
    3. 需要新增一个 ccbd_test_helper bin（或 cargo example）：内部起 TmuxServer + ensure_session + sleep forever，模拟一个完整 ccbd 实例
    4. test_no_orphan_tmux_after_test_suite：扫描 /tmp/tmux-$UID/ 下 ccbd-* 文件，对每个跑 ls-sessions，断言 stale 数为 0；标记 `#[ignore]` 让 CI 单独触发
- 验收:
    - cargo test --test mvp10_acceptance（不带 --ignored）4 个场景通过
    - cargo test --test mvp10_acceptance test_no_orphan_tmux_after_test_suite -- --ignored 单独跑通过
    - test_main_sigkill_systemd_cleans 在没有 user systemd 的环境下 panic-with-message 提示"AC3 requires user systemd; running on CI without it"，不静默 skip

---

### T2.5: G10.2 commit

- 输入:
    - T2.1 ~ T2.4 完成
- 输出:
    - 一个原子 commit
- 执行步骤:
    1. cargo fmt + cargo clippy 干净
    2. cargo test --all-targets 通过
    3. cargo test --test mvp10_acceptance test_no_orphan_tmux_after_test_suite -- --ignored 单独跑通过
    4. commit: `feat(mvp10): G10.2 test harness in systemd scope + socket file cleanup`
- 验收:
    - mvp1-9 acceptance 全套 + mvp10 全套（含 ignored）测试通过

---

## 4. 原子任务定义（G10.3 Doctor + CI 守门）

> 修订说明：原 T3.2（cleanup script）和 T3.4（一次性清理）已前置到 G10.-1 Pre-Cleanup，本 stage 仅剩 doctor 集成 + CI 守门 + 收尾 commit。

### T3.1: src/cli/doctor.rs 加 check_tmux_orphans

- 文件路径:
    - 修改: src/cli/doctor.rs
- 输入:
    - mvp10-D.md §5.1
- 输出:
    - check_tmux_orphans 函数
    - 在 ccb doctor 命令的 check 列表里新增此项
- 依赖: G10.2 完成
- 执行步骤:
    1. 实现 check_tmux_orphans，逻辑参照 D §5.1 代码
    2. 注册到 doctor 现有 checks vec
    3. 输出格式：`tmux server orphans: 0 stale (5 alive)` 或 `tmux server orphans: WARN 12 stale, run scripts/cleanup_orphan_tmux.sh`
- 验收:
    - 在 dev 环境跑 ccb doctor，输出能看到该项
    - 注入孤儿后再跑 ccb doctor，stale count > 0

---

### T3.2: ~~scripts/cleanup_orphan_tmux.sh~~ — **已迁移到 G10.-1（T(-1).1）**

> Round 1 plan review 调整：本任务前置到 G10.-1。本节保留任务编号仅作 cross-reference。

---

### T3.3: CI 集成 test_no_orphan_tmux_after_test_suite

- 文件路径:
    - 修改: .github/workflows/ci.yml（如使用 GitHub Actions）或对应 CI 配置文件
- 输入:
    - T2.4 已标记 #[ignore] 的 test_no_orphan_tmux_after_test_suite
- 输出:
    - CI 流水线最后一步显式：`cargo test --test mvp10_acceptance test_no_orphan_tmux_after_test_suite -- --ignored --nocapture`
    - 失败 hard fail
- 依赖: T2.4
- 执行步骤:
    1. 定位项目 CI 配置文件
    2. 在 cargo test 步骤后追加 ignored 测试单独跑的步骤
    3. 确保 CI runner 支持 user systemd（GitHub Actions Linux runners 默认支持）
- 验收:
    - 故意制造一个孤儿 tmux 跑 CI，断言 CI 失败
    - 清理后再跑 CI，断言 CI 通过

---

### T3.4: ~~一次性清理当前现场~~ — **已迁移到 G10.-1（T(-1).2）**

> Round 1 plan review 调整：本任务前置到 G10.-1。本节保留任务编号仅作 cross-reference。

---

### T3.5: G10.3 commit + MVP10 收尾

- 输入:
    - T3.1 / T3.3 完成（T3.2 / T3.4 已在 G10.-1 提交，本 stage 不重复）
- 输出:
    - 两个 commit:
      - `feat(mvp10): G10.3 add doctor tmux-orphans check`
      - `ci(mvp10): G10.3 fail build on tmux orphans`
- 依赖: T3.1, T3.3
- 执行步骤:
    1. cargo fmt + cargo clippy 干净
    2. cargo test --all-targets 全套通过
    3. 两个 commit 分别提交（cleanup script commit 已在 T(-1).3 完成，不重复 commit）
    4. 在 mvp10-T.md 末尾追加"实施收尾"章节，记录最终 stale socket 数 / RSS 节省与对应 commit 链接
- 验收:
    - 全套测试 + ignored 测试 + CI 守门测试都通过
    - ccb doctor 输出 stale=0
    - 当前机器孤儿数为 0

---

## 5. 跨 Stage 实施纪律备忘

### 5.1 不允许的补丁式修法清单

- ❌ 在 ensure_session 内部 if systemd_run_available && !cgroup_path_exists { fallback }——属于运行时探测，应该在 EnvState 启动期一次性确定
- ❌ 测试 Harness 用 atexit / ctrlc crate 替代 systemd-run scope——补丁式应用层兜底，不解决 SIGKILL 路径
- ❌ 在 cleanup_tmux_resources 内部加循环重试 kill-server——一次失败就 warn 退出，不要堆 retry
- ❌ scripts/cleanup_orphan_tmux.sh 用 `pkill tmux` 全杀——禁止；必须用 ls-sessions probe 判据（Round 2 修订），活 server 一律不动，无论 PPID 是什么。也禁止用"PPID=1 + ccbd- 前缀"作为判据（daemonized tmux server 在 systemd scope 下也常 PPID=1，会误判活 server）
- ❌ shutdown 路径调 `ctx.tmux_server.kill_session_window(SESSION_NAME)` —— 这是 window 级 API，session 没关 server 不会退出。必须直接走 `tmux -L <socket> kill-session -t ccbd-agents`

### 5.2 实施过程中应主动 push 进度的节点

- 每个 stage commit 完成
- 任何"修一个 bug 又冒出下一个"的连环跌倒（立刻停下找根因）
- 任何 systemd-run / cgroup 行为不符预期（先 ask 用户确认环境，再决定继续 or 退路）

### 5.3 PR 命名约定

按 ~/.claude/rules/git-workflow.md：
- feat(mvp10): G10.0 / G10.1 / G10.2 ...
- chore(mvp10): script
- ci(mvp10): pipeline
- 不允许出现 fix(mvp10) 在初始实施期——任何"fix"都是实施缺陷，必须回到原 task 定义对照检查
