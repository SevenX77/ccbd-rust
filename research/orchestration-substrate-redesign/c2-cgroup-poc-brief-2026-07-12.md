# c2 实施单 · cgroup 委托布局 PoC(轨2 并行去风险 spike,2026-07-12)

## 定位(读清楚,别做多余仪式)
这是一个**纯实验性 spike**,不是生产代码,不接入 Reconciler/状态机主脊柱,不影响冻结设计流程(轨2 §1 的 o1↔d1 分阶段辩论仍在走,这个 PoC 与其**并行、不互相等待**)。用户明确要求"有风险开 worktree、效率优先"——把结果快速做出来,不用写大量文档、不用走 r1 全套审核仪式(这是探索性验证,不是要合并进 main 的实施)。

## 工作目录(已建好隔离 worktree)
`/home/sevenx/coding/ccbd-rust-wt-cgroup-poc`,分支 `spike/cgroup-delegation-poc`(从 main `7bae3b1` 切出)。改动只在这里,`git commit` 即可,不 push(等 master 看完结果再决定要不要留存/丢弃这个分支)。

## 背景(为什么要做这个 PoC)
编排底座重构设计草稿(`research/orchestration-substrate-redesign/design-substrate-redesign-draft-2026-07-12.md` §四 Q3)依赖一个未经实测的假设:用 systemd cgroup v2 的 `Delegate=yes` transient scope,可以把"agent CLI 自己的进程"(父 scope)与"agent 派生的子进程(编译/测试/shell)"(委托子 cgroup)物理隔离监控,使得子 cgroup 的 `cgroup.events` 的 `populated` 字段能准确反映"任务是否还有活的工作进程",不受常驻 agent CLI 本身进程存活与否干扰。这是感知层北极星四题里"~85% 置信里剩的 15%"——必须先用实验验证,不能只凭设计文档假设。

当前代码库现状(已核实,你可以直接复用/参考,不用重新 grep):`platform/linux/scope.rs:125` 与 `tmux/scope.rs` 用 `systemd-run --user --scope` 拉起沙箱进程,**没有 `Delegate=`**,这是纯增量的绿地实验,不影响现有代码路径。

## PoC 实验步骤(o1 设计的五步,照此实现并验证)
1. 生成一个启用 `Delegate=yes` 的 systemd transient scope(`systemd-run --user --scope -p Delegate=yes ...`),拉起一个进程模拟 agent CLI(可以用简单的 shell/Python/Rust 脚本,不需要真的是 ah 的代码)。
2. 该"模拟 agent CLI"进程通过系统调用/文件操作在自己的 cgroup 下创建一个子 cgroup(例如命名为 `payload`),并将其后续 spawn 的子进程(shell/编译/测试)的 PID 写入该子 cgroup 的 `cgroup.procs`。
3. "模拟 agent CLI"进程本身保留在父 scope,不移动进子 cgroup。
4. 监控子 cgroup `payload/cgroup.events` 的 `populated` 字段变化。
5. **核心验证目标**:当子进程(shell)退出后,即使"模拟 agent CLI"进程依然存活且持续运行,子 cgroup 的 `populated` 是否能准确翻转为 `0`——即物理剥离"常驻管理进程"对"任务真完成"信号的干扰。

## 环境局限(诚实标注,不要假装测过)
本次 PoC 在当前沙箱(非 WSL2,常规 Linux)上跑,只能验证 `Delegate=yes` 委托机制在**这个环境**下是否work。设计草稿里点名的关键剩余风险是 **WSL2 `--user` 形态下 `Delegate=yes` 是否可用**——这个无法在本沙箱验证,请在报告里明确注明"本机验证结果 ≠ WSL2 验证结果,WSL2 场景仍需真机验证"。

## 交付物
1. 实验脚本/代码(留在 worktree,commit 即可,语言不限,Rust/Python/shell 都行——这是 spike,不要求生产代码风格)。
2. 一份简短结果报告,写入 `research/orchestration-substrate-redesign/c2-cgroup-poc-result-2026-07-12.md`,包含:
   - 本机环境信息(cgroup v2 是否启用、systemd 版本、是否支持 `Delegate=yes`)。
   - 五步实验的实际输出(populated 翻转前后的具体值/时间戳)。
   - 结论:该机制在本机是否按预期工作(populated 翻转是否准确、有没有意外的抖动/延迟)。
   - **降级路径验证(如果 Delegate=yes 不可用或行为不符预期)**:草稿里给的降级兜底是"退化到 `cgroup.procs` 枚举 + 白名单"——如果主路径验证失败,请也验证一下这个降级路径是否可行。
   - 明确写"WSL2 待验证"这一条残留风险。

## 边界
- 不要碰主脊柱代码(Reconciler/状态机)——这个 PoC 是独立验证,不是本次就要接进生产。
- 不要过度打磨代码质量——这是 spike,能验证假设即可。
- 效率优先:不需要长篇设计文档,结果报告简明扼要即可。
- 完成后在会话里回报:PoC 结论一句话(populated 机制在本机是否按预期工作)+ 报告文件路径。
