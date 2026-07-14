# 编程场景层模板 — DESIGN 方案(给 operator 过目)

任务掉头后的交付目标:**忠实复刻当前这套 master+workers 编程栈的行为,做成 external-integrator 可安装的场景层模板**(`.ah/rules/<slot>.md` + `ah.toml` + 安装说明)。成功标尺是**保真**(装到干净环境 → master+workers 行为跟现在一模一样),不是零改动。质量升级(往 Fable5 抬)另立一版,本轮不做。

对应已批设计:`.kiro/specs/ah-v1-public-release/design.md`(Design 1/2 = kernel/场景层拆分 + per-provider 注入)。
复用素材:`research/sop-current-state-inventory.md` + `sop-design-plan.md`(21 条 SOP A0–A20,每条带证据锚 + 四角色核实),本轮把它们**重定向为 slot 场景文档的内容来源**。

DESIGN 出方案后**停下等 operator 过目**,再进实施。判断点见 §8。

---

## 1. 从代码核实的机制事实(设计据此,不假设)

组合发生在 `src/provider/home_layout.rs`,已在跑:

1. **三段组合**(`compose_rules_with_layers`,`home_layout.rs:523-533`):
   `最终doc = [role kernel(内嵌)] + [bundle_layers] + [.ah/rules/<slot_id>.md 或 builtin default]`
   → 写到 **provider 对应目标文件**(claude→`.claude/CLAUDE.md`;antigravity→`.gemini/AGENTS.md`;codex→其 `AGENTS.md`/rules)。目标不同、内容同一份 markdown(v1 design Design 2)。

2. **per-slot 差异化只有一个面:`.ah/rules/<slot_id>.md`**(`composed_rules_for_slot`,`home_layout.rs:535-549`;`slot_id` = `ah.toml` 里的 agent id,测试实证 `home_layout.rs:2098-2101` 传 `"a1"`/`"a2"`)。
   → **bundle 的 `[rules]` 做不到 per-slot**:`BundleRulesManifest` 只有 `master`/`worker` 两个 role 级键(`bundles.rs:56-59`),`resolve_bundle_rules` 按 `BundleRole` 取(`bundles.rs:456-464`)。所以差异化 a1≠a2≠a3≠a4 **必须**走 `.ah/rules/<slot>.md`,不能塞进 bundle。**这是决定交付物形态的硬约束。**

3. **override 是"替换"default,不是"追加"**(`home_layout.rs:544-548`:读到 `.ah/rules/<slot>.md` 就用它、`NotFound` 才用 default)。
   → 一旦写 `.ah/rules/master.md`,shipped `defaults/master.md` **整段不进**。所以每个 slot 文件必须**自带**它仍需要的那部分 default 内容 + 差异化增量。**这是内容裁剪的硬约束。**

4. **kernel 永远前置,别复写进场景层**(否则双重注入)。kernel 已有内容清单见 §2。

---

## 2. kernel / default / 场景层 边界(裁剪基准)

**已在 kernel 的(场景层严禁重复):**
- master kernel(`assets/builtin/master_kernel.md`):cutover/revival ACK;编排契约(`ah ask <id> "<task>" [--wait]`、`ah pend/watch/logs/ps/attach` 读证据);安全边界(不越 ah 杀 pane/session/daemon/agent)。
- worker kernel(`assets/builtin/worker_kernel.md`):never self-dispatch / 只做单任务 / 完成即等派单;沙箱安全(不改 host 路径、不绕 OAuth)。

**shipped default(role-generic,不区分 a1-a4;本轮不改它,保外部开箱默认不变):**
- `defaults/master.md`:PM/CEO-lite、不问 A/B/C、不亲改 src/tests、物证纪律、Zoom-out 4 问、说人话报告。
- `defaults/worker.md`:grep-before-claim、交付 diff + `cargo test`、scope-anchoring。

**结论**:本次要造的 per-slot 场景层 = **default 里仍需要的通用部分(因 override 替换 default,得重新带上)+ 现在只活在 master 脑子/brief 里的差异化角色/纪律(增量)**。增量的来源全是 21 条 SOP 的证据锚 —— 保真 = 每行可回溯,不发明现状没做的。

---

## 3. 交付物结构

