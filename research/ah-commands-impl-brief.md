# Brief — ah-commands 内建 skill 试点(实施)

派单人 Master PM。执行:空闲 codex 实例。**实施任务**。设计依据 `research/ah-commands-skill-design.md`(operator 已审通过)。
分支:`feat/ah-commands-builtin-skill`(master 已建,主树串行)。

## 目标
建一套**通用「内建 skills 目录」机制**(`assets/builtin/skills/<name>/SKILL.md` → include_str → 物化进沙箱),并用 **ah-commands** 作试点先发(master-only)。机制要通用:以后加 ah-config / ah-runtime-state 只是**丢文件 + 加一条注册**,不再动机制。本轮**只发 ah-commands 一个 skill**,别顺手加别的。

## 铁律(worker 纪律)
- 只创建/改下列文件;别碰未跟踪;**别 push/commit**;grep-before-claim(尤其目标 skills 目录路径、现有 skill 物化写法);跑全量串行 cargo:
  `CCB_TEST_SKIP_REAL_PROVIDER=1 env -u AH_STATE_DIR -u CCBD_STATE_DIR CARGO_BUILD_JOBS=1 cargo test -- --test-threads=1`
- TDD 先红后绿,贴红灯输出。

## 交付文件
1. `assets/builtin/skills/ah-commands/SKILL.md`(内容见下,逐字创建)
2. `src/provider/builtin.rs`(加内建 skills 注册表)
3. `src/provider/home_layout.rs`(加 `materialize_builtin_skills` 写文件路径 + 三家 provider 路径按 role/scope 过滤调用)
4. `assets/builtin/master_kernel.md`(kernel 瘦身,见下)
5. 新测试文件 `tests/builtin_skills.rs`(断言见下)

## 文件1:`assets/builtin/skills/ah-commands/SKILL.md`(逐字)
```markdown
---
name: ah-commands
description: Authoritative CLI reference for 'ah' agent-facing orchestration commands (ah ps, ask, tell, pend, watch, logs, events, cancel, kill, attach, master ack-ready, prompt resolve). Use when you need to inspect agent or job status, dispatch tasks to worker agents, retrieve worker logs or outputs, cancel or kill tasks, attach to a tmux session, stream lifecycle events, resolve a blocked PROMPT_PENDING agent, or report master cutover readiness. Not for operational commands like start, stop, up, doctor, setup, config, or bundle.
---

# ah agent-facing commands

Authoritative reference for orchestrating through `ah`. The exact, current usage for any command is always available via `ah --help` and `ah <command> --help` — use that as the ground truth if anything here looks out of date. This skill covers only the agent-facing orchestration subset; operational commands (start / stop / up / doctor / setup / config / bundle) are intentionally excluded — the master orchestrates, it does not operate the daemon.

## Status inspection & monitoring
- `ah ps` — List sessions, agents, and pending evidence. See the running topology and spot a stuck or backed-up agent.
- `ah events [--format json]` — Stream runtime lifecycle snapshots as JSON lines. Watch state-machine transitions across the system.

## Dispatch & async communication
- `ah ask <agent_id> <text> [--wait] [--request-id <id>]` — Submit a task to a worker; returns a job id. Delegate a unit of work; add `--wait` to block until it finishes.
- `ah tell <target> <text> [--session <s>] [--request-id <id>]` — Deliver text to the master pane or an agent without blocking. Async notices/status where no reply is awaited.

## Result tracking & log retrieval
- `ah pend <job_id>` — Block until a submitted job finishes. Await an async `ah ask` before the next decision.
- `ah watch <agent_id> [--since-event-id <n>]` — Stream an agent's output events live. Follow a running agent.
- `ah logs <agent_id> [--since <n>]` — Print an agent's stored output. Read a finished or errored agent's full output at once.

## Runtime intervention & debugging
- `ah cancel <job_id>` — Cancel a queued or running job. When a dispatched task is stale, misparametrized, or no longer needed.
- `ah kill <target_id> [--session] [--force]` — Kill an agent, or a whole session with `--session`. Terminate an unresponsive agent or tear down a session.
- `ah attach <target> [subject] [--session <s>]` — Attach to an agent or master tmux session (`target` = master / agent / legacy id). A manual escape hatch for direct tmux inspection.

## Role handover & interactive resolution
- `ah master ack-ready [--cutover-id <id>]` — Report successor-master readiness to ahd. Run after loading the handoff during cutover, before claiming takeover.
- `ah prompt resolve <agent_id> [--action <a>] [--keys <k>] [--save-to-kb]` — Answer a worker blocked at an interactive prompt (PROMPT_PENDING). Unblock a hung worker by submitting its choice or input.
```

