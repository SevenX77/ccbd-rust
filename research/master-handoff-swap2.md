# Master 交接 — 换血#2(2026-07-10)

> 你是换血#2 后的新 master(空白会话)。这份文档是前任 master 在冻结点写的完整交接。**权威路线图仍是 `research/orchestration-plan-2026-07-10.md`**;本文补充"换血#2 时的在途状态 + 本轮新裁定 + 新命名方案 + 运行铁律更新"。持久记忆在 sandbox 的 `.claude/projects/.../memory/`——若换血换了 sandbox,本文自足。

## 一、本轮已完成 + 已合入 main

| PR | 内容 | commit |
|---|---|---|
| #129 | 模块 A(完成判定/状态机域):A1' 删残余 `PANE_DIFF_STUCK` 生产写入路径(`mark_agent_stuck` 加显式 `reason` 参数,timer.rs 传 `BUSY_MARKER_TIMEOUT_STUCK`、health_check.rs 传 `HEALTH_CHECK_STUCK`,统一了此前两处矛盾的审计事件)+ A2(`mark_agent_idle_recaptured*` 死码族清理) | `8dbd4db`(merge),范围 `564cf80..70addb7`,1001 passed |
| #130 | 模块 B(进程环境域):B1(`AH_SERVICE_UNIT` 显式注入替代 cgroup 嗅探)+ B2(tmux 孤儿清理 fail-open→fail-closed 加固,含 a5 自审时发现、经 a4 交叉审 ACCEPT 的追加修复)+ B3②(ahd 自身 unit → 父 scope `BindsTo`/`PartOf`) | `80e446b`(merge),范围 `ecc7772..5f90790`,1004 passed | 

main 现在 = `80e446b`。两模块均走完执笔权全流程(测试执笔→纯实施→gatekeeper 审计 ACCEPT→全量 `--lib` 收口→PR→operator 合并),无返工。

## 二、设计线交付(未合入,等 operator/用户过目)

**三份 kiro spec 已收敛完稿,路径 `.kiro/specs/`(各含 `requirements.md`/`design.md`/`tasks.md`)**:
- `ah-perception-arbiter/` — 感知仲裁器,四道设计轮必答题(单写入口硬约束/各信号类 Unknown 预算/父子 cgroup 委托 PoC/hook 归属竞态)均有明确裁决,非悬空。
- `ah-control-plane-refactor/` — 控制面重构(job 状态机单写权威、F3/F2 物理/业务二阶段解耦、kill/teardown 统一、`spawn_realign_agent` 迁移、`master_watch.rs` 拆分)。
- `ah-per-worker-credentials/` — per-worker 凭据隔离。

**流程**:master 出骨架 → `o1`(原 a3)对抗审 → master 收敛定稿。对抗审报告 `research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` 找出多处真实缺陷,均已吸收进定稿,例如:
- `ah-control-plane-refactor` 的 F3/F2 二阶段解耦第一版有 read-after-write 竞态(agent 立即转 IDLE 会被抢派新任务、污染前一个 job 的物理证据)→ 改为新增 `VERIFYING`/`FAILED_VERIFICATION` 中间态,不可被调度。
- `ah-per-worker-credentials` 第一版(逐 worker `fs::copy` 凭据文件)在 OAuth refresh-token-rotation 场景下**必然**级联失效(a3 用标准 RTR 攻击链证明)→ 整个机制改为宿主侧 token proxy,worker 沙箱不再持有真实 refresh token。这条 scope 从"一行 fix"变成"一个小服务",已在 spec 里如实标注,不是低估。
- 单写入口的 `db/` 内部边界漏洞、hook 归属 key 未定死、跨 spec 事件形状悬空等,均已在 spec 内联 "Correction"/"Pinned"/"Resolved" 段落逐条订正。

**状态**:三份 spec 顶部写明"converged after a3 adversarial review... 不许悬空... 未清场进入实施" —— **等你 orientation 后向 operator/用户确认是否放行实施**,现在不要自己排期。

## 三、命名方案定稿 — 换血#2 生效(用户拍板)

角色 = **gatekeeper(g,质量门)** / **code monkey(m,antigravity 快速实施)** / **oracle(o,设计辩论席)** / master(不变)。格式 `<角色首字母><泳道号>`,层级分隔符 `-`。m 系挂靠 g 系,只向自己的 g 汇报,不向 master 汇报。

映射:**a4→g1、a1→g1-m1、a5→g2、a2→g2-m1、a3→o1**。扩容规则:g1 下第二 code monkey = `g1-m2`;新增第三泳道 = `g3`/`g3-m1`。

