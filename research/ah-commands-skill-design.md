# DESIGN — ah-commands 渐进式披露 skill(给 operator 过目)

独立小 PR,与 #107 无关。目标:给 ah 的 master 一份**权威 agent-facing 命令参考**,走**渐进式披露**(不塞 kernel,做成一个按需触发的 skill),把 kernel 注意力还给安全/角色边界。

先 DESIGN 停下回 operator 过目再实施。判断点见 §8。

素材:a3(antigravity)已出触发+编排设计 `research/ah-commands-skill-trigger-design.md`;命令/机制锚点由 master 亲自 grep 核实(下 §1 全带 file:line)。

---

## 1. 机制 grounding(master 亲验 file:line,不信转述)

- **必须走项目 skill,不能走 bundle**:项目 skill 经 `resolve_project_skills`(`src/provider/skills.rs:34` 读 `.ah/skills`)物化进三家沙箱(`home_layout.rs` 的 `materialize_claude_skills`/`materialize_codex_skills`/`materialize_antigravity_skills`);而 bundle 来源的 skill **只支持 claude**(`src/provider/bundles.rs:267-270`:codex/antigravity 报错待 PR-3/4)。→ 走项目 skill。
- **skill 声明面**:ah.toml `[master] skills = [...]` / `[agents.<id>] skills = [...]`(`src/cli/config.rs:64` master 字段、`:104` agent 字段,示例 `:533-537`);运行时进 `ExtensionConfig.skills`(`src/provider/extensions.rs:13`)。
- **命令全集**:`src/bin/ah.rs` clap 定义(`enum Cmd` `:53`,`enum MasterCmd` `:208`,`enum PromptCmd` `:194`)。
- **无「默认 skill」机制(放置决策关键)**:ah 只用 `include_str` 发内建**规则**——`MASTER_KERNEL`/`WORKER_KERNEL`/`DEFAULT_MASTER`/`DEFAULT_WORKER`(`src/provider/builtin.rs:3-6`);`assets/` 下**无任何 skill**。→ 「随二进制发的默认 skill」是**新增的一小块机制**,不是现成的。
- **master 是 claude-locked**(v1,`src/rpc/handlers/sessions.rs:292` 附近),故 master-facing skill 实际只需落到 claude。
- **kernel 待瘦身行**:`assets/builtin/master_kernel.md:19`(枚举 pend/watch/logs/ps/attach 那句)。

---

## 2. 命令范围(agent-facing 编排子集)

**收录(11 + 1 候选,synopsis 已与 `ah.rs` clap 逐条核对,无编造):**

| 命令 | synopsis | 何时用(a3 归组) |
|---|---|---|
| `ah ps` | `ah ps` | 查全部 session/agent/pending evidence,判子任务卡没卡 |
| `ah events` | `ah events [--format json]` | JSON Lines 流式看生命周期快照/状态机转移 |
| `ah ask` | `ah ask <agent_id> <text> [--wait] [--request-id <id>]` | 给某 worker 派具体任务、拿 job_id 追踪 |
| `ah tell` | `ah tell <target> <text> [--session <s>] [--request-id <id>]` | 异步向 master pane/agent 通报,不阻塞等应答 |
| `ah pend` | `ah pend <job_id>` | 阻塞等某异步 job 完成再决策 |
| `ah watch` | `ah watch <agent_id> [--since-event-id <n>]` | 实时流式追某 agent 输出/动作事件 |
| `ah logs` | `ah logs <agent_id> [--since <n>]` | 一次性读某 agent 完整历史输出分析 |
| `ah cancel` | `ah cancel <job_id>` | 取消排队中/运行中的 job |
| `ah kill` | `ah kill <target_id> [--session] [--force]` | 强杀失联 agent 或整组 session |
| `ah attach` | `ah attach <target> [subject] [--session <s>]` | 人工挂进 agent/master tmux 底层调试(逃生口) |
| `ah master ack-ready` | `ah master ack-ready [--cutover-id <id>]` | successor master 就绪、向 ahd 报到接管 |
| `ah prompt resolve` **(收)** | `ah prompt resolve <agent_id> [--action <a>] [--keys <k>] [--save-to-kb]` | worker 卡 PROMPT_PENDING 时 master 代答解锁 |

