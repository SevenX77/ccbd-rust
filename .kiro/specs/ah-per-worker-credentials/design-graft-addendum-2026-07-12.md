# 模块 D 网关嫁接 · 冻结设计补丁(daemon 级所有权点)ADDENDUM

**Status**: FROZEN ADDENDUM(设计主笔 d1-claude 执笔,唯一执笔席)
**Date**: 2026-07-12
**Target Path**: `.kiro/specs/ah-per-worker-credentials/design-graft-addendum-2026-07-12.md`
**引用关系(单一权威链,不冲突)**:
- 本补丁**扩展**冻结设计 `.kiro/specs/ah-per-worker-credentials/design-graft-frozen-2026-07-11.md`(以下简称"冻结稿")。
- **冻结稿 7 个分歧点(UDS/桥/JWT/master 剥离方向/重试策略/测试策略/端口)不重开、不修改**;本补丁只补冻结稿遗漏的"网关在 `ahd` 守护进程生命周期里的所有权点"这一层,是 c1 推进 G1-G3 生产接线的前置裁决。
- 遇到本补丁与冻结稿冲突时:**冻结稿 7 分歧点方向优先;daemon 所有权层以本补丁为准**。
- 冻结稿顶部已加指针指向本补丁。

**背景**:c1 在 worktree `ccbd-rust-wt-graft-c1`(分支 `feat/gateway-graft-modD`)已按冻结稿落地可编译的 `GatewayCore`/UDS listener/internal bridge/home-env 剥离与定点 AC 测试(单飞/零凭据/通道隔离/失败缓存/桥动态端口全绿),但发现冻结稿定义了网关**核心逻辑**却未定义它**挂进 daemon 的所有权点**(`rpc::Ctx`/`ahd.rs`/master spawn plan)。这是设计完备性缺口,非实施误解。c1 原文阻塞见 `ccbd-rust-wt-graft-c1/.operator-question`。

**grep 实证基线(本补丁所有裁决据此)**:
- `Ctx` 是 `#[derive(Clone)]`、**逐连接 clone**(`src/rpc/mod.rs:17-24,52`)→ 任何网关字段必须 `Arc` 包裹。
- 既有先例:`Ctx.tmux_server: Arc<TmuxServer>`(`src/rpc/mod.rs:23`),daemon 启动期 eager 构造(`src/bin/ahd.rs:86,101-106`)——**这是本补丁所有权结构的模板**。
- c1 已在 worktree 落地:自由函数 `register_worker(core: Arc<GatewayCore<U>>, worker_id, host_uds_path) -> GatewayListener`(`claude_gateway.rs:347`)、`GatewayListener::shutdown()`(优雅关停 oneshot+join,`:340`)、`handle_gateway_connection` 内 `spawn_blocking(core.forward_messages)`(`:383`)——**缺的正是"谁持有 core 单例 + 这些 GatewayListener 句柄 + seed"**。
- c1 已把 worker 侧 UDS 绑定用 `ReadWriteBind` 落地(`ccbd-rust-wt-graft-c1/src/rpc/handlers/agent.rs:201`,`SandboxOverrides.extra_binds: Vec<ReadWriteBind>`,`src/sandbox/mod.rs:18,28`)。
- 当前 `master_command_with_env(project_id, cmd, env_state, daemon_unit, extra_env_vars)`(`src/sandbox/systemd.rs:87`;平台实现 `macos/scope.rs:166`、`windows/scope.rs:154`)**不接受 sandbox bind overrides**。
- host 侧真实登录文件锚点:`~/.claude/.credentials.json`(`src/provider/home_layout.rs:659` 旧 `link_credentials` 的 source;冻结稿删的是**沙箱内 symlink**,非此 host 本体)。

---

## 裁决 1 · daemon 级所有权结构 —— `ClaudeGatewayService`(Arc 挂 `Ctx`,eager 持有 + lazy 内部初始化)

**裁决**:新增 daemon 拥有的 `ClaudeGatewayService`,以 `Arc` 挂进 `rpc::Ctx`,**镜像 `tmux_server: Arc<TmuxServer>` 的既有形态**。

