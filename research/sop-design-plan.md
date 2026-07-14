# 编程场景 SOP 沉淀 — DESIGN 方案(给 operator 过目)

承接:`research/sop-research-clist.md` §5(设计环)。operator 已拍板 4 个决策(见文末回执)。
本文对每一条 **A 类 SOP** 给出 ①归属建议(落到 tool / skill / MCP / 规则文档) ②零回归验证方式。
定性重申:**整理沉淀现在实际在跑的 SOP,不是引入更好的 SOP**。零回归是红线。

DESIGN 出方案后**停下等 operator 过目**,不进实施。文末列出需 operator 拍板的 4 个判断点。

---

## 0. 本轮新钉实的 3 个结构事实(决定归属边界)

调研环之后,这轮 DESIGN 又核实了 ah 规则物化的真实机制,它直接决定"什么算零回归":

1. **机器 kernel 是二进制内嵌,不是仓库文件。** `role_kernel()` 返回 `builtin::MASTER_KERNEL` / `WORKER_KERNEL`(`src/provider/home_layout.rs:552-557`),`compose_rules_with_layers()` 把它作为**永远前置的第一段**(`home_layout.rs:523-533`)。→ 决策③"kernel 保持机器契约不动"= 不碰 `src/provider/home_layout.rs` 里的内嵌串。

2. **存在一个"活的、人可编辑、会进 agent 运行时 prompt"的仓库注入点:`.ah/rules/<slot>.md`。** 组合顺序是 `KERNEL(内嵌) + bundle_layers + [.ah/rules/<slot>.md 或 builtin::DEFAULT_*]`(`home_layout.rs:535-549`)。当前只有 `.ah/rules/a1.md`、`.ah/rules/master.md` 两个薄示例存在;a2/a3/a4/worker 落到 `builtin::DEFAULT_WORKER`。
   → **关键零回归红线**:往 `.ah/rules/<slot>.md` 里塞新内容 = **改 agent 运行时看到的 prompt = 行为变更(B 类)**。本轮**不动它**。

3. **仓库根 `rules/{AGENTS,CLAUDE,GEMINI}.md` 是死掉的遗留(pre-ah)。** grep 全 `src/`,materialization 只读 `.ah/rules` 和内嵌 kernel,**不读**根 `rules/`(唯一命中 `src/provider/bundles.rs:693` 是测试 fixture 造目录,不是读它)。这三个文件是 4 月 ccbd 时代的**旧模型**(Gemini=架构师、单一"主控 Claude"、`ccb` 命名、4 月的假 commit hash),现在**没生效但仍在仓库里当"看似权威"的规则**。→ 这是**第三处 stale 面**,见判断点④。

**由此得到本轮的零回归判据(贯穿全表):**

| 动作类型 | 是否碰运行时 | 归类 |
|---|---|---|
| 写**人读仓库文档**(SSOT):贡献者/新 PM 看的 | ❌ 不进任何 agent/PM 运行时 prompt 路径 | **A 类,构造性零回归** |
| **对齐仓库 `CLAUDE.md`**(决策②):它**会**进 master 运行时 context | ✅ 进 master context | **A 类,但需专门证零回归**(见 SOP-A0) |
| 往 `.ah/rules/<slot>.md` 注入新 SOP:改 agent 运行时 prompt | ✅ 进 worker/agent prompt | **B 类,defer** |
| 新建可执行 **tool/脚本**(commit 封装、brief 生成器等):新运行时产物 | ✅ 新 codepath | **B 类,defer**(见判断点②) |

一句话:**本轮 A 类 = 把在跑的 SOP 写成人读文档(SSOT)+ 一个把现有 CLAUDE.md 对齐现状的最小编辑**。凡是"新造一个会被机器/agent 执行的东西",都越过零回归线,归 B。

---

## 1. 确认的四角色口径(硬证据,非 prose)

决策②要求"以 inventory 核实后为准"。本轮从 `ah.toml` 硬配置核实(不靠叙述):

