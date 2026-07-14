# REVIEW — Gateway A/B 独立终审裁决(Plan B Fake Gateway)

- **审核席**: r1-claude(A/B 实验唯一 claude 终审,只审不写)
- **裁决问题(唯一)**: 哪一臂**代码质量**更高?(可靠性/成本/交接账本不在此裁决内)
- **权威文档**: `.kiro/specs/ah-per-worker-credentials/design-rev.md`(冻结设计)、`research/ab-experiment-gateway/task-brief-frozen.md`(AC-1~6)、`.kiro/specs/ah-per-worker-credentials/requirements.md`(根因 A/B)
- **被审输入**: 实现 A(worktree `ccbd-rust-wt-gw-a`,HEAD `7f5dc2b`,+2877/-144);实现 B(worktree `ccbd-rust-wt-gw-b`,HEAD `d55c26b`,+1587/-185)
- **实证手段**: 逐文件读源码 + 定向单测实跑(`CARGO_BUILD_JOBS=1`,单测试);未改动任一分支任何字节。

> **一句话结论**:**实现 B 代码质量更高**。B 是干净、地道、CI 全绿、测试锚定可观测契约、且把凭据根漏洞在源头彻底铲除的实现;A 虽然把运行时管道(真 UDS 网关 + 沙箱桥 + 挂载)搭得更全,但代码质量显著更低——树是红的且自相矛盾(团队 14 轮误诊)、约 180 行测试/生产处理器复制粘贴、内联 python heredoc 桥脆弱、生产函数里塞全局 env 变异、自造非标 JWT 签名。**唯一 A 胜 B 的维度是"设计运行时广度"**(A 真把服务器/桥跑起来了,B 只搭了逻辑+env,`localhost:8206` 运行时无人监听)。这条差异我在下方显著标注,合哪条由 operator 定;本裁决只答质量。

---

## 甲、实现 A 逐条 rubric 审计

### A-1. AC-1~6 契约满足度 —— 6/10
- **AC-1 单飞刷新(真实现)**:`CredentialsState::get_valid_token`(`src/provider/claude_gateway.rs:263-330`)用 `refresh_mutex.lock().await` + 双检(`:277-289`)实现单飞——首个抢锁刷新,其余阻塞后双检命中新 token 返回,恰好一次上游刷新。这是设计 watch 伪码的等价替代(未用 watch,但功能正确)。验收测试 `ac1_concurrent_expired_worker_requests_refresh_single_flight`(`tests/claude_gateway_acceptance.rs:37`)对 mock 上游断言刷新一次。**逻辑真实**。
- **AC-3 worker 零凭据(部分)**:`materialize_sandbox_home_links` 仅在 `provider=="claude" && role==Worker` 时 `continue` 跳过 `.claude/.credentials.json` 链接(`src/provider/home_layout.rs` materialize_sandbox_home_links 分支);`prepare_claude_overrides` 也仅 Worker 分支不调 `link_credentials`。**这是角色门控**:master/非-Worker claude 仍走 `link_credentials`(见 A-2),根漏洞对 master 席未消除。
- **AC-4 header 重写**:`register_worker`/`worker_gateway_for_test` 里把 `authorization` 换成真 token,且 `value.contains(fake_jwt)` 的 header 跳过(`src/provider/claude_gateway.rs:487-495`)。逻辑成立。
- **AC-2/AC-5/AC-6**:均有对应实现与测试。AC-6 `invalid_grant→401` + 事件落 `credential_event_log`(`:305-326`)。
- **扣分主因**:多数 AC 测试打的是**测试专用副本** `worker_gateway_for_test`(`tests/…:43,83,190,226,403` 全部调用它),不是生产 `register_worker`;树整体红(见 A-5)。契约"让测试绿"这一步都没走完。

