# Tasks: ah PR2 Isolation Core

## 概述

PR2 目标是完成 ah 隔离核心 cutover：删除 `bwrap` 内核沙盒依赖，改用精确定向的 provider 配置目录环境变量，并把 OAuth 凭据供给从 copy 改为 symlink。

红线：配置定向不允许出错。实现顺序必须 tests-first，先写能证明泄漏/漂移的红灯测试，再改实现让测试变绿。所有 [BREAKING] 改动必须在同一 task 内同步更新受影响测试，不能把实现和测试拆到不同 PR。

## 任务依赖图

```text
T0 Inventory
  -> T1 Config Drift Check 红灯测试
      -> T2 Provider env 精确定向
      -> T3 OAuth copy -> symlink [BREAKING]
      -> T4 删除 bwrap / 保留 systemd scope [BREAKING]
          -> T5 provider 启动链路校验
              -> T6 scripts/docs cutover
                  -> T7 全量验证与迁移说明
```

## Cutover 纪律

- [BREAKING] task 必须同时处理源码、单测、integration/e2e 断言和文档/脚本引用。
- 删除 `bwrap` 时，不保留 `CCBD_UNSAFE_NO_SANDBOX` 作为长期语义开关；若临时兼容必须在同 task 明确删除或降级为 no-op，并清理测试。
- `systemd-run --user --scope` 保留。PR2 删除的是 `bwrap` command layer，不删除 systemd scope 包装能力，PR3 还要在这里接 `BindsTo=ah.service`。
- 以下四个变量经实证为 CLI 不读取的空操作，必须彻底移除：
  `CLAUDE_PROJECTS_ROOT`、`CLAUDE_PROJECT_ROOT`、`GEMINI_ROOT`、`CODEX_SESSION_ROOT`。
  注意 `CLAUDE_PROJECTS_ROOT` 带 S，`CLAUDE_PROJECT_ROOT` 不带 S；两者是两个不同旧变量，不能只删其中一个。
- Gemini 只注入 `GEMINI_CLI_HOME`；Claude 只新增 `CLAUDE_CONFIG_DIR`；Codex 保持 `CODEX_HOME`。
- OAuth 凭据必须是 host -> sandbox symlink；测试必须断言 link target，而不是只读内容。

## 详细任务

### T0: Baseline inventory 与测试影响清单

- **类型**: 测试先 / 实现后准备任务。
- **目标**: 在改代码前固定当前 bwrap、env 注入、OAuth copy 和 provider spawn 链路的文件级影响面。
- **输入**: `design.md` §1-§4；当前 grep 结果。
- **输出**: 实施前 inventory 记录在 PR 描述或任务笔记中，不单独提交源码。
- **文件锚点**:
  - `src/provider/home_layout.rs:49-109`: claude/gemini/codex overrides。
  - `src/provider/home_layout.rs:14-21`: `PROVIDER_AUTH_WHITELIST`。
  - `src/provider/home_layout.rs:173-179`: `copy_credentials`。
  - `src/sandbox/mod.rs:3-59`: `bwrap` module export、`EnvState.bwrap_available`、`check_environment`。
  - `src/sandbox/systemd.rs:8-32`: `wrap_command` 目前拼接 `bwrap`。
  - `src/rpc/handlers.rs:29-40`、`305-340`: spawn handler 目前调用 `bwrap::build_args` 后再进 `systemd::wrap_command`。
  - `src/provider/manifest.rs:233-249`: `collect_spawn_env` 合并 manifest env 与 extra env。
- **验收 (DoD)**:
  - [ ] 用 `rg` 更新影响清单：`bwrap|bwrap_available|CLAUDE_PROJECTS_ROOT|CLAUDE_PROJECT_ROOT|GEMINI_ROOT|CODEX_SESSION_ROOT|CODEX_HOME|CLAUDE_CONFIG_DIR|GEMINI_CLI_HOME|copy_credentials|collect_spawn_env|wrap_command`。
  - [ ] 列出所有将被删除/迁移的测试文件，尤其是 `tests/r3_bwrap_workspace.rs`、`tests/r1_r3_joint.rs`、`tests/mvp2_acceptance.rs`、`tests/mvp7_acceptance.rs`、`src/sandbox/bwrap.rs` 内单测。
  - [ ] 不写实现代码。
