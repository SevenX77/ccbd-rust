# r1 审核记录 · `research/architecture-index.md` (v1) · MD1 完整性+drift 门

- 审核席:r1-claude(只审不写)
- 日期:2026-07-13
- 被审对象:`research/architecture-index.md`(c1/c2/g1 三方编目 + master 合并收敛 v1)
- 审核依据:master 派单三条硬判据 + 本轮新增「process axis 必查」轴
- 方法:实地 `find`/`rg`/读源码逐一核对,**不采信索引文字**。所有结论下附实证命令与输出摘要。

---

## 判定:**REJECT**

三条硬判据里 **判据 3(符号与源码逐一核对)** 和 **本轮新增 process-axis 轴** 各有一处硬失败,**判据 2(capability→owner 无遗漏)** 有一处硬失败 + 若干覆盖缺口。任一即足以拦下,合计三个独立 REJECT 驱动。判据 1(top-level 模块全覆盖)除下述 gateway 误编目 + 一个 cfg(test) 文件缺列外基本达标。

---

## 阻断级发现(必须修完复审)

### R1 · 【CRITICAL】`claude_gateway` 模块被整条编目错位 + 符号清单整体虚构

一条 entry 同时踩中判据 1(top-level 模块未正确出现)、判据 3(符号不符源码)、process-axis(路径/归属错)。

**源码事实(实证):**
```
$ rg -n 'mod claude_gateway' src/
src/lib.rs:2:pub mod claude_gateway;          # → 顶层 src/claude_gateway.rs

$ ls -la src/claude_gateway.rs
-rw-rw-r-- ... 37735 ... src/claude_gateway.rs   # 存在,37735 字节

$ ls src/provider/claude_gateway.rs
ls: cannot access 'src/provider/claude_gateway.rs': No such file or directory  # 不存在

$ cat src/provider/mod.rs   # 只声明 11 个模块,无 claude_gateway
pub mod builtin; bundles; extensions; fingerprint; health_check;
home_layout; init_probe; init_probe_task; manifest; plugins; skills;
```
真实调用方全部用 `crate::claude_gateway::…`(monitor/agent_watch.rs、monitor/master_watch.rs×6、runtime_events.rs、db/system.rs×4、platform/linux/scope.rs×6),**无一处** `provider::claude_gateway`。

**索引把事实说反了,且是多处系统性说反:**
- 第 206 行:「there is no top-level `src/claude_gateway.rs`. The gateway lives at `src/provider/claude_gateway.rs`」——**两句都为假**。
- 第 212 行:把 `provider::mod` 的 `pub mod` 列表写成含 `claude_gateway`——**假**(真实 11 个,无它)。
- 第 215 行:整条 `provider::claude_gateway` 行,Path 写 `src/provider/claude_gateway.rs`——**路径错**,真实 `src/claude_gateway.rs`,命名空间 `crate::claude_gateway`。
- 第 266 行「Corrections」:「There is no top-level `src/claude_gateway.rs` — it is `src/provider/claude_gateway.rs`」——**把 seed draft 里本来正确的事实倒改成错误**。注:这正是本索引宣称要防的「不知道模块在哪就设计」事故类型,却在 gateway 自己的落点上翻车。

**符号清单整体虚构(判据 3 硬失败)——第 215 行列的 13 个特征符号,真源码里一个都不存在:**
```
$ for s in get_or_init_production_gateway load_seed_credential port_from_slot_id \
  build_fake_worker_jwt_for_test decode_fake_worker_jwt_claims SeedCredential \
  WorkerGatewayEnv GatewayBind CredentialFailureCode ClaudeGatewayConfig \
  TestWorkerGateway WorkerGateway FakeClaims; do rg -n "\b$s\b" src/claude_gateway.rs; done
# 全部 MISSING(0 命中)
```
连主结构名都错:索引写 `ClaudeGateway`,真实是 `ClaudeGatewayService`(`src/claude_gateway.rs:354`)。`register_worker` 是自由函数(783/817 行),不是 `ClaudeGateway` 的方法。种子凭据加载真实符号是 `read_seed_credentials`(609 行)/ `write_seed_credentials_guarded`(650 行),**不是** `load_seed_credential`。