### A-2. 强制 rollback 自检(真堵根漏洞?)—— 6/10
- **worker 侧堵住**:定向读 `prepare_claude_overrides` Worker 分支——不 link、注入 gateway env,worker home 无 `.credentials.json`;`ac3_worker_home_contains_no_credentials_file_or_real_token_bytes` 断言无凭据文件/无真 token 字节。该测试非空转(平凡保留 symlink 的实现会红)。**Worker 沙箱:根漏洞已堵**。
- **master 侧未堵(残漏)**:`.claude/.credentials.json` **仍留在 `PROVIDER_AUTH_WHITELIST` 里**(A 未删该常量项,只在循环体加了条件跳过),且 A **未改** `master_revival.rs`/`master_watch.rs`。故非-Worker 的 claude(master)仍走共享 symlink——正是 2026-07-11 事故里被级联登出的 master/d1/g1/g2 那条链。
- **host 环境变量泄漏(残漏)**:A **未动** `src/provider/manifest.rs`。claude manifest 的 `env_passthrough` 会把宿主 `ANTHROPIC_API_KEY` 透传进 worker(B 的测试 diff 从"含 host-key"翻成"不含"证明了 base 行为是透传)。A 不做 host 凭据 env 剥离 → 宿主设了 `ANTHROPIC_API_KEY` 的话仍漏进沙箱。

### A-3. 设计契合度 —— 7/10(A 的相对强项)
- **搭全了运行时拓扑**:`register_worker` 真 `UnixListener::bind` per-worker UDS + spawn HTTP 处理器(`src/provider/claude_gateway.rs:738-914`);`handle_agent_spawn_with_db_action` 真 `get_or_init_production_gateway` + `register_worker(agent_id)` + 把 host UDS 读写挂载进沙箱 `/var/run/ah-gateway.sock`(`src/rpc/handlers/agent.rs:124-137`);`wrap_claude_bridge` 真注入沙箱内 TCP→UDS 桥(`src/platform/linux/scope.rs:283-321`)。slot_id 与 agent_id 在 spawn 路径全一致(`agent.rs:189-217` 均传 `agent_id`),故端口/JWT 自洽。**这是 A 相对 B 的核心增量:一条真能跑通的端到端管道**,并有生产路径测试 `design_production_agent_spawn_lifecycle_wires_claude_gateway_correctly`(`tests/…:809-989`)锚定挂载/桥/env。
- **偏差(扣分)**:①桥用**内联 python3 heredoc**(`build_python_bridge_script`,`scope.rs:283`)而非设计的 socat/内置转发——引入沙箱 python3 硬依赖,朴素 4096 recv 循环,后台 `&` 起、死了 CLI 不知;②假 JWT 加了 `signature` claim + **全局单一密钥** SHA256(`claude_gateway.rs:73-88,100-135`),既不合设计 3.1(alg:none、空签名)也不合 3.2(每-worker 密钥)——真隔离靠的是 per-socket 闭包里硬编码的 `worker_id_inner` 比对,签名是花架子。

### A-4. 测试有效性 —— 5/10
- **1132 行,但含大量对副本的测试**:AC-1/2/4/6 打 `worker_gateway_for_test`,它与生产 `register_worker` 是约 180 行复制粘贴孪生(`claude_gateway.rs:383-583` vs `:738-914`)。改生产处理器,孪生测试不会红 → **测试验证的不是出货代码**。
- **树自相矛盾**:`design_production_…wires_claude_gateway_correctly` 显式断言命令含 `python3 -c` 桥(`tests/…:965`);而同仓 `test_spawn_command_scrubs_inherited_env_worker` 断言 `cmd[0]=="env"`(`src/platform/linux/scope.rs:656`)。**两条共存测试编码互斥契约**。

