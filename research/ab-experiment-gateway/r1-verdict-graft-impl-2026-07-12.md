# r1 审核裁决 · 模块D网关嫁接实施(轨1)

**审核席**: r1-claude(只审不写,唯一审核关卡)
**被审**: `feat/gateway-graft-modD` @ `9c9f636`(base main `7bae3b1`),worktree `/home/sevenx/coding/ccbd-rust-wt-graft-c1`
**裁决依据**: 冻结设计 `design-graft-frozen-2026-07-11.md` + 补丁 `design-graft-addendum-2026-07-12.md` + incident `incident-2026-07-11-wsl2-symlink-logout.md`
**日期**: 2026-07-12

---

## 裁决:**REJECT**

**一句话**:进程内核心(单飞/失败缓存/JWT/凭据剥离/UDS listener/seed reader/回写/生产 refresh)出货级、测试真实(回滚自检实证非空转);但**嫁接的唯一实质目标——"把运行时管道接上,让 claude 真的经网关跑起来"(补 verdict 乙-7)——在 worker 与 master 两条路径上都是坏的**,且这两处 bug 恰好落在**没有任何端到端执行测试覆盖**的沙箱 wrapper 层,正是冻结设计分歧点6 明令禁止的"改了生产、测试全绿、真跑挂掉"模式。修好这两处 + 补对应端到端测试后复审。

---

## 一、已实证为 GOOD 的部分(亲跑,非采信 c1 自报)

| 项 | 结论 | 实证 |
|---|---|---|
| **Kill List K1–K11** | 全清 | 亲自 grep:`worker_gateway_for_test`/`python3`/`build_python_bridge`/HMAC/`8206`/`SANDBOX_TCP_BASE_URL`/`port_from_slot_id`/`link_credentials` 在 `src/`+`tests/` 均零命中;`PROVIDER_AUTH_WHITELIST`(home_layout.rs:21)已无 `.claude/.credentials.json`;`is_expired` 已换 5min 安全窗口(claude_gateway.rs:17,32-36);无缓冲版已删(K10) |
| **核心逻辑忠实冻结设计** | 通过 | 单飞双检+失败缓存全在 `refresh_lock` 内(claude_gateway.rs:140-190);瞬时错误不进 invalid_grant 缓存(:168,补丁裁决3);seed reader 容忍式 `claudeAiOauth` camelCase 主/snake_case 兜底/`expiresAt<=0=过期`(:607-633,补丁裁决2.2);生产 refresh 端点 `POST platform.claude.com/v1/oauth/token` + 错误分层映射(:460-529,补丁裁决3);守卫式原子回写 canonicalize+/mnt/c 守卫+temp+rename+0600(:635-693,补丁裁决2.3) |
| **daemon 所有权层** | 通过 | `ClaudeGatewayService` Arc 挂 Ctx 镜像 `tmux_server`(rpc/mod.rs:24,ahd.rs:90-108);holder eager / core+seed lazy 单飞(:412-422,裁决1);daemon 重启 reconcile 幂等 register 活跃 claude 席位(db/system.rs `reconcile_claude_gateway_seats`,裁决4);`master_command_with_env` 三平台+facade 加 `sandbox_overrides` 无 shim(裁决5);UDS 用 `ReadWriteBind`+`BindPaths`(非 ro,connect 需写权,裁决5) |
| **worker 身份一致性** | 通过 | worker 的假 JWT 用 `slot_id`(home_layout.rs:1731),spawn 处 `slot_id == agent_id`(agent.rs:172,177),`register_worker(agent_id)` 通道 id 亦为 agent_id(agent.rs:189)——二防线 `validate_worker_identity` 运行期能对上,不会误 403 |
| **AC 15/15 全绿** | 亲跑通过 | `cargo test --test gateway_graft_acceptance`:15 passed;0 failed(见附录 A) |
| **核心测试非空转** | 回滚自检实证 | 见第三节 |
| **incident /mnt/c 写穿防护机制** | 机制成立 | `write_seed_credentials_guarded` 先 `canonicalize` 再查 `/mnt/c`;亲验 canonicalize 解析 symlink 到目标(附录 C)——真机 WSL 场景(Linux 路径 symlink→/mnt/c)会被解析并拦截;断链场景 fallback 后 `read_to_string` 报错 fail-safe,不写穿 |

---

## 二、REJECT 阻塞项(必须修)

### R1-BLOCK-1 · 沙箱 wrapper **永不 exec 内层命令**,claude 从不启动(worker + master 双路径)

- **文件/行**:`src/claude_gateway.rs:942-957`(`bridge_wrapper_shell`),尾串 `... exec sh -lc "$1" sh '<inner>'`(:951 格式尾 + :956 ` sh {inner}` 追加)。
- **缺陷**:wrapper 字符串由 `command_with_env_prefix`(linux/scope.rs:397,返回 `["sh","-lc",bridge_wrapper_shell(...)]`)与 `master_shell_command_with_env_prefix` 以 `sh -lc "<BRIDGE>"` 执行,**运行 BRIDGE 的这个 shell 没有任何位置参数**。BRIDGE 尾部的 `"$1"` 因此展开为空 → `exec sh -lc "" sh '<inner>'` → 执行空脚本、`<inner>`(即 `claude ...`)被丢弃 → **进程 exit 0,claude 从未运行**。`sh '<inner>'` 里的 `<inner>` 只是新 shell 的 `$0`/`$1`,而新 shell 的脚本是父 shell 的空 `"$1"`。
- **实证(亲跑,非读码猜)**:忠实复现 `bridge_wrapper_shell` 输出 + stub bridge,worker 路径 `sh -lc "$BRIDGE"` 与 master 路径 `sh -lc 'oom; exec sh -lc "$1"' sh "$BRIDGE"` **两者 inner marker 均未生成,exit=0**(附录 B)。
- **期望行为**:设置 `ANTHROPIC_BASE_URL` 后应 `exec` 内层 `claude` 命令。
- **修点方向**(实施线定,示意):尾串直接 `... ANTHROPIC_BASE_URL="http://localhost:$port" exec sh -lc <shell_quote(inner)>`——去掉 `"$1" sh` 这层错误间接;或由调用方以 `sh -lc "$BRIDGE" sh "<inner>"` 传入 `$1` 且 `bridge_wrapper_shell` 不再自行追加 inner。二选一,不能维持现状。
- **为何漏网(与冻结设计分歧点6 直接抵触)**:唯一相关测试 `ac_bridge_wrapper_fail_fast_path_is_observable`(tests/gateway_graft_acceptance.rs:225-235)**只断言字符串包含 `bridge.err`/`exit 126`/`ANTHROPIC_BASE_URL=...`,从不执行 wrapper**。这正是分歧点6 定性的"改了生产、测试全绿、真跑挂掉"病根。

### R1-BLOCK-2 · master **初次** spawn 未注入网关静态 env,master 不经网关(只有 revive 路径对)

- **文件/行**:`src/rpc/handlers/sessions.rs:516-522`(`build_master_spawn_env_vars` 只注入 master 身份)与 `prepare_master_pane_plan`(:459-501,只加了 `AH_CLAUDE_GATEWAY_HOST_UDS` + RW bind + `register_master`)。
- **缺陷**:初次 master spawn **从不设置** `CLAUDE_CODE_USE_GATEWAY=1` 与 `ANTHROPIC_AUTH_TOKEN=fake_worker_jwt(session_id)`(grep 实证:`CLAUDE_CODE_USE_GATEWAY`/`fake_worker_jwt` 在 sessions.rs 零命中)。而 `master_shell_command_with_env_prefix` 的 wrapper 注入**以 `CLAUDE_CODE_USE_GATEWAY=="1"` 为门**(linux/scope.rs:494-508)→ 初次 master 该门不成立 → **不起桥、无 `ANTHROPIC_BASE_URL`、无假 JWT**;叠加宿主 `ANTHROPIC_*` 已剥离 + 无 `.credentials.json` → 初次 master claude = "Not logged in"。
- **对照**:只有 **revive** 路径 `master_watch.rs:801-816` 正确注入了这两个静态 env。补丁裁决5 明确要求初次 spawn(调用点 `sessions.rs:517`)也走静态 env 注入,冻结稿分歧点4契约3 亦然。当前只做了一半。
- **期望行为**:初次 master 与 revive 对称——`build_master_spawn_env_vars` 或 `prepare_master_pane_plan` 注入 `CLAUDE_CODE_USE_GATEWAY=1` + `ANTHROPIC_AUTH_TOKEN=fake_worker_jwt(session_id)`。
- **未覆盖**:无任何测试断言初次 master spawn 的 env 含网关静态项。

