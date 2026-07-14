# DESIGN — ah-config / ah-runtime-state 两个「ah 自我知识」内建 skill(给 operator 过目)

承接 ah-commands 试点(PR #108)。本轮**纯加数据**到已建的【通用内建 skills 机制】,再加一个**新机制点:外部交付**。只 DESIGN,停下回 operator 过目再实施,不开 PR、不自合。

素材:codex 锚点表 `research/ah-internals-anchors.md`(条条 file:line)+ a3 结构/触发设计 `research/ah-internals-skill-design.md`(PM 已审:结构/触发采纳,外部交付方案有机制错误、本文纠正)。

---

## 0. 关键依赖 + 一个纠正(先说)

- **依赖 #108 先合**:内建 skills 机制(`BUILTIN_SKILLS`/`materialize_builtin_skills`)现只在 `feat/ah-commands-builtin-skill`(PR #108,**未合**),main 顶是 #106、**没有这机制**(codex 核实 `anchors.md:65-68`)。→ **本轮实施 blocked on #108 landing**;设计可先定。
- **纠正 a3 外部交付方案 C**(它提「释放到 `.ah/skills/`,组合编译器把 skills 拼进 CLAUDE.md」)——**两处机制错**,codec 锚点坐实:
  1. **skills 不拼进 CLAUDE.md**。`materialize_builtin_rules`(→`.claude/CLAUDE.md`)与 `materialize_claude_skills`(→`.claude/skills/<name>`)是**两条独立路径**(`anchors.md:47-63`,home_layout `:213/:224`)。rules 组合 ≠ skills 发现。
  2. **外部 Claude 不扫 `.ah/skills/`**;它扫 `~/.claude/skills/`(personal)和 `<project>/.claude/skills/`(project)。且往 `.ah/skills/` 写内建名会撞上 #108 的**保留名 guard**(自相矛盾)。
  → 外部交付另设,见 §5。

- **去 claude 中心(provider-无关)**:master 的 provider 可配(`MasterConfig{cmd, provider: Option<String>}`,`config.rs:44-67`),不钉死 claude。两个 skill 的设计**不以 master=claude 为叙述主语**。诚实 caveat 见 §0.1。

### 0.1 诚实 caveat:配置层可配 ≠ 今天已等价(别误导读者)
grep 复核到的真实分层,写进设计避免读者误以为「配了 provider 就等价成熟」:
- **机制层已 provider-无关**:`materialize_builtin_skills(skills_dir, role)` 从三条 provider 路径按 role 分派——claude `.claude/skills`(home_layout.rs:224)、antigravity `.gemini/config/skills`(:291)、codex `.codex/skills`(:1007);`materialize_builtin_rules` 落点 claude `.claude/CLAUDE.md` / codex `.codex/AGENTS.md` / antigravity `.gemini/AGENTS.md`(`builtin_rules_target`)。→ **机制不认 claude,不用改**。
- **但 ah 托管 master 今天仍 claude-pinned**:master spawn 运行时硬编码 `"claude"`(`sessions.rs:301/333/338`),且 master rules 仅 claude 落(`home_layout.rs:498`:`role==Master && provider!="claude" → 不写`)。→ 「master 配任意 provider」是**配置层 / v2**(ah-v1 design.md:19 明确 v1 master 保持 claude),不是今天 ah 托管 master 的运行现实。
- **成熟度不齐**:codex/claude 完成检测已验;**antigravity master 有 log-signal 缺口**。跟本设计无关,但别让读者以为「配了就成熟等价」。
- 结论:skill 正文用 **provider-中立**措辞(主受众=外部任意 provider agent + 前瞻),同时不暗示「ah 托管 master 已经能随意换 provider」。

---

## 1. 两个 skill 的内容大纲(grounded,正文素材来自 anchors.md)

### ah-config —— ah 的「使用+配置」面
- **`.ah/` 布局**:`ah.toml`(项目根,向上发现,`CCB_CONFIG_PATH` 覆盖)+ `.ah/rules/<slot>.md` + `.ah/skills/<name>/SKILL.md` + `.ah/bundles/<name>/`。各自消费点见 `anchors.md:22-27`。
- **ah.toml 结构**(以真实 struct 为准 `anchors.md:13-20`):`version="1"`;`[master]` = cmd/provider?/enabled/window_size/hooks/plugins/**skills**/**bundle**;`[agents.<id>]` = provider/env/hooks/plugins/**skills**/**bundle**;`[completion]`/`[sandbox]`/`[daemon]{}`。**注意**:MCP **不是** `[master].mcp` 直接字段,走 ExtensionConfig/bundle(`anchors.md:20,32`)——如实写,别编 `[master].mcp`。
- **组合模型**(`anchors.md:38-52`):`kernel + bundle layers + (.ah/rules/<slot> 或 role default)`,`\n\n---\n\n` 连接;**override 替换 default 不是追加**;按 provider 落 `.claude/CLAUDE.md` / `.codex/AGENTS.md` / `.gemini/AGENTS.md`;**master rules 仅 claude 落**。
- **kernel 二进制内不可改**(`builtin.rs:1-6`,include_str);**内建 skills 机制**(#108 落地后)= 加 skill 只加数据。

### ah-runtime-state —— ah 的「状态」面
- **权威读法**(`anchors.md:136-139`):读 `ah events --format json`(逐行 `RuntimeSnapshot` JSON),**别**拿 `ah ps` 文本 + 另跑 tmux 拼(`ah ps` 是人读表、字段子集)。
- **RuntimeSnapshot 形状**(`anchors.md:77-101`):`schema_version`/`event`/`sequence`/`reason`/`runtime_state`/`ahd_alive`/`active`/`ahd_has_inventory`/`tmux_server_alive`/`master_tmux_alive`/`worker_tmux_alive`/`worker_tmux_expected_count`/`sessions[]`/`agents[]`;session snapshot(session_id/status/master_state/master_pid/active_agents…)、agent snapshot(agent_id/provider/state/sub_state/pid/tmux_alive…)。
- **枚举**:`runtime_state` = active/inactive/starting/degraded;`reason` = initial/inventory_changed/tmux_changed/agent_changed/shutdown/daemon_absent/daemon_lost。
- **状态值域消歧(关键,别混锅,`anchors.md:103-116`)**:`session.status`=ACTIVE/KILLED/FAILED;`master_state`=IDLE/BUSY;`agent.state`=SPAWNING/IDLE/WAITING_FOR_ACK/BUSY/PROMPT_PENDING/STUCK/FAILED/CRASHED/KILLED/UNKNOWN…;`jobs.status`=QUEUED→DISPATCHED→COMPLETED/FAILED/CANCELLED;`evidence.status`=PENDING/REVIEWED;`RUNNING` **不是** DB 枚举(仅日志)。cutover/recovery 有各自 phase 枚举,别与上面混。
- **cleanup 语义**(`anchors.md:143-150`):session cascade → `status=KILLED` + reap 非终态 agent;per-agent cleanup 删 I/O entry/FIFO/tmux/sandbox home。

### 1.1 正文 provider-中立写法(实施纪律)
两个 SKILL.md 正文**不以 claude 为默认读者**:
- 讲组合落点时**三家都列**(claude→`.claude/CLAUDE.md`、codex→`.codex/AGENTS.md`、antigravity→`.gemini/AGENTS.md`),别写「你的 CLAUDE.md」。
- 讲「读者(master)读哪个规则文件」时按其 provider 说:codex master 读 `.codex/AGENTS.md`、antigravity master 读 `.gemini/AGENTS.md`、claude master 读 `.claude/CLAUDE.md`;别假设读者在 claude。
- 附 §0.1 的 caveat 精神:可配 ≠ 今天 ah 托管 master 已换 provider(仍 claude-pinned)——正文别暗示相反。

---

## 2. 结构决策:**分两个(采纳 a3)**
`ah-config`(静态、低频:改 ah.toml/布局)与 `ah-runtime-state`(高频:读 snapshot/events)心智分支不同。合成一个 `ah-internals` 会让**高频状态查询每次拖上庞大配置文档**污染 context。分开 = 触发精准 + 按需加载。维护成本(两个子目录 + 注册表两条)可忽略。**PM 定:分两个。**

## 3. 触发词(采纳 a3 + 微调,以 Use when 开头、强绑专有名词 + 否定防误触发)
- **ah-config**:`Use when configuring 'ah': editing 'ah.toml', adding agents or rules under '.ah/rules/', registering skills/bundles, or understanding how kernel+bundle+slot compose into '.claude/CLAUDE.md' / '.codex/AGENTS.md' / '.gemini/AGENTS.md'. Not for general app config or non-ah 'Cargo.toml'/'package.json' settings.`
- **ah-runtime-state**:`Use when inspecting the live 'ah' runtime: reading 'ah events --format json' / the RuntimeSnapshot JSON, checking session.status / agent.state / master_state, whether ahd and tmux panes are alive, or how ah reports authoritative state. Not for general database/service health or app-level status.`
- 实施时用 codex 核实的真实字段名收口,别引不存在的字段。

## 4. Scope + **两通道交付模型(核心)**
两个 skill 的受众 = **外部集成 agent(主)+ ah 托管 master(次)**,**worker 一个都不给**。这落成**两条独立交付通道**:

| | 通道1:ah 托管沙箱(复用现机制) | 通道2:外部 agent(**新机制**) |
|---|---|---|
| 谁 | ah 自拉的 **master**(次要受众) | 开发者本机 claude / Cursor / Studio agent(主要受众,**不在 ah 沙箱**) |
| 怎么送 | `materialize_builtin_skills` + 注册表 `scope=MasterOnly` | **新** `ah skills install`(§5) |
| worker | **拿不到**(MasterOnly 过滤 + worker-absent 断言,同 ah-commands) | 不适用 |
| 本轮改动 | **纯加数据**(注册两条 + 两个 SKILL.md),不动机制;依赖 #108 | **新 CLI verb**(operator 决策,§5) |

**关键**:`scope=MasterOnly` 只管通道1(ah 沙箱内 master 有、worker 无)。通道2 是外部,由用户显式 `ah skills install` 装进自己 agent 的家,不受 scope 标志约束。两通道都不给 ah 沙箱内的 worker。

**通道1 的 provider 现实(诚实标注,见 §0.1)**:机制层 provider-无关(三家 provider 路径都调 `materialize_builtin_skills`),但 ah 托管 master 今天 spawn 仍 claude-pinned(`sessions.rs:301/338`),故当前通道1 的 master skill 实际落 `.claude/skills`。将来 master 参数化 provider(v2)后,同一注册数据自动落该 provider 的 skills 目录、无需改机制。**这正是本轮设计坚持 provider-中立的原因**——数据与措辞不绑 claude,v2 到来零改动。

## 5. 外部交付机制(新机制点 —— operator 主 gate)
现状(codec `anchors.md:160-166`):**无任何** `skills`/`export`/`install` verb;项目 skill 只 symlink 进 **ah 托管沙箱 home**,**从不**写用户真实 `~/.claude/skills`。外部交付 = 必然新机制。

**方案权衡**:

| | (A) 新 `ah skills install` **【推荐】** | (B) 纯文档手动 | (C) a3 的「释放到 .ah/skills + 拼 CLAUDE.md」 |
|---|---|---|---|
| 工作原理 | CLI 把内建 SKILL.md 写进**目标 agent 的 provider** 真正扫描的 skills 目录 | README 指引手动复制 | 写 `.ah/skills` 靠组合器拼进 CLAUDE.md |
| 目标路径 | **按 provider 映射**(见下表),非钉死 claude | 用户手指 | `.ah/skills`(外部 agent **不扫**) |
| 侵入度 | **低**:写的是各 provider 已知标准 skills 目录 | 极低(纯文档) | —— |
| 版本漂移 | 升级后重跑 install 幂等覆盖同步 | 极差(易忘) | —— |
| 可行性 | ✓ | ✓ | **✗ 机制错**(§0:skills≠rules;.ah/skills 不被扫;撞保留名 guard) |

**PM 推荐:方案 A `ah skills install`,provider 作为一等参数(去 claude 中心)**:
- 签名:`ah skills install [names...] --target <claude|codex|antigravity> [--global | --project] [--force]`。**`--target` 是 provider 维度**(不默认 claude);未给时可报错要求显式,或按当前 `ah.toml` 里存在的 provider 集合逐一装。
- **目标 skills 目录按 provider 映射**(镜像 ah 沙箱内已知约定 `home_layout.rs:224/291/1007`,`--global`=用户真实 HOME、`--project`=项目根):

  | provider | project(`--project`) | global(`--global`) |
  |---|---|---|
  | claude | `<project>/.claude/skills/<name>/` | `~/.claude/skills/<name>/` |
  | codex | `<project>/.codex/skills/<name>/` | `~/.codex/skills/<name>/` |
  | antigravity | `<project>/.gemini/config/skills/<name>/` | `~/.gemini/config/skills/<name>/` |

  (沙箱内用的是这三条相对布局;外部真实 HOME 的确切路径**实施时对每家再核实一次**,尤其 antigravity 的外部 CLI skills 位置——标待定,别硬写。)
- 默认 `--project`(项目级最省心、不碰用户全局配置;项目级提示加 `.gitignore`);`--global` 显式可选。
- 只导出**外部适用**的 skill(ah-config / ah-runtime-state);`ah-commands` 是 master 编排用,外部是否也导出另议(轻标,不默认导出)。
- 幂等覆盖;`ah skills list` 列可装的内建 skill(可选)。
- **与 scope 的关系**:install 是外部显式动作,与 `BUILTIN_SKILLS.scope`(管通道1 ah 沙箱内)正交;install 只允许非-worker-only 的 skill。

**分阶段选项(给 operator)**:①一次做全(通道1 数据 + 通道2 `ah skills install`);②先做通道1 + skill 内容(依赖 #108),`ah skills install` 作紧随第二 PR。我倾向**①一次做全**——否则主受众(外部)拿不到,skill 近乎白发;但 install 是新 verb,若你想先审 skill 内容再上 verb,②也合理。

## 6. Studio 关联(标注,不做)
本轮**只文档化现有状态面**(RuntimeSnapshot + `ah events` 形状)。Studio 那条「新增 `ah status --json` verb + 生命周期 FSM」是**另一条未立项轨**,本轮不新增 verb;将来立项时 ah-runtime-state skill 同步扩。`ah skills install`(§5)是**交付机制**、不是状态 verb,两者别混。

## 7. 实施计划(放行后,不在本轮)
1. **前置**:#108 合进 main(机制到位)。
2. 加两个 `assets/builtin/skills/{ah-config,ah-runtime-state}/SKILL.md`(正文按 §1 grounded 大纲,每条可回溯 anchors.md)。
3. 注册表加两条 `scope=MasterOnly`(纯数据)。
4. (若选一次做全)实现 `ah skills install`(§5)+ 测试:**对每个 `--target` provider** 断言写到该 provider 映射目录(claude `.claude/skills` / codex `.codex/skills` / antigravity `.gemini/config/skills`)、`--global`/`--project` 分别落临时 HOME/临时项目、幂等、只导非-worker skill。
5. 测试:通道1——ah 沙箱 master 拿到两 skill、**三家 worker 都无**(同 ah-commands 守卫);frontmatter 合法;内容不引不存在字段;正文 provider-中立(不写「你的 CLAUDE.md」,三家落点都列)。
6. dogfood:两 skill 的触发实测(同 ah-commands 的活激活门,operator 环境);`ah skills install --target <provider>` 真装进一个外部 agent 的对应 skills 目录看能否被发现(至少 claude 一家,codex/antigravity 按可得性)。
7. dispatch:锚点已 grounded;SKILL.md 正文 + 注册 + `ah skills install` + 测试派 codex;另一 codex/a4 审;a3 触发词已出。PM-audit → **回 operator 过目 → 再开 PR,别自合**。

## 8. 需 operator 拍板
1. **外部交付方案**:采纳 **A `ah skills install`**(provider 作一等参数 `--target`,写目标 provider 真实 skills 目录,**去 claude 中心**)?否决 a3 方案 C(机制错)?
2. **一次做全 vs 分阶段**:通道1数据 + 通道2 verb 一个 PR,还是 verb 拆第二 PR?我倾向一次做全(否则主受众拿不到)。
3. **结构**:分两个 `ah-config`/`ah-runtime-state`(我推荐)确认?
4. **install 默认**:`--target` 未给时报错要求显式,还是按 `ah.toml` 现有 provider 集逐一装?`--project`(需 gitignore)还是 `--global` 默认?我倾向 **--target 显式必填 + --project 默认**。
5. **ah-commands 要不要也进 `ah skills install` 可导出集**(外部 agent 若也驱动 ah CLI)?我倾向本轮不默认导出、留议。
6. **依赖**:确认本轮实施**在 #108 合并后**再起(机制前置)?