### A-5. 正确性与健壮性 —— 4/10
- **树是红的、且是确定性红**(见下"红根因定性"):HEAD 上 `test_spawn_command_scrubs_inherited_env_worker` 100% 失败。
- **失败态永久黏死**:一旦 `invalid_grant`,`last_failure` 被写入且 `get_valid_token` 每次先读它直接返回 Err(`claude_gateway.rs:267-269,305-309`),**进程不重启永不恢复**——即便人工重登也不自愈。
- **桥脆弱**:python 后台进程崩了 CLI 侧无感,表现为挂起。
- 正向:单飞逻辑本身正确。

### A-6. 可维护性与复杂度 —— 4/10
- **~180 行复制粘贴**(A-4)。
- **生产函数里全局 env 变异**:`prepare_home_layout_with_extensions_for_slot` 内 `#[cfg(test)]` 块 `unsafe { std::env::set_var("ALLOW_DUMMY_CLAUDE_CREDENTIALS","1") }`(`src/provider/home_layout.rs:149-154`)——在生产函数体里改进程全局 env,正是并行测试串扰的温床。
- 手写 base64url(`claude_gateway.rs:15-69`,B 也手写,算平手)。
- +2877 行里很大比例来自重复与冗长,**属"臃肿"而非"更完备"**。

### A-7. 安全边界 —— 6/10
- 正面:worker 沙箱内确无 OAuth refresh token;**物理 per-worker UDS 隔离真的在运行时落地了**(每个 listener 闭包锁死自己的 worker_id,`:437,779`),这是设计 3.2 第一防线的真实现,B 反而没跑起来(见乙-7)。
- 负面:master 仍共享 symlink(A-2);host `ANTHROPIC_API_KEY` 仍透传(A-2);全局密钥 JWT 签名不提供真每-worker 密码学绑定(A-3)。

---

## 乙、实现 B 逐条 rubric 审计

### B-1. AC-1~6 契约满足度 —— 8/10
- **AC-1**:`GatewayCore::valid_access_token`(`src/claude_gateway.rs:115-147`)std `RwLock`+`Mutex` 双检单飞;`ac1_concurrent_expired_requests_single_flight_refresh_once`(`tests/plan_b_gateway_acceptance.rs:19-50`)8 线程 barrier 齐发、mock `refresh` sleep 20ms 拉宽竞态窗,断言 `refresh_calls()==1 && message_calls()==8`。**我实跑该套件:7 passed(0.07s)**。
- **AC-3**:`ac3_…`(`tests/…:86-126`)驱动**真生产** `prepare_home_layout("claude",…)`,写入含 `real-refresh-token` 的宿主凭据,断言 worker home 无 `.credentials.json` 且全树无 token 字节(递归 `path_tree_contains`)。**打的是出货代码,非副本**。
- **AC-4**:`ac4_worker_fake_jwt_is_rewritten…` + `ac4_gateway_rejects_fake_jwt_from_wrong_worker_channel`(`tests/…:128-187`)断言上游见真 token、任何 header 不含假 JWT、且**通道 worker_id≠JWT worker_id → 403 + 0 上游调用**(设计 3.2 第二防线)。
- **AC-6**:`ac6_…`(`tests/…:219-244`)`invalid_grant→401` + body 含 `invalid_grant` + 可查 daemon 事件。
- **扣分**:AC-1/2/4/6 打的是**核心逻辑**(`GatewayCore`+trait mock),真 UDS 服务器/桥未起(见乙-3),端到端联通未证。