```
# 仓库内(既是 dogfood 运行配置,又是 external-integrator 复制的规范样板)
.ah/rules/master.md      # master = PM 行为 + 8 步闭环 + 三角色分派
.ah/rules/a1.md          # codex 角色(实例1)= 严谨编码: 实施 + 严审
.ah/rules/a2.md          # codex 角色(实例2)= 与 a1.md 内容一致(同一角色的第二个并发实例)
.ah/rules/a3.md          # a3 antigravity = 设计/领域分析(不写实现码)
.ah/rules/a4.md          # a4 claude = 二审 + e2e/审计
ah.toml                  # 编程栈规范拓扑(现有那份, 确认并纳入)

# external-integrator 安装物(见 §6)
examples/scenarios/dev-programming/   # 上面 5 个 .ah/rules 文件 + ah.toml 骨架的可复制副本
  README.md                           # 安装说明: 复制到项目根 → 编辑 → ah up
```

**为什么不做成 bundle**:§1.2 —— bundle rules 只有 master/worker role 级,承不住 per-slot 差异化。bundle 适合承载 shared skills/hooks/MCP,本编程栈**当前不需要**(纯 git/ah/cargo 本地动作,§附)。故 defer bundle,若将来要挂共享 skill 再议。

**关键因子分解(保真的核心论证)**:现在差异化行为分两半 —— **不变的角色/纪律**(每次 brief 都一样)+ **每任务的具体活**(改哪个文件/scope)。本轮把**不变那半**固化进 `.ah/rules/<slot>.md`,**每任务那半**仍由 master 的 brief 现场给。因为不变那半本来每个 brief 都一致,固化它 = 保真,不改净行为。

---

## 4. 每个 slot 装什么 + 证据锚 + 排除项

标注:【带】= 从 default 重新带上的通用内容(因 override 替换 default);【增】= 差异化增量(来自 SOP 锚);【禁】= kernel 已有、不写。

### 4.1 `.ah/rules/master.md`(PM + 8 步闭环)
- 【带】PM/CEO-lite、不问 A/B/C、不亲改 src/tests、物证纪律、Zoom-out、说人话报告 ← `defaults/master.md`
- 【增】**三角色分派模型**(A1,codex 一角色两实例):**codex(a1/a2)= 严谨编码,既可派去实施也可派去严审**(实施/审查按任务分,不是固定 slot 专职)/ a3 antigravity 设计不写码 / a4 claude 二审+e2e。master 按空闲挑 codex 实例,两实例可互换。锚 `ah.toml:16-26` + handoff-prompt:62-66
- 【增】**8 步闭环**(A17):调研(钉 file:line)→派单(brief 带死 scope/落点/TDD/串行 cargo/别 push/回报格式)→盯(`ah pend` 阻塞)→PM-audit(`git diff`,master 不跑 cargo)→审(a2 严审 / a4 二审 / 逼 baseline)→收敛(一次性并轮)→收口(`git add 目标`+Co-Authored-By+push)→池管理。锚 clist §2 + 本会话 T1/T2/T4 实证
- 【增】**验证门顺序**(A12):PM-audit → a2 严审 →(a4 二审/e2e)→ push。锚 `pr-dogfood-final.md:86-101`
- 【增】**cargo 委派 + PM-proxy gh**(A4):master 不跑 cargo(sandbox 无 toolchain)、worker 跑、PM-proxy 开 PR/看 CI/合。锚 `master-cannot-run-cargo.md:10-14`
- 【增】**收口 git 纪律**(A8/A9):从 main 开分支、别在 main 改、命名 `feat|fix|release/…`、只 add 目标 tracked、不 `add -A`、Co-Authored-By trailer。锚 `.claude/plans/task_plan.md:158` + `git show 86678f6`
- 【增】**池调度裁量**(A16):claude 池紧 → 审查优先 codex(a1/a2),a4 留关键处;小改可跳详细二审(裁量非硬规则)。锚 `pr-dogfood-m3a.md:73-83`
- 【增】**baseline 证伪红灯**(A13):不口头"无关",逼 stash/clean-main 或单跑证明。锚 `t4-brief.md:79`
- 【增】**worktree 现状**(A20):当前主树 main 开分支、串行 commit;不改此流程。锚 `git worktree list` + 本会话实证
- 【禁】`ah ask`/`ah pend` 机制、安全边界 ← master kernel 已有,master.md 只**引用角色去派**,不复写命令语义