- **可能撞现有 round 测试**: 无，inventory only。

### T1: 先写 Config Drift Check 红灯测试

- **类型**: 测试先。
- **目标**: 落实 design §3 的真实验收：三个 provider 的配置变量必须指向 sandbox 虚拟目录，且配置写入不得修改宿主真实配置文件。
- **改动文件**:
  - 新增 `tests/ah_config_drift.rs` 或 `tests/pr2_config_drift.rs`。
  - 可新增 fixture helper 到 `tests/common/mod.rs`，但不得改实现源码。
- **测试设计**:
  - [ ] 用 tempfile 构造 host HOME，创建宿主配置文件：
        `~/.claude/settings.json`、`~/.codex/config.toml`、`~/.gemini/settings.json` 或现有实际文件名集合。
  - [ ] 调用现有 provider materialization / spawn env 链路，断言：
        `CLAUDE_CONFIG_DIR` -> `<sandbox_home>/.claude`；
        `CODEX_HOME` -> `<sandbox_home>/.codex`；
        `GEMINI_CLI_HOME` -> `<sandbox_home>/.gemini`。
  - [ ] 断言四个旧空操作变量都不存在：
        `CLAUDE_PROJECTS_ROOT`、`CLAUDE_PROJECT_ROOT`、`GEMINI_ROOT`、`CODEX_SESSION_ROOT`。
  - [ ] 模拟 sandbox 内配置写入，或通过 provider helper 写入目标 config，断言宿主 config `mtime` 不变。
  - [ ] 跑测试前记录宿主 HOME 相关配置目录文件清单 before，测试后记录 after，断言集合完全相等；这覆盖 design §3.2 的文件残留检测，不只依赖 `mtime` 间接判断。
  - [ ] hermetic fake command 只验证 env 被设置，不能证明变量名被真实 CLI 读取。合并前必须在 VPS 跑一次真 CLI smoke 并留下证据，验证 `CLAUDE_CONFIG_DIR` / `GEMINI_CLI_HOME` / `CODEX_HOME` 三个变量名确实被对应 CLI 识别，且宿主 HOME 清单 before/after 不变。
- **文件级断言锚点**:
  - `src/provider/home_layout.rs:36-45`: `materialize_provider_home` 返回 extra env。
  - `src/provider/manifest.rs:233-249`: `collect_spawn_env` 确认 extra env 会进入 spawn command。
  - `src/sandbox/systemd.rs:99-106`: systemd wrapper 注入 env 的最终 argv。
- **验收 (DoD)**:
  - [ ] 新测试在当前实现下必须红：Claude 缺 `CLAUDE_CONFIG_DIR`，仍注入 `CLAUDE_PROJECTS_ROOT`/`CLAUDE_PROJECT_ROOT`；Gemini 仍是 `GEMINI_ROOT`；Codex 仍注入 `CODEX_SESSION_ROOT`；copy 凭据语义不满足 symlink。
  - [ ] 测试只写 tempfile，不读写真实 `~/.claude` / `~/.codex` / `~/.gemini`。
  - [ ] 测试名明确表达泄漏风险，例如 `config_redirect_env_points_to_sandbox_roots`、`host_config_mtime_does_not_change`、`host_home_file_inventory_does_not_change`。
- **可能撞现有 round 测试**:
  - 若直接复用 real provider tests，会撞 `tests/common/mod.rs:33-34` 的 bwrap requirement；实施时应把 real-provider gate 改成 tmux+systemd，不再要求 bwrap。

### T2: 实现 Provider 配置环境变量精确定向 [BREAKING]

- **类型**: 实现后，让 T1 env 红灯变绿。
- **目标**: 在 provider home layout 中注入唯一正确的三个配置目录变量，并删除四个旧空操作变量。
- **改动文件**:
  - `src/provider/home_layout.rs`
  - `tests/mvp12_home_layout.rs`
  - `src/provider/home_layout.rs` 内单测