- **`ah prompt resolve` 收录**:a3 论证成立——它是解 worker 交互悬挂的唯一手段,不给会让编排流无限卡死,属核心 agent-facing。**采纳收录**。
- **`ah attach` 保留但标注**:对自治 agent 可操作性最低(挂 tmux 偏人工逃生口),留作参考无害。

**排除(运维,master 不该跑,安全边界已拦):** `start` / `stop` / `up` / `doctor` / `setup` / `config` / `bundle`;`ah agent notify`(hook 用,非 master);`ah master cutover`(cutover 是对 master 做,master 只出 ack-ready)。完整表 `ah --help`。
a3 的排除理由采纳:master 无 daemon-mgmt 权限 + 防「幻觉自毁」(遇异常去 `ah setup` 自愈死循环)。

---

## 3. SKILL.md 设计

- **frontmatter `description`(触发成败关键,采用 a3 稿,实施可微调收紧)**:
  > Authoritative CLI reference for 'ah' agent-facing orchestration commands (ah ps, ask, tell, pend, watch, logs, events, cancel, kill, attach, master ack-ready, prompt resolve). Activate when you need to inspect agent/job status, dispatch tasks to worker agents, retrieve execution logs or outputs, cancel/kill tasks, attach to tmux sessions, stream lifecycle events, resolve a blocked agent prompt, or perform master cutover.
- **触发覆盖**(a3 双保障:精确 token + 泛化意图):①查状态 ②派活 ③读输出 ④编排干预 ⑤切换/交互;剔除 git/cargo/rust/test 防过触发。
- **正文按「何时用」分五组**(非字母序):状态查询与监视 / 任务派发与异步通信 / 结果追踪与日志拉取 / 运行期干预与调试 / 角色接管与协同交互。每条附 synopsis + 一句何时用(见 §2 表)。

---

## 4. 触发可靠性 + dogfood 验收(a3 方案,采纳)

触发是成败关键,**必须 dogfood 实测激活,不是写完就算**。两场景:
- **场景 A(悬挂诊断恢复)**:prompt「test-worker 疑似卡等 token,查它 pending 的 job、取消、再 tell master 已解决」→ 期望自动载入 ah-commands skill → 走 `ah ps` → `ah cancel <job_id>` → `ah tell master ...`;验 `transcript.jsonl` 里 skill 被 active 载入 + 命令语法符合 synopsis(没编 `ah logs --from` 之类)。
- **场景 B(cutover)**:prompt「successor master 已就绪,通知 daemon 接管」→ 期望据 description 激活 → 精确用 `ah master ack-ready`(没脑补 `ah cutover`/`ah master switch`)。
- **验收指标**:①skill 载入率(意图产生时 SKILL.md 进上下文)②命令准确率(语法/flag 与 synopsis 一致,无幻觉参数)。

---

## 5. 放置决策(PM call —— 本 DESIGN 的主 gate)

### 5.1 先定 scope:**master-only(建议)**
谁需要 ah-commands?**只有 master 编排**;worker kernel 明令 worker 不 self-dispatch、不 `ah ask`、只做单任务(`assets/builtin/worker_kernel.md:5-9`)。给 worker 这份参考 = 诱导它越界。→ **ah-commands 是 master-only skill**。这也让 §1「三家覆盖」的顾虑消解:master 是 claude,只需落 claude。

### 5.2 放哪:Option A(项目 skill)vs Option B(内建默认 skill)

| | A. 项目 skill | B. 内建默认 skill(新一小块) |
|---|---|---|
| 落点 | 仓库 `.ah/skills/ah-commands/`,ah 自己 ah.toml `[master] skills=["ah-commands"]` 声明;dev-programming 模板也带一份 | `assets/builtin/skills/ah-commands/SKILL.md` `include_str` 进二进制,master 沙箱 prep 时无条件写入 master 的 `.claude/skills/ah-commands/` |
| 新代码 | ~零(SKILL.md + ah.toml 一行 + kernel 瘦身) | 一小块:内建 const + master-only 物化(写文件非 symlink)+ 测试 |
| 通用性 | **否**:只有声明了的项目才有 | **是**:任何 ah master 都有,场景无关 |
| 随二进制/版本锁 | 否,和 kernel 可能漂移 | **是**,和 kernel 同版本、永不漂移 |
| 要不要每项目声明 | 要 | 不要 |

