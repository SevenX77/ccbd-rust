# T4 brief — ahd --version / rpc EOF 诊断 / e2e tmux teardown (a1 codex)

派单人:Master PM(operator 点的三项)。你是 a1(codex)。**分支已切好 = `t4-diagnostics-teardown`(从 main 0a48e81),直接在上面干。**
三项同一分支。项1、项2 严格 TDD 红绿;项3 是结构性护栏(见下,别硬套 TDD)。根因线索见 `docs/reports/studio-open-in-handoff-2026-07-06.md` §T4。

---

## 项1 — ahd 不认 --version,会误拉 daemon(TDD)

**现状证据**:`src/bin/ahd.rs` 的 `main()`(约 L20 起)**完全不解析命令行参数**,一进来就 sandbox check → `env::resolve_state_dir()` → `db::init` → 起 daemon。
所以 `ahd --version` 会忽略该参数、直接在 default state dir 起一个真 daemon(危险)。

**要做的**:在 `main()` **最开头、任何 daemon 副作用之前**先处理 CLI 参数:
- `--version` / `-V` → 打印版本(`env!("CARGO_PKG_VERSION")`)→ 退 0。
- `--help` / `-h` → 打印一行用法 → 退 0。
- 任何未知 flag(以 `-` 开头且不认识)→ 打印错误到 stderr → 退非 0(如 2)。**绝不落进起 daemon 的路径。**
- 无参数(正常 daemon 启动)→ 照旧走 daemon。

**可测化落法(重点)**:别把逻辑埋进 `main()`(binary 的 main 难单测)。抽一个纯函数做分类,例如:
```rust
enum AhdCliAction { RunDaemon, PrintVersion, PrintHelp, UnknownFlag(String) }
fn classify_ahd_args(args: &[String]) -> AhdCliAction { ... }  // args 不含 argv[0]
```
`main()` 开头:`match classify_ahd_args(&std::env::args().skip(1).collect::<Vec<_>>()) { RunDaemon => {/*现有逻辑*/}, PrintVersion => {println!("{}", VERSION); return SUCCESS} , ... }`。
**单测直接测 `classify_ahd_args`**:`["--version"]→PrintVersion`、`["-V"]→PrintVersion`、`["--help"]→PrintHelp`、`["--bogus"]→UnknownFlag`、`[]→RunDaemon`。
返回 `RunDaemon` 之外的任何分支都保证 `main` 早退、不碰 daemon —— 这就满足了「--version 不启 daemon」。

---

## 项2 — daemon 半途死掉时 RPC 空响应不可诊断(TDD)

**现状证据**:`src/cli/rpc_client.rs`:
- `rpc_call`(L137)在 L174-175:`stream.read_to_string(&mut raw)?;` 然后 `let response: Value = serde_json::from_str(raw.trim())?;`。
- daemon 若在回复前关闭连接(被 stop/重启),`read_to_string` 得到 **0 字节**,`from_str("")` → 走 `From<serde_json::Error>` → `CliError::InvalidJson`(L56-60),Display 成 `invalid JSON response from daemon: EOF while parsing...`(L42),用户无法定位。
- `CliError` 枚举在 L17-25;`exit_code()` 在 L95-102。

**要做的**:
1. 加一个**专门的**错误变体(别混进 InvalidJson),如 `CliError::DaemonClosedConnection`(无字段或带 socket PathBuf 均可)。Display 文案类似:
   `daemon closed the connection without replying (it may have been stopped or restarted); check the ahd service logs (journalctl --user -u <ahd unit>)`。
2. **可测化落法**:把响应解析抽成纯函数:
   ```rust
   fn parse_rpc_response(raw: &str) -> Result<Value, CliError> {
       let trimmed = raw.trim();
       if trimmed.is_empty() { return Err(CliError::DaemonClosedConnection); }
       serde_json::from_str(trimmed).map_err(CliError::InvalidJson)
   }
   ```
   `rpc_call` L175 改成 `let response = parse_rpc_response(&raw)?;`。