> **两处叠加的净效果**:即便单独修好 R1-BLOCK-2,R1-BLOCK-1 仍让 master(与所有 worker)的 claude 无法启动;即便单独修好 R1-BLOCK-1,初次 master 仍不走网关。**嫁接被 charter 的唯一实质欠交(乙-7 运行时管道)在两条路径上都未真正接通。**

---

## 三、回滚自检(铁律,实跑非采信)

临时改 `src/claude_gateway.rs` 两处核心逻辑,重编译定向重跑,确认对应测试**变红**,再 `git checkout` 复原、确认树净:

| 回滚动作 | 目标测试 | 结果 |
|---|---|---|
| `cached_failure_inside_refresh_lock` 恒返回 `None`(废失败缓存) | `ac_failure_cache_suppresses_invalid_grant_refreshes_and_records_event` | **RED**:`refresh_calls` left=2 right=1(gateway_graft_acceptance.rs:187) |
| `validate_worker_identity` 的 `if jwt!=channel` 改 `if false`(废二防线) | `ac_uds_channel_isolation_rejects_wrong_worker_jwt` | **RED**:`response.starts_with("HTTP/1.1 403")` 断言失败(:139) |

两测试均如期变红 → **核心安全契约的测试真实锚定行为,非空转**。复原后 `git diff HEAD` 为空(树净,见附录 A/B)。

---

## 四、测试真实性缺口(与 REJECT 相关,须随修补齐)

1. **无 wrapper 端到端执行测试**(直接放行 R1-BLOCK-1):须补"真起 stub/真桥 + 真跑 wrapper + 断言 inner 命令确被执行 + `ANTHROPIC_BASE_URL` 确注入 exec 环境"的测试(分歧点6:新增 listener/桥/挂载层必须真起端到端,不得只字符串匹配)。
2. **无初次 master spawn env 断言**(放行 R1-BLOCK-2):须补断言初次 master `master_env_vars` 含 `CLAUDE_CODE_USE_GATEWAY=1` + 合法假 JWT。
3. **incident /mnt/c 测试偏浅(brief 重点复核项)**:`ac_wsl_mount_guard_rejects_windows_credentials_path`(:238-245)仅测字符串谓词;`addendum_wsl_guard_skips_writeback...`(:298-306)仅测**直传** `/mnt/c` 路径跳过。**真实事故向量是 Linux 路径 symlink→/mnt/c**,靠 `write_seed_credentials_guarded` 的 `canonicalize` 拦截——生产代码机制成立(附录 C 亲验),但**无 symlink 解析场景的测试**。建议补:构造 symlink→/mnt/c 目标,断言 writeback 跳过且不写穿。(机制正确,故非独立阻塞,但属应补测试。)

---

## 五、非阻塞观察(建议处理,不单独构成 REJECT)

- **S1 · scope 卫生:整仓 `cargo fmt` 混入本 commit**。80 文件 +3225/-535 中,约 40 个与网关无关的文件是纯 `cargo fmt` 重排(实证:取 base 版跑 `rustfmt --edition 2024` 后与 c1 版**逐字节相同**,如 db/jobs.rs、completion/parser.rs、outbox/tests.rs、pane_diff/mod.rs、marker/timer.rs 等)。功能安全但污染 diff、违反"scope 恰好覆盖 brief"。**建议**:格式化拆成独立 commit,让网关 commit 只含真实改动(约 15–20 文件)。
- **S2 · 死代码**:`symlink_auth_file`(home_layout.rs:1878)在 `link_credentials` 删除后成孤儿,编译告警 `never used`。随手删。
- **S3 · macOS/Windows**:`master_command_with_env` 已加 `sandbox_overrides` 参但直接 `let _ = (...)` 忽略;macOS 亦是 unix,会 `register_master`/注 bind 到 overrides,但 macos/scope.rs 不落 bind → 该形态 master 网关 bind 不生效。**轨1 Linux 范围外**,记给 `ah-macos-port`,本单不阻塞。
- **S4 · cargo 政策**:c1 交付前未跑全量 `cargo test`(符合"全量走 CI"政策);我本审亦只跑定向 `gateway_graft_acceptance` 单二进制(`CARGO_BUILD_JOBS=1`)。非网关文件为 fmt-only churn,低风险,全量绿以 CI 为准。

---

## 六、给 c1 的复审清单(修完这些再回审)

1. 修 R1-BLOCK-1:`bridge_wrapper_shell` 真正 `exec` inner;+ 端到端执行测试(真跑,断言 inner 被执行)。
2. 修 R1-BLOCK-2:初次 master spawn 注入 `CLAUDE_CODE_USE_GATEWAY=1` + `fake_worker_jwt(session_id)`;+ 断言测试。
3. 补 incident symlink→/mnt/c writeback 跳过测试(§四.3)。
4. (建议)S1 拆 fmt commit;S2 删死代码。

修完回报后我做**回滚自检式复审**(重点跑新补的 wrapper 端到端 + master env 测试,并对其做回滚变红验证)。**不 push、不开 PR——PR 开/合归 operator**;本审仅供证据与裁决。

---

## 附录 A · AC 全绿实证

