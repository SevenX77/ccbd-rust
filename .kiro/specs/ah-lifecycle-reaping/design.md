# AH Lifecycle Reaping Design

## Scope

PR3 只改 systemd 生命周期绑定目标：agent scope、master scope、tmux scope 三类 scope 都绑定到**当前 daemon 实际运行所在的 systemd service unit**。

严禁在 PR3 中从 `project_id` 派生并硬绑 `ah-<project>.service`。如果该 unit 没有被真实创建并承载当前 daemon，`BindsTo=` 就是 inert dependency，等价于静默回归。

PR3 也不改 scope 自身命名：

- `src/sandbox/systemd.rs` 的 `--description=ccbd-agent-...` 保持不变。
- `src/tmux/scope.rs` 的 `--unit=ccbd-tmux-...` 保持不变。
- 现有 agent/workspace slice 名保持不变。

## Target Behavior

- 检测到 daemon 实际运行在 `ccbd.service`：agent/master/tmux scope 绑定 `ccbd.service`，现有 systemd reaping 不回归。
- 检测到 daemon 实际运行在 `ah-<project>.service`：agent/master/tmux scope 自动绑定该实际 unit。
- 检测不到 daemon 专属 `.service` unit：不注入 `BindsTo=`/`PartOf=`，优雅降级为进程内 shutdown 与启动重扫。
- `PartOf=` 与 `BindsTo=` 同步注入，且只在检测到实际 unit 时注入。

## Current Evidence

只基于允许读取的文件确认：

- `src/sandbox/systemd.rs:8`：`wrap_command(agent_id, project_id, daemon_marker, ...)` 已接收 `project_id`。
- `src/sandbox/systemd.rs:27`：agent scope 已使用 per-project agent slice：`agent_slice_for_project(project_id)`。
- `src/sandbox/systemd.rs:28`：agent scope 当前硬编码 `--property=BindsTo=ccbd.service`。
- `src/sandbox/systemd.rs:35`：`master_command(project_id, ...)` 已接收 `project_id`。
- `src/sandbox/systemd.rs:44`：master scope 已使用 per-project workspace slice：`workspace_slice_for_project(project_id)`。
- `src/sandbox/systemd.rs:56`：`agent_slice_for_project(project_id)` 生成 `ccb-<slug>-ccbd-agents.slice`。
- `src/sandbox/systemd.rs:67`：`sanitize_project_id(project_id)` 是 slice slug helper；PR3 不用它生成 bind target。
- `src/tmux/scope.rs:7`：`UnitConfig` 已有 `binds_to: Option<String>`。
- `src/tmux/scope.rs:55`：`detect_scope_policy` 当前通过 `detect_self_in_service().then(|| "ccbd.service")` 决定 tmux scope bind。
- `src/tmux/scope.rs:73`：`detect_self_in_service` 当前只判断 cgroup 是否包含 `ccbd.service`。
- `src/bin/ah.rs:251`：daemon dev 启动路径是裸 `Command::new(&ccbd_bin).spawn()`，并移除 `INVOCATION_ID`；因此不能假设派生出来的 service unit 已存在。
- `src/bin/ccbd.rs:28`：daemon 只检查 `INVOCATION_ID`，没有公开当前 service unit 名。
- `scripts/install_ah.sh`：只安装 `ah` 与 `ccbd-rs` wrapper，不注册 daemon service unit。

## Inherited Field Table

| Field / helper | Current source | Current behavior | PR3 behavior |
|---|---:|---|---|
| `wrap_command(agent_id, project_id, daemon_marker, ...)` | `systemd.rs:8` | agent scope command builder，已有 `project_id` | 继续保留现有签名；bind target 来自动态检测到的 actual service unit |
| agent slice | `systemd.rs:27` | `--slice=ccb-<slug>-ccbd-agents.slice` | 保持不变 |
| agent bind | `systemd.rs:28` | `--property=BindsTo=ccbd.service` | `[BREAKING]` 改为 guarded dynamic bind：`BindsTo=<actual-unit>` + `PartOf=<actual-unit>`；无 actual unit 则不注入 |
| `master_command(project_id, ...)` | `systemd.rs:35` | master scope command builder，已有 `project_id` | `[NEW]` 同 agent 使用 actual service unit 注入 `BindsTo=`/`PartOf=` |
| workspace slice | `systemd.rs:44` | `--slice=ccb-<slug>-ccbd-workspace.slice` | 保持不变 |
| `agent_slice_for_project` | `systemd.rs:56` | 生成 per-project agent slice | 保持不变 |
| `sanitize_project_id` | `systemd.rs:67` | slice slug helper | 保持不变；不得用于派生 PR3 bind target |
| `UnitConfig.binds_to` | `tmux/scope.rs:7` | tmux scope 可选 bind target | 保持可选；值来自 actual service unit |
| `detect_scope_policy` | `tmux/scope.rs:55` | systemd 可用时，若 bool 检测命中则绑定 `ccbd.service` | 使用 `detect_current_service_unit() -> Option<String>`；Some 则绑定 actual unit，None 则不绑定 |
| `detect_self_in_service` | `tmux/scope.rs:73` | bool: cgroup contains `ccbd.service` | 替换为 actual unit detector |

