# Master 交接 — 换血#1(2026-07-10)

> 你是换血后的新 master(空白会话)。本文 = 前任 master 的关键上下文。**权威路线图是 `research/orchestration-plan-2026-07-10.md`(operator 主笔),先读它。** 本文补充"在途状态 + 前任的裁定 + 铁律"。你的持久记忆在 sandbox 的 `.claude/projects/.../memory/`(读 `MEMORY.md` 索引)——若换血换了 sandbox 而记忆没带过来,本文的"铁律"节自足。

## 一、已完成 + 已合入 main
| PR | 内容 |
|---|---|
| #122/#123 | G2 假完成检测器 + parser.rs 启发式(含返工) |
| #124 | fixture 脱敏搬迁 |
| #125 | 硬化 A/B(QUEUED 看门狗 + PROMPT_PENDING 压制升级) |
| #126 | Fix C(删 unknown→park 推断,park 白名单化) |
| #127 | **P0-1**:删 3 毒推断器(PANE_DIFF_STUCK-in-pane_diff / UI-recapture / health-pane-recapture)→ alert;LogAndUi→LogOnly 枚举收敛 |
| #128 | **P0-2**:熔断清零洞(K8s CrashLoopBackOff:respawn Ok 分支改 increment retry_count,清零延到 confirm_agent_stable_sync 稳定≥300s)+ 认领 cancel 过滤+CANCELLED 收敛。main=b363dce |

## 二、执笔权闭环(每单跑通,铁律)
**a4/a5(claude)写验收 RED 测试并 commit → a1/a2(agy)纯实施变绿、绝不碰测试文件 → claude 审实施(不审自己写的测试)。** 旧测试与新契约冲突时,由 **claude 改测试**(a1 从不碰任何测试)。缝不存在时 claude 加签名桩(体仅 `unimplemented!`,零逻辑),实施者填真身。共享 worktree 严格串行。

