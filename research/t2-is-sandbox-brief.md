# T2 brief — worker root 逃生口下沉到 claude provider 层 (a1 codex)

派单人:Master PM。你是 a1(codex)。**分支已给你切好 = `t2-worker-is-sandbox`(从 main 0a48e81 起)**,直接在上面干,别再切分支。
根因见 `docs/reports/studio-open-in-handoff-2026-07-06.md` §T2。

## 目标(一句话)

worker 以 `claude --dangerously-skip-permissions` 在 WSL/root HOME 下会被 claude CLI 拒跑秒死;要靠 `IS_SANDBOX=1` 放行。
现在这个环境变量是 Studio 在模板里手写 `[env] IS_SANDBOX="1"` 兜的(临时方案,要废止)。
**本条把这个知识下沉到 ah 的 claude provider spawn 层**:当 provider=claude、带 `--dangerously-skip-permissions`、且 worker 的 HOME 指向 ah 自管 sandbox 时,由 ah 自己注入 `IS_SANDBOX=1`。

## 现状 ground-truth(已侦察,file:line 精确,别再全仓找)

- worker spawn 的 env 组装在 `src/rpc/handlers/agent.rs`。**只有一个** env builder(非 per-provider):
  `build_agent_spawn_env_vars_for_hook_push`(`agent.rs:429`,只塞 `CCB_SOCKET`)。
- spawn handler 内的关键顺序:
  - `agent.rs:140` `let mut spawn_env_vars = build_agent_spawn_env_vars_for_hook_push(...)`
  - `agent.rs:145` `if manifest.requires_home_materialization {` … 块内 `:153-161`
    `prepare_home_layout_with_extensions_for_slot(...)` 返回 `home_overrides`(类型 `HomeOverrides`,结构体 `home_layout.rs:29-32`,带 `home_root: PathBuf` + `extra_env`)。
  - `agent.rs:166` `spawn_env_vars.extend(home_overrides.extra_env);` —— **worker 的 HOME 就是在这一步进 spawn_env_vars 的**(HOME 来自 `home_env`,`home_layout.rs:1614`)。
  - `agent.rs:170-183` 之后把 `&spawn_env_vars` 传给 `wrap_command_with_recovery_and_sandbox_overrides(...)`。
- `--dangerously-skip-permissions` **不在 agent.rs 里加**,来自 manifest.command:claude 是 `["claude","--dangerously-skip-permissions"]`(`manifest.rs:391`)。
  provider 名从 `manifest.provider_name` 拿(`agent.rs:149` 一带可见)。合法 provider:`["bash","codex","claude","antigravity"]`(`manifest.rs:326`)。
- sandbox HOME 判据**已有现成谓词**:`is_ccb_sandbox_home(path)`(`home_layout.rs:1579`),匹配 `"/.cache/ah/sandboxes/"` 或旧 `"/.cache/ccb/sandboxes/"`。
  **现在是模块私有 `fn`**,你需要把它提到 `pub(crate)` 才能在 agent.rs 用(或在 agent.rs 内复用等价判断——优先复用这个谓词,别另写一份字符串匹配)。
- 全仓 **无任何 `IS_SANDBOX` 字样**(grep 为证),你是第一处引入。ENV_PASSTHROUGH 白名单在 `manifest.rs:242`(不含 IS_SANDBOX,也不该往那加——那是 host 透传,不是我们要的条件注入)。
  `CLAUDE_INJECTED_ENV`(`manifest.rs:301`)是**无条件**静态表,表达不了「HOME 是 sandbox 才注入」这种条件,所以别走 manifest 那条路。

## 落点与实现(PM 点名:就落在 agent.rs spawn env 组装)

在 `agent.rs` 的 `if manifest.requires_home_materialization { … }` 块内、`spawn_env_vars.extend(home_overrides.extra_env)` **之后**(此时 HOME/home_root 已知),做条件注入:

判定条件(三者皆满足才注入 `IS_SANDBOX=1`):
1. `manifest.provider_name == "claude"`;
2. manifest.command 带 `--dangerously-skip-permissions`(即 `manifest.command.iter().any(|a| *a == "--dangerously-skip-permissions")`——别硬编只判 provider 名,带上这条更稳:哪天 claude manifest 去掉该 flag 就不该再注入);
3. worker HOME 指向 ah sandbox:`is_ccb_sandbox_home(&home_overrides.home_root)` == true。

注意 `home_overrides.extra_env` 被 `extend` **move** 掉后,`home_overrides.home_root` 仍可读(部分移动);稳妥起见可在 extend 前把 `home_root` clone 进一个 local 再用。

**为了可单测**:把这三条判定抽成一个纯函数,例如
`fn should_inject_is_sandbox(provider_name: &str, command: &[&str], home_root: &Path) -> bool`(放 agent.rs 或就近),
注入处调用它:`if should_inject_is_sandbox(...) { spawn_env_vars.insert("IS_SANDBOX".into(), "1".into()); }`。
这样测试直接测这个纯函数,不用起整套 spawn。

## 测试(TDD 红绿,断言 spawn env / 判定)

参考现有 spawn-env 断言风格:`agent.rs:531` `hook_push_worker_spawn_env_injects_deterministic_ccb_socket`(直接测 builder 对 HashMap 的效果),
以及 `src/sandbox/systemd.rs` 里 `cmd.contains(&"KEY=VALUE".to_string())` 那类。至少覆盖:
1. claude + 带 `--dangerously-skip-permissions` + HOME=`.../.cache/ah/sandboxes/<hex>` → `should_inject_is_sandbox == true`(且若能走到 env 组装,断言 spawn env 里有 `IS_SANDBOX=1`)。
2. claude + **非 sandbox** HOME(如 `/home/user`)→ false,不注入。
3. **非 claude**(codex,manifest 命令是别的 flag)+ sandbox HOME → false,不注入。
4.(可选)provider=claude 但 command 里没有 `--dangerously-skip-permissions`(构造一个)→ false。

## 落地后(你回执里带上,PM 来做)

实现完提醒我:要发 issue/PR 给 **agent-harness** 删掉模板里那份 `[env] IS_SANDBOX="1"`(agent-harness #457 加的),别静默留双源。这步归 PM,你只需在回执里提示。

## 纪律(钉死)

- **就在已切好的 `t2-worker-is-sandbox` 分支上干**(从 main 0a48e81),别切别的、别在 main 改。
- 串行 cargo:`env -u AH_STATE_DIR CARGO_BUILD_JOBS=1 cargo test -- --test-threads=1`,跑**完整** cargo test,贴全量结果。先红后绿。
- 只改 `src/rpc/handlers/agent.rs` +(把谓词提 pub 的)`src/provider/home_layout.rs`;必要时 CHANGELOG 补一条。**不要动** T1 的 runtime_events.rs、不要碰未跟踪的 .kiro/research、不 reset/discard 任何东西。
- 若全量里 `mvp11_real_*` 又瞬时红:那是既有 real-provider flake(T1 轮已 baseline 证伪),但你仍要单跑确认、并说明它和本改动面(agent.rs env 注入)无耦合;别口头「无关」,要给证据。
- **别 push**。回执给我:改动摘要、失败→通过测试输出、`git diff --stat`、以及上面「提醒 PM 发 agent-harness issue」那句。

有判定/落点疑问先问我,别自行扩大范围(比如别顺手改 codex/antigravity 的注入、别动 manifest 的静态注入表)。