### 4.2 codex 角色文档 → **落成 a1.md == a2.md(同一份复制两份)**
codex 是**一个角色两个并发实例**,只为并行跑任务,不是两种角色。a1.md 与 a2.md **内容完全一致**。该角色 = **严谨编码:既实施也严审**(master 按任务把某个空闲 codex 实例派去实施或派去审;审查不是固定 slot 专职)。
- 【带】grep-before-claim、交付 unified diff、scope-anchoring ← `defaults/worker.md`
- 【增】角色:**严谨编码**——两副职,按 master 当次派单确定:
  - **实施职**:写 src + 单元/集成测试,按 master brief 实施。
  - **严审职**:grounded review,要求 file:line 举证,可带证据 **REJECT** 无据设计/实现(实证:codex 曾 REJECT antigravity 无据设计 v1)。产出 must-fix / nice-to-have 分级,并**逼 baseline 证伪红灯**(A13,不接受"无关失败",要 baseline diff)。
- 【增】**TDD 红绿**(A11,实施职):先写失败测试、贴红灯输出、再转绿。锚 `tasks-m2.md:89-120` + `pr-dogfood-m2.md:29-31`
- 【增】**串行全量 cargo**(A10):`CCB_TEST_SKIP_REAL_PROVIDER=1 env -u AH_STATE_DIR -u CCBD_STATE_DIR CARGO_BUILD_JOBS=1 cargo test -- --test-threads=1`,不用子集过滤替代。锚 `brief-workers-full-cargo-test.md:10-14`;不本地跑 `cargo dist`(OOM)锚 `cargo-dist-build-ooms-vps.md:10-14`
- 【增】**real-provider flake**(A14):full test 里 `mvp11_real_*` 等失败 → 单跑确认 + 说明耦合/baseline。锚同上 memory
- 【增】交付纪律(A7):只改指定文件、别碰未跟踪、别 push、回报 diff stat + test 输出
- 【增】审查证据锚:handoff-prompt:19-20,63;`pr-dogfood-final.md:86-101`
- 【禁】never self-dispatch、单任务、沙箱安全 ← worker kernel 已有

### 4.3 `.ah/rules/a3.md`(antigravity 设计/领域分析)
- 【带】evidence-first、scope
- 【增】角色:**架构/设计/领域分析,不写实现代码**(grounding 弱,不交实施)。锚 handoff-prompt:63-65("antigravity a3 — architecture/decision exploration; do not hand it the impl")
- 【增】设计产出形状:Scope / Non-goals / Evidence anchors(file:line);设计需绑代码事实。锚 `.kiro/specs/.../design.md:20-29`
- 【增】**调研纪律**(A15):读不到标不可达、拿不准标待定+原因、结论带 file:line
- 【禁】worker kernel 项

### 4.4 `.ah/rules/a4.md`(claude 二审 + e2e)
- 【带】evidence-first、diff+cargo、scope
- 【增】角色:**二审 / 审计 / e2e 测试**。二审在 a2 严审之后,把关键改动 + 池够时复审。锚 handoff-prompt:64
- 【增】e2e 纪律:真 stdout/真 tmux/bash provider 路径、不 fake completion;e2e 残留要 teardown 护栏。锚 `pr-dogfood-final.md:7-18` + `t4-brief.md:55-70`
- 【增】**可跳性**(A16):池紧/小改时 master 可不派 a4(裁量)
- 【禁】worker kernel 项

### 4.5 `ah.toml`(编程栈拓扑,确认纳入)
现有那份即规范拓扑,确认并作为模板一部分:`[master] cmd=claude`;`[completion] hook_push_providers=[claude,codex,antigravity]`;`[agents.a1]=codex`、`a2=codex`、`a3=antigravity`、`a4=claude`。锚 `ah.toml:1-26`。external-integrator 复制后按自己 provider 账号编辑。

---

## 5. 保真验证方案(成功标尺 = 装干净环境跑一遍对比现在行为)

四层,从便宜到贵:

1. **组合层(单测,复用现有 pattern `home_layout.rs:2098+`)**:对每个 slot 断言 `最终doc == kernel + 我们的 .ah/rules/<slot>.md`,且写到正确 provider 目标文件(a1/a2 codex→AGENTS、a3→`.gemini/AGENTS.md`、a4/master claude→`.claude/CLAUDE.md`)。
2. **无重复注入(单测/脚本)**:grep 每个 slot 文件,断言**不含** kernel 已有句子(`ah ask`、never self-dispatch、safety boundary…),防双重注入。这是 §1.4 的机器化守卫。
3. **内容可回溯(一个空闲 codex 实例派去严审)**:逐条把 slot 文件断言 diff 对照 §4 证据锚,抓"发明了现状没做的规则"。这是"保真"的评审门。
4. **行为保真(e2e,真跑)**:把 `examples/scenarios/dev-programming/` 装进一个干净的一次性项目 + `ah up`,派一个代表性小编程任务,观测:master 按四角色分派(实施→a1、审→a2/a4)、workers 跑串行 cargo + TDD + 不 push、PM 亲自 commit+PR。以本会话 T1/T2/T4 的行为轨迹作对照基线。**这是最终"装了跟现在一样"的验收**。

