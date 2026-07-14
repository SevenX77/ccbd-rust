# ah 自我知识 skills · 交付与打包 — Design

日期:2026-07-08。承接 PR #108(ah-commands 试点 + 通用内建 skills 机制,已合 main 298cb09)。
上游素材:`research/ah-internals-design.md`(master 设计,含锚点表 `research/ah-internals-anchors.md`)、`research/ah-commands-skill-design.md`。

## Goal

让「操作 ah 的 agent」——**外部集成 agent(主受众:开发者自己的 Claude Code / Cursor / Studio agent)+ ah 托管 master(次)**——不读源码就能正确使用、配置、观测 ah,并能持续驱动任务闭环。worker 一律不给(用户拍板:无用例,给了稀释 context)。

## Scope(6 项)

### R1 两个知识 skill(纯加数据,机制已在 main)

- `assets/builtin/skills/ah-config/SKILL.md` — 配置面:`.ah/` 布局(ah.toml + .ah/rules/<slot>.md + .ah/skills/<name>/)、ah.toml 真实字段(以 struct 为准,MCP 不是 `[master].mcp` 直字段)、组合模型 kernel+bundle+slot → provider 落点(claude `.claude/CLAUDE.md` / codex `.codex/AGENTS.md` / antigravity `.gemini/AGENTS.md`)、kernel 在二进制内不可改。
- `assets/builtin/skills/ah-runtime-state/SKILL.md` — 状态面:权威读法 = `ah events --format json`(RuntimeSnapshot 逐行 JSON,src/runtime_events.rs:50),别拿 `ah ps` 文本+另跑 tmux 拼;状态值域消歧(session.status=ACTIVE/KILLED/FAILED、agent.state 枚举、jobs.status 流转、RUNNING 不是 DB 枚举);cleanup 语义。
- 内容锚点全部来自 `research/ah-internals-anchors.md`(codex 逐条 file:line 核实),实施时不许引不存在字段。
- 正文 provider-中立:不写"你的 CLAUDE.md",三家落点并列;caveat 如实——master provider 配置层可配但今天 spawn 仍 claude-pinned(sessions.rs:301/338,v2 解)。

### R2 受众统一(修正 #108 的不对称)

受众按**角色**分,不按 skill 分:凡"操作 ah 的 agent"三个 skill 全拿;worker 零。

- 通道1(ah 托管沙箱):三个 skill 都注册 MasterOnly(worker-absent 断言,同 ah-commands 现有守卫)。
- 通道2(外部):三个 skill 都可装(ah-commands 不再"留议不导出"——外部 agent 用 ah 就是派活读状态,正是 ah-commands 内容)。

### R3 kernel skill 索引(兜底缺口修复)

master_kernel.md 现只指 ah-commands。扩成一个 2-3 行索引:命令参考→ah-commands;配置→ah-config;运行时状态→ah-runtime-state;`ah --help`/`ah <cmd> --help` 永真兜底保留。守卫测试:kernel 提及全部三个 skill 名 + 保留 `ah ask`/`ah master ack-ready` 锚(tests/builtin_skills.rs 已有的锚断言扩展)。

### R4 外部交付:文件夹复制(基线)+ 一键 plugin(增强)

**基线(本轮必做)**:skill 文件夹就是公开仓普通文件,README 加安装表:

| 目标 agent | 项目级 | 用户级 |
|---|---|---|
| Claude Code | `<项目>/.claude/skills/` | `~/.claude/skills/` |
| Codex | `<项目>/.codex/skills/` | `~/.codex/skills/` |
| Antigravity | `<项目>/.agents/skills/` | `~/.gemini/config/skills/` |

antigravity 路径依据官方文档(agy 自带 `builtin/skills/agy-customizations/SKILL.md`:workspace=`.agents/`,用户级=config 目录;ah 沙箱 home_layout.rs:2055 用 `.gemini/config/skills` 一致)。

**增强(一键安装,同轮或紧随)**:按 provider 的 plugin 打包,同一批 SKILL.md 文件不复制内容:
- Claude Code:公开仓加 plugin/marketplace 声明(`.claude-plugin/` + marketplace.json),用户 `/plugin marketplace add SevenX77/ah` + `/plugin install`。实施时核实官方 schema。
- Antigravity:`plugins/<name>/plugin.json` bundle(官方 Customization System 文档明载 plugin 可打包 skills+rules+MCP)。
- Codex:无 plugin 概念(以复制为准);实施时再核实一次。
- 不做 `ah skills install` CLI verb(用户否决:过度设计)。