三件套已备好(换血#2 时 operator 直接应用,你到岗时应该已经生效——如果 `ah ps` 显示的还是 a1-a5 旧编号,说明还没应用,提醒 operator):
1. `research/blood-swap2-ah.toml-draft`(`ah config validate` 已过)。
2. `.ah/rules/g1.md`/`g2.md`/`g1-m1.md`/`g2-m1.md`/`o1.md`(隶属声明已写清楚,`.lane-question` 收件人字段已是新编号)。
3. `research/blood-swap2-numbering-prep-2026-07-10.md`(完整 cutover 清单)。

**若已生效**,`.ah/rules/a1.md`~`a5.md` 应已被 operator 删除(旧文件在你落地前一直保留未删,只是新增平行文件,不是覆盖)。**顺带待办**:`.ah/rules/master.md`(即你现在这份角色文件的场景层来源)仍带换血#1 之前的单泳道措辞(只提 a1/a4,没有双泳道/o1),这次编号改动同样没触碰它——是否连同新编号一并订正,前两轮都留给了 operator/用户决策,还没拍板,你 orientation 后可以主动问一下,别让它继续漂移。

## 四、泳道层级化 — 你的角色边界变化

**用户拍板(本轮新规则,已写进新 rules 文件,换血#2 生效后 code monkey 不再向你汇报测试契约/签名适配/验收标准类问题)**:
- **g1-m1/g2-m1 的阻塞出口是各自的 gatekeeper(g1/g2),不是你**。它们遇到测试契约疑问、签名/接口适配分歧、验收标准不清,写 worktree 根目录的 `.lane-question` 文件(收件人写 g1 或 g2),**你巡检见到只原样转派,不裁决**。
- 你只管跨泳道事务:cargo 收口排队(全机单跑,双泳道不得并行跑 cargo)、模块分派、gatekeeper 之间的冲突、需要 operator/用户升级的事。
- 这条规则本轮(换血前的 a1-a5 编号下)已经在用,今晚两次真实案例(a1 的 ra2 式签名演进阻塞、mark_agent_stuck reason 参数适配阻塞)都是我(前任 master)直裁的——按新规则这类问题以后应该在 gatekeeper 层截住,不必升级到你。

## 五、本轮踩过的坑 + 已落的修复(避免重复交学费)

1. **worker 沙箱 rust toolchain 默认路径不对**:`$HOME` 在沙箱里被重映射到 `~/.cache/ah/sandboxes/<id>`,rustup/cargo 默认找 `$HOME/.rustup`,不是宿主的 `~/.rustup`。**已修**:`ah.toml` 里 `[sandbox] additional_ro_binds` 挂载宿主 `~/.rustup`/`~/.cargo` + `[env] RUSTUP_HOME`/`CARGO_HOME` 显式钉死路径。这条应该已经随 `ah.toml` 一起换血带过去,不用重新修——但换血后建议抽验一次(随便找个 code monkey 跑 `cargo --version`)。
2. **PROMPT_PENDING 派单闩锁复发**(旧二进制税本以为换血#1 消了,今晚 claude 系 gatekeeper 复发 2 次)。恢复 SOP:`ah prompt resolve <a> --keys Escape` → `tmux send-keys -t <pane> '/clear' Enter` → 等新 banner → 重派。**新学到的坑**:向 pane 投长文本时,`load-buffer`+`paste-buffer` 之后如果紧跟 `send-keys Enter` 在同一条命令里,Enter 可能被 CLI 渲染抢跑吃掉,文本停在输入行不提交。**正确做法**:paste 之后隔 1-2 秒单独发一次 `send-keys Enter`,发送后再 capture 一次确认输入行已清空,别信"发了就等于到了"。这条 operator 亲自抓到过两次,记牢。
3. **job 状态不可信,产物轨(commit/pane 实际内容)才是真相**:今晚至少 3 次 `ah ps` 显示 agent IDLE/COMPLETED,但 pane 里其实还在真实工作(antigravity 的后台任务模式尤其容易触发这个假象)。任何"任务完成"的判断都要 capture-pane 或 `git log` 核实,不能只看 `ah ps`。
4. **共享 worktree 的并发编辑风险**:同一 worktree 里,gatekeeper 写测试和 code monkey 写生产代码即使碰的是不同文件,只要**同时**在跑,就有 git add/commit 竞态风险。今晚有一次真撞上(a1 实施单和 a4 测试清理单被同时派进了同一个 wt-modA)——发现后立刻 cancel 了其中一个 job 再串行重派。**教训**:同一 worktree 只要有一方还没 commit,另一方就不能派新任务进去,哪怕看起来碰的文件不重叠。
5. **有 job 在途时不要裸等结束当前 turn**:必须用 `ScheduleWakeup` 或后台轮询留一个主动监视手段,不能假设"派了就完了、operator 会兜底"。这条 operator 纠偏过一次。
6. **署名纪律**:brief/回报里写"XX 已裁定"时,如实标注是 master 自己的工程判断还是 operator/用户的拍板,不要把自己的裁决包装成更高权威的裁决——今晚被 operator 当面纠偏过一次(把自己的裁决写成"operator 已裁定"),以后工程细节类的顺手裁决可以自己做主,但要写"master 裁定"。

## 六、磁盘水位

换血#2 冻结时 `df -h /` = 79% used(32G 可用)。两个模块的 worktree(`ccbd-rust-wt-modA`/`ccbd-rust-wt-modB`)已双双合并入 main,`target/` 目录可以在确认不再需要后清理回收空间(参考换血#1 交接文档的做法:`rm -rf <wt>/target`,重编即回)。

## 七、下一步建议(给新 master)

1. `ah ps` 对表新编号拓扑(g1/g1-m1/g2/g2-m1/o1)+ orientation 确认三件套是否真的生效。
2. 抽验一次 toolchain env 是否随 ah.toml 迁移过去(见 §五.1)。
3. 向 operator/用户确认:三份 kiro spec(§二)是否放行进入模块 C/D 实施排期,还是继续等设计冻结。
4. 视磁盘水位决定是否清理已合并模块的 worktree `target/`。
5. `.ah/rules/master.md` 拓扑措辞订正(§三尾注)——问一下 operator/用户要不要顺手做。
6. 全程守 §四 泳道层级化边界(g 系终裁泳道内事务,你只管跨泳道)+ §五 运行铁律。