### B-2. 强制 rollback 自检(真堵根漏洞?)—— 9/10
- **源头铲除,全角色**:`PROVIDER_AUTH_WHITELIST` **删除** `.claude/.credentials.json` 项(`src/provider/home_layout.rs:19-20`),`link_credentials` 函数**整体删除**(原 `:652` 区块)。故**任何 claude 角色(worker 与 master)都不再产生该 symlink**——直接消灭事故的 master 级联向量。
- **纵深防御(A 没有)**:`collect_spawn_env` 对 claude 剥离宿主 `ANTHROPIC_API_KEY/AUTH_TOKEN/BASE_URL` 透传,且 extra_env 仅放行"合法假 worker JWT"与等于 sandbox base_url 的 `ANTHROPIC_BASE_URL`(`src/provider/manifest.rs:478-517`);`auth_mount_paths` 清空(`:388`,注:该字段仅 `doctor.rs:94` 预检用,非挂载,属一致性清理而非安全必需)。
- **波及路径补齐**:master 复活链同步改为 gateway 模型(`src/monitor/master_watch.rs:789-806`、`src/master_revival.rs`),把凭据移除的涟漪追进 master——B 顺着改动查全了下游,A 漏了。
- **保留风险**:见乙-5 的"持续 invalid_grant 重试风暴"。

### B-3. 设计契合度 —— 6/10
- **假 JWT 忠于设计**:alg:none、空第三段、`exp=32503680000`、`sub=ah-worker-session`(`src/claude_gateway.rs:235-268`,`fake_jwt_worker_id` 严格校验三段/alg/typ/exp/sub)。测试 `assert_fake_jwt_claims`(`tests/…:281-294`)逐字段锚定。
- **核心逻辑齐全**:单飞、header 重写(`rewrite_authorization:182-195`)、多租户身份校验(`validate_worker_identity:150-172`)、失败可观测(`CredentialEvent`)全部忠实实现。
- **重大缺口(扣分)**:设计 Phase 1"宿主 HTTP Gateway 服务" + Phase 2"沙箱 TCP→UDS 桥"的**运行时未实现**——见乙-7。B 交的是"逻辑 + env + 拓扑串",不是能跑的系统。

### B-4. 测试有效性 —— 9/10
- **454 行,条条锚定可观测契约**:状态码、header 值、`refresh_calls` 计数、文件不存在、token 字节不存在、403 错误码——非实现内部态,平凡/空转实现骗不过(保留 symlink → AC-3 红;每请求刷新 → AC-1 红;不重写 → AC-4 红)。
- **纪律**:`#[serial_test::serial(global_env)]` + `ENV_LOCK` 串行化改 env 的测试(`tests/…:17,87,411-437`),正是 A 缺失的隔离。
- 体量是 A 的 40%,信息密度更高:**"更周全"归 B,"更臃肿"归 A**。

### B-5. 正确性与健壮性 —— 8/10
- **CI 绿(我实跑复核:7 passed)**;单飞正确;错误映射清晰(`map_refresh_error`/`map_upstream_error`)。
- **残留缺陷**:`valid_access_token` 刷新失败**不缓存失败态**(`:132-146` 直接返回 Err、不改 token),故持续 `invalid_grant` 下**每个后续请求都会再打一次上游刷新** → 重试风暴,恰是 requirements 根因节点名的"反复失败刷新触发账号级速率保护"风险。(与 A 的"永久黏死"是对称取舍:A 免风暴但不自愈,B 自愈但可能风暴。)
- 无运行时服务器(乙-7)。

### B-6. 可维护性与复杂度 —— 8/10
- trait 抽象(`ClaudeUpstream`)让核心对 mock 可测、零网络、零 async 依赖;`GatewayCore<U>` 泛型、DRY、地道 Rust。
- 手写 base64url(`src/claude_gateway.rs:312-362`,与测试里再抄一份,小瑕疵,与 A 平手)。
- 命名/结构/错误处理清晰,常量集中(`INVALID_GRANT_ERROR_CODE` 等 `:6-11`)。

### B-7. 安全边界 —— 7/10
- 正面:沙箱内无真 token;通道 worker_id vs JWT 身份冲突 → 403(逻辑已测);宿主凭据 env 剥离;symlink 全铲。
- **缺口(须复验/致命于"能工作")**:多租户**物理 UDS 隔离(设计 3.2 第一防线)在运行时未落地**——无 per-worker UDS listener,`GatewayRequest.worker_id`(通道身份)由"谁来 populate"?生产里没有那个 server 去把物理 socket 映射成 worker_id。故 B 的隔离是**逻辑自洽但运行时未证**;A 反而真把 per-worker UDS listener 跑起来了。

