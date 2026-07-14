# ah 模块状态台账(operator 维护 · git 事实为准)

**用途**:一眼看清"哪些模块/任务合入了、哪些没做、live 栈缺什么",不再翻代码。
**更新纪律**:每次 PR merge / 换血后当场更新本表。状态列只认 git merge 事实(PR#),不凭印象。
**最后更新**:2026-07-11(operator,补建)。

---

## ⚠ 命名陷阱(必读,避免混淆)

历史上有**两套独立的 C/D 编号**,曾被 operator 自己混淆过:

1. **ABCD 四模块**(`research/rebatch-plan-2026-07-10.md`)——机械修重编排的四个大模块,域=完成判定/进程生命周期/恢复熔断/凭据隔离。
2. **感知层 C1/D1**(`research/perception-final-convergence-2026-07-09.md` 落地的 spec)——感知设计轮的 checker(C)+ job-state gate(D),编号与上面的 C/D **撞名但无关**。

> 从不存在 "C2-C8/D2-D7" 这种模块集——那是 operator 2026-07-11 记串了架构评估里的审阅判决标签(`[C2 REVISE]`/`[D2 AGREE]` 是两个审阅者的编号,不是模块)。

---

## 一、ABCD 四模块(rebatch-plan-2026-07-10)

| 模块 | 域 | 任务 | 状态 | 证据(PR / commit) |
|---|---|---|---|---|
| **A** | 完成判定/状态机 | A1 白名单对称化 · A2 orphan recapture 删死码 · A3 300s log 超时 · A4 unsafe_no_sandbox 透传 | ✅ **MERGED** | PR #129 `feat/modA-completion-statemachine` (8dbd4db, 2026-07-09) |
| **B** | 进程环境/生命周期 | B1 身份显式注入替 cgroup 嗅探 · B2 tmux 清理兜底 · B3 C2 teardown 两向量 | ✅ **MERGED** | PR #130 `feat/modB-process-env` (80e446b, 2026-07-09) |
| **C** | 恢复/熔断 | P0-2 熔断清零洞 + 认领时刻 cancel 检查 | ✅ **MERGED** | PR #128 `feat/p02-breaker-cancel-fix` (b363dce, 2026-07-09) |
| **D** | per-worker 独立凭据 | OAuth 凭据 symlink → per-worker 隔离(治共享凭据轮换登出) | ✅ **MERGED** | PR #146 `feat/gateway-graft-modD`(8f2aab5,2026-07-12);疗效=CI 绿仅代码闭环,活栈实证挂下次换血(见 pr-efficacy-ledger #146) |

**小结:ABCD 四模块全部 MERGED(2026-07-12 D 收口);D 的活栈疗效验证债在 pr-efficacy-ledger。**

---

## 二、感知层(perception,设计轮收敛后落地)

| 编号 | 域 | 状态 | 证据 |
|---|---|---|---|
| **C1** | perception checker baseline + write-gate | ✅ **MERGED** | PR #136 `feat/c1-perception-write-gate` (93c2bf3) + #138 `feat/c1-checker-baseline-jobstate` (0f529ff),2026-07-10 |
| **D1** | job-state gate | ✅ **MERGED** | PR #137 `feat/d1-job-state-gate` (fdc4566, 2026-07-10) |
| 其余感知层 | 仲裁 FSM 全量 / 完成协议 / 控制面 | 🟡 **设计已收敛,未转 spec/未实施** | `research/perception-final-convergence-2026-07-09.md`(章节 1.1–1.5 / 2.1–2.5) |

**小结:感知层落了 C1/D1 两块地基;其余是收敛设计,还没转 spec+tasks,没写代码。**

---

## 三、#13 respawn storm 修复

| 项 | 状态 | 证据 |
|---|---|---|
| 代码修复 | ✅ **MERGED** 到 dev main | PR #141 `fix/issue-13-respawn-storm` (97104cd, 2026-07-10);public ah#13 已关 |
| follow-up | 🟡 开了 issue 未做 | ccbd-rust #139(SIGKILL orphan-reap + systemd e2e)、#140(flaky grand_tour) |

---

## 三.5、R1 outbox(journal-first 完成信号传输,A/B 实验产物)

| 项 | 状态 | 证据 |
|---|---|---|
| R1-Q1 A′(opus 单体参照实现) | ✅ **MERGED** 到 dev main | PR #142 `ab/r1-outbox-ref-aprime`(merge 18c9e54, 2026-07-11);g2 终门 1501 passed/0 failed 串行全量 |
| 已知缺口 | 🟡 未做 | reap-on-RPC-success 未接线(A′ COMPLETION-REPORT §5 自报延期);agy 泳道分支 `ab/r1-outbox-lane` 保留未合(DNF) |
| agy hook-delivery 修复 | ✅ **MERGED** 到 dev main | PR #143(merge 7bae3b1, 2026-07-11);g2 交叉审 ACCEPT;两条非阻塞观察(is_ah_owned_hook_item 旧串匹配器、ah.rs:608 debug log)留后续 |

---

## 四、发布 / live 栈换血状态(关键!)

- **dev main HEAD**:97104cd(含 A/B/C + 感知 C1/D1 + #13 修复;本地 main 已 ff 拉平 origin)。
- **最新 release tag**:v1.5.0(#134,e06b8f9),已发 dev 仓 + 公开仓 SevenX77/ah。
- **live PM 栈当前二进制**:**Gen-4 = main 97104cd**(2026-07-11 换血#4 完成,用户指令)。
  - ✅ 已含全部:ABCD 的 A/B/C + 感知 C1/D1(#136/#137/#138)+ #13 修复(#141)。
  - 栈坐标:session sess_334718e9;ahd 挂 systemd user unit `ah-2ee4e0dfc3b5034c.service`(Restart=on-failure,不再裸进程);panes master %0 / g1 %1 / g1-m1 %2 / g2 %4(%3 为多余 bash pane,观察项)/ g2-m1 %5 / o1 %6。
  - 备份:`~/.local/bin/{ah,ahd}.old-gen3` + `target/release/*.old-gen3`。
  - **验收实证**:换血后活栈 `ah up` 全员 NO_CHANGE、pane pid 零变化(旧血同命令=全栈风暴)。交接文档 `research/master-handoff-swap3.md`(文件名 swap3 实为换血#4,内容为准)。
- **发版债**:main 领先 v1.5.0 的 6 个 PR 尚未发版(#136/#137/#138/#141/#142/#143;v1.5.1/v1.6.0 待用户点头),公开仓用户还拿不到 #13 修复。
- **✅ 换血#5 已执行(2026-07-11)**:live 栈 = **Gen-5 = main 7bae3b1**,session sess_169a3a04;拓扑改组:g1/g2 换回 **codex** 闸门 + 新增 **r1**(claude opus4.8 专职审核位,只审不写);agy 沙箱重建,#143 hook 修复材料化实证(绝对路径+timeout 5);#142 冷扫首触发实证(观察日志 #46,附孤儿事件 follow-up);daemon 重启=全栈连坐重生(观察日志 #44)。疗效报告 Gen-5 开窗表见 research/gen-efficacy-reports.md。

---

## 五、未合入 backlog(非本台账主线,防遗漏指针)

- 模块 **D** per-worker 凭据(见上)。
- test-hygiene #6(--lib 起真 tmux)、db/ 重命名归位(§三.5,长期)。
- #139 / #140 follow-up。
- 感知层设计轮收敛稿 → 转 spec → 实施(大件)。

---

## 六、模块化解耦(MD1 索引 + MD2 Wave-1,2026-07-13)

用户指令"模块化"。方法:MD1 建架构索引 → MD2 按索引对 god-file 做行为保持解耦(每模块独立 worktree/PR、收口点本地全量 cargo test、并行 CI 绿、r1 审、merge)。资源:实施 codex 并行,claude 只用 d1 设计 + r1 审(O9)。

| 项 | 状态 | 证据 |
|---|---|---|
| MD1 架构索引 | ✅ **MERGED** | PR #152(93bbb2d);6 层模块图+capability→owner+进程轴;operator grep 复核过 |
| MD2 target1 pilot(agent_io passivation) | ✅ **MERGED** | PR #153(0faf8e3);验证解耦流水线可跑通 |
| MD2 target2(master_cutover RPC 拆分) | ✅ **MERGED** | PR #155(7ce2bb5) |
| MD2 target3 PR-A(master_watch saga 抽取,恒等变换) | ✅ **MERGED** | PR #156(49b2083) |
| MD2 target3 PR-B(reap 链上提 master_reaper) | ✅ **MERGED** | PR #158(ea8e296);operator 亲审 gate;r1 揪出 :671 唯一非恒等行为变更→打回补钉住测试→条件授权合入 |
| 实证债 | 🟡 未观测 | 可维护性收益 + :671 硬化真实疗效(下次真实 finalize-stale 不泄漏孤儿)+ 活栈 dogfood 换血,均挂观察;详 pr-efficacy-ledger MD1/MD2 段 |

- **dev main HEAD 推进**:ea8e296(含 MD1/MD2 Wave-1 全部 + #151 凭据 + 之前载荷)。
- **Wave-2 目标**:待 operator 从架构索引 capability→owner 表选下一批 1000+ 行 ownership center(db::system 3485 / state_machine 2978 / jobs 2662 / rpc::handlers::sessions 剩余 / home_layout 3039 等)。

---

## 七、v1.7.0 发版(2026-07-13,operator)

| 项 | 状态 | 证据 |
|---|---|---|
| ccbd-rust v1.7.0 | ✅ **RELEASED** | PR #159 + tag v1.7.0(87bd01d);release.yml 出 installer;github.com/SevenX77/ccbd-rust/releases/tag/v1.7.0 |
| 公开仓 SevenX77/ah v1.7.0 | ✅ **RELEASED** | commit cf1bd90 + tag v1.7.0;逆向 curation+leak-gate(踢 master.md 活栈手册);github.com/SevenX77/ah/releases/tag/v1.7.0 |
| 凭据 tier-3 ②③(刷新原地写回/多席位不互登出) | 🟡 **解除阻塞·待 Windows 实证** | v1.7.0 是唯一前置,已发;待用户 Win11/WSL2 装 v1.7.0 dogfood |
| 发版机制债 | 🟡 记账 | publish-release.sh(含 leak-gate)未脚本化;dev master.md 漂成活栈手册需从 committed 挪走 |