- **具体改动**:
  - [ ] [BREAKING] 删除 `src/provider/home_layout.rs:68` 的 `CLAUDE_PROJECTS_ROOT` 注入。
  - [ ] [BREAKING] 删除 `src/provider/home_layout.rs:72` 的 `CLAUDE_PROJECT_ROOT` 注入。
        注意带 S / 不带 S 是两个不同旧变量，必须同时覆盖。
  - [ ] `prepare_claude_overrides`: 增加 `CLAUDE_CONFIG_DIR = sandbox_path(".claude")`。
  - [ ] `prepare_codex_overrides`: 保持 `CODEX_HOME = sandbox_path(".codex")`。
  - [ ] [BREAKING] 删除 `src/provider/home_layout.rs:111` 的 `CODEX_SESSION_ROOT` 注入。
  - [ ] `prepare_gemini_overrides`: [BREAKING] 删除 `src/provider/home_layout.rs:96` 的 `GEMINI_ROOT = sandbox_path(".gemini/tmp")`，改为 `GEMINI_CLI_HOME = sandbox_path(".gemini")`。
  - [ ] 不注入 `CLAUDE_PROJECTS_ROOT`、`CLAUDE_PROJECT_ROOT`、`GEMINI_ROOT`、`CODEX_SESSION_ROOT` 等辅助变量。
- **测试同步**:
  - [ ] 更新 `tests/mvp12_home_layout.rs:72`，从 `GEMINI_ROOT` 改断言 `GEMINI_CLI_HOME`。
  - [ ] 增加 Claude extra env 断言，覆盖 `CLAUDE_CONFIG_DIR`。
  - [ ] 保留 `tests/mvp12_home_layout.rs:141` 的 `CODEX_HOME` 断言，并确认路径仍指向 `.codex` 根。
  - [ ] [BREAKING] 迁移 `tests/mvp12_home_layout.rs:109` 的 `CLAUDE_PROJECTS_ROOT` 断言：删除旧断言，改为断言 `CLAUDE_CONFIG_DIR` 指向 `.claude` 配置根。
  - [ ] [BREAKING] 迁移 `tests/mvp12_home_layout.rs:145` 的 `CODEX_SESSION_ROOT` 断言：删除旧断言；若需要 session root 行为，改为断言不会注入该变量，且 `CODEX_HOME` 指向 `.codex` 配置根。
  - [ ] 更新 `src/provider/home_layout.rs` 现有 provider override 单测。
- **验收 (DoD)**:
  - [ ] T1 中 env redirect 测试 green。
  - [ ] `rg -n "CLAUDE_PROJECTS_ROOT|CLAUDE_PROJECT_ROOT|GEMINI_ROOT|CODEX_SESSION_ROOT" src tests` 无生产/测试残留；历史文档若保留需注明。
  - [ ] `rg -n "CLAUDE_CONFIG_DIR|CODEX_HOME|GEMINI_CLI_HOME" src/provider tests` 能看到三变量断言。
- **可能撞现有 round 测试**:
  - `tests/mvp12_home_layout.rs:72`、`:109`、`:145` 的旧断言会失败，这是正确 cutover signal，必须在本 task 同步迁移。
  - `tests/mvp7_acceptance.rs:249-251` 只断言 `CODEX_HOME`，应继续通过；若 bwrap 删除后该测试仍读取 bwrap argv，则交给 T4 同步迁移。

### T3: OAuth 凭据 copy -> symlink [BREAKING]

- **类型**: 测试先 / 实现后。
- **目标**: 将 host OAuth 凭据供给从复制改为 symlink，实现宿主登录和 sandbox token 双向同步。
- **迁移路径 [BREAKING]**:
  - `src/provider/home_layout.rs:173-179` 的 `copy_credentials` 改名为 `link_credentials`。
  - `src/provider/home_layout.rs:487-499` 的 copy helper 改为 symlink helper，保留 parent dir 创建和坏链替换策略。
  - `tests/mvp12_home_layout.rs:97-107`、`128-138` 从“不是 symlink”改为 `read_link` 指向 host source。
  - `src/provider/home_layout.rs` 内 Claude/Codex/Gemini auth 单测同步改为 symlink 断言。
- **具体改动**:
  - [ ] 对 `PROVIDER_AUTH_WHITELIST` 每个存在的 host file 建立 `sandbox_path -> host_path` symlink。
  - [ ] 若目标是旧 copy 文件，应在 PR2 cutover 中替换为 symlink；替换前后记录 `tracing::info!`。
  - [ ] 若目标是正确 symlink，保持幂等。
  - [ ] 若目标是目录或指向其他路径的 symlink，返回 typed error 或 warn+明确跳过策略；不得静默覆盖不明数据。