---

## 丙、实现 A "CI 红"根因定性(亲自论证)

**定性:确定性的"产品/测试契约互斥",不是并行测试全局 env 串扰。**

**实证(我亲跑,单测试、单线程、完全隔离)**:
```
$ CARGO_BUILD_JOBS=1 cargo test --lib test_spawn_command_scrubs_inherited_env_worker -- --test-threads=1
running 1 test
test platform::linux::scope::tests::test_spawn_command_scrubs_inherited_env_worker ... FAILED
thread '…' panicked at src/platform/linux/scope.rs:656:9:
assertion `left == right` failed
  left: "bash"
 right: "env"
test result: FAILED. 0 passed; 1 failed; …
```
**在 `--test-threads=1`、仅此一个测试、无任何并发的条件下仍然确定性失败**——这正是 A 团队声称"本地 --exact 单跑过"的那个条件。**并行全局 env 串扰假说被直接证伪**:串扰要求并发,单跑单线程还红就不是串扰。

**机理**:`wrap_command_with_recovery_and_sandbox_overrides` 对 claude **无条件**前置 `bash -c '<python3 桥> & exec "$@"'`(`src/platform/linux/scope.rs:274-283` 的 `if manifest.provider_name=="claude" { exec_cmd = wrap_claude_bridge(...) }`,连 `unsafe_no_sandbox` 分支也走),于是 `cmd[0]` 从 `"env"` 变成 `"bash"`;而该 env-scrub 单测仍断言旧契约 `cmd[0]=="env"`(`:656`)。env 前缀被推到 `bash -c … --` 之后(`command_with_env_prefix` 产 `["env",…]`,见 `:439-467`),`cmd[0]` 不再是 `env`。

**归桶**:属**产品侧改动未与自身测试收口**——A 团队自己的生产路径测试 `design_production_…wires_claude_gateway_correctly` 已经**拥抱** `python3 -c` 桥(`tests/…:965`),却把互斥的旧 env-scrub 单测(`:656`)留红 14 轮,并误判为"并行 env 串扰"。这既是产品缺陷(桥的无条件粗放包裹——连非沙箱模式也套,且引入 python3 依赖)也是测试缺陷(旧契约未随产品改动更新)。**修复只需一行**(把旧断言更新为桥后契约,如生产测试所示),但"一行没修 + 误诊 14 轮"本身就是对该实现清晰度与团队对自身改动掌控力的强负面信号,且**评审 HEAD 上树事实为红且自相矛盾**。

---

## 丁、分维度评分表(同一把尺,7 维)

| # | 维度 | A | B | 依据(一句) |
|---|------|---|---|------|
| 1 | AC-1~6 契约满足度 | 6 | 8 | 两臂逻辑均真;B 全绿且 AC-3 打出货代码,A 树红且多数 AC 打副本孪生 |
| 2 | 真堵根漏洞(rollback 自检) | 6 | 9 | B 源头删 symlink(全角色)+env 剥离+master 波及补齐;A 仅 Worker 门控,master 仍 symlink、host API key 仍漏 |
| 3 | 设计契合度 | **7** | 6 | **唯一 A 胜**:A 真搭了服务器/桥/挂载(Phase1-2 运行时);B 只搭逻辑+env,但假 JWT 更忠实、A 桥用 python heredoc+自造签名有偏差 |
| 4 | 测试有效性 | 5 | 9 | B 454 行锚定可观测契约、非空转、绿、串行化 env;A 1132 行多打副本、且自相矛盾 |
| 5 | 正确性与健壮性 | 4 | 8 | B 绿、单飞正确(残:重试风暴);A 确定性红、失败态永久黏死、桥脆弱 |
| 6 | 可维护性与复杂度 | 4 | 8 | B DRY+trait 抽象+地道;A ~180 行复制粘贴、生产函数内全局 env 变异、臃肿 |
| 7 | 安全边界 | 6 | 7 | B 凭据面更小+身份 403 已测(残:物理 UDS 隔离运行时未证);A 真跑起物理 UDS 隔离,但 master/API key 双漏 |
| | **合计** | **38** | **55** | B 胜 6 维,A 仅胜"设计运行时广度"1 维 |