**真实公共符号(实证,应据此重写该行):**
```
$ rg -n '^pub (fn|struct|enum|async fn|const|trait)' src/claude_gateway.rs
consts: INVALID_GRANT_ERROR_CODE, REFRESH_FAILED_ERROR_CODE, WORKER_ID_MISMATCH_ERROR_CODE,
        AUTH_INVALID_ERROR_CODE, SANDBOX_UDS_PATH, GATEWAY_SANDBOX_ROOT_ENV, FAILURE_CACHE_TTL
types:  TokenSet, GatewayRequest, GatewayResponse, GatewayError, UpstreamError,
        ClaudeUpstream(trait), GatewayWorkerTopology, CredentialEvent, RecordedCredentialEvents,
        GatewayCore<U>, ClaudeGatewayService, ProductionUpstream, GatewayListener
fns:    validate_worker_identity, fake_worker_jwt, fake_jwt_worker_id,
        validate_credential_path_not_wsl_windows_mount, gateway_worker_topology,
        read_seed_credentials, write_seed_credentials_guarded, register_worker,
        run_internal_bridge, bridge_wrapper_shell
```

**连带污染 capability→owner 表:** 第 248 行(Claude seed credential loading → `provider::claude_gateway::load_seed_credential` @ `src/provider/claude_gateway.rs`)与第 250 行(Gateway bridging → `provider::claude_gateway::{get_or_init_production_gateway, ClaudeGateway::register_worker}`)——符号 + 路径 + 命名空间三重错。

**归属方修正:** seed draft(o1)原本把 gateway 记为顶层是**对的**;是本次 master 合并阶段把它「更正」倒了。修由 **master(合并者)+ g1(layer6-provider 审计者)** 共同负责——g1 的 layer6 审计若产出了这条错误落点,需一并核对。

---

### R2 · 【REJECT 驱动 · 本轮新增 process-axis 轴】capability→owner 表整表缺 process-axis 标注

本轮硬要求:「索引里每条模块/capability 行必须标注 ah CLI / ahd daemon / 两者;缺标注 = 视为符号核实失败,计入 REJECT」。

```
$ rg -n '^\| Capability \|' research/architecture-index.md
231:| Capability | Owner symbol(s) | Path |
```
表头只有三列,**24 条 capability 行全部没有 process-axis 列**。模块层(Layer 1–6)的 process axis 标注是齐的(逐行,或用 section 级「all `ahd` daemon」集体声明,均可接受;已抽查一致),但 capability 表这一块直接漏了整轴 → 硬失败。

**修:** master(表的合并者)给 capability 表加 process-axis 列并逐行填。

---

### R3 · 【判据 2】capability→owner 表有明显能力无 owner(遗漏)

判据 2 要求「每个明显能力都能找到 owner」。以下明显能力中心在表里**无对应行**:
- **编排/调度派单循环 + 唤醒总线** — `orchestrator::{spawn_orchestrator_task, WAKER, wake_up}`(`src/orchestrator/mod.rs` 3054L)。表里「Job lifecycle」只指向 db::jobs/job_state(持久化),真正驱动派单的 orchestrator 无 owner 行。
- **JSON-RPC 服务/路由** — `rpc::run_server` / `rpc::router::dispatch`(整个 rpc 子系统),无 owner 行。
- **idle/prompt/unknown vt100 标记分类** — `marker::matcher::MarkerMatcher`。与 prompt_handler(prompt 检测)、pane_diff(卡死)、completion(日志完成)是并列的另一条感知能力,表里无 owner 行。
- **崩溃恢复/重排队** — `db::recovery::*`(spawn spec、recovery intent、interrupted requeue),与「Job lifecycle」不同层,无独立 owner 行。
- (次要)配置加载/校验 `cli::config`、状态目录/project-id 解析 `state_layout` 无行。

