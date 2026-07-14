# ah 编程场景 SOP 现状物化盘点

调研范围: 仅读取仓库规则、可达全局/ah sandbox 规则、`.claude/`、`.ah/`、memory、`.kiro/specs/`、docs/reports、git log/branch/worktree。`gh pr list` 因本机 gh 未登录不可用。

## §A 物化载体清单

### 1. 仓库项目规则: `CLAUDE.md`

- `CLAUDE.md:5-10`: 定义 Master PM 与 Worker agents 两类身份, Master 规划/分派/审阅/收敛, Worker 专职单任务。
- `CLAUDE.md:12-20`: Worker 铁律: 只做当前明确任务; 不自主 `ccb ask`; 不自命 PM; 空闲等待派单; 拿不准就等 master。
- `CLAUDE.md:22-25`: 旧三角色模型: `a1` codex 主力实施, `a2` gemini 设计/分析/审阅不写码, `a3` claude e2e + 分担实施 + PM 替身审计。
- `CLAUDE.md:27-29`: Master PM 按全局宪法工作, worker 规则不约束 master。

结论: 角色边界和 worker 不自派已物化, 但仍使用 `ccb ask` 和 a1/a2/a3 旧模型, 与当前 ah 四角色运行口径有漂移。

### 2. 全局规则: `/home/sevenx/.claude/CLAUDE.md`

- `/home/sevenx/.claude/CLAUDE.md:1-3`: 声明这是全局 Claude 配置。
- `/home/sevenx/.claude/CLAUDE.md:5-10`: 全局层不放项目操作规则; 每个项目自己的 workflow、角色模型、编排机制、质量门由 `<repo>/CLAUDE.md` 和项目 rules 注入。

结论: 全局层只承载“项目规则下沉到项目”的元纪律, 不承载 ah SOP 细节。

### 3. ah master/worker coordination kernel

可达路径样本:

- Master kernel: `/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7/.claude/CLAUDE.md:1-24`
- Worker kernel: `/home/sevenx/.cache/ah/sandboxes/cc5e2ed69a92/.claude/CLAUDE.md:1-35`、`/home/sevenx/.cache/ah/sandboxes/def2ff598f36/.claude/CLAUDE.md:1-35` 等

已物化纪律:

- `/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7/.claude/CLAUDE.md:5-14`: cutover/revival successor master 必须读 `$AH_MASTER_HANDOFF`, 再执行 `ah master ack-ready --cutover-id "$AH_CUTOVER_ID"`, ACK 成功前不能声称 takeover 完成。
- `/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7/.claude/CLAUDE.md:16-20`: Master 通过 `ah ask <agent_id> "<task>" [--wait]` 分派; 通过 `ah pend` / `ah watch` / `ah logs` / `ah ps` / `ah attach` 读结果和证据; 不发明不存在的 ah 子命令。
- `/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7/.claude/CLAUDE.md:22-24`: Master 只能通过 ah 编排, 不越过 ah 杀 pane/session/daemon unit/agent process。
- `/home/sevenx/.cache/ah/sandboxes/cc5e2ed69a92/.claude/CLAUDE.md:5-13`: Worker 不 self-dispatch、不运行 `ah ask`、不转派、不当 PM; 只做当前 ah prompt 的单任务; 不改 host 系统路径; 不绕过认证。
- `/home/sevenx/.cache/ah/sandboxes/cc5e2ed69a92/.claude/CLAUDE.md:21-35`: 默认 worker 场景层含 evidence-first、报告时引用文件/命令/测试输出、代码变更给 diff summary、跑相关 `cargo test`、不碰无关范围。

结论: ah 新编排契约主要物化在 sandbox kernel, 不是仓库 `CLAUDE.md`。

### 4. `.ah/` 目录

- `.ah/rules/a1.md:1-5`: 仅是 a1 worker slot 示例层, 说明 ah 会先 prepend fixed worker kernel, 再 append 本项目 scenario layer。
- `.ah/rules/master.md:1-5`: 仅是 master slot 示例层, 说明 ah 会先 prepend fixed master kernel, 再 append 本项目 scenario layer。

结论: `.ah/rules` 当前不是实质 SOP, 只物化了“kernel + scenario layer”的 layering 模型。

### 5. `.claude/` 目录

仓库内目录只有 `.claude/backups`、`.claude/cache`、`.claude/plans`、`.claude/sessions`; 未发现 `.claude/rules/`、`settings.json`、`settings.local.json`、hooks 定义、`skills/`、`commands/`。

