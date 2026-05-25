# Agent 沙箱隔离实施任务

## 任务清单

- [ ] 1. [BREAKING] Rust bwrap 二进制挂载改为 provider 精确白名单
  - 涉及文件：`src/sandbox/bwrap.rs:66-70`, `src/sandbox/bwrap.rs:129-148`, `src/sandbox/bwrap.rs:564-626`
  - 依赖：无
  - Facet：配置隔离、dispatch 切断
  - tests-first：先改 `test_build_args_binds_materialized_home_for_home_aware_manifest`，让它断言 materialized home 只出现 provider/runtime 白名单挂载，且不再出现 `.local/bin`、`.claude`、`.codex`、`.gemini`、`.claude.json` 这些宿主同路径整目录/整文件 bind；为 Claude、Codex、Gemini 各新增一条参数构造测试。
  - 实现：把 `push_provider_binary_path_binds(args)` 改成接收 `manifest.provider_name`，在 `build_args` 的 materialized home 分支按 provider 调用；删除当前 `for relative in [".npm-global", ".local/bin", ".local/share/claude", ".codex", ".gemini", ".claude", ".claude.json"]` 的全量挂载。
  - 实现：Claude 只允许挂载 `~/.local/bin/claude` 入口文件和其真实运行目录 `~/.local/share/claude` 或 `readlink -f ~/.local/bin/claude` 所在版本目录；Codex/Gemini 只允许挂载 `~/.npm-global` 以及系统 `node` 可执行路径，禁止把含 `ccb`/`ask` 的 `~/.local/bin` 暴露给 sandbox。
  - cutover discipline：同一 PR 必须同步改 `src/sandbox/bwrap.rs:564-626` 的旧整目录断言；集成验证脚本覆盖 V1/V2：agent 内 `command -v ccb`、`command -v ask` 失败，绝对路径 `/home/sevenx/.local/bin/ccb`、`/home/sevenx/.local/bin/ask` 不可执行。

- [ ] 2. [BREAKING] Rust spawn 环境改为 caller 注入 + control-plane 黑名单
  - 涉及文件：`src/provider/manifest.rs:50-104`, `src/provider/manifest.rs:233-251`, `src/provider/manifest.rs:355-372`, `src/sandbox/bwrap.rs:242-251`
  - 依赖：无
  - Facet：配置隔离、dispatch 切断
  - tests-first：先在 `src/provider/manifest.rs:355-372` 旁新增 failing test：设置 `CCB_CALLER_ACTOR=a1`、`CCB_TMUX_SOCKET=/tmp/tmux`、`CCB_TMUX_SOCKET_PATH=/tmp/tmux-path`、`CCB_KEEPER_PID=1`、`CCB_MASTER_CLAUDE_PID=2`、`PATH=/home/sevenx/.local/bin:/usr/bin` 后调用 `collect_spawn_env`，断言 `CCB_CALLER_ACTOR` 存在，四个 control-plane 变量不存在。
  - 实现：在 `ENV_PASSTHROUGH` 加入 `CCB_CALLER_ACTOR`；从透传集合剔除 `CCB_TMUX_SOCKET`、`CCB_TMUX_SOCKET_PATH`、`CCB_KEEPER_PID`、`CCB_MASTER_CLAUDE_PID`。
  - 实现：新增 `ENV_BLACKLIST` 或等价过滤函数，在 `collect_spawn_env` 收集宿主环境时统一过滤黑名单；保持 injected env 和 extra env 的覆盖顺序，但不得让黑名单通过 extra env 漏回 agent。
  - cutover discipline：同一 PR 必须同步改 manifest 单测；如果保留 `PATH` 透传，必须在任务 3 同 PR 增加安全 PATH 重建测试，不能只改业务代码。

- [ ] 3. [BREAKING] Rust sandbox PATH 重建为 provider-only PATH
  - 涉及文件：`src/provider/manifest.rs:89`, `src/provider/manifest.rs:233-251`, `src/sandbox/bwrap.rs:90-97`, `src/sandbox/bwrap.rs:242-259`
  - 依赖：任务 1、任务 2
  - Facet：dispatch 切断、免权限可靠性
  - tests-first：先新增单测，构造宿主 `PATH=/home/sevenx/.local/bin:/usr/local/bin:/usr/bin`，断言 spawn env 中的 `PATH` 不含 `/home/sevenx/.local/bin`，且含 sandbox provider bin 目录和安全系统路径。
  - 实现：不要直接透传宿主 `PATH`；在 materialized home 分支通过 `push_home_override_env` 或 manifest env 收敛出固定 `PATH=/home/agent/.local/bin-agent:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`。
  - 实现：配合任务 1 在 `/home/agent/.local/bin-agent` 只暴露 `claude`、`codex`、`gemini` provider 入口；不得把 `ccb`、`ask`、`autonew`、`ctx-transfer`、`claude-ccb-orchestrator` 放入 PATH。
  - cutover discipline：同一 PR 必须同步 bwrap 参数测试和 manifest env 测试；集成验证覆盖 V1：`ccb`/`ask` command not found，同时 provider 命令仍能启动。