---

## 戊、head-to-head 裁决

**实现 B 代码质量更高。** 决定性理由:

1. **B 能绿、A 确定性红且自相矛盾**:B 我实跑 7/7 绿;A 的 `scope.rs:656` 在单线程完全隔离下仍确定性 panic(`"bash"!="env"`),且两条共存测试编码互斥契约,团队误诊 14 轮。CI 绿是质量的必要门,A 连必要门都没过。
2. **B 的测试锚定出货代码与可观测契约;A 的测试大面积打复制粘贴孪生**:A 的 AC-1/2/4/6 打 `worker_gateway_for_test`(与生产 `register_worker` 约 180 行重复),改生产不会红;B 的 AC-3 直接驱动真 `prepare_home_layout`,单飞/隔离/重写打真 `GatewayCore`。
3. **B 把根漏洞在源头彻底铲除、A 只做角色门控留了 master 残漏**:B 删 `PROVIDER_AUTH_WHITELIST` 项 + 删 `link_credentials` + 补 master 复活链 + env 剥离宿主凭据;A 仅 Worker 跳过、master 仍共享 symlink(正是事故被登出的席位)、宿主 `ANTHROPIC_API_KEY` 仍透传。
4. **B 地道 DRY、A 臃肿且有生产函数内全局 env 变异**:A 的 +2877 近 B 两倍,大头是重复与冗长(含 `home_layout.rs:149-154` 在生产函数体里 `set_var`),属"臃肿"非"更完备"。

**须显著标注的互补事实(留给 operator 决策,不参与"质量"终裁)**:**A 真把运行时管道跑起来了**(per-worker UDS 服务器 + 沙箱 python 桥 + 读写挂载 + 生产路径测试),**B 没有**——B 只注入了指向 `localhost:8206` 的 env、拓扑串与 manifest 剥离,`GatewayCore`/`forward_messages`/`gateway_worker_topology` 在非测试 src 里**无任何调用点**(grep 实证:src 里只用到 `fake_worker_jwt`/`SANDBOX_TCP_BASE_URL`/`fake_jwt_worker_id` 这些字符串助手)。故真 claude worker 在 B 上会连到无人监听的 `:8206`。**若 operator 的优先级是"最接近端到端可跑",A 做了更多管道但需清理并修绿;若优先级是"干净正确、可长期演进的地基",B 明显更优。** 可嫁接:把 A 的运行时服务器/桥/挂载接到 B 的干净 `GatewayCore` 与凭据铲除之上,是两臂最佳组合。

---

## 己、各臂关键风险 / 遗留缺陷清单

### 实现 A
- **[阻断]** `scope.rs:656` 确定性红;树自相矛盾,不修不能合(修复为一行,但需同时决定桥是否该在 `unsafe_no_sandbox` 也套)。
- **[安全]** master/非-Worker claude 仍走共享 `.credentials.json` symlink(`PROVIDER_AUTH_WHITELIST` 项未删、master 复活链未改)——事故向量对 master 未消除。
- **[安全]** 宿主 `ANTHROPIC_API_KEY` 仍透传进 claude worker(未剥离 env passthrough)。
- **[健壮]** `last_failure` 永久黏死,`invalid_grant` 后不重启不自愈。
- **[维护]** `register_worker` 与 `worker_gateway_for_test` 约 180 行复制粘贴;生产函数内 `#[cfg(test)]` 全局 `set_var`。
- **[依赖/脆弱]** 沙箱 python3 硬依赖;内联 heredoc 桥朴素、后台崩溃 CLI 无感。
- **[设计偏差]** 假 JWT 加全局单密钥签名,既偏离设计 3.1 也偏离 3.2,隔离实际靠 per-socket 闭包而非签名。