```
$ CARGO_BUILD_JOBS=1 cargo test --test gateway_graft_acceptance
running 15 tests
test ac_bridge_wrapper_fail_fast_path_is_observable ... ok
test ac_rewrite_upstream_sees_real_token_not_fake_jwt ... ok
test ac_uds_channel_isolation_rejects_wrong_worker_jwt ... ok
test ac_failure_cache_suppresses_invalid_grant_refreshes_and_records_event ... ok
test ac_wsl_mount_guard_rejects_windows_credentials_path ... ok
test ac_single_flight_expired_workers_refresh_once ... ok
test ac_zero_credentials_worker_home_has_no_real_token_bytes ... ok
test addendum_seed_reader_accepts_real_claude_oauth_schema_and_expired_zero ... ok
test addendum_seed_writeback_rotates_refresh_token_atomically_for_linux_path ... ok
test addendum_service_register_master_is_idempotent_across_reconcile ... ok
test ac_uds_header_limit_returns_400 ... ok
test addendum_transient_refresh_errors_do_not_poison_failure_cache ... ok
test addendum_wsl_guard_skips_writeback_without_touching_windows_path ... ok
test addendum_production_refresh_maps_only_400_invalid_grant_to_invalid_grant ... ok
test ac_bridge_dynamic_ports_do_not_conflict ... ok
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## 附录 B · R1-BLOCK-1 wrapper 空转实证(忠实复现 bridge_wrapper_shell 输出)

```
===== WORKER PATH: sh -lc "$BRIDGE"  (== command_with_env_prefix 构造) =====
exit=0
>>> !!! INNER DID NOT RUN — claude would never launch
===== MASTER PATH: sh -lc 'echo oom; exec sh -lc "$1"' sh "$BRIDGE" =====
oom-set
exit=0
>>> !!! INNER DID NOT RUN — claude would never launch
```
stub `fake-ah internal-bridge --port-file Y` 写端口后 linger;inner=`echo INNER_EXECUTED_MARKER > .../inner.marker`;两路径 marker 均未生成。

## 附录 C · incident /mnt/c canonicalize 机制亲验

```
symlink  .../canon/link-creds.json -> .../canon/realtarget/creds.json
realpath(= Rust Path::canonicalize) => .../canon/realtarget/creds.json   # 解析到目标
# 若目标是 /mnt/c/... → canonicalize 得 /mnt/c/... → 守卫命中 → 跳过 writeback
# 断链(目标不存在)→ canonicalize Err → fallback 原始路径 → 随后 read_to_string 报错 fail-safe,不写穿
```
结论:生产写穿防护机制成立;缺的是 symlink 解析场景的**测试**(§四.3)。

## 附录 D · 回滚自检复原实证

```
$ git checkout -- src/claude_gateway.rs
$ git diff HEAD --stat        # 空 → 树净复原
$ git status --short
?? .operator-question          # c1 遗留,非本审产物
```

---

*r1-claude 审核。verdict=REJECT,阻塞项 R1-BLOCK-1/2 钉到 file:line + 实证 + 期望行为。核心工程质量高、测试真实,退回项集中在运行时 wrapper 接线与其缺失的端到端测试。PR 开/合与 git push 归 operator。*

---
---

# 复审(RE-REVIEW)· rework `6e67f31`(2026-07-12)

> 保留上方 REJECT 原记录不动(返工轨迹)。本节针对 c1 在 `9c9f636` 之上的返工 commit `6e67f31`(未 amend、未 push,4 文件 +126/-28)复审。方法:回滚自检式——亲跑新补测试 + 对每处修复做**回滚变红**验证,并确认原 REJECT 的实证复现手法在新代码上不再复现。

## 复审裁决:**ACCEPT**

两个阻塞项 R1-BLOCK-1/2 均已修复且被**非空转**的测试锁定;§四.3 建议的 symlink 场景测试已补(且带生产侧防御增强);S2 死代码已删。返工 commit 本身 scope 干净(仅 4 文件,聚焦)。

### 逐项复核(全部亲跑,非采信 c1 自报)

| 原阻塞/建议 | c1 修法 | 复核结论 | 实证 |
|---|---|---|---|
| **R1-BLOCK-1**(wrapper 空转) | `bridge_wrapper_shell` 尾串改 `exec sh -lc {inner}`(`shell_quote(inner)`),删除 `"$1" sh` 错误间接层(claude_gateway.rs:973,978) | **已修** | ① 我附录B 的 stub-bridge 手法在新串上**重跑**:worker 路径 `sh -lc "$BRIDGE"` 与 master 路径均 **inner 确执行 + `ANTHROPIC_BASE_URL=http://localhost:54321` 确注入**——原空转不再复现。② 新增端到端测试 `ac_bridge_wrapper_executes_inner_with_gateway_base_url`(真起 stub-ah、真跑 wrapper、断言 inner marker=="ran" 且 base-url marker=="http://localhost:48231")。③ `ac_bridge_wrapper_fail_fast_path_is_observable` 加回归护栏 `assert!(!shell.contains("exec sh -lc \"$1\""))` |
| **R1-BLOCK-2**(初次 master 无网关 env) | `build_master_spawn_env_vars` 注入 `CLAUDE_CODE_USE_GATEWAY=1` + `ANTHROPIC_AUTH_TOKEN=fake_worker_jwt(session_id)`(sessions.rs:521-525) | **已修** | 新增测试 `initial_master_spawn_env_contains_process_identity`(sessions.rs:1514)断言初次 master plan 的 env 含 `USE_GATEWAY=1` 且 `fake_jwt_worker_id(token)==session_id`。JWT worker_id==session_id==`register_master` 通道 id → master 二防线运行期一致对上 |
| **§四.3**(symlink→/mnt/c 缺测试) | 新增生产守卫 `symlink_target_is_wsl_windows_mount`(canonicalize 失败时 `read_link` 兜底判 /mnt/c,claude_gateway.rs:705-716)+ 测试 `addendum_wsl_guard_skips_writeback_when_symlink_targets_windows_mount` | **已补(且强于最低要求)** | 测试造 symlink→`/mnt/c/...`、断言 writeback 跳过(Ok)+ symlink 仍在 + 无 `.tmp`。生产侧比原实现多一层断链兜底(canonicalize 失败也能拦) |
| **S2**(孤儿 `symlink_auth_file`) | 整函数删除(home_layout.rs -25) | **已删** | grep 确认(`symlink_auth_file_checked` 另一函数保留,仍在用) |

### 回滚自检(铁律,实跑;逐处修复→对应测试变红)

临时把三处修复各自回退,重编译定向重跑,确认对应测试如期**变红**;复原后树净:

| 回退动作 | 目标测试 | 结果 |
|---|---|---|
| wrapper 尾串改回 `exec sh -lc "$1" sh {inner}`(复现原 bug) | `ac_bridge_wrapper_executes_inner_with_gateway_base_url` | **RED**:inner marker 未生成,`read_to_string(inner_marker).unwrap()` panic(:289) |
| 删 `build_master_spawn_env_vars` 的两条 insert | `initial_master_spawn_env_contains_process_identity` | **RED**:`USE_GATEWAY` left=`Some("0")` right=`Some("1")`(:1559) |
| symlink 守卫改回 `canonicalize().unwrap_or_else(raw)` | `addendum_wsl_guard_skips_writeback_when_symlink_targets_windows_mount` | **RED**:断链 symlink → 读原文件 Err → `.unwrap()` panic(:374) |

三处均变红 → **新补测试真实锚定被修行为,非空转**。复原后 `git diff HEAD` 空(树净,HEAD=`6e67f31`)。

### 全绿实证(rework 后)

```
$ cargo test --test gateway_graft_acceptance      # 17 passed; 0 failed (原 15 + 2 新)
  ... ac_bridge_wrapper_executes_inner_with_gateway_base_url ... ok
  ... addendum_wsl_guard_skips_writeback_when_symlink_targets_windows_mount ... ok
$ cargo test --lib initial_master_spawn_env_contains_process_identity   # 1 passed
```

### 残留(非阻塞,advisory)

- **A1 · S1 未处理(整仓 fmt-churn 仍在 base commit `9c9f636`)**:rework commit 自身干净,但基线 commit 仍把整仓 `cargo fmt` 与网关改动混在一起。**建议 operator 合并前把 `9c9f636` 的 fmt-only churn 与网关改动拆开**(或至少在 PR 描述里标注),以免 diff 污染。不阻塞功能。
- **A2 · unsafe_no_sandbox 边角**:`build_master_spawn_env_vars` 现无条件设 `USE_GATEWAY=1`;在 `unsafe_no_sandbox` 开发逃生路径下,master 会带 `USE_GATEWAY=1`+假 JWT 但无桥/无 BASE_URL(wrapper 门需 `AH_CLAUDE_GATEWAY_HOST_UDS`,该路径不注入)。这与 worker 侧 `home_layout` 一贯行为对称,且 unsafe 路径本就绕过凭据语义、非设计目标形态。**记为观察,不阻塞**;若将来要在 unsafe 模式跑真 claude 需另处理。
- **Kill List**:rework 未触碰任何 kill-list 项(桥仍为 `copy_bidirectional` 纯 Rust,无 python/HMAC/固定端口回潮);无需重验。

### 结论与交接

**ACCEPT。** 嫁接的运行时管道现已在 worker 与 master 两条路径端到端接通(实证),核心与运行时层测试均真实(回滚自检全数变红)。建议 master 可 `git push` 触发 CI(全量门);合并前建议按 A1 处理 fmt-churn。**本审仍不 push、不开 PR——PR 开/合与 git push 归 operator。** worktree 已复原为净树(仅 c1 遗留 `.operator-question` 未跟踪)。

*r1-claude 复审。verdict: REJECT(9c9f636)→ **ACCEPT**(6e67f31)。逐处修复经回滚自检实证锁定,原 REJECT 实证手法不再复现。PR 开/合与 git push 归 operator。*

---
---

# CI 修复 delta 审(PR #146 shipping tip)· `6e67f31..aabd45d`(2026-07-12)

> 保留上方历史不动。本节审 c1 针对 PR #146 CI 红的两 commit:`c46cd10`(Windows cfg)、`aabd45d`(CI test expectations)。delta = 3 文件 +66/-1。

## delta 裁决:**REJECT**

三处改动里 **2 处正确**(manifest 断言收严、master cutover seed fixture),但 **Windows cfg 修复不完整**——`windows-msvc-check` 仍会红,只是从原来的 `GatewayListener/register_worker` 报错**换成** `run_internal_bridge` 报错。delta 的既定目标(修 CI 红)未达成,故 REJECT。

### D-BLOCK-1 · Windows cfg 修复不完整:`run_internal_bridge` 仍会在 windows 编译失败