### R5 #107 场景模板修复(4 处,operator review 发现)

1. README 安装命令指向 dev 仓 `ccbd-rust`——改公开仓 SevenX77/ah 的正式安装方式(一行装脚本/release)。
2. 本项目私货打标:a1.md 的 `CCB_TEST_SKIP_REAL_PROVIDER`/`mvp11_real_*`/VPS-OOM、master.md 的"沙箱无 toolchain"→ 标 "⚠ replace with your project's …" 占位符化。
3. master.md close-out(第 7 步 push/PR/merge)与真实运行不符(实际 operator 做,master 沙箱无 gh auth)→ 改为如实描述 + 标前提(master 需 git/gh 凭据才自 close-out)。
4. README 补一句:ah 自动给 master 注入内建 skills(ah-commands 等)。

### R6 新 skill:ah-operate(外部 agent 驱动 ah 的 playbook)

来源 = operator(本会话人类代理人)实战 SOP 提炼。ah-commands 是**参考**(每条命令干什么),ah-operate 是**流程**(怎么持续驱动闭环)。核心内容:

1. **派活姿势**:brief 落文件 → `tmux load-buffer` + `paste-buffer -p` + `send-keys Enter`(绝不 printf/echo 双引号注入,反引号=命令替换);paste 折叠时补一次 Enter 确认提交。
2. **不干等靠机制**:监听 jobs/agent-state **转移**(diff,非轮询全量);每个阶段转移点亲自 `capture-pane` 核对 pane 真相(状态可能撒谎)。
3. **卡点解锁**:PROMPT_PENDING(rate-limit 弹窗/交互向导)→ 先 capture 看清选项,方向键选中(核对 `›` 位置)再 Enter;或 `ah prompt resolve`。STUCK 死胡同 → 先读 pane 真相,再 cancel+kill+up 重派(从未开始的任务不用担心半成品)。
4. **门禁节奏**:brief → 设计停下 → operator 过目 → 放行 → 实施 → 双审 → PM-audit → operator close-out(精确文件 commit/PR/CI/合并)。master 绝不自合;中途 scope 漂移用注入纠偏。
5. **收尾纪律**:只 add 目标文件;CI 红先证伪(同 commit 并行 job 一红一绿=flake 特征,rerun 一次+找 pre-existing 证据)再 rerun,绝不带红合并。
6. **升级边界**:只有产品方向选择才 escalate 给人;工程细节(commit 落点、修复顺序)自决。

受众与 R2 相同(外部+master;worker 零)。触发词:"Use when driving an ah master/stack through a multi-step task: dispatching briefs, monitoring job transitions, unblocking a stuck or prompt-pending agent, gating design→implementation→review, or closing out a task to PR."

## Non-goals

- `ah skills install` CLI verb(否决)。
- Studio 的 `ah status --json` 新 verb + 生命周期 FSM(另轨未立项;ah-runtime-state 只文档化现有面)。
- master 换 provider(v2)。
- worker 侧任何 skill 暴露。

## 验收

- 单测:三个(+ah-operate 四个)skill master 三家 provider 都物化、worker 三家都无、kernel 索引三名齐+锚不丢、frontmatter 合法、正文无不存在字段(抽查断言)。
- 活体触发门:操作按用户决定留实战验证(用户在真实使用中观察触发);plugin 安装至少 claude 一家真装通。
- 全量串行 cargo 绿 + CI 并行绿。

## 决策记录

- 受众=操作者(外部+master),worker 零 —— 用户 2026-07-07/08 拍板。
- ah-commands 触发留实战验,不做隔离活体门 —— 用户 2026-07-08 拍板。
- ~~外部交付=文件夹复制,不做 CLI~~ → **被 v2 修订推翻**(scope 从"纯 skills"扩到"skills+CLI 配置",见下)。

---

# v2 修订(2026-07-08 用户反馈轮)

## R1+ 配置面扩栏:CLI 配置(model / effort / autoCompact / statusLine)

