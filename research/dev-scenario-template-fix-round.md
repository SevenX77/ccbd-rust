# Fix Round 1 — dev-programming 场景模板(合并两审)

派单人:Master PM。执行:空闲 codex 实例。这是**一轮合并修复**(a2 严审 + a4 二审已收敛)。
铁律同上轮:只改下列文件、别碰未跟踪、别 push/commit、跑全量串行 cargo 回报 diff+test。
分支:`feat/dev-programming-scenario-template`(已在此分支主树)。

改动文件:`examples/scenarios/dev-programming/.ah/rules/{master,a1,a2,a4}.md`、`README.md`、`tests/dev_scenario_template.rs`。**a3.md 不动**。

---

## F1 [must-fix 保真] master.md 补回 default 的 Zoom-Out + Reporting
override 替换 default,当前 master.md 丢了这两段 → integrator 会比出厂 default 还弱。
在 master.md 末尾(Worktree posture 之后)追加,逐字来自 `assets/builtin/defaults/master.md:17-28`:
```markdown
## Zoom-Out

For high-risk or ambiguous work, check:

1. What user outcome matters?
2. What assumption could be false?
3. What evidence would disprove success?
4. What is the smallest safe next action?

## Reporting

Report in plain language using current state, root cause, and next step.
```

## F2 [must-fix 保真] a4.md 补回 worker default 的 Delivery
a4 跑 e2e/测试,应带 diff+cargo 交付纪律。在 a4.md 的 "Evidence first & scope" 段之前(或紧随 Review discipline)追加,逐字来自 `assets/builtin/defaults/worker.md:11-14`:
```markdown
## Delivery

- For code changes, provide a unified diff summary.
- Run the relevant `cargo test` command and report the result.
```
(a3.md 是设计不写码,**不加**此段——保持现状。)

## F3 [must-fix 保真] README 组合公式补 bundle 中间段
`README.md:7` 现写 `[embedded kernel] + [.ah/rules/<slot>.md or factory default]`,漏了真实中间段(`src/provider/home_layout.rs:523-533` 的 `compose_rules_with_layers(kernel, bundle_layers, override_or_default)`)。
改成:`[embedded kernel] + [bundle layers] + [.ah/rules/<slot>.md or factory default]`,并补一句"bundle layers 通常为空,本模板不带 bundle"。

## F4 [must-fix 保真] master.md 软化"不切第二分支"
`master.md:64-65` "do not spin a second branch for one task" 是硬锚不足的过硬规则。改软:
`- Prefer a single branch with serial close-out for one task; spin another branch only when the brief calls for it.`

## F5 [收窄 a2 MF3 / 兼顾 a4] 简化 kernel 免责指引句(不枚举 kernel 具体项)
保留"kernel 会自动前置、别复写"的指引(a4 认为有价值),但**去掉枚举**,避免与强化后的 guard 冲突,也消除 a2 的"复述"顾虑。
- master.md:3-6 改为:`The ah master coordination kernel is prepended automatically by ah — do not restate kernel content here.`(删去 "dispatch contract, cutover/revival ACK, safety boundary" 枚举)
- a1.md/a2.md:3-5 改为:`The ah worker coordination kernel is prepended automatically by ah — do not restate kernel content here.`(删去 "never self-dispatch, single-task-only, sandbox safety" 枚举)
- **a1.md 与 a2.md 改完仍必须逐字节一致。**

## F6 [test 强化:保真主门] 测试走真实注入路径 + 覆盖全 slot + 顺序断言
现测试只用两参 `compose_rules` 且只测 master/a1(a4 NTH3 + a2 MF2)。强化 `tests/dev_scenario_template.rs`:
1. **覆盖全 slot 语义 sentinel**:对 master/a1/a2/a3/a4 各断言其角色特征句出现在组合结果里(如 a3 含 "do NOT write implementation" 或 "design";a4 含 e2e/"second review";a2==a1 已有)。防某个 slot 被清空/损坏仍绿。
2. **真实注入路径 + 顺序**:走真实 materialization(先 grep 核实 API:`prepare_home_layout_with_extensions_for_slot`/`materialize_builtin_rules`/`compose_rules_with_layers`,`src/provider/home_layout.rs:139/213/523`),构造一个临时 project 让其 `.ah/rules/` = 本模板,遍历 ah.toml 的 master/a1/a2/a3/a4,断言各自内容写到正确目标文件(`home_layout.rs:504-506`:claude→`.claude/CLAUDE.md`、codex→`.codex/AGENTS.md`、antigravity→`.gemini/AGENTS.md`)。
3. **sentinel bundle 顺序**:插一个 sentinel bundle layer,断言组合顺序 `kernel < bundle < slot`(证明 F3 公式为真)。
   —— 若真实 materialization 在 master-less/无 sandbox 环境难以整跑,退一步:至少用 `compose_rules_with_layers(kernel, &[sentinel], slot)` 断言三段顺序 + 遍历全 slot 的目标映射用 `builtin_rules_target`(grep 核实函数名)单测。以能落地为准,别硬造不存在的 API。

## F7 [guard 强化] 无重复注入守卫补洞
`tests/dev_scenario_template.rs:38` 的 forbidden 列表有洞(a4 NTH2 + a2 NTH):
- 补 worker "Sandbox Safety" sentinel:`"host system paths"`、`"Never bypass OAuth"`(worker kernel 原文 `assets/builtin/worker_kernel.md:11-13`,grep 核实大小写)。
- 补 master 编排命令族:`"ah pend"`、`"ah master ack-ready"`。
- **更稳的做法(优先)**:直接从 `assets/builtin/master_kernel.md` / `worker_kernel.md` 抽若干**指令原句片段**做 forbidden,而非手写常量,后续 kernel 改文案不漏检。你判断可落地就用抽取式;不行就至少把上面 4 个 sentinel 加上。
- 注意:F5 简化后的免责句**不得**命中强化后的 guard(指引句是名词短语,不是 kernel 指令原句)——改完自查 guard 仍绿。

## F8 [README 澄清] 目标文件"碰撞"其实是每 agent 隔离 home
`README.md:13-19` 表里 master/a4 都写 `.claude/CLAUDE.md`、a1/a2 都写 `.codex/AGENTS.md`,读者会误以为撞车。补一行:
`Destinations are relative to each agent's isolated per-sandbox provider home (each agent gets its own home_root), so identical paths across agents do not collide.`

## F9 [README onboarding 补齐]
- 安装步骤前补"取得二进制":`cargo install --git https://github.com/SevenX77/ccbd-rust --bin ah --bin ahd`(设计 §6;先确认 README 里没有再补)。
- 步骤2 澄清:master 的 provider 由 `ah.toml` 的 `[master] cmd` 决定,**不在** slot 文件里;别让 integrator 去 master.md 找 provider 开关。

---

## 回报
- 6 文件 diff stat;
- 完整全量串行 cargo 输出;
- guard 强化后:贴 kernel 原句 grep 证据 + 自查 5 个 slot 文件均不命中 guard、a1==a2 仍相等;
- F6 若因环境退到"退一步"方案,说明为什么真实 materialization 跑不了。