- **已正确的部分**:c1 给 `GatewayListener` 补 `#[cfg(not(unix))]` 单元 struct + `shutdown` 空实现,给 `register_worker` 补 non-unix fail-closed stub(返回 `Unsupported`)。签名与 unix 版一致,`ClaudeGatewayService`(非 gated,lib 内逐连接引用)因此能在 windows 通过类型检查。**我用一个无 C 依赖的最小 crate 交叉 check 到 `x86_64-pc-windows-msvc` 实证:这种"cfg(unix)+non-unix stub"模式在 windows 上正常解析,无报错。**
- **遗漏(阻塞)**:`run_internal_bridge`(claude_gateway.rs:943-944)是 **`#[cfg(unix)]` 独此一份、无 non-unix stub**;而 `src/bin/ah.rs:275` 的 `Cmd::InternalBridge` 分支(**未 cfg-gate**,ah.rs:57 变体 + :274 arm 均无 cfg)无条件调用它。CI 的 windows job 跑 `cargo check --all-targets --target x86_64-pc-windows-msvc`(ci.yml:92)——`--all-targets` **含 bins**,`ah` 是无 target 门控的 `[[bin]]`(Cargo.toml:72-73)。
- **为何原 CI 只报 `GatewayListener/register_worker`、没报这个**:cargo 先编 lib(bins 依赖 lib);原来 lib 就因 `GatewayListener/register_worker` 编译失败 → cargo 中止、根本没走到 `ah` bin 的检查 → `run_internal_bridge` 报错被**前序 lib 失败掩盖**。c1 修好 lib 后,lib 能编了,cargo 继续检查 `ah` bin → **此前被掩盖的 `run_internal_bridge` not-in-scope 现在会浮现**。
- **实证(在没有 Windows 环境下也拿到了硬证据)**:
  1. 本机 `x86_64-pc-windows-msvc` target std 已装;直接跑 CI 原命令 `cargo check --all-targets --target x86_64-pc-windows-msvc` 卡在**依赖** `ring` 的 build.rs(缺 `lib.exe`)——与 c1 所述缺 MSVC 一致,走不到 `ah` crate 自身类型检查阶段。
  2. 遂建一个**无 C 依赖**的最小 crate,忠实复刻两种模式,交叉 check 到同一 windows target:`register_worker` 式(有 non-unix stub)→ **通过**;`run_internal_bridge` 式(cfg(unix) 独份 + 被无门控 bin 调用)→ **`error[E0425]: cannot find function run_internal_bridge` ... note: the item is gated here `#[cfg(unix)]`**。这正是真实 windows runner(有 `lib.exe`、能编过 ring 与 lib)在检查 `ah` bin 时会撞上的同一报错。
- **期望行为**:windows-msvc-check 绿(`ah` bin 在 windows 能通过 `cargo check`)。
- **修点方向(二选一)**:① 给 `run_internal_bridge` 补 `#[cfg(not(unix))]` stub 返回 `Err(Unsupported)`(与 `register_worker` 对称,最小改动);② 把 `ah.rs` 的 `InternalBridge` 变体(:57)与 match arm(:274)一起 `#[cfg(unix)]` 门控。补完后重跑 `cargo check --all-targets --target x86_64-pc-windows-msvc`(真机 CI)确认绿。

### D-OK-1 · manifest `test_collect_spawn_env_precedence` 断言更新 —— 方向正确(收严贴合冻结设计),非弱化 ✅(重点审项)

- **旧断言**:`env.contains(ANTHROPIC_API_KEY="host-key")`——期待**宿主凭据透传**。这是网关前的旧(不安全)行为。
- **新断言**:host `ANTHROPIC_API_KEY`/`AUTH_TOKEN`/真 `BASE_URL` **均被剥离**(不在 env);`extra_env` 里**仅**放行合法假 JWT(`fake_jwt_worker_id==worker-a`)+ localhost gateway URL(`http://localhost:49152`);`CCB_CLAUDE_MD_MODE` extra 覆盖 host(precedence 不变)。
- **对照冻结契约**:精确匹配 **分歧点4契约2**(`collect_spawn_env` 剥离宿主 `ANTHROPIC_*`;extra_env 仅放行 `fake_jwt_worker_id(value).is_ok()` 与 localhost base_url)+ **K7**(禁宿主 `ANTHROPIC_*` 透传)+ **K11**(BASE_URL 改 host=localhost 前缀校验)。**方向是"往更严格贴合冻结设计"改,不是为过 CI 弱化断言**——旧断言本就编码了被网关设计废弃的旧行为,生产改为剥离后旧断言正当地红,c1 把断言更新到新契约。
- **非空转实证(回滚自检)**:临时把生产 `is_claude_gateway_blocked_host_env` 改成恒 `false`(停止剥离)→ `test_collect_spawn_env_precedence` **RED**:`!env.iter().any(|(k,_)| k=="ANTHROPIC_API_KEY")` 断言失败(manifest.rs:826)。→ 新断言真实锚定剥离行为。复原后树净。linux 上该测试 **green**。

### D-OK-2 · master cutover seed fixture —— 合法的必要前置,非弱化断言 ✅

- `seed_claude_credentials(&source_home)` 在 3 个 master cutover 测试的 `HOME/.claude/.credentials.json` 写入一份**未来到期**(`expiresAt:4102444800000`,约 2100 年)的合法 seed。master spawn 现会触发网关 lazy seed 读取(`register_master`→`core()`→`read_seed_credentials`),无 seed 则 spawn 报错。这是**补充新契约所需的前置 fixture**,测试原有断言(seed-before-spawn 顺序、metadata 写入)**未改动**,不构成弱化。未来到期避免触发刷新,契合"测试不打真上游"。
- **实证**:`daemon_master_cutover_orders_seed_before_spawn_and_writes_metadata` 与 `test_collect_spawn_env_precedence` 在 linux 上均 **green**(2 passed)。

### 结论与交接

**REJECT(delta `aabd45d`)。** 唯一阻塞是 D-BLOCK-1:Windows cfg 修复漏了 `run_internal_bridge`,`windows-msvc-check` 仍会红(报错点前移,非消除)。manifest 断言收严方向正确且经回滚自检证实非空转;master seed fixture 合法。**请 c1 按 D-BLOCK-1 修点方向补第三处(建议方案①,与 `register_worker` 对称),重跑真机 windows-msvc-check 确认绿后回审。**

> **给 operator 的提醒**:auto-merge 需 CI 全绿才触发;按本审判断 windows-msvc-check 仍会红,故即便挂了 auto-merge 也不会误合——但需要 c1 再补一处才能让 CI 转绿。本审不 push、不开 PR。worktree 已复原净树(`git diff HEAD` 空,HEAD=`aabd45d`,仅 `.operator-question` 未跟踪)。

*r1-claude · CI 修复 delta 审。verdict: **REJECT**(`aabd45d`)——Windows cfg 修复不完整(`run_internal_bridge` 无 non-unix stub,`ah` bin 仍会在 windows check 失败,已用最小 crate 交叉 check 到 windows-msvc 实证 E0425);manifest 断言收严正确(回滚自检非空转)、master seed fixture 合法。PR 开/合与 git push 归 operator。*

---
---

# D-BLOCK-1 修复复审 · `aabd45d..a24fafa`(2026-07-12,PR #146 CI 修复末关)

> commit `a24fafa`(已 push origin)只加 8 行:给 `run_internal_bridge` 补 `#[cfg(not(unix))]` stub。审这一处是否闭合上一节 D-BLOCK-1。

## 裁决:**ACCEPT**(D-BLOCK-1 已闭合)

### 逐点复核(读码 + 交叉 check 实证)

1. **签名/返回/cfg 属性与既有 stub 对称** ✅
   - unix(claude_gateway.rs:944)`pub async fn run_internal_bridge(uds_path: &Path, port_file: Option<&Path>) -> io::Result<()>`;non-unix(:963)`pub async fn run_internal_bridge(_uds_path: &Path, _port_file: Option<&Path>) -> io::Result<()>`——**签名/返回类型逐字一致**,仅未用参数加 `_` 前缀(避免 unused 告警),`pub async fn` 一致。stub 体 `Err(io::Error::new(io::ErrorKind::Unsupported, ...))` 与 `register_worker` 那处**同一手法**。
   - 调用点 `ah.rs:275` `run_internal_bridge(&uds, port_file.as_deref()).await.map_err(CliError::Io)`——`.await` 得 `io::Result<()>`、`.map_err` 得 `CliError`,类型链在 windows 分支同样成立,无新的类型不匹配。