- `.claude/hook_audit.log:1-13`: 仅是 hook 历史审计日志, 不是规则定义。
- `.claude/plans/task_plan.md:8-10`: dogfood 监督计划把“真相来源”写成进程树/tmux pane/systemd/文件系统/git diff, 不信 ah/ccb 状态自报。
- `.claude/plans/task_plan.md:153-159`: 历史 dogfood 约束: isolated state + 精确 kill; provider OAuth-only; VPS cargo 串行; PM 代理不亲自写 src/tests; git 永不 `add -A`; commit 结尾 Co-Authored-By。

结论: `.claude/plans` 是历史 dogfood 运行态/纪要, 可作证据, 但不是稳定规则入口。

### 6. memory

可达路径: `/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7/.claude/projects/-home-sevenx-coding-ccbd-rust/memory/`

- `MEMORY.md:1-4`: 索引三条项目 memory: master 不能跑 cargo、worker brief 要求全量 cargo test、cargo dist 本地 OOM。
- `master-cannot-run-cargo.md:10-14`: Master sandbox 无 rustup toolchain; cargo build/check/test 必须委派给 worker; master 独立通过 git/disk 状态验证; gh/PR actions 属于 PM-proxy。
- `brief-workers-full-cargo-test.md:10-14`: worker brief 必须要求完整 `CARGO_BUILD_JOBS=1 cargo test`; 不用 targeted filter 替代; 推荐 `CCB_TEST_SKIP_REAL_PROVIDER=1 env -u AH_STATE_DIR -u CCBD_STATE_DIR CARGO_BUILD_JOBS=1 cargo test`; 说明 real-provider tests 在 provider-less VPS 失败是环境态; 测试不应污染 process-global state。
- `cargo-dist-build-ooms-vps.md:10-14`: 不让 worker 本地跑 `cargo dist build/plan`; cargo-dist 不可靠遵守 `CARGO_BUILD_JOBS=1`, 会 OOM; installer landing 测试应在 CI。

结论: cargo 验证纪律在 memory 中最清晰, 但未沉淀到仓库规则文件。

### 7. `.kiro/specs/*`

整体判断: 大多数是 feature-scoped 研究/设计/tasks/验收证据, 不是可复用 SOP; 但其中反复物化了局部流程纪律。

样本:

- `.kiro/specs/ah-master-tell-observability/design.md:3-18`: 单 feature 的 scope / target / non-goals。
- `.kiro/specs/ah-master-tell-observability/design.md:20-29`: 设计要求带 repo file:line evidence anchors。
- `.kiro/specs/ah-master-tell-observability/design.md:46-48`: 单 feature 铁律: request-id 只是 observability metadata, 不 gate/veto master_state; SQLite migration 风格要求。
- `.kiro/specs/ah-dogfooding-closure/tasks-m2.md:25-47`: TDD task 列表, 明确文件、依赖、内容、验收。
- `.kiro/specs/ah-dogfooding-closure/tasks-m2.md:89-120`: 明确红灯 tests 与红灯原因。
- `.kiro/specs/core-fixes/tasks.md:3-16`: 需求/研究/设计/tasks 的 DAG 化实施顺序。
- `.kiro/specs/core-fixes/tasks.md:20-33`: task 粒度包含依据、改动、锚点、测试、验收、置信度。
- `.kiro/specs/studio-req1-provisioning-design/spec.md:3-8`: implementation spec 明确 goal 且“不改变代码本身”。

结论: Kiro specs 沉淀的是“每个 feature 的一次性规格与验收”, 不是中央 SOP, 但体现了调研→设计→tasks→TDD 的惯用形状。

### 8. docs/reports 与 handoff

- `docs/reports/pr-dogfood-m1.md:23-24`: commit 列表显式 tests-first 红灯与 src impl 红绿。
- `docs/reports/pr-dogfood-m1.md:41-62`: PR report 记录 targeted dogfood/e2e 与 full `CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1`。
- `docs/reports/pr-dogfood-m1.md:72-77`: a2/a3 audit 结果与 must-fix/nice-to-have。
- `docs/reports/pr-dogfood-m2.md:29-31`: tests-first 与实现红绿 commit 形状。
- `docs/reports/pr-dogfood-m2.md:46-77`: 验证命令和 grep verify。
- `docs/reports/pr-dogfood-m2.md:79-88`: audit 结果 + 主控自审。
- `docs/reports/pr-dogfood-m3a.md:73-83`: 小改动可跳详细 a3 audit, 但主控亲审 verify。
- `docs/reports/pr-dogfood-final.md:86-101`: a3 初审抓 must-fix、修复后二审 PASS、主控自审。
- `docs/reports/studio-open-in-handoff-2026-07-06.md:65-77`: T2 handoff 要求从 clean worktree 做, 不动主仓根 WIP, 并说明后续提醒 agent-harness 删临时 env。
- `docs/reports/studio-open-in-handoff-2026-07-06.md:104-105`: 记录 e2e 残留 ahd 进程问题, 推出 T4 teardown 护栏。