## 三、在途/待办(换血后按 orchestration-plan §五 节点2)
- **模块 A(已由前任重铸,裁定见下)**:worktree `ccbd-rust-wt-modA`(off #127 main),重铸工单 `WORKORDER-MODA-REFORGED-2026-07-10.md`。**换血后基于含 P0-2 的新 main 重拉分支**。
- **模块 B**:草稿 `research/modB-workorder-draft.md`(身份注入替 cgroup 嗅探 + tmux 清理兜底 + C2 teardown 两向量)。换血后泳道2(a2 实施 / a5 测试+审)。
- **设计轮**:a3 双盲发散**已完成** → `research/perception-divergence-a3-round2-2026-07-10.md`(34KB,7 问全覆盖,独立机制+失效模式,推翻了部分问法如"平级兄弟 cgroup")。**下一步=master 收敛**:据它 + `research/perception-final-convergence-2026-07-09.md`(裁决终稿,含设计轮必答四题)主笔设计终稿 → 冻结给 operator/用户 → 再派 a3 对抗审你的收敛(忠实度检查)。**尚未开始收敛。**
- 其余 backlog:test-hygiene(--lib 起真 tmux #6)、C1 空壳 daemon 设计、host-parity #7、unsafe-flag #8、db/ 重分层(§三.5 长期)、per-worker 凭据(§三.4)。

## 四、模块 A 重铸裁定(前任 master 已裁,operator 已认)
原模块 A 四项情报失真(a4 git/grep 实证):
- **A3(300s 超时)/A4(时间戳优先级)= 已被 a7c9d34(7/3)修好** → 删除,不写测试(空转)。
- **A2 孤儿 recapture 三件套 = 死码**(`mark_agent_idle_recaptured{,_with_pane,_health_check_with_pane}` + sync 变体,零生产 caller,唯一引用 `tests/antigravity_completion_terminality.rs:116`)→ a1 以 grep 零 caller + 编译过证明,**不写行为测试**。
- **A1 重铸(关键)**:accept-gate(`late_health_completion_stuck_allows_terminal` state_machine.rs:1176-1207)已单认 HEALTH_CHECK_STUCK 无死枝;真问题是 **#127(P0-1)没删净 PANE_DIFF_STUCK——生产源仍活**:`mark_agent_stuck` 仍写 `reason="PANE_DIFF_STUCK"`,活调用点 `marker/timer.rs:113` + `provider/health_check.rs:133`(**今晚 a3 被误判 STUCK 的 error_reason 正是 PANE_DIFF_STUCK,live 实证**)。正解=并入 G3「pane 推断整体删除」:**删这两个残余 PANE_DIFF_STUCK 写入点→降级 alert-only,保留 liveness HEALTH_CHECK_STUCK(health_check.rs:144 那条不动)**。⟹ 模块 A = 「删残余 pane 推断(A1',有真 RED 面)+ 死码清理(A2)」。

## 五、换血后拓扑(orchestration-plan §三)
a1/a2=agy 双实施泳道;a3=agy 设计辩论;**a4=claude 泳道1 质量门(服务 a1)、a5=claude 泳道2 质量门(服务 a2,e2e 归 a5)**。互审可交叉,不审自己产出。配置草稿:`.ah/rules/a5.md`、`research/blood-swap-ah.toml-draft`(operator 换血时应用)。**换血-每模块 cadence**:每大模块合入即换血、后续任务 dogfood。

## 六、运行铁律(血泪背书,务必守)
1. **agy 监控只盯 git commit 落地(HEAD 变)/ .operator-question,绝不信 job 状态**——agy turn-end 假 COMPLETED 频发(今晚 ≥3 例,reply 是 ANSI 垃圾/"I will pause..."),真完成=commit。假 STUCK/停摆也常见。
2. **全机 cargo 单跑**:`CARGO_BUILD_JOBS=1` + `--test-threads=1`;**模块批量**:模块内改完收口才跑一次 `cargo test --lib`,中途只定向 `cargo check`;双泳道收口 cargo **排队**不并行;本机全量/集成/e2e 禁跑,**严禁后台跑测试**(晚上全栈 OOM 覆灭即此),CI 是最终门;worker commit **不 push**(operator 推,auto-merge)。
3. **agent /clear 机制**:`tmux -L ahd-2ee4e0dfc3b5034c send-keys -t <pane> '/clear' Enter`(pane id 现查勿硬编码:`tmux -L ahd-2ee4e0dfc3b5034c list-panes -a -F '#{session_name} #{pane_id}'`);**只清 IDLE agent**,清后等新 banner;规则:worker 每完成 2 单派新单前先 clear。
4. **a4/a5(claude)每次派单必 PROMPT_PENDING 闩死(旧二进制税,换血后应消)**;SOP:`ah prompt resolve <a> --keys Escape` → `/clear <pane>` → 等 ~10s 派发器自动送达验 DISPATCHED。**换血后先抽验这条是否消失(G1)。**
5. **不重复注入正忙 agy**(队列堆积);**worker 的 .operator-question 答完别立刻删**(它被 re-clear 时靠它重锚,改名/留到确认消费)。
6. **磁盘满是独立故障族**:agent 零产出停摆/写失败/300s 静默假 STUCK 都可能是盘满——先 `df -h`;回收:merged worktree 的 `target/` 可删(`rm -rf <wt>/target`,重编即回)。今晚已清 4 个 done worktree 释放 18G(现 ~72%)。
7. **执笔权**(§二)+ **遇阻/与终稿冲突落 .operator-question 报 operator,不自行改判**;**外部锚定验收**(测试名/断言 master 钉死,防 agy 自证);**回滚自检**(回滚核心改动测试须变红)。

## 七、换血瞬间的 in-flight(会随会话终止,新 master 重新对表)
- 前任有若干后台监控(a1 closeout/a4 等)——**换血重启后全失效**,新 master 用 `ah ps` + `git -C <wt> log` 重新对表各 worktree HEAD 与 agent 态。
- P0-2 已合(#128),wt-p02 可清 target/ 省盘。
- **换血后 operator 会抽验 G1(a4 闩死是否消)/G2(agy 假完成是否消)**——这两条是换血直接疗效。

## 八、下一步建议(给新 master)
1. `ah ps` 对表五 agent + orientation 确认。
2. 抽验 G1/G2(配合 operator)。
3. 双泳道开工:模块 A(重铸单,泳道1 a1+a4)/ 模块 B(草稿,泳道2 a2+a5),基于含 P0-2 新 main 各拉 worktree,收口 cargo 排队。
4. 设计线:收敛感知设计终稿(a3-round2 + perception-final-convergence)→ 冻结 → a3 对抗审。
5. 全程守 §六 铁律 + orchestration-plan §四/五。
