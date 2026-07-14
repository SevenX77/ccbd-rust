# Fix Round 1 — ah-commands 内建 skill(合并 a2+a4 两审)

派单 Master PM。执行:空闲 codex 实例。一轮合并修复。
分支 `feat/ah-commands-builtin-skill`(已在此分支主树)。
铁律:只改下列文件、别碰未跟踪、别 push/commit、grep 核实、跑全量串行 cargo、TDD 先红后绿。

改动文件:`src/provider/home_layout.rs`、`src/provider/builtin.rs`(如需暴露 reserved 名单)、`assets/builtin/skills/ah-commands/SKILL.md`、`tests/builtin_skills.rs`;若在配置期加校验则含 `src/cli/config.rs`。**不改** master_kernel.md(已定稿)。

## FX1 [must-fix] 内建 skill 名做保留名,冲突 fail-loud(别静默覆盖)
现状 `materialize_builtin_skills`(home_layout.rs:783-791)在项目/bundle skill 物化后**静默删除同名路径**,会 clobber 用户显式声明的同名 `ah-commands`。与既有 bundle 冲突语义不一致(`src/provider/bundles.rs:309-313` 对同名是 **REJECT**)。
修法(对齐既有语义):
- **在解析/物化项目(和 bundle)skill 之前**加**保留名检查**:若任何 resolved 项目/bundle skill 的 name ∈ `BUILTIN_SKILLS` 的 name 集合 → 返回清晰 `CcbdError`,消息点名保留名,如 `skill name "ah-commands" is reserved by an ah builtin skill; rename the project skill`。
- 检查放在 skill 名可得、且早于 `materialize_builtin_skills` 的位置(grep `resolve_skills`/`resolve_project_skills` 调用点;三家 provider 路径都要覆盖,建议抽一个 helper 复用,别三处重复)。
- **保留 `materialize_builtin_skills` 对自身 prior write 的幂等覆盖**(重跑 prep 要幂等);因为上游已拒绝同名项目 skill,这里只会覆盖自己上轮写的内建目录,安全。
- 可选加强:在 `src/cli/config.rs` 配置校验期也加同款保留名检查(早失败,镜像 bundle 校验风格);做不做由你判断可落地性,runtime 那道是必须。

## FX2 [nice→收] 三家 provider 的 master-role 物化都测
`tests/builtin_skills.rs` 现只证 claude master 拿到。机制接了三家(home_layout.rs:225/293/971)。补:对 `("claude","master")`、`("codex","master")`、`("antigravity","master")` 都断言 `<home>/<各自skills路径>/ah-commands/SKILL.md` 存在(路径映射:claude=`.claude/skills`、codex=`.codex/skills`、antigravity=`.gemini/config/skills`)。锁「通用机制对三家一致」。

## FX3 [nice→收] 断言 skill 目录本身非 symlink
现 `master_gets_...`(tests:99-103)只对 SKILL.md 断言非 symlink。补断言 skill **目录** `<...>/skills/ah-commands` 本身 `symlink_metadata().file_type().is_symlink() == false`。

## FX4+FX5 [nice→收] description 改写(触发力 + 写 skill 惯例)
把 `assets/builtin/skills/ah-commands/SKILL.md` frontmatter 的 `description` 改成**以「Use when…」触发句开头**,并补 **pend/watch 意图**,保留命令 token 与负向 scope。目标文本(可微调,别超 1024 字符、保留 name+description 两字段):
```
description: Use when you need to inspect agent or job status, dispatch a task to a worker agent, wait for a dispatched job to finish, follow or retrieve a running agent's output, cancel or kill tasks, attach to a tmux session, stream lifecycle events, resolve a blocked PROMPT_PENDING agent, or report master cutover readiness. Authoritative CLI reference for 'ah' agent-facing orchestration commands (ah ps, ask, tell, pend, watch, logs, events, cancel, kill, attach, master ack-ready, prompt resolve). Not for operational commands like start, stop, up, doctor, setup, config, or bundle.
```
正文(SKILL.md:6-8 那段「Authoritative reference…」identity 框架)保持不动。改完确认 `ah_commands_skill_frontmatter_and_scope_are_valid` 仍绿(它断言含 "agent"/"dispatch"/name/description、不含 "ah start/setup/bundle")。

## FX6 [nice→收] 测试断言 kernel 保留两个 must-not-dangle 字面量
`master_kernel_points_...`(tests:123)现只断言删了枚举、加了指针。补正向断言:`builtin::MASTER_KERNEL` **仍含** `ah master ack-ready` 与 `` `ah ask ``(dispatch 那句)。防未来 over-slim 删掉使「永不悬空」成立的兜底锚。

## 回报
- 全部改动 diff stat + 完整全量串行 cargo 输出;
- FX1:贴保留名冲突报错的测试(项目声明 `.ah/skills/ah-commands` → prepare 报错)结果 + 幂等重跑仍绿的说明;
- FX2/FX3/FX6 新断言测试名 + 结果;
- description 改后 frontmatter 测试仍绿。