## Design

### 1. Dynamic Actual Unit Detection

[NEW] 新增动态检测 helper：

```rust
fn detect_current_service_unit_from_cgroup(cgroup: &str) -> Option<String>
```

语义：

- 从 `/proc/self/cgroup` 内容中提取 daemon 专属 `.service` unit 名。
- 只接受 `.service`，不接受 `.scope`、`.slice`。
- 排除 systemd/session 基础设施 unit：
  - `user@*.service`
  - `init.scope`
  - `session-*.scope`
  - `*.slice`
  - `user.slice`
- 无 daemon 专属 `.service` 时返回 `None`。
- 不从 `project_id` 推导 unit 名。
- 当前只把 daemon 专属 service 视为合法 actual unit：`ccbd.service` 与未来 `ah-<project>.service`。
- 裸 spawn/dev 下常见 cgroup 只包含 `user@<uid>.service`；该 case 必须返回 `None`，不能误绑 user manager。

运行时 wrapper：

```rust
fn detect_current_service_unit() -> Option<String> {
    std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|cgroup| detect_current_service_unit_from_cgroup(&cgroup))
}
```

推荐判定规则：

1. 将 cgroup path 按 `/` 分段。
2. 对每个分段做 systemd unit name unescape。
3. 只保留 `is_daemon_service_unit(unit) == true` 的候选。
4. 返回最后一个候选；无候选返回 `None`。

`is_daemon_service_unit` 初始规则：

```rust
fn is_daemon_service_unit(unit: &str) -> bool {
    unit == "ccbd.service"
        || (unit.starts_with("ah-") && unit.ends_with(".service"))
}
```

该白名单式规则比“最后一个 `.service`”稳健：它排除了 `user@1001.service` 这类 user manager 基础设施，同时允许 PR3 兼容当前 daemon unit 与未来 per-project daemon unit。

如果后续 daemon service 命名新增别名，必须显式扩展 `is_daemon_service_unit`，不得退回“任意 `.service`”启发式。

### 1.1 Detection Data Flow

PR3 不应让 `wrap_command` 这类 argv builder 在内部读 `/proc/self/cgroup`。读取时机应在 daemon 启动/初始化后集中完成一次：

```rust
let daemon_unit = detect_current_service_unit();
```

然后把 `Option<String>` 作为配置/上下文字段传入 agent/master/tmux 三处 scope builder。

理由：

- builder 保持纯函数，单测可用 fixture 直接覆盖 `Some("ccbd.service")`、`Some("ah-p1.service")`、`None`。
- 三类 scope 使用同一个 detection 结果，避免 agent/master/tmux 各自读 proc 产生不一致。
- 裸 spawn/dev 下 `daemon_unit=None` 可以稳定传递到所有 builder，兑现“不注入”降级语义。

### 2. Shared Injection Rule

agent/master/tmux 三类 scope 使用同一个规则：

```text
Some(unit):
  --property=BindsTo=<daemon-unit>
  --property=PartOf=<daemon-unit>

None:
  不注入 BindsTo
  不注入 PartOf
```

`BindsTo=` 保证 daemon unit 停止/失活时停止 scope。`PartOf=` 保证显式 stop/restart daemon 时传播到 scope。两者必须绑定同一个 actual unit。

### 3. Agent Scope

`wrap_command` under-systemd 分支保留：

```text
--slice=ccb-<slug>-ccbd-agents.slice
```

然后按 shared injection rule 注入 dependency。

示例：

- actual unit `ccbd.service`：

```text
--property=BindsTo=ccbd.service
--property=PartOf=ccbd.service
```

- actual unit `ah-p1.service`：

```text
--property=BindsTo=ah-p1.service
--property=PartOf=ah-p1.service
```

- no detect：无 `BindsTo=`/`PartOf=`。

### 4. Master Scope

`master_command` under-systemd 分支保留：

```text
--slice=ccb-<slug>-ccbd-workspace.slice
```

然后按 shared injection rule 注入 dependency。

PR3 不改变 `master_command` 的 shell 执行方式，不改变 scope 名或 description。

### 5. Tmux Scope