- **`Ctx` 新增字段**:`pub claude_gateway: Arc<ClaudeGatewayService>`(`src/rpc/mod.rs` Ctx 结构)。因 `Ctx` 逐连接 clone,`Arc` 保证廉价 clone、单例共享。
- **`ClaudeGatewayService` 拥有(三件)**:
  1. **core 单例(lazy)**:`OnceCell<Arc<GatewayCore<ProductionUpstream>>>`(或 `Mutex<Option<...>>`),持唯一真实 seed 派生的 `GatewayCore`。**全进程一个 core / 一份 seed**(冻结稿拓扑铁律:隔离在 listener 层不在 core 层)。
  2. **listener 注册表**:`Mutex<HashMap<String, GatewayListener>>`,key = agent_id(worker)或 session_id(master),value = c1 已有的 `GatewayListener` 句柄(`claude_gateway.rs:340`)。
  3. **事件 sink**:`RecordedCredentialEvents`(冻结稿失败可观测,c1 已有)。
- **初始化时机(裁决:eager 持有 + lazy 内部初始化)**:
  - **holder eager**:`ahd.rs:101-106` 构造 `Ctx` 时**同步 new 一个空的 `ClaudeGatewayService`**(不读凭据、不起 listener),塞进 `Ctx`。廉价、无 IO、不会因未登录 claude 而拖垮 daemon 启动。
  - **core + seed lazy**:**首个 claude 席位(worker 或 master)spawn 时**才读 seed、构造 `GatewayCore`。用 `OnceCell`/单飞锁保证并发首 spawn 只初始化一次。
  - **理由(为何 lazy 而非 eager 读 seed)**:①非每个 session 都用 claude(a1=codex、a2=gemini),eager 读 seed 会对无关 session 徒劳失败;②用户可能在 ahd 启动**之后**才在 host 侧 `/login`,eager 会锁死"启动时的未登录态";③lazy 让"seed 不存在"退化为**该 claude 席位 spawn 失败并给出清晰错误**(见裁决 2),而非 daemon 起不来。
- **驳回 c1 阻塞选项 A 的"顺手扩大 daemon 重构"担忧**:本裁决**不重构 daemon**——只加一个 `Arc` 字段 + 一个 service 结构体,复用 `tmux_server` 既有模式,不触碰轨2 编排底座。范围克制满足。

**c1 落点契约**:
- `ClaudeGatewayService::new()` → 空 holder(eager,`ahd.rs`)。
- `ClaudeGatewayService::register_worker(&self, agent_id, host_uds_path) -> Result<GatewayWorkerTopology,_>`:首调 lazy-init core(单飞)→ 调既有自由函数 `register_worker(core.clone(), agent_id, host_uds_path)` → 把 `GatewayListener` 存进注册表 → 返回拓扑供 spawn 注入 env/bind。
- `ClaudeGatewayService::register_master(&self, session_id, host_uds_path)`:同上,key=session_id,生命周期见裁决 4。
- `deregister(&self, key)`:从注册表取出 `GatewayListener` 并 `.shutdown().await`(agent 退出/session 结束时)。

---

## 裁决 2 · seed credential 来源与回写 —— host `~/.claude/.credentials.json`,容忍式读取,内存为主 + 守卫式原子回写

### 2.1 来源(裁决:复用 host 本体登录文件,不新造 ahd 独占副本)

- **读取路径**:`<host_home>/.claude/.credentials.json`,`host_home` = ahd 进程真实用户 home(env `HOME`,复用代码库既有 home 解析)。这正是 `home_layout.rs:659` 旧 `link_credentials` 的 source 本体——**冻结稿删的是沙箱内 symlink,不是此 host 文件**,二者不冲突。
- **不新造 `~/.config/ah/credentials.json` 独占副本**(design-rev 拓扑图 §39 的设想):范围克制,读用户既有登录文件更简单,且是本补丁 brief 明确指向。将来若要 ahd 独占目录,另立独立任务。

### 2.2 存储文件 schema(裁决:容忍式规范化读取——**修正 A 臂的 schema bug**)

> **关键:存储文件 schema 与 OAuth 线上响应 schema 是两层,不可混用。** A 臂 `perform_real_refresh` 的**存储文件解析**(`ARM-A-diff.patch:1626`)用了 top-level snake_case `access_token`/`refresh_token`——**这不匹配 Claude Code 真实存储格式**(camelCase,证据:事故链 `expiresAt: 0`、B fixture `accessToken`/`refreshToken` @ `ARM-B-diff.patch:1686`、CLI 自身 JS `expiresAt: n*1000` @ `ARM-A-diff.patch:694`)。**c1 禁止复制 A 的 `:1626` 解析**;B 则**根本没写文件 reader**(`GatewayCore::new` 直接吃 `TokenSet`)。故此 reader 是本补丁**新定义的契约边界**。

