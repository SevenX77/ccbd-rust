# Brief — ah-internals skills 锚点核实(纯调研,codex)

派单 Master PM。执行:codex 实例。**纯只读调研**:产出一份 grounded 锚点表,不改代码、不碰 git。给 ah-config / ah-runtime-state 两个内建 skill 的正文当权威素材来源。**每条结论必须带 file:line**;读不到标不可达,拿不准标待定+原因。

落盘 `research/ah-internals-anchors.md`。分五节:

## §1 ah-config 面:.ah/ 布局 + ah.toml 声明面
- `.ah/` 项目根布局:`ah.toml`、`.ah/rules/<slot>.md`、`.ah/skills/<name>/SKILL.md`、`.ah/bundles/<name>/`——各自被谁读(grep 消费点,给 file:line)。
- ah.toml 结构:`[master]`(字段:cmd/enabled/skills/bundle/... 有哪些?)、`[agents.<id>]`(provider/skills/bundle/...)、`[completion]`、`[master].mcp`?——**以 `src/cli/config.rs` 的真实 struct 字段为准**(master/agent config struct 定义行 + 每个字段)。
- rules/skills/mcp/bundle 的声明面到运行时的路径:`ExtensionConfig`(`src/provider/extensions.rs`)的字段;ah.toml→ExtensionConfig 怎么填(grep 填充点)。

## §2 组合模型(kernel + bundle + slot/default → provider 落点)
- `compose_rules_with_layers`(`src/provider/home_layout.rs:519` 起)的三段顺序:kernel + bundle_layers + (override 或 default);确认 override 替换 default 不是追加。
- provider 目标文件映射:claude→`.claude/CLAUDE.md`、codex→`.codex/AGENTS.md`、antigravity→`.gemini/AGENTS.md`(核实 `home_layout.rs:504-506` 或现值)。
- kernel/default 来源:`builtin.rs:3-6`(MASTER_KERNEL/WORKER_KERNEL/DEFAULT_MASTER/DEFAULT_WORKER,include_str,二进制内不可改)。
- 内建 skills 机制(已在 main):`BUILTIN_SKILLS` 注册表 + `materialize_builtin_skills`(写文件、按 role/scope、三家接线)——给 file:line(供 ah-config 正文引用「内建 skill 怎么下发」)。

## §3 ah-runtime-state 面:RuntimeSnapshot 全字段
- `RuntimeSnapshot` / `RuntimeSessionSnapshot` / `RuntimeAgentSnapshot`(`src/runtime_events.rs:49` 起)逐字段列出(名+类型+一句含义)。
- `RuntimeState`(`runtime_events.rs:23`:Active/Inactive/Starting/Degraded,serde snake_case)、`RuntimeSnapshotReason`(7 值)。
- **状态值域消歧(重要)**:grep 出的 IDLE/BUSY/DISPATCHED/DONE/FAILED/QUEUED/REVIEWED/VERIFYING/WAITING_FOR_ACK/CRASHED/ACTIVE/RUNNING 分别属于**谁**——`session.status`?`agent.state`?`agent.sub_state`?job status?各自定义/转移点在哪(给 file:line)。别混成一锅。

## §4 怎么读【权威】状态 + `ah events`/`ah ps` 形状
- `ah events [--format json]`:`cmd_events`(`src/bin/ah.rs:1310`)输出什么——是不是逐行 RuntimeSnapshot JSON?schema_version 字段?
- `ah ps`:`cmd_ps` 输出是文本表(核实),即「非权威、给人看」的那种。
- 结论素材:为什么应读 `ah events` 的 RuntimeSnapshot(结构化、含 ahd_alive/tmux_*_alive/各 snapshot)而不是 `ah ps` 文本 + 另跑 tmux 拼——把权威来源的证据钉出来。

## §5 cleanup 语义 + 外部交付可行性
- cleanup/reap 语义:session/agent 结束后状态怎么收(grep cleanup/reap/teardown/DONE→? 的转移),给 file:line。
- **外部交付可行性(为设计服务)**:grep ah 有没有**任何**把 skills/rules 写到「ah 托管沙箱之外」(如用户真实 `~/.claude/skills`)的机制或子命令?有没有 `ah skills`/export/install 类 verb?(`src/bin/ah.rs` 子命令全集里找)。ah 怎么定位一个外部 agent 的 home?——如实报「有/无」,这决定外部交付是不是新机制。

## 回报
落盘 `research/ah-internals-anchors.md`,五节齐全、条条 file:line。unreachable/待定 如实标。