- ah-config 的内容加一节「provider CLI 配置」:claude 的 `settings.json`(model、statusLine、autoCompact 等)放哪、ah 沙箱内谁写它。
- **ah 已有 claude settings.json 物化路径**(home_layout.rs:2028,测试 :2353 读 `.claude/settings.json`)→ ah.toml/bundle 携带 provider settings(给 master/worker 沙箱统一钉 model/effort/statusline)是**顺着现有机制加字段**,不是新机制。列为本 spec 的候选 T7(设计后实施)。
- 外部 agent 的 settings:plugin 装不了 settings(plugin 只有 skills/commands/hooks/MCP)→ 走指南或 CLI 合并,**绝不静默覆盖用户已有 settings.json**(打印 diff / 交互确认)。

## R4-v3 外部交付:统一 CLI 一键(用户 2026-07-08 拍板,plugin track 舍弃)

plugin 能力有限(装不了 settings、三家覆盖不齐)→ **舍弃 plugin,统一走一个 ah 安装 verb**:

- `ah agent-setup --target <claude|codex|antigravity> [--project|--global]`(名字实施时定):一条命令写 skills(该 provider 的 skills 目录)+ 合并 settings(model/statusLine/autoCompact 等),三家同一条代码路径。
- 纪律:**交互确认、绝不静默覆盖**用户已有配置(存在冲突打印 diff 让用户选)。
- **安装指南保留为兜底文档**(每样东西是什么/放哪/怎么验证),同时就是 verb 行为的人读说明。
- 复制文件夹 = 指南里的手动路径,不再是主交付。

## R6-v2 ah-operate playbook 修订(自我纠错 + 三项新职责)

**纠错 1(派活)**:首选 **`ah tell master`**(产品正道),tmux 直注**降级为 fallback**——此前活栈 tell 失效是 daemon 未登记 master_pane_id 的 bug,workaround 不能写成 SOP 主路径。伴随 backlog:修 tell 可靠性,让 fallback 退休。
**纠错 2(监控)**:首选订阅 **`ah events --format json`**(agent/state 转移事件流)+ **`ah pend <job_id>`**(单 job 阻塞等待),DB 轮询降级为补充。**产品缺口备案:events 流里没有 job 层事件**(RuntimeSnapshot 只有 sessions/agents),补上后轮询彻底退休。

**新职责(operator 三监控,用户 2026-07-08 增派)**:
1. **context 守门**:每次 capture-pane 顺读状态栏 context 提示(如 "/clear to save NNNk tokens");**任务/阶段收敛点主动让 master `/clear`**,下一步注入 = 新 orientation(指向落盘的 spec/design/handoff 文件)+ 开工 brief 合一,替代写 handoff。依据:产出全落盘(research/ + .kiro/specs/)才敢清,这是"产出必须落盘"纪律的另一个回报。
2. **model/effort 守门**:常态 = 最强模型 + effort high;仅超大/思考量极重任务升 max。发现 master/worker 掉到弱模型(如 rate-limit 弹窗诱导降级)→ 立刻纠正,绝不默许"低配继续跑"。
3. **额度守门**:盯订阅用量信号(pane 状态栏 "Now using usage credits"、codex rate-limit 弹窗);出现先**核实新鲜度**(pane 状态栏是渲染残留,可能滞后于额度刷新——2026-07-08 实例:credits 提示挂着但订阅已续),向用户确认后再动作;确认属实才给选项(降 model/effort / 暂停非关键派发 / 用户处理额度),**不许静默烧 credits 或静默降级硬跑,也不许拿滞后 UI 当实时告警**。

## v2 已拍板(用户 2026-07-08)

- ✅ `ah agent-setup` 统一安装 verb 立项(R4-v3,plugin 舍弃)。
- ✅ T7(ah.toml 携带 provider settings)入轮。

## 关联 backlog(不在本 spec,记录防丢)

- **worktree-per-task**(单独立项):派发时建任务 worktree(worker cwd=树内)+ cargo 全局串行锁(VPS 内存)+ 合并后回收 + 共享文件单 owner。
- **验收模式开关**(随 worktree 项设计):牵涉 UI/前端的验收,按任务可选 `user`(用户在场亲验,更快)或 `agent`(playwright/computer-use 自动验);后端/纯代码一律自动验收不经人。用户实践中会频繁切换,开关要轻(brief 级/派发级,不是重配置)。
- **`ah tell master` 可靠性修复**(daemon 未登记 master_pane_id 时 tell 失效)——修好后 tmux 直注 fallback 退休。
- **events 流补 job 层事件**——补好后 DB 轮询退休。