**存储文件 reader 契约(容忍式,机制不断言)**:
1. 定位 OAuth 对象:若存在 top-level `claudeAiOauth` 键 → 用其子对象(Claude Code 真实嵌套格式);否则用根对象(容忍 B fixture 那种扁平变体)。
2. 字段提取(camelCase 主、snake_case 兜底):
   - `accessToken`(兜底 `access_token`)→ `TokenSet.access_token`
   - `refreshToken`(兜底 `refresh_token`)→ `TokenSet.refresh_token`
   - `expiresAt`(**epoch 毫秒**,兜底 `expires_at`)→ `TokenSet.expires_at`(`UNIX_EPOCH + Duration::from_millis(expiresAt)`)
3. **`expiresAt <= 0` 或缺失 → 视为已过期**(而非报错):触发首次请求立即刷新——这恰好优雅处理事故里的 `expiresAt:0` 登出残根(残根 → 立即刷新 → 若 refresh_token 也失效则 `invalid_grant` → 30s 失败缓存 → CLI 见清晰 401,而非静默挂死)。
4. 文件缺失/JSON 不可解析/缺 `refreshToken` → **lazy-init 失败,该 claude 席位 spawn 报清晰错误**(如 "Claude seed credentials not found/invalid on host; run /login"),非 panic、非静默。
5. **c1 复核项(诚实标注)**:上述嵌套/字段名以事故+CLI-JS+B fixture 三方证据推定;实施时须**对一台真实已登录 host 的 `.credentials.json` 实测校验**,若真实 schema 与此有出入,以真实文件为准调整 reader(reader 是兼容边界,不是硬断言)。

### 2.3 回写策略(裁决:内存为主 + **守卫式原子回写**,支持 RTR 跨重启存活)

- **内存为主**:刷新成功后新 `TokenSet` 写进 `GatewayCore.token` RwLock(c1 已有 `*self.token.write() = refreshed`),daemon 生命周期内即时生效。
- **回写(RTR 跨 ahd 重启存活,必需)**:OAuth 刷新可能**轮转 refresh_token**;若不持久化,ahd 重启后重读旧 refresh_token,服务端已 RTR 作废 → `invalid_grant` → 须重登。故**必须回写**轮转后的 refresh_token 到 host 文件。
- **回写守卫(incident 铁律落地,不可省)**:回写前 `canonicalize` 目标路径,过 `validate_credential_path_not_wsl_windows_mount`(冻结稿 B `:270`):
  - 若解析路径落在 `/mnt/c`(或 symlink 逃逸出 Linux home)→ **不回写**,仅内存持有 + 记 warning(退化为"ahd 重启后须重登",但**绝不写穿登出用户 Windows claude**——直接堵死事故复现)。
  - 否则(正常 Linux 本体文件)→ **原子回写**:写临时文件 + `rename` + `chmod 0600`,保持文件存在(不是删除重建)。原子 rename 杜绝 torn-write 残根。
- **驳回"纯内存 + 定期 flush"备选**:纯内存不持久化 → RTR 跨重启必断,而 RTR 冲突正是本 spec 根因,不可接受;"定期 flush"比"刷新后即写"多一个丢窗口且无收益。裁决=刷新成功即守卫式原子回写。

---

## 裁决 3 · `ClaudeUpstream::refresh` 生产实现 —— A 臂实证端点 + **修正 A 的错误合并 bug**

**来源标注**:endpoint/schema = **A 臂从 CLI 逆向实证**(`ARM-A-diff.patch` `TOKEN_URL` 常量 `:656`、`perform_real_refresh` `:1110`),经本补丁 brief 复核,**非 d1 新猜**。

### 3.1 线上契约(OAuth 标准 snake_case,与 §2.2 存储文件 camelCase 不同层)

- **Endpoint**:`POST https://platform.claude.com/v1/oauth/token`
- **请求**:form-encoded body:`grant_type=refresh_token&refresh_token=<refresh_token>`
- **成功响应**:JSON,`access_token`(string)、`refresh_token`(string,**可能轮转**→ 触发 §2.3 回写)、`expires_in`(u64 秒)→ `expires_at = now + Duration::from_secs(expires_in)`
- **失败态**:HTTP 400 + body `{"error":"invalid_grant"}` → **不可恢复**的 `invalid_grant`