2. **`run_internal_bridge` 是最后一处未门控的 cfg(unix) 引用** ✅
   - claude_gateway.rs 里 **pub 的 cfg(unix) 项**仅三处:`GatewayListener`、`register_worker`、`run_internal_bridge`——现**三处均有 non-unix sibling**。其余 cfg(unix)-only 项(`handle_gateway_connection`/`read_http_request`/`write_http_response`/`drain_oversized_headers`/`wait_for_uds_ready`/`bridge_one`)全是 **private**、只被其它 cfg(unix) 函数内部调用,windows 下一并缺席、无悬空引用。`bridge_wrapper_shell` 非 gated(纯字符串,不引用 unix 类型)、且只被 linux-gated 的 scope.rs 调用。
   - **bin 侧引用**:ahd.rs 用非 gated 的 `ClaudeGatewayService`(windows 可编);ah.rs:275 的 `run_internal_bridge` 现已解析。→ 无残留。

3. **交叉 check 实证(两向)** ✅——本机 `x86_64-pc-windows-msvc` target std 可用,`cargo check` 不需 MSVC 链接器:
   - 真实 `cargo check --all-targets --target x86_64-pc-windows-msvc` 仍卡在**依赖** `ring` 的 build.rs(缺 `lib.exe`),走不到 `ah` crate 自身——与 c1、与上一节同一环境限制(**真机 windows-msvc-check 有 lib.exe,不受此限**)。
   - 遂复用最小复刻 crate(无 C 依赖、bin 无条件引用、lib 提供 unix/non-unix 双实现):
     - **上一节**(无 stub)→ `error[E0425]: cannot find function run_internal_bridge ... gated here #[cfg(unix)]`(RED)。
     - **本节**(照 a24fafa 加 non-unix stub)→ `cargo check --all-targets --target x86_64-pc-windows-msvc` **Finished,exit 0**;另用 `.await.map_err(...)` 链形复刻 ah.rs:275 亦 **exit 0**。
   - 两向对照 = stub 对 windows 编译**是承重的**:去掉→E0425,加上→绿。E0425 模式已消除。

4. **不影响 linux** ✅:delta 仅 +8 行 cfg(not(unix)) 块,unix 版 `run_internal_bridge`(:944)一字未动;linux 下新块被 cfg 编译掉,行为逐字节不变。上一节已验的 17 acceptance + master-env + 2 ubuntu 测试不受影响。

### 诚实边界
我**仍无法**在本机跑完整真机 `cargo check --all-targets --target x86_64-pc-windows-msvc`(ring/`lib.exe` 阻塞,同 c1)。本 ACCEPT 依据:①签名对称读码核实;②在**真实 windows-msvc target** 上用最小 crate 双向实证该修复模式消除 E0425;③静态穷举确认 `run_internal_bridge` 是最后一处未门控 cfg(unix) 引用。这是无 MSVC C 工具链下能给出的最强验证;**最终以真机 windows-msvc-check(有 lib.exe)为准**——按上述证据我判其应转绿。

### 残留(advisory,非阻塞,不影响本 ACCEPT)
- **A1(重申)**:整仓 `cargo fmt` churn 仍在基线 commit `9c9f636`。纯格式、CI 安全;建议 operator 合并/压缩时知悉,或让 c1 后续拆分。不阻塞合并。

### 结论与交接
**ACCEPT。** D-BLOCK-1 闭合,PR #146 本轮三处 CI 破坏(round1 lib 两符号、round2 遗漏的 bin 符号、两 ubuntu 测试)据本审证据均已修。commit `a24fafa` 已 push origin。**本审不 push、不开 PR、不 merge**;worktree 净树(`git diff HEAD` 空,HEAD=`a24fafa`,仅 `.operator-question` 未跟踪)。**可以等 CI 结果:CI 全绿后 auto-merge 自动合,不需人工 merge**;唯一诚实保留是我无法本机复现真机 windows job 的完整链接前编译(依赖 C 工具链),但已证该修复消除已知 E0425,预期绿。

*r1-claude · D-BLOCK-1 修复复审。verdict: **ACCEPT**(`a24fafa`)。stub 签名/返回/cfg 与 register_worker 对称、类型链成立;`run_internal_bridge` 确为最后一处未门控引用;最小 crate 交叉 check 到 windows-msvc 双向实证 E0425 消除;linux 逐字节不变。PR 开/合与 git push 归 operator。*

---
---

# CI round4 修复复审 · `a24fafa..0b450be`(2026-07-12,test job 全量套件回归)

> commit `0b450be`(已 push origin)修 CI 全量 `cargo test --lib` 暴露的 3 处回归。此前各审只跑定点测试、未跑过全量,故这些是全量套件才现形的问题。delta = 3 文件 +38/-1。

## round4 裁决:**ACCEPT**

三处改动逐条经回滚自检式实证,均正确且必要。全量套件我**亲跑复核**:`1052 passed, 0 failed, 3 ignored`(59s,`--test-threads=1`,复核 c1 自报无误)。

### R4-1 · handlers.rs seed fixture(`test_handle_session_spawn_master_pane_uses_isolated_claude_home`)—— 合法必要前置 ✅

- 该测试 `set HOME=host_home` 后调 `handle_session_spawn_master_pane` → 触发网关 `register_master` → **lazy 读 host seed**(`HOME/.claude/.credentials.json`)。无 seed 则 `SandboxMountFailed` → spawn 失败。
- 补的 `seed_claude_credentials` 正是**冻结补丁裁决2** 的前置:seed 来源 = host `~/.claude/.credentials.json`,首个 claude 席位 spawn 时 lazy 读取。这是**提供设计要求的前置 fixture,非绕过检查**(与已 ACCEPT 的 D-OK-2 master cutover seed 同一类)。测试自身已 save/restore HOME(`old_home`)。

### R4-2 · systemd.rs 断言更新(`test_wrap_command_injects_passthrough_and_forced_env`)—— 与 manifest 同一剥离机制,且经回滚自检确认在此 code path 生效 ✅(重点审项)

- 该测试用 `get_manifest("claude")` + `wrap_command(...)`,是**冻结设计宿主凭据剥离**在 `wrap_command`→`command_with_env_prefix`→`collect_spawn_env` 端到端路径上的体现;剥离逻辑就是与 D-OK-1 **同一个** `is_claude_gateway_blocked_host_env`。旧断言期待 `ANTHROPIC_API_KEY=host-anthropic` 透传(网关前旧行为),新断言改为**不得泄漏进 sandbox command**——方向是收严贴合冻结契约(分歧点4契约2 / K7),非弱化。
- **回滚自检(实跑)**:临时把 `is_claude_gateway_blocked_host_env` 改恒 `false`(停剥离)→ **两测试同时 RED**:
  - `test_collect_spawn_env_precedence` 失败(manifest.rs:827);
  - `test_wrap_command_injects_passthrough_and_forced_env` 失败(systemd.rs:716),失败信息实证泄漏:`env ANTHROPIC_API_KEY=host-anthropic ANTHROPIC_AUTH_TOKEN=host-token ANTHROPIC_BASE_URL=https://api.anthropic.com ...` 进了 sandbox command。
  - → 二者共享同一剥离机制,且该机制在 systemd `wrap_command` 路径**确实生效**(不是各写各的、也非空转)。复原后树净。

### R4-3 · sessions.rs HOME restore(c1 主动发现的潜藏跨测试污染)—— 经实证**必要**,非防御性冗余 ✅