另**标签错**:第 246 行「Bundle/credential parsing」→ `provider::bundles`。bundles 解析的是 `bundle.toml`(能力支持),**不解析 credential**;真正 credential 加载在 claude_gateway。「credential」措辞误导,建议改为「Bundle parsing/validation」。

这些是判据 2 的遗漏项。是否补齐或显式声明「本轮不纳入」由 master 定,但需给出处置,不能默认覆盖。

---

## 非阻断 gap(建议一并修,不单独拦门)

### N1 · `src/db/perception/phase1_acceptance.rs`(14978B)未列 + 行描述不全
```
$ sed -n '47,52p' src/db/perception/mod.rs
pub(crate) mod events; gate; types;
#[cfg(test)] mod phase1_acceptance;
```
该文件是 `#[cfg(test)]` 测试模块(g1 先写的 RED 验收测试),非生产模块——故非硬失败。但:①索引已选择逐一枚举 perception 子树,却漏了这个子文件;②第 132 行 `db::perception::mod` 描述写「declares `pub(crate) mod events/gate/types`」,漏了 `#[cfg(test)] mod phase1_acceptance`。建议补列并标注 (test-only),或在行描述里补齐声明。归属:master/g1(perception 属 g1 泳道)。

### N2 · 未列的 test/helper 文件
`src/bin/ahd_test_helper.rs`、`src/outbox/tests.rs` 未在索引出现——均为测试/辅助文件,按架构索引口径可不列,仅备注留痕,不计入判据 1 失败。

---

## 通过项(为公平留证 —— 这些实测无误)

**符号抽样(判据 3):** 抽查 25 个符号(20 跨模块 + 5 rpc handler),外加 master_cutover 4 个、Layer-4 层 5 个,**除 R1 的 gateway 整块外全部实地命中**:
```
start_from_options@cli/start.rs:63  run_server@rpc/mod.rs:28
handle_session_create@rpc/handlers/sessions.rs:67  handle_agent_spawn@agent.rs:41
handle_job_submit@jobs.rs:14  stream_event_subscribe@events.rs:33  handle_system_dump@system.rs:6
spawn_orchestrator_task@orchestrator/mod.rs:61  rearm_active_master_watches_on_startup@master_watch.rs:110
classify_master_death@master_revival.rs:61  check_environment@sandbox/mod.rs:42
reconcile_startup_with_tmux_socket_and_gateway@db/system.rs:1283  transit_agent_state_sync@state_machine.rs:78
dispatch_job_to_agent_sync@db/jobs.rs:376  scan_prompt_and_apply_outcome@integration.rs:86
cold_scan_all_agents@outbox/mod.rs:566  build_runtime_snapshot@runtime_events.rs:207
process_pane_diff_observations@pane_diff/mod.rs:80  run_log_monitor_tick@completion/monitor.rs:20
collect_spawn_env@manifest.rs:475  build_ah_hook_command@home_layout.rs:721
inject_worker_identity@process_identity.rs:9  pipe_pane_to_fifo@tmux/session.rs:762
spawn_marker_timer_task@marker/timer.rs:39
handle_master_ack_ready@sessions.rs:979  spawn_master_pane_inner@sessions.rs:532
write_handoff_bundle@master_cutover.rs:43  seed_claude_project_conversation@master_cutover.rs:89
resolve_state_dir@env.rs:3  resolve_state_layout/StateLayout/StateLayoutRequest@state_layout.rs
CcbdError@error.rs:4  CcbdError::to_rpc_error@error.rs:66
```
注:`handle_session_create` 等 handler 在子文件(sessions.rs/agent.rs/…)而非 `handlers.rs` 顶层——索引 Path 列已透明列出这些子文件,`rpc::handlers::{…}` 是其 re-export 命名空间,不算缺陷。

**负向声明核实无误:** `runtime_events` 确无 `write_state_snapshot`(0 命中);`resolve_ah_binary` 确为 home_layout 内 `pub(crate)` 私有 helper(741 行)——与第 208 行 current_exe pitfall 注释一致。