### 实现 B
- **[完备缺口/须复验]** 宿主 HTTP Gateway 服务与沙箱 TCP→UDS 桥**运行时未实现**——`localhost:8206` 无监听、无 per-worker UDS listener;设计 3.2 第一防线(物理 UDS 隔离)运行时未落地,多租户隔离目前只是逻辑自洽。此为对设计 Phase 1-2 的实质欠交,须后续补齐服务器+桥+挂载。
- **[健壮]** 持续 `invalid_grant` 下每请求重试上游刷新(不缓存失败态)→ 潜在刷新风暴/账号级速率保护风险。
- **[适用面/须复验]** master 复活路径也注入 gateway env 指向未运行的 `:8206`;在服务器补齐前,master claude 亦无可用凭据(但 symlink 已铲,不会再级联登出,属"退化为不可用"而非"级联登出")。
- **[小瑕]** 手写 base64url 并在测试里重复;可用现成 crate。

---

*实证纪律*:本审计所有结论均带 `file:line`;两处实跑证据(A 的 `scope.rs:656` 确定性红、B 套件 7/7 绿)为亲自命令输出。未修改任一分支任何字节。标注"须复验"处即为逻辑推断而非实跑处(B 的运行时服务器缺失系 grep 全 src 无调用点所证)。

---

## 庚、operator 附录:可靠性 / 成本 / 交接账本(不参与质量终裁,仅记录)

> 由 operator 编制,数据源:ahd DB、CI 记录、观察日志 #47-#52、observations.md。

| 轴 | 实现 A(泳道:codex 闸门 g1 + antigravity 实施 g1-m1) | 实现 B(codex 单干 g2) |
|---|---|---|
| **CI 圈数 / 结果** | ~14 圈,**0 绿**(13 failure)——DNF,从未过 test job | 8 圈,第 8 圈**绿**(7 failure→1 success) |
| **派单/job 数** | 19(g1 闸门 4 + g1-m1 实施 15) | 10 |
| **返工轮次** | 14 轮(#1~#14) | 无独立"返工"编号,CI 自迭代 |
| **自足性** | 低:依赖 operator/master 大量干预 | 高:自主收敛到绿,会理性止损报阻塞 |
| **operator/master 干预** | 三部署失误修正(文档/机制/政策)+ 摆渡模式 + 救僵尸 + 催单 livelock + 队列排水事故止损 + 冻结 | 一次主树误写(早期,已修)+ 对称政策修订 |
| **基础设施故障牵连** | 重:agy Stop 钩子不触发 + log 300s 超时 + 催单器注入(obs #51)+ 排水事故(obs #52) | 轻:仅受"本地禁跑测试"政策拖累(已对称放宽) |
| **最终态** | 冻结于 `7f5dc2b`(红、自相矛盾),DNF | `d55c26b`,绿,COMPLETION-REPORT 在位 |

**方法论读数**:泳道臂(闸门+实施双 agent)在本任务上**成本数倍于单干臂且未能收敛**,主因是 agy 完成通道三重故障(infra,非泳道设计本身之过)叠加实施 agent 对自身改动掌控力弱(14 轮误诊一个一行可修的产品/测试契约冲突)。单干 codex 臂在同一 brief 下自主达绿、代码质量与自足性双高。**但需注记**:两臂"成本"不可天真对比——A 臂承受了本实验暴露的全部 infra bug(#51/#52),这些故障与"泳道 vs 单干"方法论正交;若在完成通道健全的前提下重跑,泳道臂成本画像可能显著不同。此为方法论结论的**置信度边界**,非定案。