### 3.2 错误映射(裁决:**区分瞬时错误 vs invalid_grant——修正 A 的合并 bug**)

> A 臂 `perform_real_refresh` 把**所有**失败(网络错误、非 400、解析失败)一律 `Err(SeedRefreshInvalidGrant)`(`ARM-A-diff.patch:1150,1155`)——**这是 bug**:一次网络抖动会被当成"登出",毒化 30s 失败缓存,把可重试的瞬时故障错判为账号级失效。c1 必须修正。

映射到冻结稿既有 `UpstreamError`(c1 `claude_gateway.rs`)与失败缓存状态机的对接:

| 上游结果 | 映射 | 与失败缓存对接 |
|---|---|---|
| 400 + `{"error":"invalid_grant"}` | `UpstreamError::InvalidGrant{body}` → `INVALID_GRANT_ERROR_CODE` | **进 30s 失败缓存**(冻结稿分歧点 5):后续请求直接返 401,不打上游;30s 后重试自愈 |
| 其它非 200(401/403/5xx 等) | `UpstreamError::Http{status,body}` | 瞬时,**不进 invalid_grant 缓存**;按冻结稿失败态处理(可下次请求重试),但仍受单飞门控 |
| 网络错误 / 超时 / 连接失败 | `UpstreamError::Http{status: 502/504 等网关语义, body}` | 同上,**瞬时可重试,严禁误判 invalid_grant** |
| 解析失败(200 但 body 无 access_token) | `UpstreamError::Http{status:502}` | 瞬时,记事件,可重试 |

- **单飞不变量**:刷新的读/写/失败缓存更新全部在冻结稿 §四 `refresh_lock` 内(c1 已实现),瞬时错误也不破坏单飞——任意时刻打向上游的刷新恒为串行单飞。
- **可观测**:`invalid_grant` 与瞬时错误都走 `RecordedCredentialEvents::record`,但 `error_code` 不同(前者 `INVALID_GRANT_ERROR_CODE`,后者 `REFRESH_FAILED_ERROR_CODE`),使"账号级失效"与"瞬时抖动"在 daemon 事件里可区分(对接冻结稿 AC-失败缓存)。
- **实现介质**:阻塞 HTTP 客户端(`ureq` 或 `reqwest::blocking`)在 `spawn_blocking` 内(c1 `forward_messages` 已在 blocking 线程调用,一致)。

---

## 裁决 4 · master UDS 生命周期所有权边界 —— `Ctx` 持有的 in-memory session-scoped 注册表(c1 选项 C 之"内存注册表"),DB 持久化 FD 驳回

**裁决**:master 的 `GatewayListener` 由**裁决 1 的 `ClaudeGatewayService` in-memory 注册表**持有(key=session_id),**不引入 DB 持久化 listener**。

**理由(第一性)**:
1. **FD 不可序列化**:listener 是活的 `UnixListener` FD + bound socket,是 OS 运行时资源,DB 存不了 FD。DB 能存的只有"该 session 有个 claude master 需要网关"这类事实——而这个事实**已经**在既有 session/agent 记录里,无需新表。
2. **"绑定 daemon session、不随 revive 销毁"天然满足**:master **revive** 是**子进程重启**,**不是 daemon 重启**。`ClaudeGatewayService` 挂在 `Ctx`、`Ctx` 活在 daemon 进程,**daemon 及其注册表寿命 >> master revive**。故 master listener 在 session 启动时 `register_master(session_id)` 建立一次后,**revive 只重启 master 命令 + 沙箱桥**(重连仍存活的 host UDS),listener 本身不销毁不重建——精确满足冻结稿分歧点 4 约束。
3. **daemon 重启才需 recovery,而那时所有 socket 都已失效**:daemon 重启后,`sandboxes/*/tmp/*.sock` 全是陈旧文件,每个 session 的 master+worker 都要重新拉起。故 daemon-restart 的 listener 重建**并入既有 agent/session reconcile 路径**:reconcile 在重新确立活跃 claude 席位时,对每个活跃 claude 席位**幂等调用** `register_worker/register_master`(register 内已 `remove_file` 陈旧 sock 再 bind,c1 `claude_gateway.rs:355-358`)。**不新增 DB schema、不写 FD**。