**process-axis 二进制锚点(第 16–17 行)实测准确:**
- `ah.rs` 确 import `ah::cli::*`、`ah::tmux::{TmuxServer,…}`、并在 803 行调 `ah::systemd_unit::detect_current_scope_or_service()`。
- `ahd.rs` main() 确调:`db::init`(83)、`reconcile_startup_with_tmux_socket_and_gateway`(91)、`cold_scan_all_agents`(125)、`rearm_active_master_watches_on_startup`(111)、`spawn_orchestrator_task`(138)、`rpc::run_server`(172)、`sandbox::check_environment`(60)、`cli::service_unit::derive_unit_name`(74)、`platform::sys::scope::active_daemon_unit_or_none`(73)。全部一致。

**模块覆盖(判据 1,除 R1/N1):** `find src -maxdepth 2 *.rs` 全清单逐一核对——db/ 直接文件 19 个、provider/ 直接文件 11 个、cli/ 17、prompt_handler/ 11、completion/ 6、marker/ 5、monitor/ 4、agent_io/ 4、tmux/ 5、sandbox/ 3、orchestrator/ 2、顶层 .rs(env/error/master_cutover/master_revival/process_identity/runtime_events/state_layout/systemd_unit)——**均已在索引出现**。唯一 top-level 落点问题是 R1 的 claude_gateway 误编目。模块层 process-axis 标注齐(逐行或 section 级集体声明)。

---

## REJECT 复审清单(按归属方)

| # | 修点 | 归属 | 复审通过标准 |
| --- | --- | --- | --- |
| R1 | 把 `claude_gateway` 行改回顶层:命名空间 `crate::claude_gateway`、路径 `src/claude_gateway.rs`;删除「no top-level src/claude_gateway.rs」等 4 处反向陈述(206/212/215/266);符号清单按真源码重写(ClaudeGatewayService / read_seed_credentials / register_worker 自由函数 / GatewayCore / gateway_worker_topology / bridge_wrapper_shell …);同步修 capability 表 248/250 行 | master + g1 | 我方 rg 复核该行每个符号命中,路径 `ls` 存在,命名空间与真实调用方一致 |
| R2 | capability→owner 表加 process-axis 列,24 行逐条填 ah/ahd/both | master | 表头含 process-axis 列且无空行;抽查与调用方一致 |
| R3 | capability 表补 orchestrator 调度、rpc 服务、marker 分类、db::recovery 恢复 等明显能力的 owner 行(或显式声明本轮不纳入的理由);修 246 行「credential」误标 | master | 每条明显能力可定位到 owner 行,或有明确 scope-out 说明 |
| N1 | perception 补 `phase1_acceptance.rs`(标 test-only)+ 修 132 行行描述漏声明 | master/g1 | 建议项,复审时确认已处理或明确暂缓 |

**过审后**方可交 operator 一眼并进 MD2 解耦轮。修完请回我复审(重点复核 R1 每个符号 + R2 每行 process-axis)。

— r1

---

## 复审结论(round 2,2026-07-13)

master 已直接修复 R1/R2/R3/N1 四项。方法同上轮:实地 `rg`/读源码逐一复核,不采信索引文字。

### 判定:**ACCEPT**

四项修复逐条复核,均与真源码一致,无一处「随手填」;上轮的三个 REJECT 驱动全部关闭,复审未发现新的硬失败。

### 逐条复核记录

**R1 · claude_gateway 编目 + 符号 —— 关闭**
- 路径/命名空间已改回顶层 `crate::claude_gateway` / `src/claude_gateway.rs`(实证:`src/lib.rs:2: pub mod claude_gateway;`;`src/provider/claude_gateway.rs` 仍不存在;`src/provider/mod.rs` 仍只 11 模块无它)。索引 207 行 correction + 213 行「claude_gateway is NOT one of them」+ 273 行 seed-draft 更正,三处表述均与事实一致。
- **符号清单逐一命中(判据 3 核心)**:重跑
  ```
  $ rg -n '^pub (fn|struct|enum|async fn|const|trait)' src/claude_gateway.rs
  ```
  索引 216 行列的 7 consts + 13 types + 10 fns **与源码 pub 清单一一对应,零虚构、零遗漏**(register_worker / run_internal_bridge / GatewayListener 各有 cfg(unix)/非-unix 两份,索引各列一次,正确)。上轮那批虚构符号(get_or_init_production_gateway / load_seed_credential / ClaudeGateway / WorkerGateway …)已全部清除。