- [ ] 4. [BREAKING] Rust Gemini home materialization 停止复制宿主配置和状态
  - 涉及文件：`src/provider/home_layout.rs:79-98`, `src/provider/home_layout.rs:182-230`, `src/provider/home_layout.rs:617-645`
  - 依赖：无
  - Facet：配置隔离
  - tests-first：先改 `test_gemini_overrides_creates_state_and_settings_with_auth`：在 source 写入 `.gemini/settings.json`、`.gemini/trustedFolders.json`、`.gemini/state.json` 的宿主专属字段，断言 target 不包含这些字段，只包含 agent-local minimal `security.auth.selectedType=oauth-personal`、workspace trust 和默认 state。
  - 实现：`prepare_gemini_overrides` 不再调用会复制/合并宿主内容的逻辑；删除或改写 `materialize_gemini_settings` 中 `fs::copy(source_settings, layout.settings_path)`；删除或改写 `materialize_trusted_folders` 对 source trustedFolders 的读取合并；删除或改写 `materialize_gemini_state` 对 source state 的复制。
  - 实现：保留 agent-local 文件生成：`settings.json` 只写 Gemini 启动必要 auth selectedType，`trustedFolders.json` 只写 `/home/agent` 的 `TRUST_FOLDER`，`state.json` 只写默认空/最小状态。
  - cutover discipline：同一 PR 必须同步 `src/provider/home_layout.rs:617-645` 测试；集成验证覆盖 V3：agent 中读取宿主 `.gemini/settings.json`、`trustedFolders.json`、`state.json` 的私有字段失败。

- [ ] 5. Rust ccbd 服务端 gate 拒绝 Worker 派单
  - 涉及文件：`src/rpc/handlers.rs:504-536`, `src/rpc/handlers.rs:1831-1929`, `src/error.rs:1-120`, `src/rpc/router.rs:67-96`
  - 依赖：任务 2
  - Facet：dispatch 切断
  - tests-first：先在 `src/rpc/handlers.rs:1831-1929` 附近新增 failing test：设置进程环境 `CCB_CALLER_ACTOR=a1` 后调用 `handle_job_submit`，断言返回权限类错误且数据库没有插入 job；再加 master/空 caller 的正向测试确保现有 `test_handle_job_submit_queues_job` 仍通过。
  - 实现：在 `handle_job_submit` 解析参数前或插入 job 前读取 `std::env::var("CCB_CALLER_ACTOR")`；当值是 worker 标识时直接拒绝。worker 判断至少覆盖 `a1`、`a2`、`a3` 和通用 `worker`，避免只写死一个 agent。
  - 实现：新增明确错误类型或复用现有 `IpcInvalidRequest`/权限错误，错误消息包含 “worker dispatch forbidden” 一类可测试文本。
  - cutover discipline：同一 PR 必须同步 RPC handler 单测；集成验证覆盖：在带 `CCB_CALLER_ACTOR=a1` 的环境运行 `ccb-rust ask a2 test` 应被 daemon 拒绝，不依赖 PATH 是否隐藏。

- [ ] 6. Python sandbox home 白名单保持纯鉴权，移除配置属性项
  - 涉及文件：`/home/sevenx/.local/share/codex-dual/lib/launcher/sandbox_home.py:9-31`, `/home/sevenx/.local/share/codex-dual/lib/launcher/sandbox_home.py:38-45`, `/home/sevenx/.local/share/codex-dual/lib/launcher/sandbox_home.py:100-106`
  - 依赖：无
  - Facet：配置隔离
  - tests-first：先补 Python 单测，断言 `whitelist_symlinks()` 只包含 `.ssh`、`.gitconfig`、`.git-credentials`、`.netrc`、`.claude.json`、`.claude/.credentials.json`、`.codex/auth.json`、`.codex/installation_id`、`.gemini/oauth_creds.json`、`.gemini/google_accounts.json`、`.gemini/installation_id`，且不包含 `.codex/config.toml`、`.codex/skills`、`.codex/commands`、`.gemini/settings.json`、`.gemini/trustedFolders.json`、`.gemini/state.json`、`.claude/settings.json`、`CLAUDE.md`、`CODEX.md`、`GEMINI.md`。
  - 实现：收紧 `PROVIDER_AUTH_WHITELIST`，只保留 design 允许的鉴权/身份文件；如果 provider-specific home 投影需要配置文件，必须由 launcher 写入 sandbox-local 文件，不能 symlink 宿主配置。
  - cutover discipline：同一 PR 必须同步 sandbox_home 单测；集成验证覆盖 V3：agent home 下主控配置文档和 settings/trusted/state 不存在或不是宿主 symlink。