- 机制:3 个 master cutover 测试 `set HOME=source_home`(tempdir)后**旧代码不 restore**;测试结束 `tmp` TempDir drop → source_home **被删** → HOME 悬指已删目录 → 后续读 HOME 的测试(网关 lazy seed 读)拿到陈旧/已删路径 → 失败。这在"全量套件同进程顺序执行"下才现形(此前只跑定点测试从未触发)。
- **回滚自检(实跑,决定性)**:把 `restore_env` 改为 no-op(模拟 pre-fix「set 不 restore」)→ 全量 `cargo test --lib -- --test-threads=1` **RED**:
  ```
  test_handle_session_spawn_master_pane_uses_isolated_claude_home ... FAILED
  SandboxMountFailed { details: "Claude seed credentials not found on host at
    /tmp/.tmpd5ou42/source-home/.claude/.credentials.json ...; run /login" }
  1051 passed; 1 failed
  ```
  失败路径 `.../source-home/...` 正是 cutover 测试的 `source_home` 命名 → **确证跨测试 HOME 污染**:cutover 测试遗留的 HOME 让后一个 spawn-master 测试去已删的 source-home 找 seed。
- 加回 restore(HEAD 原状)→ 全量 **1052 passed**。→ **change 3 是承重修复,不改则全量套件在 CI 的 `--test-threads=1` 下确定性变红**(不只是"某并行度下 flaky",在 CI 自身配置下就红)。R4-1 引入的 seed 读取正是让该污染致命的原因,故 R4-1 与 R4-3 相互咬合、都必要。

### 残留(advisory,非阻塞)

- **B1**:R4-3 用「await 后手动 restore」而非 RAII Drop guard;若某 cutover 测试在 restore 前 panic,HOME 不会被恢复(但那时该测试已失败)。绿路径无影响;若日后想更稳,可换 `TestHomeEnv` 那种 Drop guard(gateway_graft_acceptance.rs 已有先例)。不阻塞。
- **A1(重申)**:整仓 `cargo fmt` churn 仍在基线 `9c9f636`,纯格式、CI 安全,合并/压缩时知悉即可。

### 结论与交接

**ACCEPT。** 三处均正确且必要:R4-1 是设计要求的 seed 前置;R4-2 是同一剥离机制在 systemd 路径的收严断言(回滚自检双红实证);R4-3 是经实证必要的跨测试 HOME 污染修复(回滚自检全量变红实证)。全量 `cargo test --lib` 我亲跑 **1052 passed / 0 failed / 3 ignored**。commit `0b450be` 已 push origin。本审不 push、不开 PR、不 merge;worktree 净树(`git diff HEAD` 空,HEAD=`0b450be`,仅 `.operator-question` 未跟踪)。

**windows-msvc-check 上轮已判绿(D-BLOCK-1 闭合)、本轮修的是 test job 全量套件——两条 CI 破坏线据本审证据均已闭合。可以等真实 GitHub Actions 结果:CI 全绿后 auto-merge 自动合,不需人工 merge。** 唯一诚实保留仍是无法本机复现真机 windows job 的 C 工具链链接前编译(见上一节),test job 全量则已本机 1:1 复现绿。

*r1-claude · CI round4 复审。verdict: **ACCEPT**(`0b450be`)。R4-1 seed 前置合规(补丁裁决2);R4-2 与 D-OK-1 同一剥离机制、回滚自检双红证其在 systemd 路径生效;R4-3 HOME 污染修复经回滚自检全量变红证其必要;全量 1052 passed 亲跑复核。PR 开/合与 git push 归 operator。*

---
---

# CI round5 复审 · `0b450be..eba314a`(2026-07-12,真机 CI 抓出的 seed-home 捕获时序 bug)

> **前情勘误(诚实标注)**:上一节我的 round4 "全量 1052 passed" 是在**我沙箱真实 `$HOME` 下有遗留 credentials** 的脏环境跑的,这**掩盖**了本节 bug——真机全新 runner(`/home/runner` 下无 credentials)照了出来。本轮我改用**真正空 HOME**(`mktemp -d`,已核实无 `.claude/.credentials.json`)复核,不再依赖沙箱脏状态。commit `eba314a`(已 push origin),delta = 1 文件 +25/-10。

## round5 裁决:**ACCEPT**

### 根因确认(读码实证,非采信自报)
- `test_ctx()`(handlers.rs)构造 `Ctx { ..., claude_gateway: Arc::new(ClaudeGatewayService::new()) }`;`ClaudeGatewayService::new()`→`default_seed_path()`**在构造时**读 `std::env::var_os("HOME")` 并 join `.claude/.credentials.json`——**seed path 在构造那一刻定死**。
- 旧顺序:先 `test_ctx()`(此刻 HOME=真实进程 HOME)→ 再 `set HOME=fake`。故网关捕获的是**真实进程 HOME**,不是测试想要的 fake home。本地脏 HOME 下恰有 credentials → 蒙混过关;CI 干净 runner 下 `/home/runner/.claude/.credentials.json` 不存在 → 暴露。

### 修法验证:是根因修复,非碰巧绕过 ✅
- 新顺序(handlers.rs:571):先建 fake host_home + 写 seed → `EnvGuard::set("HOME", host_home)` / `XDG_CACHE_HOME` → **再** `test_ctx()`。于是 `ClaudeGatewayService::new()` 在 fake HOME 已就位后构造,`default_seed_path()` 捕获 **fake host_home** → seed 命中。**顺序调整精确对准"构造时捕获 HOME"这一根因**。

### EnvGuard panic-safety:执行实证,不只信注释 ✅
- 逐字复刻 `EnvGuard`(`set` 先存旧值再 set,`Drop` 调 `restore_env`)到最小 crate,`std::panic::catch_unwind` 内 set 后**故意 panic**:
  - panic 后该 env var **恢复为原值**(Drop 在 unwind 时执行);
  - 对"之前未设置"的 key,Drop **正确 remove**(不残留)。
- → EnvGuard 真正 panic-safe(比 round4 的 await 后手动 restore 更稳,上节 B1 advisory 就此闭合)。`restore_env` 在 handlers.rs:274 存在,`EnvGuard::drop` 引用可解析、编译通过。

### 空 HOME 复现(按要求,不依赖沙箱脏状态)——三向实证
1. **pre-fix RED**:checkout `0b450be` 的 handlers.rs,空 HOME(`/tmp/tmp.m3zg5L9zu5`,核实无 creds)跑定点测试 → **FAILED**:`SandboxMountFailed { ... "Claude seed credentials not found on host at /tmp/tmp.m3zg5L9zu5/.claude/.credentials.json ... run /login" }`。seed 路径 = **空进程 HOME**,与 CI 报的 `/home/runner/.claude/.credentials.json` **同一模式**——CI 失败 1:1 复现。
2. **fix GREEN(定点)**:HEAD `eba314a`,同样空 HOME → 定点测试 **ok(1 passed)**。
3. **fix GREEN(全量)**:HEAD `eba314a`,空 HOME(`/tmp/tmp.AFHQWZca72`,核实无 creds)`cargo test --lib -- --test-threads=1` → **1052 passed,0 failed,3 ignored**(60s)。全量在干净 HOME 下无其它同类 HOME 依赖漏网。

### 结论与交接
**ACCEPT。** 根因(构造时捕获 HOME)定位准确,修法(fake HOME 前置于 test_ctx)正对根因;EnvGuard panic-safe 经执行实证;CI 失败在**真正空 HOME** 下 pre-fix 复现红、fix 后定点+全量皆绿(1052)。我 round4 的验证盲区(脏 HOME)本轮已用干净 HOME 纠正。commit `eba314a` 已 push origin。本审不 push、不开 PR、不 merge;worktree 净树(`git diff HEAD` 空,HEAD=`eba314a`,仅 `.operator-question` 未跟踪)。

**两条 CI 破坏线状态**:① windows-msvc-check —— round3 D-BLOCK-1 闭合,真机 CI 已绿(operator 确认);② test job 全量套件 —— round4 收严/污染修复 + 本轮 seed-home 时序修复,现于**干净空 HOME**(匹配 CI runner 条件)本机复现绿。**这是本轮 PR #146 CI 修复链条的终点,可以放心等真实 GitHub Actions 结果;CI 全绿后 auto-merge 自动合,不需人工 merge。**

*r1-claude · CI round5 复审。verdict: **ACCEPT**(`eba314a`)。根因=构造时捕获 HOME,修法顺序调整正对根因;EnvGuard panic-safety 执行实证;空 HOME 三向实证(pre-fix 复现 CI 红、fix 定点绿、fix 全量 1052 绿),纠正 round4 脏 HOME 盲区。PR 开/合与 git push 归 operator。*