- **测试先**:
  - [ ] 先写/改红灯测试：host credential 更新后 sandbox path 读到新内容，证明不是 copy。
  - [ ] 断言 `std::fs::read_link(sandbox_credential) == host_credential`。
- **验收 (DoD)**:
  - [ ] `rg -n "copy_credentials|copy_auth_file_if_missing_or_symlink" src tests` 无旧语义残留，或只剩迁移注释。
  - [ ] T1 中 host mtime 不变测试 green。
  - [ ] OAuth 文件不存在时不创建 dangling symlink，保持当前“缺失则跳过”的可用性。
- **可能撞现有 round 测试**:
  - `tests/mvp12_home_layout.rs:99-102`、`130-133` 旧断言会失败，必须同 task 修改。
  - 任何依赖复制内容的测试都应改为 symlink target + 读内容双断言。

### T4: 删除 bwrap 路径并保留 systemd scope [BREAKING]

- **类型**: 测试先 / 实现后。
- **目标**: 彻底删除 `bwrap` command layer，同时保留 `systemd-run --user --scope` 包装。
- **迁移路径 [BREAKING]**:
  - 删除 `src/sandbox/bwrap.rs`。
  - `src/sandbox/mod.rs:3` 删除 `pub mod bwrap`。
  - `src/sandbox/mod.rs:12-59` 删除 `EnvState.bwrap_available`、`bwrap_probe`、`SandboxBwrapNotFound` fail-closed 路径；`check_environment` 只检查 systemd/tmux 需要的环境。
  - `src/sandbox/systemd.rs:8-32` 删除 `bwrap_args` 参数和 `cmd.push("bwrap")`；wrapper 直接执行 provider command。
  - `src/rpc/handlers.rs:29` 删除 `bwrap` import；`305-340` 删除 `bwrap::build_args`，保留 sandbox home materialization 与 `systemd::wrap_command`。
- **测试同步**:
  - 删除或重写 `src/sandbox/bwrap.rs` 内所有单测。
  - 删除 `tests/r3_bwrap_workspace.rs`，或改为新的 config-drift/systemd-scope 测试。
  - `tests/r1_r3_joint.rs:154-184` 的 bwrap workspace 测试删除/迁移。
  - `tests/mvp2_acceptance.rs:13`、`189-192`、`357` 附近 bwrap API/缺失 bwrap fail-closed 测试删除/迁移。
  - `tests/mvp7_acceptance.rs:15`、`206-251` 的 bwrap argv HOME/CODEX_HOME 断言迁移到 systemd/env argv 或 provider layout 测试。
  - 所有构造 `EnvState { bwrap_available: ... }` 的测试同步删除字段：
        `tests/mvp6_acceptance.rs:36`、`mvp8_acceptance.rs:38`、`mvp9_acceptance.rs:44`、
        `tests/prompt_handler_e2e.rs:53`、`tests/r1_r3_joint.rs:89`、
        `tests/r3_absolute_path_propagation.rs:49,111`、real provider tests 等。
  - `tests/common/mod.rs:33-34` real-provider gate 从 “tmux+bwrap+systemd” 改为 “tmux+systemd”。
- **systemd scope 保留断言**:
  - [ ] 保留/更新 `src/sandbox/systemd.rs` tests：`wrap_command` argv 仍以 `systemd-run` 开头，仍有 `--scope`、`--slice`、`--property=BindsTo=ccbd.service` 当前语义。
  - [ ] 删除断言 `argv contains "bwrap"`，改断言 provider argv 直接出现在 systemd 分隔符后。
  - [ ] `src/rpc/handlers.rs` spawn log 不再包含 bwrap，但仍能看到 systemd scope command。
- **验收 (DoD)**:
  - [ ] `rg -n "bwrap|Bwrap|bwrap_available|SandboxBwrapNotFound" src tests` 只允许历史文档或已删除文件无命中。
  - [ ] `cargo test sandbox::systemd` 通过，证明 scope wrapper 没被删。
  - [ ] `cargo test --all-targets -- --test-threads=1` 串行通过。