- [ ] 7. Python provider 启动 PATH 改为 safe PATH + provider override 拦截
  - 涉及文件：`/home/sevenx/.local/share/codex-dual/lib/provider_core/runtime_shared.py:7-41`, `/home/sevenx/.local/share/codex-dual/lib/provider_backends/claude/launcher_runtime/service.py:48-68`, `/home/sevenx/.local/share/codex-dual/lib/provider_backends/codex/launcher_runtime/command_runtime/service.py:32-46`, `/home/sevenx/.local/share/codex-dual/lib/provider_backends/gemini/launcher_runtime/service.py:61-68`
  - 依赖：任务 6
  - Facet：dispatch 切断
  - tests-first：先补 `runtime_shared` 单测：给定宿主 PATH 含 `/home/sevenx/.local/bin` 且该目录可找到 `ccb`/`ask` 时，safe PATH 过滤掉该目录；设置 `CLAUDE_START_CMD=/home/sevenx/.local/bin/ccb` 时，`provider_start_parts("claude")` 或新校验函数拒绝。
  - 实现：在统一 helper 中构造 safe PATH：`<provider-virtual-bin>` 加过滤后的系统 PATH；过滤规则至少移除 `/home/sevenx/.local/bin` 和任何能解析到 dispatch CLI 的目录。
  - 实现：Claude/Codex/Gemini 的 env prefix 都导出安全 PATH；禁止 `*_START_CMD` override 指向 `ccb`、`ask`、`autonew`、`ctx-transfer` 或含 dispatch CLI 的 wrapper。
  - cutover discipline：同一 PR 必须同步三 provider start command 测试；集成验证覆盖 V1/V2：Python agent 内 PATH 查找不到 dispatch CLI，绝对路径绕过由任务 9 的 gate 拦截。

- [ ] 8. [BREAKING] Python 免权限 flag 变为 launcher invariant
  - 涉及文件：`/home/sevenx/.local/share/codex-dual/lib/provider_backends/claude/launcher_runtime/service.py:55-63`, `/home/sevenx/.local/share/codex-dual/lib/provider_backends/codex/launcher_runtime/command_runtime/service.py:55-74`, `/home/sevenx/.local/share/codex-dual/lib/provider_backends/gemini/launcher_runtime/service.py:50-55`, `/home/sevenx/.local/share/codex-dual/lib/ccbd/socket_client_runtime/endpoints.py:61-70`, `/home/sevenx/.local/share/codex-dual/lib/ccbd/app_runtime/policy.py:27-55`
  - 依赖：无
  - Facet：免权限可靠性
  - tests-first：先补三 provider command 单测：无论 `command.auto_permission` 是 `False`、socket payload 缺省还是 restore/recovery 路径，Claude 命令都含 `--dangerously-skip-permissions`，Codex 命令都含 `--dangerously-bypass-approvals-and-sandbox`、`approval_policy="never"`、`sandbox_mode="danger-full-access"`，Gemini 命令都含 `--yolo`。
  - 实现：删除 `if command.auto_permission` 条件追加，改为 Worker launcher 固定追加免权限 flag；`auto_permission` 可保留为兼容字段，但不得控制 provider worker 启动命令。
  - 实现：处理 runtime 复用风险：如果现有 session 的 `start_cmd` 缺少 invariant flag，启动/恢复流程必须重建 runtime 或拒绝复用并提示重启，不能 attach 到旧的无免权进程。
  - cutover discipline：同一 PR 必须同步三 provider launcher 单测和复用路径测试；验证脚本检查 `.ccb/.*-session` 中记录的 `start_cmd` 均包含对应免权 flag。