结论: PR report/handoff 是实际流程习惯的重要物化证据, 但不是集中可执行 SOP。

### 9. git / gh 只读采样

- `git log --oneline -50` 显示常见提交/PR形状: `feat(...)`, `fix(...)`, `docs:`, `release:`; feature/fix/release 分支名稳定存在, 如 `feat/master-tell-observability`, `t2-worker-is-sandbox`, `t4-diagnostics-teardown`。
- `git show -s --format=full 86678f6`: commit body 是多段问题/修复/测试说明, 带 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- `git show -s --format=full 1e721ca` 和 `ebe67f5`: 同样带 Co-Authored-By trailer。
- `git worktree list --porcelain`: 当前存在多个 worktree: 主仓 `/home/sevenx/coding/ccbd-rust`、`ccbd-rust-completion`、`ccbd-rust-skills`、`ccbd-rust-wsl-onboarding`、以及一个 `/tmp/.../release-worktree`。
- `gh pr list --state merged --limit 20`: 不可达, 报 `gh auth login` / 缺 `GH_TOKEN`。

结论: commit/branch/PR 习惯可从 git 历史推断, gh PR 列表当前不可用; worktree 已在当前仓实际使用。

## §B 分环 SOP 走查

### 1. 需求环

现在怎么走:

- 用户/PM 给出目标、边界、铁律、分支和验证要求; 大任务常先形成 `.kiro/specs/<feature>/research.md` / `design.md` / `tasks.md`; 小任务可直接放 `research/t*-brief.md` 或 handoff 文档。

约定与物化:

- Master PM 负责规划、分派、审阅、收敛: `CLAUDE.md:7-10`。
- Worker 只执行明确指派的单任务: `CLAUDE.md:14-20`; worker kernel 也有同款 ah 规则 `/home/sevenx/.cache/ah/sandboxes/cc5e2ed69a92/.claude/CLAUDE.md:5-9`。
- Feature spec 明确 goal/scope/non-goals: `.kiro/specs/ah-master-tell-observability/design.md:3-18`, `.kiro/specs/studio-req1-provisioning-design/spec.md:3-8`。
- 大任务拆 DAG: `.kiro/specs/core-fixes/tasks.md:3-16`。

待定/in-head:

- “什么时候必须走完整 Kiro spec, 什么时候可用 brief/handoff”没有中央规则; 只能从实践推断。

### 2. 调研环

现在怎么走:

- 先读 brief/spec, 再读代码和文档证据; 调研产物要求 file:line, 不确定就标待定/不可达。

约定与物化:

- 默认 worker evidence-first: grep-before-claim, 报告引用具体文件/命令/测试输出: `/home/sevenx/.cache/ah/sandboxes/cc5e2ed69a92/.claude/CLAUDE.md:21-25`。
- Kiro design 要 evidence anchors: `.kiro/specs/ah-master-tell-observability/design.md:20-29`。
- 历史计划强调真相来源是进程树/tmux/systemd/文件系统/git diff, 不信状态自报: `.claude/plans/task_plan.md:8-10`。

待定/in-head:

- “baseline 对照证伪红灯”有 brief 级物化, 但不在中央规则; 见 `research/t4-brief.md:79`、`research/t2-is-sandbox-brief.md:63`。

### 3. 设计环

现在怎么走:

- 设计文档按 Scope / Non-goals / Evidence anchors / Storage/Transitions/Touch points 或按 Decisions / Shared Contract 展开; 设计需要 file:line 绑定代码事实。

约定与物化:

- 单 feature design 形状: `.kiro/specs/ah-master-tell-observability/design.md:3-29`。
- implementation spec 先定接口、语义、边界、reasoning: `.kiro/specs/studio-req1-provisioning-design/spec.md:11-35`。
- Windows handoff 记录“design approved + a1 grounded rewrite + a2/a4 review”这种设计审查轨迹: `.kiro/specs/ah-windows-native/handoff-prompt.md:19-20`。