---
---

# CI round6 复审 · `eba314a..6974a8f`(2026-07-12,集成测试 tests/*.rs 契约 + UDS 路径 bug)

> 真机 CI 在 `eba314a` 报集成测试失败(前几轮全量只跑 `cargo test --lib`,未覆盖 `tests/*.rs`):`ah_config_drift.rs provider_auth_files_are_symlinks...`——旧断言要求 `.claude/.credentials.json` 是 symlink,正是 K7 要铲的旧行为。c1 修 3 处 + 额外主动加了一处 UDS 路径 bug 修复。delta = 4 文件 +86/-24。

## round6 裁决:**REJECT**

**契约更新两处(改动 1/2)正确、绿;但 c1 额外加的 UDS 路径短化(改动 3)在修一个真 bug 的同时引入了一个新的运行时回归——网关桥 `sandbox_root` 派生被打断,`bridge.err`/`bridge.port` 会落到 `/`(沙箱内不可写 + 所有 worker 撞同一路径)。且该回归无任何测试覆盖(CI 会绿,真跑会挂——与最初 R1-BLOCK-1 同类)。整包 REJECT,退回改动 3。**

### R6-OK-1 · ah_config_drift.rs 契约更新 —— 方向正确、codex 无误伤、非空转 ✅

- claude 段旧断言(sandbox `.credentials.json` 是 symlink + 追踪 host 刷新)→ 新断言:`!sandbox_claude.exists()` + host seed 只留 host HOME + `CLAUDE_CODE_USE_GATEWAY==1` + `fake_jwt_worker_id(AUTH_TOKEN)=="worker"` + `ANTHROPIC_BASE_URL` none + host 刷新后 sandbox 仍无凭据。**精确贴合 K6/K7(claude 沙箱零真实凭据、gateway 供 token),是真断新契约,非删断言空转**(风格同 `gateway_graft_acceptance` 的 `ac_zero_credentials`)。
- **codex 无误伤**:codex 段仍断言 `sandbox_codex` 是 symlink + `read_link==host_codex` + host 刷新传导(未改一行;diff 里 codex 相关只多了一句注释)。符合 K7 仅针对 claude、codex `.codex/auth.json` 仍在 `PROVIDER_AUTH_WHITELIST`。**实跑:`ah_config_drift` 5 passed。**

### R6-OK-2 · mvp12_home_layout.rs 契约更新 —— 同族同向、非空转 ✅

- 同样把 claude symlink 断言换为"沙箱无 `.credentials.json` + host seed 留 host + `USE_GATEWAY=1` + fake JWT worker_id + 无 BASE_URL"。方向正确。**实跑:`mvp12_home_layout` 7 passed。**

### R6-OK-3(部分)· UDS 路径长度 bug 真实存在 ✅——但修复方式见 R6-BLOCK

- **bug 真实**:旧 `host_uds = worker_sandbox_root.join("tmp/ah-gateway.sock")` 把 UDS 放进沙箱根下;沙箱根长时超 Unix socket `sun_path` **108 字节**硬限(`sys/un.h`)。实证:回归测试的 long root(180 个 `a`)下旧路径 = **231 字节 > 107** → `bind()` `ENAMETOOLONG`。故"短化 socket 路径"这个**动机成立**,`gateway_graft_acceptance` 新回归测试(18 passed,含 `ac_gateway_host_uds_path_stays_short_for_long_sandbox_root`)对 socket 路径本身的断言也没问题。

### R6-BLOCK · 改动 3 引入运行时回归:网关桥 `sandbox_root` 派生被打断 ❌(阻塞)

- **文件/行**:`src/claude_gateway.rs:340` 新 `short_host_uds_path` 返回 `std::env::temp_dir().join("ah-gw-<hash16>.sock")`(即 `/tmp/ah-gw-XXXX.sock`);但 **`src/platform/linux/scope.rs:448-452` `claude_gateway_bind_context` 未随之更新**——它仍以 `AH_CLAUDE_GATEWAY_HOST_UDS.parent().parent()` 派生 `sandbox_root`。
- **断裂链**:`AH_CLAUDE_GATEWAY_HOST_UDS` = `topology.host_uds_path`(agent.rs:193 / sessions.rs:497 / master_watch.rs:812 都这么设),现在 = `/tmp/ah-gw-XXXX.sock`。`parent().parent()` = `/tmp` → **`/`**。该 `sandbox_root` 正是喂给 `bridge_wrapper_shell` 写 `bridge.err`/`bridge.port` 的目录(scope.rs:441/508 → claude_gateway.rs:970-971 `sandbox_root.join("bridge.err"/"bridge.port")`)。
- **后果(两重,均严重)**:
  1. `bridge.err`=`/bridge.err`、`bridge.port`=`/bridge.port`——沙箱内以 `sevenx`(`--user --scope`)身份**无权写 `/`** → `2>/bridge.err` 重定向失败 / `--port-file /bridge.port` 写失败 → 端口文件永不出现 → wrapper `exit 126` **fail-fast**,**worker 与 master 的网关桥都起不来**(与最初 R1-BLOCK-1 同类失效)。
  2. 即便 `/`(或某 `TMPDIR` 的父目录)可写,`sandbox_root` 对**所有 worker 恒为同一路径** → 全部 worker 争抢**同一个** `/bridge.port` → 跨 worker 端口串台,per-worker 隔离被打破。
- **实证(路径算术,确定性)**:`dirname $(dirname /tmp/ah-gw-abcdef0123456789.sock)` = **`/`**(对照旧路径 `dirname×2` = `/home/.../sandboxes/<uuid>` 正确的 per-worker 目录)。
- **为何测试全绿却仍是回归**:`gateway_graft_acceptance` 的 wrapper 端到端测试(`ac_bridge_wrapper_executes_inner...`)传的是**显式 sandbox_root**,不走 scope.rs 的派生;新回归测试只断言 socket 路径长度;**没有任何测试覆盖 `claude_gateway_bind_context` 的派生**。故 CI 会绿、真跑(真起沙箱 worker)才挂——这正是需要对抗式审核而非只信 CI 绿的地方。
- **修点方向**:短化 socket 路径**保留**(bug 真实);但必须把 `sandbox_root` 与被短化的 UDS 路径**解耦**——例如 agent.rs/sessions.rs/master_watch.rs 在设 `AH_CLAUDE_GATEWAY_HOST_UDS` 的同时另设 `AH_CLAUDE_GATEWAY_SANDBOX_ROOT = worker_sandbox_root`,`claude_gateway_bind_context` 改读后者(不再 `parent().parent()`);或让 `bridge.err`/`bridge.port` 落在别的、仍知道沙箱根的来源。补一个**真跑 wrapper 派生路径**的端到端测试(断言 bridge.err/port 落在 per-worker 沙箱目录、非 `/`),这样此类回归下次会被测试拦住。

### 结论与交接

**REJECT(delta `6974a8f`)。** 改动 1/2 契约更新正确且绿(codex 无误伤),UDS 长度 bug 真实、短化动机成立——**这些请保留**;唯一阻塞是改动 3 未同步更新 `claude_gateway_bind_context` 的 `sandbox_root` 派生,导致网关桥 `bridge.err`/`bridge.port` 落到 `/`(沙箱不可写 + 跨 worker 撞车),且无测试覆盖。**请 c1 按 R6-BLOCK 修点解耦 sandbox_root 并补端到端派生测试后回审。** 本审不 push、不开 PR;worktree 净树(`git diff HEAD` 空,HEAD=`6974a8f`,仅 `.operator-question` 未跟踪)。

> **给 operator 的提醒**:本轮 CI(集成测试)按证据**会绿**——因为回归落在无测试覆盖的运行时桥派生上。**CI 绿 ≠ 可合**:改动 3 会让真机上 claude worker/master 的网关桥 fail-fast(exit 126)。建议**不要因 CI 转绿就 auto-merge**,先退回 c1 修 R6-BLOCK。