3. `exit_code()` 给新变体加映射:归到 daemon 连接类(和 `DaemonNotAccepting` 一致 = **1**;理由:daemon 走掉了,属连接层,不是协议层的 3)。**这条改了观测到的 exit code(旧 EOF 走 3),在回执里显式说明,让 a2/PM 确认。**

**单测**:测 `parse_rpc_response`:`""`/`"  \n"` → `DaemonClosedConnection`;`"not json"` → `InvalidJson`;合法 JSON → `Ok(value)`。可选再加一条 Display 断言,确认新文案含 "closed the connection" 和 "journalctl"。

---

## 项3 — e2e teardown 护栏(结构性,别硬套 TDD;**范围钉死**)

**背景**:测试泄漏了 ~179 个 tmux server(fixture 名 agent_w1 / master_p1 / p_rearm_* 等),没收。活体清理 operator 自己做,**你只做防再泄的最小护栏**。

**现状证据**:`tests/common/mod.rs` **已有** `TmuxServerGuard`(L51-92),`impl Drop`(L85-91)会 `tmux -L <socket> kill-server` + 删 socket 文件。走这个 guard 的测试是收干净的。
所以泄漏源是**没走这个 guard** 的地方:要么某些测试直接 `TmuxServer::new(...)` 起了 server 不带 guard;要么起真 `ahd` 二进制的 e2e(daemon 自己经 `new_with_daemon_unit` 建了 tmux server),其 fixture 收尾没 kill 掉那个 server。

**要做的(最小护栏,二选一或都做,取决于泄漏源)**:
1. 先**定位泄漏源**:grep 出哪些测试/fixture 起了 tmux server 却没经 `TmuxServerGuard`(或起真 ahd daemon 却没在 teardown kill 掉其 tmux server)。把定位结论写进回执。
2. 对定位到的源,加**最小** teardown:优先把它们**改走已有的 `TmuxServerGuard`**;若是真-ahd-daemon fixture,给该 fixture 的收尾加一个 Drop 或显式 `tmux -L <sock> kill-server`(镜像 TmuxServerGuard 的做法)。目标:**该测试跑完不留 ahd-* tmux server**。
3. 若泄漏源其实是 runbook 驱动的手动 e2e(不是自动 fixture),那就在对应 runbook 里补一条明确 teardown 步骤(kill 自己起的 tmux server)。

**范围红线(operator 钉死)**:**别重构测试框架**,只做最小护栏。
**如果你发现泄漏源分散、要动的地方超过「给现有 fixture 加个 Drop / 改走 guard」这种最小改动量 —— 停,把定位结论回给我(master),别自行扩大。** 项1+项2 可以先独立成型,项3 拿不准就报。

**验证**(项3 无需 TDD):说明你怎么确认护栏生效(例如:跑相关 e2e 前后 `tmux -L <sock> ls` / 确认无残留 ahd-* server;或断言 guard 的 Drop 被走到)。

---

## 纪律(钉死)

- 就在 `t4-diagnostics-teardown` 上干(从 main 0a48e81),别切别的、别在 main 改。
- 串行 cargo:`env -u AH_STATE_DIR CARGO_BUILD_JOBS=1 cargo test -- --test-threads=1`,跑**完整** cargo test,贴全量结果。项1/项2 先红后绿(贴红灯输出)。
- 改动范围:项1 `src/bin/ahd.rs`;项2 `src/cli/rpc_client.rs`;项3 `tests/`(+ 必要 runbook md)。**不要动** T1/T2 的文件(runtime_events.rs / agent.rs / home_layout.rs)、不要碰未跟踪的 .kiro/research、不 reset/discard 任何东西。
- 若全量里 `mvp11_real_*` 又瞬时红:既有 real-provider flake(前两轮已 baseline 证伪),单跑确认+说明与本改动面无耦合即可,别口头「无关」。
- **别 push**。回执给我:三项各自改动摘要 + 失败→通过测试输出 + `git diff --stat` + 项2 exit-code 变更的显式说明 + 项3 泄漏源定位结论。有拿不准(尤其项3 范围)先问我。