待定/in-head:

- 当前四角色中 a3 antigravity 设计不写码、a2 codex 严审、a4 claude 二审/e2e 的稳定定义未在仓库中央规则找到; `CLAUDE.md:23-25` 是旧三角色。

### 4. 实施环

现在怎么走:

- PM 指定分支、scope、允许修改文件、TDD 红绿顺序; worker 在指定分支做实现, 不自派、不扩大范围, 回报 diff/stat/test。

约定与物化:

- Worker 单任务/不自派: `CLAUDE.md:16-20`, worker kernel `/home/sevenx/.cache/ah/sandboxes/cc5e2ed69a92/.claude/CLAUDE.md:7-9`。
- Master 分派命令是 `ah ask <agent_id> "<task>" [--wait]`: master kernel `/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7/.claude/CLAUDE.md:16-20`。
- Worker scope 不碰无关代码: worker kernel `/home/sevenx/.cache/ah/sandboxes/cc5e2ed69a92/.claude/CLAUDE.md:32-35`。
- TDD 红灯 tests 写法在 feature tasks 中物化: `.kiro/specs/ah-dogfooding-closure/tasks-m2.md:89-120`; PR report 中记录红灯→绿灯 commit: `docs/reports/pr-dogfood-m2.md:29-31`。
- 串行 cargo 和全量 cargo 在 memory 物化: `brief-workers-full-cargo-test.md:10-14`。
- 本地不跑 `cargo dist`: `cargo-dist-build-ooms-vps.md:10-14`。

待定/in-head:

- “必须先红后绿并贴红绿输出”常在具体 brief 里写, 不是中央规则。
- “只改这些文件/不要碰未跟踪文件/别 push”常在具体 brief 里写, 不是中央规则。

### 5. 审查环

现在怎么走:

- 常见链路是 PM 自审 + a2/a3/a4 审查; PR report 记录 must-fix/nice-to-have, must-fix 修完后二审 PASS。

约定与物化:

- `docs/reports/pr-dogfood-m1.md:72-77`: a2 audit + a3 audit 结果。
- `docs/reports/pr-dogfood-m2.md:79-88`: a3 audit + 主控自审。
- `docs/reports/pr-dogfood-final.md:86-101`: a3 初审抓 2 must-fix, 修复后二次终审 PASS, 主控自审。
- `docs/reports/pr-dogfood-m3a.md:73-83`: 小改可跳详细 a3 audit, 但主控亲审 verify。

待定/in-head:

- “PM-audit → a2 严审 → a4 二审/e2e → push”的精确强制门没有中央文件; 当前只在 briefs/PR reports 中按任务体现。

### 6. e2e / 验证环

现在怎么走:

- 常规代码改要求 worker 全量串行 cargo test; 特定功能还跑 targeted e2e、ignored dogfood tests、grep verify、真实 tmux/bash provider 路径; real-provider flake 要单跑/基线证明。

约定与物化:

- 完整串行 cargo: `brief-workers-full-cargo-test.md:10-14`。
- Master 不亲自 cargo, 由 worker 验证: `master-cannot-run-cargo.md:10-14`。
- PR report 验证块: `docs/reports/pr-dogfood-final.md:43-84`, `docs/reports/pr-dogfood-m2.md:46-77`。
- 真 stdout/e2e 不用 fake completion: `docs/reports/pr-dogfood-final.md:7-18`, `docs/reports/pr-dogfood-final.md:39-42`。
- e2e 残留需要 teardown 护栏: `docs/reports/studio-open-in-handoff-2026-07-06.md:104-105`, `research/t4-brief.md:55-70`。

待定/in-head:

- “完整 cargo test 失败时何时可以放行”的 baseline 证伪流程主要在 `research/t4-brief.md:79`、`research/t2-is-sandbox-brief.md:63` 等 brief 内, 不是全局 SOP。

### 7. PR / 合并环

现在怎么走:

- worker 通常不 push/不开 PR; PM-proxy/用户侧处理 PR、CI、合并。提交信息多为 conventional-ish `feat(...)`/`fix(...)`/`docs:`/`release:`; commit body 写背景/修复/测试, 带 Co-Authored-By trailer。

约定与物化:

- Worker 不自行启动 PR 工作流: `CLAUDE.md:18`。
- gh/PR actions 属于 PM-proxy, 不是 master: `master-cannot-run-cargo.md:14`。
- Git 不 `add -A`, commit 带 Co-Authored-By 是历史计划约束: `.claude/plans/task_plan.md:153-159`。
- git 历史证据: `git show -s --format=full 86678f6` / `1e721ca` / `ebe67f5` 均带 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 分支命名证据: `git log --oneline -50 --decorate` 中 `feat/master-tell-observability`, `t2-worker-is-sandbox`, `t4-diagnostics-teardown`, `release/v1.3.4`; `git branch -a` 中大量 `feat/*`, `fix/*`, `release/*`。

待定/in-head:

- “从 main 开分支、别在 main 改、只 commit 目标 tracked 文件、串行 commit、push 后 PM 开 PR+CI+合并”的完整流程没有集中物化; 多数靠具体 brief 与 PM 操作习惯。

## §C in-head 清单

以下条目未在一个稳定规则文件中找到完整定义; 有些在 brief/历史报告中局部出现, 但仍应视为待沉淀候选。

1. 当前四角色模型: `a1` codex 主力实施、`a2` codex 严审、`a3` gemini/antigravity 设计不写码、`a4` claude 二审+e2e。仓库 `CLAUDE.md:23-25` 是旧 a1/a2/a3 模型; `.kiro/specs/ah-windows-native/handoff-prompt.md:62-64` 只局部提到 a1/a2 codex 和 a4 claude。
2. 当前分派纪律的命令已从 `ccb ask` 演进到 `ah ask`; master kernel 已物化 `ah ask`, 但仓库 `CLAUDE.md:17` 仍写 `ccb ask`。
3. “a4 claude 池紧时可能不派 / 小改可跳详细二审”的调度裁量。只有 `docs/reports/pr-dogfood-m3a.md:73-75` 有一次小改跳审计记录, 非通用规则。
4. 完整的 TDD 红绿纪律: 先写失败测试、贴红灯输出、再实现转绿。Kiro tasks 和 reports 有样本, 但中央规则未写。
5. baseline 对照证伪红灯: 不能口头称无关, 要 stash/clean main 或单跑 baseline 证明既有/环境态。仅在 `research/t4-brief.md:79`、`research/t2-is-sandbox-brief.md:63` 等 brief 局部物化。
6. 每个 worker brief 必须写“只改某些文件、别碰未跟踪文件、别 push、回报 diff stat/test 输出”的模板化纪律。常见于 brief, 未集中物化。
7. 分支策略完整规则: 从指定 main commit 或 origin/main 开分支、别在 main 直接改、branch naming、同一任务不再切分支。git 历史和 brief 可推断, 中央规则无。
8. 提交策略完整规则: 只 add feature 文件、不要 `git add -A`、不要带纯格式漂移、Co-Authored trailer。`.claude/plans/task_plan.md:158` 和 git history 有证据, 但不是稳定规则。
9. PR/CI/合并权责: worker 不开 PR/不 push; PM-proxy 开 PR、看 CI、决定合并。`CLAUDE.md:18` 与 `master-cannot-run-cargo.md:14` 有部分物化, 完整门禁仍 in-head。
10. 验证门顺序: PM-audit → a2 审 → a4 二审/e2e → push/PR。PR reports 有审查记录, 没有统一 gate 文档。
11. real-provider flake 处理准则: full test 中 `mvp11_real_*` 等失败时, 单跑确认并说明耦合/基线。memory 说明 provider-less VPS 环境态, brief 说明单跑/baseline, 但无中央 SOP。
12. worktree 规则: 当前证据显示项目已经使用 git worktree。`git worktree list --porcelain` 列出多个 worktree; `docs/reports/studio-open-in-handoff-2026-07-06.md:71-77` 明确 T1/T2 可在 clean worktree 干活、别动主仓根 WIP。是否“以后所有隔离任务默认用 worktree”仍待定。
13. 沙箱隔离细则: worker sandbox HOME、`IS_SANDBOX` 来源、OAuth-only、host 路径禁改分别散落在 worker kernel、memory、handoff 和具体 brief 中, 未汇成一份工程 SOP。
14. master 不运行 cargo、worker 跑 cargo、PM-proxy 做 gh/PR 的三方职责虽然 memory 清晰, 但不在仓库项目规则中, 新 worker 仅靠 prompt/brief 注入才会知道。
15. “读不到路径要标不可达、拿不准标待定+原因”的调研纪律来自当前任务 brief, 未见中央规则。

in-head 条数: 15。