- **可能撞现有 round 测试**:
  - `scripts/r3_e2e.sh:249-285` 仍 grep bwrap argv，会失败；T6 同步改脚本。
  - `scripts/mvp13-e2e-sandbox.sh` / `mvp13-e2e-sandbox-probe.sh` 文案仍说 bwrap sandbox，会失败或误导；T6 同步改。
  - real provider tests 以前要求 bwrap，删除后如果仍 gate bwrap 是 cutover 未完成。

### T5: Provider 启动链路确认三变量进入 agent 进程

- **类型**: 测试先 / 实现后。
- **目标**: 从 provider layout 到 spawn command 的最终链路可验证，防止 “layout 有 env 但 agent 没收到”。
- **改动文件**:
  - `src/provider/manifest.rs`
  - `src/sandbox/systemd.rs`
  - `src/rpc/handlers.rs`
  - 对应单测/integration tests
- **链路说明**:
  - `materialize_provider_home` 产出 `SandboxOverrides.extra_env`。
  - `rpc/handlers.rs` agent spawn 将 extra env 传给 `systemd::wrap_command`。
  - `systemd::wrap_command` 调 `collect_spawn_env(manifest, extra_env_vars)`，最终以 env argv 注入 provider 进程。
- **测试先**:
  - [ ] 在 `src/sandbox/systemd.rs` 单测中构造 manifest + extra env，断言 argv 包含 `CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `GEMINI_CLI_HOME`，并且 extra env 优先级覆盖 manifest default。
  - [ ] 在 `src/provider/manifest.rs` 保留/扩展 `test_collect_spawn_env_precedence`，明确 extra env 是最终来源。
  - [ ] 在 integration test 中用 fake provider 输出 `env`，断言三变量都指向 sandbox root。
- **实现后检查**:
  - [ ] 删除 bwrap 后，`HOME` 是否仍需要设置为 sandbox home：若保留，必须在 systemd env 注入中明确；若不保留，必须解释 provider 只依赖 config dir env。
  - [ ] `CODEX_HOME` / `GEMINI_CLI_HOME` / `CLAUDE_CONFIG_DIR` 不能被 manifest injected env 覆盖。
- **验收 (DoD)**:
  - [ ] T1 Config Drift Check 全部 green。
  - [ ] spawn log 或 test argv 能证明 env 进入最终 command。
- **可能撞现有 round 测试**:
  - `tests/mvp7_acceptance.rs` 之前检查 bwrap `--setenv CODEX_HOME`；删除 bwrap 后必须改为检查 systemd/env command 或 provider layout。

### T6: scripts/docs cutover 与 bwrap 测试脚本迁移

- **类型**: 测试先 / 实现后。
- **目标**: 活脚本和当前行为文档不再宣称 bwrap sandbox，同时给 [BREAKING] 用户迁移路径。
- **改动文件**:
  - `scripts/r3_e2e.sh`
  - `scripts/mvp13-e2e-sandbox.sh`
  - `scripts/mvp13-e2e-sandbox-probe.sh`
  - `scripts/mvp13-e2e-no-sandbox.sh`
  - `scripts/ac_mvp9_e2e.sh`
  - `scripts/core_fixes_full_e2e.sh`
  - `docs/DESIGN.md` 或 PR2 迁移文档
  - `.kiro/specs/ah-isolation-core/design.md` 不在实现 PR 中随意改，除非主控确认设计更新。
- **具体改动**:
  - [ ] `scripts/r3_e2e.sh:249-285` 从 grep `bwrap` argv 改为验证 provider env 指向 sandbox root、systemd scope 仍存在。
  - [ ] 删除 “NO_SANDBOX” 作为正常路径的措辞；PR2 后默认就是 env isolation。
  - [ ] 文档写明 [BREAKING] 迁移路径：
        不再需要安装 `bwrap`；
        旧 `.cache/ah/sandboxes` 可保留/清理；
        OAuth 文件从 sandbox copy 变为 symlink；
        Claude 用户不要再依赖 `CLAUDE_PROJECTS_ROOT` / `CLAUDE_PROJECT_ROOT`；
        Codex 用户不要再依赖 `CODEX_SESSION_ROOT`；
        Gemini 用户需要确认 `GEMINI_CLI_HOME`，不要再依赖 `GEMINI_ROOT`。
  - [ ] `doctor` 如仍检查 `bwrap`，同步移除或改成不检查。
- **验收 (DoD)**:
  - [ ] `rg -n "bwrap|NO_SANDBOX|CCBD_UNSAFE_NO_SANDBOX" scripts docs src/cli/doctor.rs` 无活语义残留；历史文档例外需明确不在活路径。
  - [ ] 活 e2e 脚本能在无 bwrap 环境表达正确验收。
- **可能撞现有 round 测试**:
  - 老 R3/R13 脚本会因不再见到 bwrap argv 失败；这是必须同步改的 cutover signal。

### T7: 全量验证、失败分流与发布前迁移说明

- **类型**: 验证任务。
- **目标**: PR2 合并前确认所有 breakage 都是预期 cutover，且没有配置泄漏。
- **命令纪律**:
  - [ ] 串行 build：`CARGO_BUILD_JOBS=1 cargo build --bin ah --bin ccbd`。
  - [ ] 串行全量测试：`CARGO_BUILD_JOBS=1 cargo test --all-targets -- --test-threads=1`。
  - [ ] real provider smoke 默认 gated；若运行，必须先确认不会改宿主配置 mtime。
- **验收 (DoD)**:
  - [ ] `rg -n "bwrap|Bwrap|bwrap_available|CLAUDE_PROJECTS_ROOT|CLAUDE_PROJECT_ROOT|GEMINI_ROOT|CODEX_SESSION_ROOT|copy_credentials" src tests scripts` 无活残留。
  - [ ] Config Drift Check 通过：三 provider config env 指向 sandbox，host config mtime 不变。
  - [ ] systemd scope tests 通过：删除 bwrap 后仍保留 scope 包装能力。
  - [ ] `git diff --stat` 与迁移说明列明 [BREAKING]：删除 bwrap、OAuth symlink、Gemini env 变量变更。
- **可能撞现有 round 测试**:
  - real Claude/Codex/Gemini tests 可能因宿主 CLI 版本/认证状态不稳定失败；若失败但 hermetic drift tests 通过，应 flag 给主控，不在 PR2 内扩大范围修 provider 行为。

## 验收矩阵

| 设计要求 | 必须覆盖的 task | 关键验证 |
|---|---|---|
| 删除 bwrap | T4/T6/T7 | `rg` 无活 bwrap 残留；systemd scope 仍 green |
| Claude 配置定向 | T1/T2/T5 | `CLAUDE_CONFIG_DIR=<sandbox>/.claude` |
| Codex 配置定向 | T1/T2/T5 | `CODEX_HOME=<sandbox>/.codex` |
| 空操作变量删除 | T1/T2/T7 | 无 `CLAUDE_PROJECTS_ROOT` / `CLAUDE_PROJECT_ROOT` / `GEMINI_ROOT` / `CODEX_SESSION_ROOT` 活残留 |
| Gemini 配置定向 | T1/T2/T5 | `GEMINI_CLI_HOME=<sandbox>/.gemini`；无 `GEMINI_ROOT` |
| OAuth symlink | T3 | `read_link` 指向 host auth file；host token 更新 sandbox 可见 |
| 宿主配置不被改 | T1/T5/T7 | host config `mtime` 不变 |
| PR3 systemd 插槽 | T4/T5 | `systemd-run --scope` wrapper 保留 |

## 实施注意

- 不要把 “无 bwrap” 命名为 “no sandbox bypass”。PR2 的默认隔离模型就是 env config redirect + sandbox home materialization。
- 不要为了让旧 bwrap tests 过而保留假 bwrap shim；删除就是 cutover。
- 不要把 `GEMINI_CLI_HOME` 指向 `.gemini/tmp`；它必须是 `.gemini` 配置根。
- 不要把 `CLAUDE_PROJECTS_ROOT` 和 `CLAUDE_PROJECT_ROOT` 混成一个变量；两者都是旧空操作变量，都必须删除。
- 不要保留 `CODEX_SESSION_ROOT` 作为兼容变量；真实配置定向只靠 `CODEX_HOME`。
- 不要在单元测试中修改真实 `HOME` 下配置；全部用 tempfile 和注入路径。
- 如果现有 round 测试要求 bwrap，优先判定为 cutover 测试未迁移，不要把实现倒退回 bwrap。