**boundary(不越界轨2)**:本裁决**不改 DB recovery spec**;只要求既有 startup reconcile 在恢复活跃 claude 席位时,回调 `ClaudeGatewayService::register_*` 重建 listener。这是一行接线级要求,不是持久化设计,轨2 编排底座重构不受影响。

**驳回 c1 选项 C 的"DB 持久化 + startup reconcile 重建"分支**:驳回"DB 持久化 listener 状态"部分(FD 不可持久化,且徒增一张无意义的表);采纳其"startup reconcile 重建"精神,但重建源是**既有活跃席位记录**,不是新持久化的 listener 台账。

**c1 落点**:
- session 启动(master spawn)→ `register_master(session_id, host_uds_path)`,host UDS = `~/.cache/ah/sandboxes/{session_id}/master/tmp/ah-gateway.sock`(过 `/mnt/c` 守卫)。
- master revive → **不动注册表**,只重跑 master 命令 + 桥(见裁决 5)。
- session 结束 → `deregister(session_id)` → `GatewayListener::shutdown().await`。
- daemon 重启 reconcile → 对每个活跃 claude session/worker 幂等 `register_*`。

---

## 裁决 5 · `master_command_with_env` 改动范围 —— 新增 `sandbox_overrides` 参数(承载 `ReadWriteBind` UDS 绑定)

**裁决**:`master_command_with_env` 必须能接受 **sandbox bind overrides**,以把 session 专属 UDS **读写**挂进 master 沙箱 `/var/run/ah-gateway.sock`,与 worker 路径完全对称。

**函数签名级方向(不写代码,写清"必须能做到 X")**:
- **facade** `src/sandbox/systemd.rs:87` 及**三平台实现**(`platform/linux/scope.rs` 的 `master_command_with_env`、`macos/scope.rs:166`、`windows/scope.rs:154`)统一**新增一个参数** `sandbox_overrides: &SandboxOverrides`(即 c1 已有、承载 `extra_binds: Vec<ReadWriteBind>` 的那个结构,`src/sandbox/mod.rs:14-28`)。
- 该函数**必须把 `sandbox_overrides` 穿进 worker `wrap_command` 已经在用的同一条 scope 组装 bind 路径**(即 master 与 worker 复用同一 bind-mount 装配逻辑,不为 master 另造分支)。
- **UDS 绑定必须是 `ReadWriteBind`,不能是 `ReadOnlyBind`**:CLI→桥→UDS 的 `connect()` 需要对 socket 的写权限;`ro` bind-mount 会使 `connect()` 得 `EROFS`/`EACCES`。这与 c1 worker 侧 `agent.rs:201` 用 `ReadWriteBind` 一致——master 照抄。
- **调用点** `src/rpc/handlers/sessions.rs:517` `spawn_prepared_master_pane`:构造 `SandboxOverrides { extra_binds: [ReadWriteBind{ host: <session_uds>, sandbox: "/var/run/ah-gateway.sock" }], ..default() }`,传入 `master_command_with_env`。
- **master env 分离注入(与冻结稿分歧点 7 一致)**:静态 env(`CLAUDE_CODE_USE_GATEWAY=1`、`ANTHROPIC_AUTH_TOKEN=fake_worker_jwt(session_id)`)走既有 `extra_env_vars`(c1 `master_watch.rs:798-805` 已有);**`ANTHROPIC_BASE_URL` 走沙箱内 wrapper exec 期动态端口注入**(与 worker 同一 `ah internal-bridge` 机制),不在 `master_command_with_env` 里写死端口。
- **兼容性**:现有其它调用点(`macos/scope.rs:163`、`windows/scope.rs:151` 用 `&HashMap::new()` 的 `master_command` 包装)按第一性同步加 `&SandboxOverrides::default()`,不留旧签名兼容 shim(冻结稿 §五 不后兼容原则)。

---

## 对 tasks.md G-Phase 的增量(供 c1 续跑 G1-G3)

在冻结稿第六节 tasks 大纲基础上,把 daemon 所有权落点并入相应 phase:

```
## Phase G1(补):daemon 网关服务所有权
- [ ] G1-5: ClaudeGatewayService 结构体(core 单例 OnceCell + Mutex<HashMap<String,GatewayListener>> 注册表 + RecordedCredentialEvents)
- [ ] G1-6: Ctx 新增 claude_gateway: Arc<ClaudeGatewayService>;ahd.rs:101 eager new 空 holder
- [ ] G1-7: seed reader(host ~/.claude/.credentials.json,容忍式 claudeAiOauth camelCase 规范化,expiresAt ms,<=0=过期)
        Test: 真实 schema 实测校验;缺文件→spawn 清晰报错;expiresAt:0→触发刷新
- [ ] G1-8: ProductionUpstream 实现 ClaudeUpstream::refresh(POST platform.claude.com/v1/oauth/token;错误映射区分 invalid_grant vs 瞬时)
        Test: 400 invalid_grant→失败缓存;网络错误→瞬时不毒化缓存
- [ ] G1-9: 守卫式原子回写(canonicalize + /mnt/c 守卫 + 临时文件 rename + 0600)
        Test: /mnt/c 路径→跳过回写仅内存;正常路径→原子回写轮转 refresh_token

## Phase G2(补):worker 接入 service
- [ ] G2-7: worker spawn 调 ClaudeGatewayService::register_worker → 存 GatewayListener 入注册表 → 注入 env/RW bind;agent 退出 deregister().shutdown()

## Phase G3(补):master 接入 service
- [ ] G3-3: master_command_with_env 新增 sandbox_overrides: &SandboxOverrides 参数(三平台 + facade),ReadWriteBind UDS
- [ ] G3-4: sessions.rs:517 构造 session UDS RW bind + register_master(session_id);revive 不动注册表只重跑命令+桥
- [ ] G3-5: session 结束 deregister(session_id).shutdown();daemon 重启 reconcile 对活跃 claude 席位幂等 register_*
```

---

## 补丁裁决一句话摘要(5 点)

1. **daemon 所有权** = 新增 `ClaudeGatewayService`(Arc 挂 `Ctx`,镜像 `tmux_server`),持 core 单例 + listener 注册表 + 事件 sink;**holder eager 构造 / core+seed 首个 claude spawn 时 lazy 单飞初始化**(不因未登录拖垮 daemon 启动)。
2. **seed 来源** = host `~/.claude/.credentials.json`(非沙箱侧,与冻结稿删 symlink 不冲突);**容忍式规范化 reader**(`claudeAiOauth` camelCase 主/snake_case 兜底,`expiresAt` 毫秒,≤0=过期);**内存为主 + 守卫式原子回写**(canonicalize 过 `/mnt/c` 守卫→安全才 temp+rename+0600,否则仅内存)——修正 A 臂 snake_case top-level 解析 bug。
3. **`refresh` 生产实现** = A 臂实证端点 `POST platform.claude.com/v1/oauth/token`(form `grant_type=refresh_token`,响应 snake_case `access_token/refresh_token/expires_in`);**错误映射修正 A 的合并 bug**——仅 `400 invalid_grant`→进 30s 失败缓存,网络/超时/其它非 200→瞬时可重试不毒化缓存。
4. **master UDS 所有权** = `ClaudeGatewayService` in-memory session-scoped 注册表(key=session_id);**驳回 DB 持久化 FD**(FD 不可序列化);"不随 revive 销毁"因 daemon 寿命 >> master revive 而**天然满足**;daemon 重启由既有 agent reconcile **幂等 register 重建**,不新增 DB schema、不越界轨2。
5. **`master_command_with_env` 改动** = 新增 `sandbox_overrides: &SandboxOverrides` 参数(三平台+facade),穿进 worker 已用的同一 bind 装配路径;UDS 必须 **`ReadWriteBind`**(connect 需写权限,ro→EROFS);调用点 `sessions.rs:517` 构造 session UDS RW bind + `register_master`;env 分离注入(静态走 extra_env,BASE_URL 走 wrapper 动态端口)。

---

*执笔:d1-claude(设计主笔,唯一执笔席)。本补丁为冻结扩展稿,交 c1 续跑 G1-G3;不重开冻结稿 7 分歧点;仅产 markdown 未碰代码/构建/git。所有裁决据 grep 实证锚点(A=`ARM-A-diff.patch`,B=`ARM-B-diff.patch`,c1=`ccbd-rust-wt-graft-c1`,main=当前 repo)。*