| slot | provider(`ah.toml`) | 职责(inventory + handoff 佐证) |
|---|---|---|
| a1 | `codex`(`ah.toml:16-17`) | 主力实施(src + 单元/集成测试),TDD,串行 cargo |
| a2 | `codex`(`ah.toml:19-20`) | 严审(grounded review,要 file:line;曾 REJECT antigravity 无据设计 v1) |
| a3 | `antigravity`(`ah.toml:22-23`) | 架构/决策探索,**不写实现代码**(grounding 弱,不交实施) |
| a4 | `claude`(`ah.toml:25-26`) | 审计 / 二审 / e2e;池紧时可不派、小改可跳详细二审 |

佐证:`.kiro/specs/ah-windows-native/handoff-prompt.md:62-66`("codex a1/a2 impl+review;antigravity a3 architecture-not-impl;a4 claude audit/second-review;multi-agent review gate mandatory")。
→ 这个口径取代仓库 `CLAUDE.md:22-25` 的旧三角色(a1codex/a2**gemini**/a3**claude**)。

---

## 2. 归属分层框架

按 §0 的零回归判据,本轮 A 类只用两个落点 + 一个 defer 清单:

- **落点 A(主):人读 SSOT 文档** —— 收编所有声明性内容(角色、纪律、验证门、沙箱/cargo 约定、8 步闭环)。构造性零回归。
- **落点 B(最小编辑):仓库 `CLAUDE.md` 对齐**(决策②) —— 只做两处 stale 修正(`ccb ask`→`ah ask`;旧三角色→指向 SSOT 的四角色),并把角色细节**收敛到 SSOT 单源**避免双写漂移。
- **skill**:仅 1 条(master 8 步编排闭环),且是**advisory checklist**(PM 已经在这么做),不进 worker prompt。—— 见判断点②,可选本轮做或也 defer。
- **tool/脚本 / MCP**:本轮**不做**。理由见 §0。MCP 本轮确认用不到(全是本地 git/ah 动作,无跨会话外部集成需求)。