- **process axis「both」的支点已实证成立**:索引称 `ah` CLI 通过 `claude_gateway::run_internal_bridge`(`src/bin/ah.rs:275`)跑沙盒内桥接。实测 `src/bin/ah.rs:275`:
  ```
  Some(Cmd::InternalBridge { uds, port_file }) =>
      ah::claude_gateway::run_internal_bridge(&uds, port_file.as_deref())
  ```
  确在 `main()` 命令分发内,是 `ah internal-bridge` 子命令的真实生产路径(非 test)。ahd 侧持有生产 `ClaudeGatewayService`/`GatewayCore` 生命周期。「both」判定成立。
- **caller 列表非注水**:216 行新增声称的调用方实测均命中——`orchestrator/mod.rs`(1)、`provider/health_check.rs`(1)、`provider/manifest.rs`(3)、`prompt_handler/integration.rs`(1),`rpc/` 下 6 文件引用 claude_gateway。
- capability 表 253/255 行(seed cred read/write、gateway bridging)符号 + 路径 + 命名空间同步改对。

**R2 · capability 表 process-axis 列 —— 关闭**
- 表头(232 行)已含第四列 `Process axis`,24 条(含新增)全部填齐。
- 抽验各行 axis **对应真实调用关系,非臆填**:调度环→`ahd`(spawn_orchestrator_task 由 ahd.rs 启);rpc→`ahd` server / `ah` client(run_server 仅 ahd.rs:172,ah 走 rpc_client);db::recovery→`ahd`(4 符号皆 pub(crate) 守护侧);seed cred read/write→`ahd`(实测 `ah.rs` 无引用,全 daemon 侧);cli::config→`ah` CLI;gateway bridging→`both`(见 R1 支点)。

**R3 · capability 遗漏补齐 + bundles 措辞 —— 关闭**
- 新增行符号实地命中:orchestrator(`spawn_orchestrator_task`✓)、rpc(`run_server`✓)、`db::recovery::{persist_agent_recovery_intent_sync:181, requeue_interrupted_job_from_captured_intent_sync:335, replace_killed_agent_and_requeue_job_sync:434, try_claim_agent_recovery_sync:604}`✓、`marker::matcher::MarkerMatcher::{from_manifest:42, scan:60}`✓、`cli::config::{load_project_config:146, find_config:163, validate_project_config:167}`✓、state_layout✓。
- 251 行 bundles 已改为「Bundle parsing/validation (... NOT credential parsing — see gateway rows below)」,措辞误导消除。
- 无有害 overlap:db::recovery「interrupted-job requeue」与 Job lifecycle「requeue」分属崩溃恢复 vs 正常态迁移,行内已区分;marker vt100 分类与 prompt_handler KB 流水线属不同感知层,可接受。

**N1 · perception —— 关闭**
- 132 行 `db::perception::mod` 符号列表补了 `#[cfg(test)] mod phase1_acceptance`;136 行新增 `db::perception::phase1_acceptance` 行并标 (test-only)。与 `src/db/perception/mod.rs:51-52` 一致。

### 非阻断 nit(不影响 ACCEPT,建议顺手修)
- 216 行体量注「~1030L」,实测 `wc -l src/claude_gateway.rs` = **1129L**(最后一个 pub 符号在 1007 行,但文件含实现体/测试续到 1129)。约低估 9%,建议改「~1130L」。纯注记,不拦门。

### 结论
索引 v1 通过 MD1 完整性 + drift + process-axis 三轴门。**可交 operator 一眼,并作为 MD3「design-before-code」的必读基线进入 MD2 解耦轮。** freshness 机制(278–279 行:每个 module/PR close 点随 PR 更新对应行)已写明,后续维护责任清晰。

— r1