`src/tmux/scope.rs` 在 PR3 scope 内。

迁移：

- `detect_self_in_service() -> bool` 替换为 `detect_current_service_unit() -> Option<String>`。
- `detect_scope_policy` 在 `systemd-run` 可用时仍返回 `ScopePolicy::Systemd(UnitConfig { ... })`。
- `UnitConfig.binds_to` 填 `detect_current_service_unit()` 的返回值。
- `wrap_in_scope` 在 `binds_to=Some(unit)` 时同时注入 `BindsTo=<unit>` 与 `PartOf=<unit>`。
- `unit_name_for_socket` 仍返回 `ccbd-tmux-<suffix>`，不改名。

## Breaking Change

[BREAKING] `BindsTo` target 从硬编码 `ccbd.service` 改为动态 actual service unit。

这不是 scope 改名，也不是 daemon service 注册方案。它只改变 dependency target 的选择方式：

- 今天 daemon 真在 `ccbd.service` 内：输出仍是 `ccbd.service`。
- 未来 daemon 真在 `ah-<project>.service` 内：输出自动变成该 actual unit。
- 没有 actual service unit：不输出 dependency，避免绑定不存在 unit。

## Migration Path

1. 实现 `detect_current_service_unit_from_cgroup(cgroup) -> Option<String>`，覆盖真实 cgroup 形态：
   - `0::/user.slice/user-1001.slice/user@1001.service/app.slice/ccbd.service` -> `Some("ccbd.service")`
   - `0::/user.slice/user-1001.slice/user@1001.service/app.slice/ah-p1.service` -> `Some("ah-p1.service")`
   - `0::/user.slice/user-1001.slice/user@1001.service/app.slice/app-org.gnome.Terminal.slice/vte-spawn.scope` -> `None`
   - cgroup 只有 `.scope`/`.slice` 或 `user@*.service` -> `None`
   - 同时出现 `user@1001.service` 与 daemon service -> 返回 daemon service，不返回 `user@1001.service`
2. daemon 初始化时计算一次 `daemon_unit: Option<String>`，并通过上下文字段/参数传给三类 scope builder。
3. 在 `src/sandbox/systemd.rs` under-systemd 分支复用传入的 `daemon_unit`：
   - agent `wrap_command`：保留 `--slice=ccb-p1-ccbd-agents.slice`，按 actual unit 注入 `BindsTo`/`PartOf`。
   - master `master_command`：保留 `--slice=ccb-p1-ccbd-workspace.slice`，按 actual unit 注入 `BindsTo`/`PartOf`。
4. 在 `src/tmux/scope.rs`：
   - 替换 `detect_self_in_service`。
   - `UnitConfig.binds_to` 使用 actual unit。
   - `wrap_in_scope` 对 tmux scope 同步注入 `PartOf`。
5. 测试同步：
   - `systemd.rs:151`：改成 detected-unit fixture；检测到 `ccbd.service` 时断言 `BindsTo=ccbd.service` 与 `PartOf=ccbd.service`。
   - `systemd.rs:186`：继续断言不绑定 `ccbd-session-*`；测试名改为 actual daemon unit，不再表达全局 daemon。
   - `systemd.rs:253`：under-systemd 测试用 detected-unit fixture，不断言派生 `ah-p1.service`。
   - 新增 no-detect 测试：agent/master 不注入 `BindsTo=`/`PartOf=`。
   - 新增 parser fixture：真实 cgroup 字符串只含 `user@1001.service` 时返回 `None`。
   - `r1_bindsto_alignment.rs`：属于 tmux scope 绑定测试；`--unit=ccbd-tmux-abc123de` scope 名保持不变，只改 binds_to 相关断言。

## Acceptance Criteria

- actual unit = `ccbd.service`：
  - agent scope 包含 `--property=BindsTo=ccbd.service`
  - agent scope 包含 `--property=PartOf=ccbd.service`
  - master scope 包含 `--property=BindsTo=ccbd.service`
  - master scope 包含 `--property=PartOf=ccbd.service`
  - tmux scope 包含 `--property=BindsTo=ccbd.service`
  - tmux scope 包含 `--property=PartOf=ccbd.service`
- actual unit = `ah-p1.service`：三类 scope 全部绑定 `ah-p1.service`。
- actual unit = `None`，包括真实 cgroup 只含 `user@1001.service` 的裸 spawn/dev case：三类 scope 全部不输出 `BindsTo=`/`PartOf=`。
- agent/workspace slice 名不变。
- tmux `--unit=ccbd-tmux-...` 不变。
- 不存在任何从 `project_id` 硬派生并绑定 `ah-<project>.service` 的路径。