---

## 6. 打包 / 安装形态(对齐 v1 design §4)

- **仓库内**:把 `.ah/rules/{master,a1,a2,a3,a4}.md` 从空壳填成真内容 —— 既是 ah 自身 dogfood 配置,又是规范样板。
- **integrator 安装物**:`examples/scenarios/dev-programming/` = 上述 5 文件 + `ah.toml` 骨架 + `README.md`(说明:①复制到项目根;②按自己 provider 编辑 `ah.toml` 和各 slot;③`ahd` 起、`ah up`;④slot→provider→目标文件映射表;⑤kernel 自动前置、别在 slot 里复写)。
- **README/install**(v1 design Design 4):主 `README.md` 增一节"编程场景模板"指向 `examples/scenarios/dev-programming/`;一行安装沿用 `cargo install --git …`。
- bundle:defer(§3 理由)。

---

## 7. 复用与素材映射(不从零)

| 交付文件 | 主要来源 SOP(A0–A20) | 证据锚出处 |
|---|---|---|
| master.md | A1,A17,A12,A4,A8,A9,A16,A13,A20,A15 | inventory §2/§B + memory + git |
| a1.md == a2.md(codex 一份两份) | A7,A10,A11,A14,A13 + 实施/严审两副职 | memory + tasks-m2 + PR reports + handoff-prompt |
| a3.md | A15 + 设计角色 | handoff-prompt + kiro design |
| a4.md | A16 + 二审/e2e 角色 | handoff-prompt + t4-brief |
| ah.toml | 拓扑确认 | ah.toml:1-26 |

`sop-design-plan.md` 里的"归属=SSOT 人读文档"作废;**同一批已核实内容,落点从人读文档改为 per-slot 场景文档**。四角色核实(`ah.toml` 硬证据)照用。

---

## 8. 判断点 —— 已全部拍定(operator lock)

1. **dogfood 切换时机**:出文件 + `examples/` + 单测,先在**干净一次性项目**跑 e2e 验保真;**确认行为一致后,再把 ccbd-rust 本仓 `.ah/rules/` 填真切上去**。别在活栈干活时改它自己的规则 → 本仓切换是**最后一步**。✅
2. **安装形态**:copy-in 样板(`examples/scenarios/dev-programming/` + README),不动 bundle 代码。✅
3. **角色口径(已由本次修正化解)**:codex 是**一角色两实例**(a1==a2),可互换;master 用空闲的 codex 实例即可、不固定给谁。a4 claude 保持**二审+e2e 干净口径,不写"分担实施"**。原"谁分担实施"问题随之消失。✅
4. **stale 债不搭车**:仓库 `CLAUDE.md`(`ccb ask`/旧角色)+ 死 `rules/*.md` 本轮不动,另立小 PR。✅
5. **shipped `defaults/{master,worker}.md` 不动**(动它会改所有外部 integrator 开箱默认)。✅

---

## 9. 实施顺序(本轮执行)

1. 起分支(从 main 或当前 HEAD)。
2. 写 3 份角色文档(§4):**codex 那份落成 `a1.md` == `a2.md`(内容一致)** + `a3.md`(antigravity)+ `a4.md`(claude)+ `master.md`;每行带锚,严格排除 kernel 内容。**注意:这些先写进 `examples/scenarios/dev-programming/`(和分支工作区),本仓 `.ah/rules/` 暂不填**(§8.1)。
3. `ah.toml` 拓扑纳入模板(§4.5);建 `examples/scenarios/dev-programming/` + README(§6)。
4. 加组合层单测 + 无重复注入守卫(§5.1-5.2)—— 派一个 codex 实例实施,TDD 红绿。
5. 审:另一个空闲 codex 实例严审内容可回溯性(§5.3);a4 claude 跑 e2e(§5.4)在**干净一次性项目**装模板跑真任务,对照现在行为;a3 antigravity 若需要出设计说明。
6. e2e 保真通过后:**最后一步**才把本仓 `.ah/rules/{master,a1,a2,a3,a4}.md` 填真切上去(§8.1)。
7. PM-audit(`git diff`,master 不跑 cargo)→ 收口(`git add` 目标 + Co-Authored-By)→ **回 operator 过目 → 再开 PR(先别自己合)**。
   —— 注意:核心是新增 `.md` + 少量单测;src 改动仅测试文件,cargo 由 codex 实例跑。