### 5.3 决定性论据:kernel 指针与放置**耦合**
Design #3 让 kernel **无条件**指向「见 ah-commands skill」。要让这个指针**永不悬空**,skill 必须**无条件在场** → 只有 **B** 能保证。走 A,任何没声明 `skills=["ah-commands"]` 的 master 会拿到一个**指向不存在 skill 的悬空指针**(比不瘦身还糟)。

**→ PM 建议:选 B(内建默认 skill,master-only)。** 它是唯一同时满足「通用 + 随二进制 + 版本锁 + 无需声明 + kernel 指针不悬空」的选项;scope 收到 master-claude 后它是自洽的一小块,且本 PR 本就要改二进制(kernel 瘦身),B 不新增风险类别。
**A 作为低成本回退**:若你想先用最小改动 dogfood 验证「触发可靠性」这个成败关键再决定要不要建 B 机制,可先 A(仅 ah 自己声明)、B 留作紧随的第二步。代价:kernel 指针得写软(「若 ah-commands skill 可用则用,否则 `ah <cmd> --help`」),或本 PR 先不瘦身 kernel、只加 skill,瘦身随 B。

这条**定不了由你拍**;我倾向 B。

---

## 6. kernel 瘦身 diff(精确,改二进制 asset;worker kernel 不动)

`assets/builtin/master_kernel.md` 的 Orchestration Contract 段:

```diff
 ## Orchestration Contract

 - Dispatch through ah with `ah ask <agent_id> "<task>" [--wait]`.
-- Read results and evidence through implemented ah commands such as `ah pend <job_id>`, `ah watch <agent_id>`, `ah logs <agent_id>`, `ah ps`, and `ah attach`.
+- For the full agent-facing command reference (status, results, control, cutover), use the `ah-commands` skill.
 - Report status through ah-managed channels and the current user conversation. Do not invent unavailable ah subcommands.
```

- 保留 `ah ask` 一行(dispatch 是 master 最核心动作,kernel 留一个具体锚点),只把**枚举那句**换成 skill 指针。
- 若选 A 且不建 B,这段指针需写软(见 §5.3),避免悬空。选 B 则如上直接指。

---

## 7. 实施计划(放行后,不在本轮执行)

1. 起独立分支(off main,别碰 #107)。
2. 写 `SKILL.md`(§3;description + 五组正文,每条带核对过的 synopsis)。
3. 放置:按 §5 决定——B 则加 `assets/builtin/skills/ah-commands/SKILL.md` + 内建 const(`builtin.rs`)+ master-only 物化路径 + 测试;A 则加 `.ah/skills/ah-commands/` + ah.toml 声明。
4. kernel 瘦身(§6)。
5. 测试(codex,TDD 红绿):①SKILL.md frontmatter 合法/description 含关键触发词;②(B)master 沙箱无条件拿到 ah-commands skill 文件、worker 沙箱**没有**它;③kernel 不再含枚举句、含 skill 指针;④命令 synopsis 与 `ah --help`/clap 不漂移(可对 `ah.rs` 断言关键子命令仍存在)。全量串行 cargo。
6. **dogfood 触发实测(§4 两场景)**——成败关键,必须真跑观察 skill 激活 + 命令准确,不是单测绿就算。
7. PM-audit → **回 operator 过目 → 再开 PR(别自己合)**。

**dispatch 分工**(按角色):设计重判(命令选择/触发策略)a3 已出;锚点核实/kernel diff/实施/测试派 codex;另一 codex 或 a4 审;dogfood 触发实测由 a4(claude,e2e)在能观察 transcript 的环境跑,或交 operator 在活栈观察。

---

## 8. 需 operator 拍板

1. **放置决策(主 gate)**:A(项目 skill,零新代码但不通用+kernel 指针会悬空)vs **B(内建默认 skill,一小块新代码,通用+版本锁+指针不悬空)**。**我建议 B**;若想先廉价验证触发再建 B,可先 A(见 §5.3 的 kernel 软指针代价)。
2. **scope 确认**:ah-commands **master-only**(worker 不给,防越界)?我建议是。
3. **kernel 瘦身**:确认按 §6 把枚举句换成 skill 指针、保留 `ah ask` 一行?(选 A 时指针需写软。)
4. **`ah prompt resolve` 收录**、**`ah attach` 保留**:确认?我建议都收(prompt resolve 是解悬挂唯一手段;attach 留作逃生口参考)。
