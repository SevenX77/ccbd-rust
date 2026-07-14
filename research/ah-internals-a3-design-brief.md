# Design brief for a3 (antigravity) — ah-config / ah-runtime-state 的结构·触发·外部交付

你是 a3(设计/领域分析,不写实现码、不碰 git)。产出一份 markdown 设计分析,落盘 `research/ah-internals-skill-design.md`。锚点细节(字段名等)由 codex 并行核实,你聚焦**设计判断**,基于下面已确认的形状即可推进。

## 背景(已确认,直接采信)
- ah 已有【通用内建 skills 机制】(main 就位):`assets/builtin/skills/<name>/SKILL.md` → include_str 注册表 + `materialize_builtin_skills` 写进沙箱,按 role/scope 过滤,三家 provider 接线。加新 skill = **纯加数据**,不动机制。试点 `ah-commands`(MasterOnly)已开 PR #108。
- 本轮加两个「ah 自我知识」skill:
  - **ah-config** = ah 的「使用+配置」面:`.ah/` 布局(ah.toml + `.ah/rules/<slot>.md` + `.ah/skills/<name>/`)、ah.toml 结构(`[master]`/`[agents.<id>]` 的 cmd/provider/skills/bundle 等声明面)、组合模型(kernel + bundle + (`.ah/rules/<slot>` 或 default) → 按 provider 落 `.claude/CLAUDE.md` / `.codex/AGENTS.md` / `.gemini/AGENTS.md`)、kernel 在二进制内不可改。
  - **ah-runtime-state** = ah 的「状态」面:`RuntimeSnapshot`(schema_version/runtime_state/ahd_alive/tmux_*_alive/sessions[]/agents[] 等)/ `session.status` 生命周期 / `ah events [--format json]` 逐行 JSON 形状 / 怎么读**权威**状态(用 `ah events` 结构化,别拿 `ah ps` 文本 + 另跑 tmux 拼)/ cleanup 语义。

## SCOPE(operator+用户已收紧,务必按此设计)
- 受众 = **外部集成 agent(主)+ master(次)**;**worker 一个都不给**(worker 是被动执行者,给它 ah 自我知识只稀释 context+糊角色)。两个 skill 都按 **MasterOnly** 声明,并跟 ah-commands 一致加「worker 拿不到」断言。
- **但主受众是外部 agent**(开发者自己的 claude / Studio 的 agent,**不在 ah 托管沙箱里**)。所以有个**新机制点必须在设计里回答**(见下第3问)。

## 你要产出/回答(4 点)
1. **结构决策(PM 要你论证,我最后拍)**:两个 skill(`ah-config` + `ah-runtime-state`)分开,还是合成一个 `ah-internals`?按**渐进式披露**权衡:
   - 分两个 = 两组聚焦触发词、各自正文小、注意力精准,但两个 skill 文件;
   - 合一个 = 一个宽触发词、正文体量大(配置+状态)、可能触发不准/正文稀释。
   给明确推荐 + 理由(可含「description 触发精度 vs 维护简单」的权衡)。
2. **触发词策略**:对你推荐的结构,给每个 skill 的 `description`(以「Use when…」触发句开头,和 ah-commands 定稿风格一致)。ah-config 要覆盖的意图:「ah 怎么配置 / 规则放哪 / ah.toml 怎么写 / 组合模型怎么落 provider」;ah-runtime-state 要覆盖:「怎么读 ah 运行态 / session·agent 状态 / RuntimeSnapshot JSON 形状 / 权威状态从哪来」。列触发词 + 论证不过度触发(剔除通用 config/status 泛词导致的误触发)。
3. **外部交付机制(新机制点,重头)**:主受众是**非 ah 托管**的外部 agent。现有 `materialize_builtin_skills` 只把内建 skill 装进 **ah 自己拉起的沙箱**。外部 agent(开发者本机的 claude/Studio agent)怎么拿到 ah-config/ah-runtime-state?给**方案选项 + 权衡**,至少覆盖:
   - (a) 新增 `ah` 子命令(如 `ah skills install/export`)把内建 skills 写进外部 agent 的真实 `~/.claude/skills`(或 codex/antigravity 对应);
   - (b) 随包/文档发(README 指引手动放);
   - (c) 别的(你提)。
   每个方案给:怎么工作、装到哪、更新/版本漂移怎么办、和「随二进制版本锁」的关系、对 ah 现有机制的侵入度。给推荐。**注**:codex 会并行核实 ah 现在有没有任何「往沙箱外写 skills」的机制;你先按「大概率没有、是新机制」设计,并标明依赖 codex 核实结论。
4. **关联标注(不做)**:Studio 那条「新增 `ah status --json` verb + 生命周期 FSM」是**另一条未立项轨**,本轮只文档化现有状态面,不新增 verb。在设计里标一句关联即可。

## 产出
落 `research/ah-internals-skill-design.md`,结构对应上面 4 点。不写实现码、不碰 git。
