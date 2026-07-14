# 模块 B 工单草稿 — 进程环境/生命周期域(换血后泳道2:a5 写测试 → a2 实施 → a5 审)

**draft(master 备,换血后移入 wt-modB)。模块 B = 机械修,设计不 gate。文件面:`platform/linux/identity.rs`、spawn 命令构造层、`agent_io/registry.rs`、kill/teardown 站点。执笔权:a5 从本工单契约写 RED 测试 → a2 纯实施变绿(不碰测试)→ a5 审 a2 实施。收口一次 cargo test --lib 串行,红绿按模块批出。**

## 三项

### B1 · 身份显式注入替代 cgroup 嗅探(§一.10)
**根因**:`src/platform/linux/identity.rs:3` daemon 靠读 `/proc/self/cgroup` **嗅探自身 systemd unit** → 测试子进程可误认活栈身份,teardown 时掐死活栈 cgroup(事故背书)。
**契约**:daemon 的 unit/身份**由显式参数/env 注入**(spawn 时传入),**禁环境嗅探**;嗅探路径删除或降级为仅当无注入时的最后兜底(且告警)。
**a5 测试目标**:注入身份存在时,identity 用注入值、**不读 /proc/self/cgroup**;测试子进程无法通过嗅探误得活栈身份。RED:现码恒嗅探。

### B2 · tmux 清理泄漏兜底(§一.12)
**根因**:`src/agent_io/registry.rs:145-160` `expected_pid` 存在但**进程已死**时,`kill_*_if_owned` 失败**无兜底** → tmux session 存活泄漏。
**契约**:kill_if_owned 失败(尤其 expected_pid 已死)时有**兜底清理**(按 session 名/scope 兜底杀 + 事件留痕),不留泄漏 session。
**a5 测试目标**:构造 expected_pid 已死场景 → 断言兜底清理触发、session 不泄漏。RED:现码 kill 失败即漏。

### B3 · C2 teardown 逃逸两机械向量(§三.2,C2 investigation 已 CONFIRMED)
**根因(master C2 投查结论)**:e2e harness teardown 的 kill 路径在 **panic/failure 时被跳过**(清理非 Drop-guaranteed),ahd 逃逸存活(实证 /home/ahe2e/.../bin/ahd ×4 存活 5 天)。锚点:spawn_ahd_direct(bin/ah.rs)、spawn_daemon、r1_master_exit_shutdown.rs:463-500、kill_path_ownership_a4.rs、orphan_reap.rs。
**契约(两机械向量)**:①**teardown kill 在 panic/failure 路径也执行**(RAII Drop guard 覆盖,或 catch panic 后清理),不再只走成功路径;②**unit BindsTo 关系修正**(ahd unit 与其父/scope 的 BindsTo,使父死时 ahd 被 systemd 级联收割)。
**a5 测试目标**:①teardown 在 panic/failure 场景仍杀净 ahd(RED:现码逃逸);②(BindsTo 若属 unit 模板配置、无行为测试面,a5 在 .operator-question 说明建议以配置断言/集成层验,别硬造)。
**注意范围重叠**:B3 的 kill/teardown 站点若触 `orchestrator/mod.rs`,与 C(P0-2)改的 :798 respawn 段同文件——**模块 B 必须基于含 P0-2 的新 main 拉分支**(换血在 P0-2 合入后,天然满足)。

## 收口
- 收口一次 `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`(与泳道1 的 cargo **排队**,不并行)。
- 禁扩 scope 出三项;不碰模块 A/C 文件;执笔权铁律(a2 不碰测试文件);a2 收尾贴全量 --lib 绿灯(a5 无 toolchain 或让 cargo)。
- a5 批审 a2 实施 → ACCEPT → operator 推 PR + auto-merge。

## 待 master 换血后定
- B1 注入参数的确切形态(spawn 命令层怎么传)、B3 BindsTo 的 unit 模板改法——a5 写测试遇缝不定时 .operator-question 问 master,master 钉死缝契约(同 P0-1/P0-2 先例)。