*r1-claude · CI round6 复审。verdict: **REJECT**(`6974a8f`)。改动 1/2(K7 契约更新)正确、绿、codex 无误伤;UDS 108B 长度 bug 真实(231B 实证);但改动 3 短化 UDS 后未更新 `claude_gateway_bind_context`,`sandbox_root` 派生成 `/` → 桥 `bridge.err/port` 沙箱内不可写 + 跨 worker 撞车,且无测试覆盖(CI 会绿真跑会挂)。退回改动 3,解耦 sandbox_root + 补端到端派生测试。PR 开/合与 git push 归 operator。*

---
---

# POST-MERGE HOTFIX 复审 · `8f2aab5..f635902`(2026-07-12,R6-BLOCK 回归修复)

> **背景(流程洞,operator 已记观察 #55)**:R6 的 REJECT 来得太晚——CI 先绿、auto-merge 抢跑,把含 R6-BLOCK 回归(`sandbox_root=/`)的 `6974a8f` 经 merge `8f2aab5` 合进了 main。c1 在新 worktree `ccbd-rust-wt-graft-hotfix`(分支 `fix/gateway-uds-sandbox-root-regression`,基于含回归的 main)出 hotfix `f635902`(已 push origin)。本节审这一个 hotfix commit(5 文件 +150/-26)。

## HOTFIX 裁决:**ACCEPT**(R6-BLOCK 闭合)

修法=引入显式 `AH_CLAUDE_GATEWAY_SANDBOX_ROOT` env,**彻底删除**旧的 `AH_CLAUDE_GATEWAY_HOST_UDS.parent().parent()` 反推,`claude_gateway_bind_context` 改读显式 env。三条 spawn 路径全下发、无第四条漏网、漏发不会静默退回旧错误行为,均经实证。

### 复核点 1 · 三条 spawn 路径全下发新 env,无遗漏,尤其 master revive ✅

- grep 实证:设 `AH_CLAUDE_GATEWAY_HOST_UDS` 的生产站点**恰好 3 处**——worker `agent.rs:193`、初次 master `sessions.rs:497`、**master revive `master_watch.rs:812`**;设 `GATEWAY_SANDBOX_ROOT_ENV` 的也**恰好同 3 处**(`agent.rs:197`/`sessions.rs:501`/`master_watch.rs:816`),且每处都紧贴 HOST_UDS 插入。**没有任何站点只设一个不设另一个**——这正是我 R6-BLOCK 教训"只改一部分接线"的反面,这次两个 env 严格配对。各处值 = 对应 sandbox `dir`/`sandbox_dir`(即 `gateway_worker_topology` 与 bind 用的同一根),正确。
- **master revive 单独核**:`master_watch.rs:815-816` 在 revive 的 sandbox 分支里 `master_env_vars.insert(GATEWAY_SANDBOX_ROOT_ENV, sandbox_dir.display())`——`sandbox_dir` 正是 `gateway_worker_topology(&sandbox_dir, &session_id)` 的入参。已覆盖。

### 复核点 2 · 无第四条漏网 spawn/reconcile 路径 ✅

- **realign**:`spawn_realign_agent`(realign.rs:432)→ `handle_agent_spawn_with_db_action`(451)——复用 worker spawn 路径,自动带新 env。
- **master cutover**:`run_master_cutover_with_spawn` → `prepare_master_pane_plan`(sessions.rs:1069)+ `spawn_prepared_master_pane`——复用初次 master 路径,自动带新 env。
- **daemon 重启 reconcile**(`reconcile_claude_gateway_seats`):只对存活席位重建 host 侧 **listener**(`register_*`),**不重建 spawn 命令**(进程跨 daemon 重启存活),故不经 command-build、不需该 env。
- 结论:所有构建 claude spawn 命令的路径都汇入上述 3 个已覆盖函数,无第四条。

### 复核点 3 · 漏发 env **不会**静默退回旧错误派生(operator 重点)✅——实证

- **旧 `parent().parent()` 反推代码已彻底删除**(grep `parent().*parent()` in scope.rs = 无)。故"退回旧错误派生"在物理上不可能。
- **漏发 env 的行为 = fail-visible,非 silent-wrong**:`claude_gateway_bind_context` = `get(GATEWAY_SANDBOX_ROOT_ENV)?` → 缺则 `None` → `claude_gateway_bridge_shell` → `None` → wrapper gate 不成立 → **不装桥**(claude 无 BASE_URL → 可见失败),**绝不**产生 `/` 路径。
- **回滚自检实证(临时加测试)**:构造含 `USE_GATEWAY=1`+`HOST_UDS=/tmp/ah-gw-x.sock` 但**故意不含** `SANDBOX_ROOT` 的 env,断言 `claude_gateway_bridge_shell(...).is_none()` → **PASS**。即漏发 env 确实退化为"无桥",不是 `/`。

### 复核点 4 · 新端到端测试真的验证"两 worker 不冲突",非单 worker ✅

- `claude_gateway_wrapper_uses_explicit_sandbox_root_not_short_uds_parent`(scope.rs:624)造**两个** worker(root_a/root_b,不同 sandbox root + 不同短 UDS `/tmp/ah-gw-worker-a|b.sock`),各自真跑 wrapper(stub ah 写端口),断言:
  - `shell_a/b` 各含 `root_a/b` 下的 `bridge.port`/`bridge.err`,且 `!contains("rm -f /bridge.port;")`(**直接反 R6-BLOCK 的 `/` 回归**);
  - **`assert_ne!(root_a/bridge.port, root_b/bridge.port)`** —— 这就是"两 worker 不冲突"的关键断言,per-worker 路径互不相同;
  - 两 shell 真跑 `status.success()`,各自 `bridge.port` 读回 `45678`,`out_a/out_b` = `http://localhost:45678`(inner 真执行 + BASE_URL 正确)。
- **回滚自检实证**:把 `claude_gateway_bind_context` 临时改回旧 `parent().parent()` → 该测试 **RED**(`assert shell_a.contains(root_a/bridge.port)` 失败,因 sandbox_root 又变 `/`)→ 证明此测试真能拦住 R6-BLOCK 回归,非空转。

### 全量实证(干净空 HOME,应用 R5 教训)

- 空 HOME(`mktemp -d`,核实无 `.claude/.credentials.json`)`cargo test --lib -- --test-threads=1`:**1054 passed**(c1 报 1053 + 我临时 missing-env 测试 1),0 failed,3 ignored;新 e2e 测试与 missing-env 测试均 ok。移除临时测试后 worktree 净树(`git diff HEAD` 空,HEAD=`f635902`)。

### 结论与交接

**ACCEPT。** hotfix 正确闭合 R6-BLOCK:sandbox_root 与短化 UDS 解耦(显式 env),三条 spawn 路径严格配对下发、无第四条漏网、漏发退化为 fail-visible(非 `/`),新端到端测试真验两-worker 隔离且经回滚自检证明能拦回归;全量在干净 HOME 绿(1054)。commit `f635902` 已 push origin。本审不 push、不开 PR、不 merge。

> **给 operator**:这是 R6-BLOCK 的修复,回归目前**已在 main**(经 `8f2aab5` 合入)。本 hotfix PR 合入后 main 才干净。按你要求:ACCEPT 后你开新 PR、**先不挂 auto-merge**,等 CI 也绿了再手动确认合并——这次不再让 auto-merge 抢跑(流程洞 obs #55)。**注意**:R6-BLOCK 与本 hotfix 的正确性都落在"真起 wrapper 派生路径"上,CI 集成测试**现在有覆盖了**(新 e2e 测试),但真机端到端(真起沙箱 claude)仍无自动化——建议真机冒烟一次确认 claude worker/master 网关桥能起。

*r1-claude · post-merge hotfix 复审。verdict: **ACCEPT**(`f635902`)。显式 `AH_CLAUDE_GATEWAY_SANDBOX_ROOT` 解耦 sandbox_root 与短化 UDS;三 spawn 路径严格配对下发(含 master revive)、realign/cutover 复用、reconcile 不涉、无第四条;旧 parent().parent() 删净→漏发退化为 fail-visible(实证 missing-env→None),非 `/`;新两-worker 端到端测试经回滚自检(改回旧派生→RED)证其拦得住回归;干净空 HOME 全量 1054 绿。PR 开/合与 git push 归 operator。*