**SSOT 内部结构建议**(落点):
- 仓库根 `AGENTS.md` 作为**跨工具单一入口**(codex/antigravity/claude 都朝它收敛,= harness 借鉴 #6),体量控制在"角色 + 铁律 + 指向明细";
- 明细放 `docs/sop/*.md`(调研环/实施环/审查环/验证环/PR 环 + 沙箱约定),`AGENTS.md` 链接过去;
- `CLAUDE.md` 保留 **Claude-master 特有的身份判定**(有无 `CCB_CALLER_ACTOR`),其余"指向 `AGENTS.md`"。
- ⚠️ 命名冲突预警:根 `AGENTS.md`(新,人读 SSOT)vs 死掉的 `rules/AGENTS.md`(旧 sandbox 遗留)—— 见判断点④,建议同一轮处理掉遗留,否则两个 AGENTS.md 会误导人。

---

## 3. 逐条 A 类 SOP:归属 + 零回归验证(核心交付)

**通用零回归验证法(声明性条目共用,记作 [V-transcribe]):** 每条 SSOT 文字必须可回溯到一个**已存在的证据锚**(kernel / memory / brief / PR report / git 历史 / `ah.toml` 的 file:line),且**不得引入任何当前未在实践的新指令**。验收 = 派 a2(codex)逐条把 SSOT 断言 diff 对照所引锚点;a4(claude)二审"有没有夹带私货(现状没做的规则)"。因为不碰任何运行时路径,行为恒等 → 构造性零回归。下表凡标 [V-transcribe] 即用此法,只补充条目特有的注意点。

### 3.1 角色与编排契约

| ID | SOP(在跑的行为) | 现散落在 | 归属 | 零回归验证 |
|---|---|---|---|---|
| A0 | **`CLAUDE.md` 对齐**:`ccb ask`→`ah ask`;旧三角色→四角色(收敛到 SSOT) | `CLAUDE.md:17,22-25`(stale) | 落点 B(最小编辑 CLAUDE.md)+ 单源在 SSOT | **专门证**:stale 文本**当前已不被遵守**——master 分派实走 `ah` 二进制、角色实取自 `ah.toml`,没有任何 codepath 解析 CLAUDE.md 的 `ccb ask`/角色串(grep 证:无程序读取)。对齐 = 移除一条死矛盾,master 实际行为不变。验收:grep 确认无程序依赖旧串 + a2/a4 审"改后 == 现状口径" |
| A1 | 四角色模型(§1 口径) | 无中央;CLAUDE.md 旧版 | SSOT(角色页)+ A0 指过去 | [V-transcribe],锚 `ah.toml:16-26` + handoff:62-66 |
| A2 | 分派命令 `ah ask <id> "<task>" [--wait]` | master kernel 有,仓库 stale | SSOT(编排契约页,标注"机器契约在 kernel,此为人读镜像") | [V-transcribe],锚 master kernel:16-20 |
| A3 | PR/CI/合并权责:worker 不 push;PM-proxy 开 PR/看 CI/合 | `CLAUDE.md:18` + memory | SSOT(PR 环页) | [V-transcribe],锚 `CLAUDE.md:18` + `master-cannot-run-cargo.md:14` |
| A4 | 三方职责:master 不跑 cargo / worker 跑 / PM-proxy gh | memory 清晰,不在仓库规则 | SSOT(角色页) | [V-transcribe],锚 `master-cannot-run-cargo.md:10-14` |
| A5 | 沙箱隔离细则:sandbox HOME / `IS_SANDBOX` 来源 / OAuth-only / 禁改 host 路径 | worker kernel + memory + handoff 散落 | SSOT(沙箱约定页,汇一份) | [V-transcribe],锚 worker kernel:5-13 + handoff。注意:**只汇总,不新增约束** |
| A6 | 安全边界:不越 ah 杀 pane/session/daemon/agent | master kernel:22-24 | SSOT(编排契约页,人读镜像 + 指 kernel 为机器源) | [V-transcribe],锚 master kernel:22-24 |

### 3.2 派单 / 实施纪律

| ID | SOP | 现散落在 | 归属 | 零回归验证 |
|---|---|---|---|---|
| A7 | worker brief 模板(只改X文件 / 别碰未跟踪 / 别 push / 回报 diff+test) | 每个 brief 手写 | SSOT(实施环页,以"模板文本"形式登记);**tool 化 defer** | [V-transcribe],锚多份 brief。注意:登记为**文档模板**(人复制粘贴),不是生成器脚本——脚本=B(判断点②) |
| A8 | 分支策略:从 main/origin main 开 / 别在 main 改 / 命名 `feat|fix|release/...` / 同任务不再切分支 | git 习惯 + brief | SSOT(PR 环页) | [V-transcribe],锚 `git log --decorate`(分支名实证)+ inventory §9 |
| A9 | 提交策略:只 add 目标 tracked / 不 `add -A` / Co-Authored-By trailer | task_plan + git 历史 | SSOT(PR 环页);**tool 化 defer** | [V-transcribe],锚 `.claude/plans/task_plan.md:158` + `git show 86678f6`(trailer 实证)。commit 封装脚本=B |
| A10 | 串行全量 cargo(`env -u AH_STATE_DIR CARGO_BUILD_JOBS=1 ... --test-threads=1`,不过滤子集) | memory | SSOT(验证环页) | [V-transcribe],锚 `brief-workers-full-cargo-test.md:10-14` + `cargo-dist-build-ooms-vps.md:10-14` |
| A11 | TDD 红绿:先写失败测试、贴红灯输出、再转绿 | brief / PR report 样本 | SSOT(实施环页) | [V-transcribe],锚 `tasks-m2.md:89-120` + `pr-dogfood-m2.md:29-31` |

### 3.3 审查 / 验证门

| ID | SOP | 现散落在 | 归属 | 零回归验证 |
|---|---|---|---|---|
| A12 | 验证门顺序:PM-audit → a2 严审 → (a4 二审/e2e) → push | PR report,无门文档 | SSOT(审查环页) | [V-transcribe],锚 `pr-dogfood-final.md:86-101`。注意:门是**描述当前链路**,不是新增强制 gate |
| A13 | baseline 证伪红灯:不口头"无关",要 stash/clean main 或单跑 baseline 证明既有/环境态 | 仅 t4/t2 brief | SSOT(验证环页) | [V-transcribe],锚 `research/t4-brief.md:79` + `t2-is-sandbox-brief.md:63` |
| A14 | real-provider flake 处理:full test 中 `mvp11_real_*` 等失败 → 单跑确认 + 说明耦合/baseline | memory + brief | SSOT(验证环页) | [V-transcribe],锚 `brief-workers-full-cargo-test.md` + brief |
| A15 | 调研纪律:读不到标"不可达"、拿不准标"待定+原因"、结论带 file:line | 本任务 brief | SSOT(调研环页) | [V-transcribe],锚 worker kernel evidence-first:21-25 + 本任务 brief |
| A16 | 池调度裁量:claude 池紧 → 审查优先 codex(a1/a2),a4 留关键处;小改可跳详细二审 | `pr-dogfood-m3a.md:73-75` 一次 | SSOT(审查环页,标注"裁量非硬规则") | [V-transcribe],锚 `pr-dogfood-m3a.md:73-83`。**保留"裁量"语气**,别写成必执行 |

### 3.4 编排闭环(唯一 skill 候选)

| ID | SOP | 现状 | 归属 | 零回归验证 |
|---|---|---|---|---|
| A17 | master 8 步闭环:调研→派单→盯(`ah pend` 阻塞)→PM-audit(`git diff`)→审(a2 严审/a4 二审/逼 baseline)→收敛(一次性并轮)→收口(`git add 目标`+trailer+push)→池管理 | 几乎全 in-head(§2 本会话实证) | **① SSOT(编排闭环页,人读全文)为主**;**② 可选 skill**(把同一 8 步做成 PM 可 invoke 的 checklist) | SSOT 版:[V-transcribe],每步锚 clist §2 / 本会话 T1/T2/T4 实证。skill 版零回归:skill **只编码这 8 步、advisory、PM 本就在做、不进 worker prompt**;验收 = 逐步 diff skill 文本 vs §2,确认零夹带;invoke 可选 |

### 3.5 harness A 类借鉴

| ID | SOP | 归属 | 零回归验证 |
|---|---|---|---|
| A18 | 跨工具单一 SSOT(`AGENTS.md` 入口,`CLAUDE.md`/未来 CONTRIBUTING 只指过去,不复述) | 本方案的**落点结构本身**(§2)= 落实 #6 | 元层面:建立 SSOT 不改任何在跑行为,纯新增人读入口。验收:确认无双写(角色只在 SSOT 定义一次,CLAUDE.md 指过去) |
| A19 | 声明"哪些共享文件不能并行改 / owns_files 不相交" | SSOT(派单页,登记为**声明惯例**) | [V-transcribe]。注意:当前主树串行 commit 下冲突少,此条是**登记既有意识**,不是引入文件锁系统(锁系统=B) |

### 3.6 worktree 现状(决策④:记录现状,采纳归 B)

| ID | SOP | 归属 | 零回归验证 |
|---|---|---|---|
| A20 | worktree 现状:**当前派单跑主树 main 开分支、串行 commit**;仓库虽存在其他 feature worktree(`ccbd-rust-completion/-skills/-wsl-onboarding` + release worktree),但 **worker 派单流程不用**它们 | SSOT(PR 环页,如实记录现状 + 一句"worktree 化采纳属 SOP 增强议题,见 B 类") | [V-transcribe],锚 `git worktree list` + 本会话 T1/T2/T4 实证(主树串行)。**只描述,不改流程**(决策④) |

**A 类合计:21 条(A0–A20)。** 全部落 SSOT / CLAUDE.md 对齐;A17 另有可选 skill;零可执行 tool/脚本/MCP(本轮)。

---

## 4. 明确划到 B 类 / 遗留处理的清单(本轮不做,防止 scope 蔓延)

- **B-1** worktree 化派单流程(一任务一 worktree)—— harness #1/#2/#4/#5。决策④已定 defer。
- **B-2** 开工脚本(`ah task start` 级:从 origin/main 切树 + 预热依赖)。
- **B-3** commit/PR 收口脚本(`git add 目标 + Co-Authored-By commit` 封装、push+PR 封装)。**注意**:这是把 A7/A9/A17 收口步"tool 化",越零回归线 → B。
- **B-4** auto-merge / CI 机器合并门(ah 要保 PM/a2 审查门,**不可**学 0-approval auto-merge)。
- **B-5** 文件所有权锁系统(A19 只登记声明,系统化 = B)。
- **B-6** 把 SOP 注入 agent 运行时(往 `.ah/rules/<slot>.md` 塞明细)—— 改 prompt = B。
- **遗留清理**(非 A 非 B,是 stale 债):死掉的 `rules/{AGENTS,CLAUDE,GEMINI}.md`(§0.3)。见判断点④。

---

## 5. 需 operator 拍板的 4 个判断点(DESIGN gate)

1. **SSOT 落点形态**:采纳"根 `AGENTS.md`(跨工具入口,精简)+ `docs/sop/*.md`(分环明细)"两层结构?还是你更想要**单文件** `AGENTS.md` 全塞 / 或全塞 `docs/sop/`?
   —— 我的建议:**两层**(入口 + 明细),对齐 harness #6 的跨工具单源,且明细分环可读。

2. **A17 skill 本轮做不做**:我把所有可执行 tool/脚本(commit 封装、brief 生成器)判为 B 类 defer(理由:新运行时产物越零回归线),这与我上轮 clist §5"tool 优先"的倾向**相反**——上轮低估了"新建可执行物=行为面"。唯一留在 A 的可执行物候选是 **A17 的 advisory skill**(纯 checklist、不改 worker prompt)。
   —— 我的建议:本轮**先只做 SSOT 文档**,A17 skill 也**暂缓**,连同 B-3 一起进"SOP 增强"议题统一评。这样本轮 100% 是文档沉淀,零回归最干净。**但若你希望本轮就有一个可 invoke 的 PM 闭环 checklist,我可以把 A17 skill 纳入**(它仍是 advisory、零回归可控)。请你在"仅文档" vs "文档+A17 skill"之间拍。

3. **CLAUDE.md 对齐的边界(A0)**:确认对齐**只**改两处 stale(`ccb ask`→`ah ask`、旧三角色→指向 SSOT 的四角色),角色明细**收敛到 SSOT 单源**(CLAUDE.md 不再自带角色表,改为一行指过去),避免双写漂移?

4. **死掉的 `rules/{AGENTS,CLAUDE,GEMINI}.md`(本轮新发现)**:它们描述**旧模型**(Gemini=架构师、单一主控、`ccb`),不生效但会误导任何读到的人/工具,且与新根 `AGENTS.md` 命名撞车。三选一:
   (a) 本轮一并**删除**(推荐,债一次清);
   (b) 顶部加"SUPERSEDED,见 /AGENTS.md"横幅、保留;
   (c) 本轮不碰,另立清理议题。
   —— 我的建议:**(a) 删除**,但因为是"改/删仓库文件"而非纯新增,按你的红线习惯我不擅自动,**等你点**。

---

## 6. 若放行,实施顺序(仅预告,不在本轮执行)

1. 建 `docs/sop/` 分环明细 + 根 `AGENTS.md` 入口(A1–A20 逐条 [V-transcribe] 落文,带证据锚)。
2. 最小编辑 `CLAUDE.md`(A0):两处 stale 修正 + 角色收敛指向 SSOT。
3. (判断点②若选)写 A17 advisory skill。
4. (判断点④若选 a/b)处理死 `rules/*.md`。
5. 派 a2(codex)严审:逐条 SSOT 断言 vs 证据锚 diff,抓"夹带私货";a4(claude)二审零回归。
6. PM-audit(`git diff`,不跑 cargo)→ 收口(`git add` 目标文档 + Co-Authored-By)→ 回你开 PR。
   —— 注意:本沉淀基本是新增 `.md`,**无 src 改动、无需 cargo**;A0 改 CLAUDE.md 也不触发 cargo。验证门相应放宽为"文档评审"。

---

## 附:operator 已拍板回执(4 决策)

1. 范围:只做 A 类纯沉淀(零回归);B 类(worktree/脚本/auto-merge/CI 门)另立"SOP 增强"议题。✅
2. 对齐仓库 `CLAUDE.md`:`ccb ask`→`ah ask`;旧三角色→当前四角色(以 inventory 核实为准 = `ah.toml` a1 codex / a2 codex / a3 antigravity / a4 claude)。✅
3. 新建仓库内人读 SSOT(`AGENTS.md` 或 `docs/sop/`);sandbox kernel 保持机器契约不动。✅
4. 本次不改派单流程为 worktree,维持主树串行 commit。✅