## 文件4:kernel 瘦身 `assets/builtin/master_kernel.md`(Orchestration Contract 段)
把枚举那句换成 skill 指针 + `--help` 兜底地基(operator 加的「永不悬空」):
```diff
 - Dispatch through ah with `ah ask <agent_id> "<task>" [--wait]`.
-- Read results and evidence through implemented ah commands such as `ah pend <job_id>`, `ah watch <agent_id>`, `ah logs <agent_id>`, `ah ps`, and `ah attach`.
+- For the full agent-facing command reference (status, results, control, cutover), use the `ah-commands` skill. The exact usage of any command is always available via `ah --help` and `ah <command> --help`.
 - Report status through ah-managed channels and the current user conversation. Do not invent unavailable ah subcommands.
```
只动这一行,别改 kernel 其它段(cutover ACK / safety boundary 不动)。worker_kernel.md 不动。

## 文件2+3:通用内建 skills 机制(实施判断,下面是设计意图,细节你 grep 现有惯例落地)
**注册表(builtin.rs)**——通用、可扩展,加 skill 只加一条数据:
```rust
pub enum BuiltinSkillScope { MasterOnly, AllAgents }
pub struct BuiltinSkill {
    pub name: &'static str,
    pub skill_md: &'static str,
    pub scope: BuiltinSkillScope,
}
pub const BUILTIN_SKILLS: &[BuiltinSkill] = &[
    BuiltinSkill {
        name: "ah-commands",
        skill_md: include_str!("../../assets/builtin/skills/ah-commands/SKILL.md"),
        scope: BuiltinSkillScope::MasterOnly,
    },
];
```
(命名/风格对齐 builtin.rs 现有 const;上面是形状,不必逐字。)

**物化(home_layout.rs)**:
- 新增 `materialize_builtin_skills(skills_dir: &Path, role: HomeLayoutRole)`:遍历 `BUILTIN_SKILLS`,scope 与 role 匹配的(`MasterOnly`→仅 `role==Master`;`AllAgents`→都要),把 `skill_md` **写入**(不是 symlink,内容在二进制)`<skills_dir>/<name>/SKILL.md`。`<skills_dir>` 用**和现有项目 skill 相同的目标目录**——先 grep `plan_claude_skill_materialization`/`plan_codex_skill_materialization` 及 antigravity 对应,确认 claude=`<claude_dir>/skills`、codex/antigravity 各自 skills 目录,内建 skill 落同一处,和项目 skill 并列。
- **三家 provider 路径都接线**(通用机制要求):`prepare_claude_overrides`(在 `materialize_claude_skills` 之后,line~224)、`prepare_codex_overrides`、`prepare_antigravity_overrides` 各调 `materialize_builtin_skills(<该provider skills_dir>, role)`。本轮唯一 skill 是 MasterOnly,故实际只有 master(claude)会落文件、worker 全被过滤——但接线三家,保证下轮 AllAgents skill 只加数据即可 fan out,不再动机制。

## 文件5:测试 `tests/builtin_skills.rs`(TDD,断言)
1. **master 拿到 ah-commands**:走真实 `prepare_home_layout_with_extensions_for_slot("claude", …, HomeLayoutRole::Master, "master", &ExtensionConfig::default(), …)`(参考 `tests/dev_scenario_template.rs` 的 EnvGuard/temp home 写法),断言 `<home>/.claude/skills/ah-commands/SKILL.md` 存在且含 "agent-facing" 与 "ah ps"。
2. **worker 没有 ah-commands**(关键回归断言):对 worker 角色(claude a4、codex、antigravity 各来一发)断言其沙箱 skills 目录**不存在** `ah-commands/SKILL.md`。
3. **无需 ah.toml 声明**:上面 master 用的是 `ExtensionConfig::default()`(skills 为空),仍拿到 ah-commands → 证明是内建无条件下发,不靠声明。
4. **kernel 瘦身**:断言 `builtin::MASTER_KERNEL` **不含** `such as` 那句的 `ah watch <agent_id>`, `ah logs <agent_id>`,`ah ps`, and `ah attach` 枚举;**含** `ah-commands` 与 `ah <command> --help`。
5. **SKILL.md 合法**:frontmatter 含 `name: ah-commands` 与 `description:`;description 含触发关键词(如 "agent" 与 "dispatch")。
6. **命令不漂移(轻)**:断言 SKILL.md 不含被排除的运维命令行(不出现 "ah start"/"ah setup"/"ah bundle" 作为命令项)。

## 回报
- 全部文件 diff stat;
- 完整全量串行 cargo 输出;
- 确认 worker 沙箱无 ah-commands 的断言通过(贴测试名+结果);
- 目标 skills 目录路径的 grep 证据(证明内建 skill 和项目 skill 落同一处)。