- [ ] 9. Python ccbd 服务端 gate 拒绝 Worker 派单和启动调度
  - 涉及文件：`/home/sevenx/.local/share/codex-dual/lib/ccbd/handlers/submit.py:6-20`, `/home/sevenx/.local/share/codex-dual/lib/ccbd/handlers/start.py:4-20`, `/home/sevenx/.local/share/codex-dual/lib/cli/ask_sender.py:38-53`, `/home/sevenx/.local/share/codex-dual/lib/provider_core/caller_env.py:7-12`
  - 依赖：无
  - Facet：dispatch 切断
  - tests-first：先补 handler 单测：环境或 payload 中 `CCB_CALLER_ACTOR=a1`/`worker` 时，`build_submit_handler(...).handle(...)` 不调用 `dispatcher.submit` 并返回/抛出权限错误；`build_start_handler(...).handle(...)` 同样拒绝 Worker 从 agent 内启动或扩容。
  - 实现：新增统一 `caller_actor` helper，优先读取 payload 里的 caller 字段，其次读取 `os.environ["CCB_CALLER_ACTOR"]`；worker 标识覆盖 `a1`、`a2`、`a3`、`worker`。
  - 实现：在 `submit` handler 创建 `MessageEnvelope` 之前拦截；在 `start` handler 调 `app.runtime_supervisor.start` 和 `app.persist_start_policy` 之前拦截。这是 Python 无 mount namespace 时的硬闸门。
  - cutover discipline：同一 PR 必须同步 submit/start handler 单测；集成验证覆盖：即使 agent 通过绝对路径执行 `/home/sevenx/.local/bin/ccb ask ...`，daemon 也拒绝。

- [ ] 10. Worker System Prompt 注入角色约束
  - 涉及文件：`src/provider/manifest.rs:106-121`, `src/rpc/handlers.rs:504-536`, `/home/sevenx/.local/share/codex-dual/lib/provider_core/caller_env.py:7-12`, 三 provider launcher env prefix 文件
  - 依赖：任务 2、任务 9
  - Facet：dispatch 切断
  - tests-first：先补 Rust/Python 启动命令或 env 构造测试，断言 Worker 进程可见统一身份提示文本或环境变量，且 master/daemon 自身不被误标为 worker。
  - 实现：按 design §4 注入 Worker 角色描述：agent 是隔离沙箱内 Worker，不能访问 CCB 或其他 Agent，拆分与跨 Agent 协作由主控负责。可通过 provider-specific system prompt、env 或启动前注入文件实现，但三 provider 行为必须一致。
  - cutover discipline：同一 PR 必须同步三 provider 的 prompt/env 测试；验收覆盖 V8：面对诱导派单指令，agent 明确表示不能调度。

- [ ] 11. 双实现端到端验收脚本
  - 涉及文件：新增 `scripts/verify_agent_sandbox_isolation.sh` 或现有集成测试目录；Rust 入口 `src/bin/ccb-rust.rs:383-405`；Python 入口 `/home/sevenx/.local/share/codex-dual/lib/cli/phase2_runtime/handlers_ask.py:4-35`
  - 依赖：任务 1-10
  - Facet：配置隔离、dispatch 切断、免权限可靠性
  - tests-first：先写验证脚本骨架并让它在当前代码上失败，失败点至少包括 dispatch CLI 可见、敏感 env 泄漏、Python gate 缺失或免权 flag 漂移。
  - 实现：脚本分别启动 Rust ccbd 和 Python ccbd 的 Claude/Codex/Gemini worker，执行 V1-V8：PATH 无 `ccb`/`ask`，绝对路径 dispatch 被拒绝，`CLAUDE.md`/`GEMINI.md`/`CODEX.md` 不可读，`env` 无 `CCB_TMUX_SOCKET`/`CCB_TMUX_SOCKET_PATH`/`CCB_KEEPER_PID`/`CCB_MASTER_CLAUDE_PID`，provider 保持登录，git 可用，`/workspace` 可写，免权 flag 存在，worker 不派单。
  - cutover discipline：该脚本随 breaking PR 一起落地，不能作为后续补测；脚本输出必须标明 rust/python、provider、facet 和失败命令，方便定位。

## 执行顺序

1. 先做任务 2、4、6、8、9，先把 env、配置复制、Python 免权和服务端硬闸门收住。
2. 再做任务 1、3、7，完成 Rust bwrap 和 Python PATH 的 dispatch 物理/软隔离。
3. 最后做任务 5、10、11，补齐 Rust gate、Worker prompt 和双实现端到端验收。
