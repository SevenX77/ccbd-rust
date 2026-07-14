# 模块 D 网关嫁接 · 冻结设计(FROZEN)

**Status**: FROZEN(设计主笔 d1-claude 执笔收敛,唯一执笔席)
**Date**: 2026-07-11
**Target Path**: `.kiro/specs/ah-per-worker-credentials/design-graft-frozen-2026-07-11.md`

> **补丁扩展(2026-07-12)**:本冻结稿定义了网关**核心逻辑**;其在 `ahd` 守护进程生命周期里的**所有权点**(`ClaudeGatewayService`/seed 来源与回写/生产 `refresh`/master UDS 生命周期/`master_command_with_env` 改动)由 **`.kiro/specs/ah-per-worker-credentials/design-graft-addendum-2026-07-12.md`** 补裁,是单一权威链的一部分。本稿 7 个分歧点方向不受补丁影响;daemon 所有权层以补丁为准。
**收敛输入**:
- A/B 终审:`research/ab-experiment-gateway/REVIEW-gateway-ab-verdict.md`(结论:B 代码质量胜,A 运行时管道更全,路径=嫁接)
- 冻结架构权威:`.kiro/specs/ah-per-worker-credentials/design-rev.md`(Plan B Fake Gateway,§3.1 JWT、§3.2 双防线、§4 单飞锁)
- 验收铁律:`.kiro/specs/ah-per-worker-credentials/incident-2026-07-11-wsl2-symlink-logout.md`(worker 不得持有写穿宿主凭据的链)
- 辩论材料:`research/ab-experiment-gateway/o1-debate-memo-graft-2026-07-11.md`(7 个分歧点的正反立场与红队防线)

**嫁接源**(两个 worktree,不合并整枝,按本设计逐件搬运):
- B 干净核:`ccbd-rust-wt-gw-b` @ `d55c26b`(`src/claude_gateway.rs`,362 行)
- A 运行时管道:`ccbd-rust-wt-gw-a` @ `7f5dc2b`(`src/provider/claude_gateway.rs`,920 行 + `scope.rs` 桥 + `agent.rs` 挂载)
- 嫁接目标:`main` @ `7bae3b1`(当前**无任何网关代码**,grep 实证:`GatewayCore/register_worker/CLAUDE_CODE_USE_GATEWAY` 在 main 的 src 里零命中)

---

## 〇、一句话收敛结论

**以 B 的 `GatewayCore` 干净核为"脑"(单飞、身份校验、header 重写、alg:none JWT、凭据面铲除全部保留),把 A 的运行时管道(per-worker UDS listener + 沙箱桥 + bind-mount + 上游 HTTP)作为"肢"降解重建接到脑上,二者用 `tokio::task::spawn_blocking` 缝合。A 的实现代码一行都不直接搬——只搬"它证明能跑通的拓扑",按第一性重写。** 冻结后无开放问题,7 个分歧点全部锁死(见第二节逐条裁决)。

嫁接的本质不是"取 A 补 B 的洞",而是:**B 已把设计的逻辑核心与凭据安全做到出货级且全绿,唯一实质欠交是"运行时服务器/桥/挂载没接上"(verdict 乙-7);A 证明了这条运行时管道能端到端跑通但代码质量不达标(verdict 甲、丙)。故嫁接=在 B 的干净核外面,按第一性重建 A 曾跑通的那层管道,而不是把 A 的臃肿/自造签名/python 桥/复制粘贴一起搬进来。**

---

## 一、嫁接后架构(总纲)

### 1.1 三层拆分(唯一合法结构)

```
                     ┌──────────────────────────────────────────────────────────┐
                     │ ahd 进程(宿主机,持唯一真实 seed 凭据)                   │
                     │                                                          │
   ┌─────────┐       │   ┌────────────────────────────────────────────────┐    │
   │credentials│──────┼──▶│ GatewayCore(进程内单例,B 干净核)              │    │
   │.json 0600 │  内存 │   │  - RwLock<TokenSet> + Mutex refresh_lock 单飞  │    │
   └─────────┘  分发  │   │  - valid_access_token / forward_messages       │    │
                     │   │  - validate_worker_identity(通道 vs JWT → 403)│    │
                     │   │  - + 失败状态缓存(30s TTL,本设计新增)         │    │
                     │   │  - ClaudeUpstream trait(生产=阻塞 HTTP 客户端) │    │
                     │   └────────────────────────────────────────────────┘    │
                     │        ▲ spawn_blocking            ▲ spawn_blocking       │
                     │        │(同一个 core 实例)        │                      │
                     │   ┌────┴─────────┐            ┌────┴─────────┐            │
                     │   │Listener(A)   │            │Listener(B)   │  ← 薄翻译层 │
                     │   │per-worker UDS │            │per-worker UDS │  字节流↔    │
                     │   │worker_id=A    │            │worker_id=B    │  GatewayReq │
                     │   └────┬─────────┘            └────┬─────────┘            │
                     └────────┼──────────────────────────┼─────────────────────┘
              host UDS: sandboxes/A/tmp/ah-gateway.sock   sandboxes/B/tmp/...
                        (bind-mount → /var/run/ah-gateway.sock,各自沙箱内)
                              │                            │
     ┌────────────────────────┼──────┐   ┌─────────────────┼───────────────────┐
     │ Sandbox A(--user --scope)     │   │ Sandbox B                            │
     │  ah internal-bridge(内置 Rust)│   │  ah internal-bridge                  │
     │   127.0.0.1:<dynP_A> ↔ UDS     │   │   127.0.0.1:<dynP_B> ↔ UDS           │
     │  claude CLI                    │   │  claude CLI                          │
     │   USE_GATEWAY=1                │   │   USE_GATEWAY=1                      │
     │   BASE_URL=localhost:<dynP_A>  │   │   BASE_URL=localhost:<dynP_B>        │
     │   AUTH_TOKEN=FAKE_JWT_A(UUIDv4)│   │   AUTH_TOKEN=FAKE_JWT_B              │
     └────────────────────────────────┘   └──────────────────────────────────────┘
                              │(host 127.0.0.1 与沙箱共享,见 §三 安全)
                              ▼
                     https://api.anthropic.com(host 侧 HTTPS,真 Access Token)
```

**拓扑铁律**:
- **一个 `GatewayCore` 实例 / 一份真实 seed 凭据**——不是每 worker 一个核。多租户隔离在 **listener/通道层**,不在核层。这与 B `gateway_worker_topology`(`wt-gw-b/src/claude_gateway.rs:278`)+ A `register_worker(agent_id)` per-worker listener(`wt-gw-a/src/provider/claude_gateway.rs:738`)一致。
- **同步核 + 异步肢的缝合**:B 的 `GatewayCore` 是同步的(std `RwLock`/`Mutex`,`forward_messages` 是 `fn` 非 `async fn`,`wt-gw-b/src/claude_gateway.rs:108`)。异步 UDS listener 解析出 `GatewayRequest` 后,在 `tokio::task::spawn_blocking` 里调 `forward_messages`(其中含阻塞的上游 HTTPS 刷新/转发),再把 `GatewayResponse` 写回 UDS。**保留 B 干净同步核,阻塞 IO 全部隔离进 blocking 线程池——这是 A(`spawn_blocking`+`ureq`,`wt-gw-a/...:218,485,827`)已验证可行的模型,但接到 B 的核上而非 A 的复制粘贴处理器上。**
- 生产 `ClaudeUpstream`(B trait,`wt-gw-b/src/claude_gateway.rs:55`)实现用阻塞 HTTP 客户端(`ureq` 或 `reqwest::blocking`,实施择一)做 `refresh` 与 `messages` 两个真上游调用;测试实现继续用 B 的 `RecordingUpstream` mock。

### 1.2 请求生命周期(冻结时序)

1. **spawn 期(ahd)**:为 worker 生成 UUIDv4 `worker_id`;`GatewayCore` 单例已持 seed;调 `register_worker(worker_id)` 起该 worker 专属 `UnixListener::bind(host_uds_path)`;host UDS = `~/.cache/ah/sandboxes/{worker_id}/tmp/ah-gateway.sock`(B `gateway_worker_topology:283`,已过 `/mnt/c` 守卫 `:270`);bind-mount 进沙箱 `/var/run/ah-gateway.sock`(A `agent.rs:135`)。注入**静态** env:`CLAUDE_CODE_USE_GATEWAY=1`、`ANTHROPIC_AUTH_TOKEN=fake_worker_jwt(worker_id)`(B `:235`)。
2. **exec 期(沙箱内 wrapper)**:先起 `ah internal-bridge`,桥自身 `bind 127.0.0.1:0` 拿到内核分配的**动态端口 dynP**(消除 TOCTOU,见分歧点 7 裁决);wrapper 用 dynP 组出 `ANTHROPIC_BASE_URL=http://localhost:<dynP>` 注入,健康探测通过后 `exec claude`。
3. **调用期**:CLI 见 `USE_GATEWAY=1` → 不读 `.credentials.json`(沙箱内根本没有),带 `Authorization: Bearer <FAKE_JWT>` 打 `localhost:<dynP>` → 桥 `copy_bidirectional` 转发到 UDS → host listener 收连接,**通道身份=该 listener 绑定的 worker_id**(不由客户端提供)→ `spawn_blocking(forward_messages)`:`validate_worker_identity`(通道 worker_id vs JWT worker_id,不符 403,B `:150`)→ `valid_access_token`(单飞刷新 + 失败缓存)→ `rewrite_authorization`(假 JWT 换真 Access Token,B `:182`)→ 阻塞 HTTPS 转发官方 → 响应原样写回 UDS → 桥回 CLI。

---

## 二、七个分歧点逐条裁决(全部锁死,无开放问题)

> 格式:**裁决** → 理由 → 对 o1 立场的采纳/驳回 → **冻结契约**(可测)。

### 分歧点 1 · UDS 服务器落地方式 —— 裁决:**B 的 `GatewayCore` + 薄 per-worker UDS 翻译层;禁止搬 A 的 `register_worker`**

- **理由**:A 的 `register_worker`(`wt-gw-a/...:738`)与测试副本 `worker_gateway_for_test` 约 180 行复制粘贴(verdict 甲-4、丙),改生产不红,是"测试验证的不是出货代码"的病根。B 的 `GatewayCore<U>`(`wt-gw-b/...:91`)已把鉴权/刷新/重写降解为可 mock 的纯结构化处理器。嫁接只需补一层**只负责"字节流 ↔ `GatewayRequest`/`GatewayResponse`"翻译**的 listener,核心逻辑全交 `forward_messages`。
- **采纳 o1**:强烈推荐立场(o1 分歧点 1 核心倾向)。
- **驳回 o1 反方**:"A 单文件扁平更好打补丁"——驳回。verdict 丙已实证 A 团队在单处函数里 14 轮误诊一个一行可修的契约冲突,"好打补丁"是伪优势。
- **冻结契约**:
  1. listener 只做翻译;**禁止存在任何 `worker_gateway_for_test` / 网关测试平行副本**(与分歧点 6 联锁)。
  2. **裸 HTTP 解析必须硬编码上限**(o1 红队 Slowloris/内存耗尽防线):Header 累计 ≤ **8 KB**、Body ≤ **10 MB**,超限立即 `400 Bad Request`;UDS 读写注入 **15s tokio `timeout`**,防慢连接死锁 tokio 协程。这三条是安全防线,必须落地并有测试(超限→400、超时→连接关闭)。
  3. HTTP 行/头具体切分算法属实施自由度,不在 spec 锁死;只要满足 8KB/10MB/15s 三个可观测边界。

### 分歧点 2 · 沙箱桥接方式 —— 裁决:**`ah` 二进制内置 Rust 转发子命令 `ah internal-bridge`;彻底删除 python3 heredoc 桥**

- **理由**:A 的 `build_python_bridge_script`(`wt-gw-a/scope.rs:301`)+ `wrap_claude_bridge`(`:333`,`bash -c "python3 -c '...' & exec \"$@\""`)引入沙箱 **python3 硬依赖**、朴素 `recv(4096)` 循环、后台 `&` 崩溃 CLI 无感挂起(verdict 甲-3/甲-5)。且它**无条件**前置桥(连 `unsafe_no_sandbox` 也套),正是 A 那条确定性红测试(`scope.rs:656` `"bash"!="env"`,verdict 丙)的产品侧病根。
- **采纳 o1**:强烈推荐立场(o1 分歧点 2)。用 `std::env::current_exe()` 定位当前 `ah` 可执行文件,在沙箱内以 `ah internal-bridge --port <P> --uds /var/run/ah-gateway.sock` 拉起;转发用 `tokio::io::copy_bidirectional`(零拷贝、全异步、内存安全)。
- **驳回 o1 反方**:"python 内联热插拔灵活"——驳回。热插拔灵活性换不来生产可靠性;桥是安全/可用关键路径,必须编译期锁定、可观测、生命周期可判死。
- **冻结契约**:
  1. 沙箱内**零外部运行时依赖**:不得依赖 `python3`/`socat`/`nc` 等沙箱内工具存在。
  2. **桥不得静默挂死**(o1 红队核心失效模式):
     - wrapper 在 `exec claude` 前,对 `127.0.0.1:<dynP>` 做**主动健康探测**(留 ~500ms 启动宽限,阈值/重试次数属实施自由度),探测失败**立即终止 spawn 并报错**,严禁 silently hanging。
     - 桥 `stderr` 重定向到沙箱日志(如 `{sandbox_root}/bridge.err`),不得丢弃,保证事故可追溯。
  3. `current_exe()` 在沙箱内无执行权限的失效路径(o1 反方点):由 bind-mount 保证 `ah` 二进制路径在沙箱内可执行;若健康探测失败即走 (2) 的 fail-fast,不退化为挂起。

### 分歧点 3 · Fake JWT 签名方案 —— 裁决:**`alg:none`(B 忠实实现)+ worker_id 强制 UUIDv4;彻底删除 A 的全局 HMAC 签名**

- **理由(第一性重述威胁模型)**:沙箱是 `systemd-run --user --scope`(main `scope.rs:225-227`),**无 `PrivateNetwork`,与宿主共享 loopback 命名空间**。故真实威胁不是"伪造签名",而是:①同宿主同用户(`sevenx`)下 worker A 能扫到 worker B 桥的 `127.0.0.1:<dynP_B>`;②最坏情形下能读 B 的 `/proc/<pid>/environ` 直接拿到**已签好的** `ANTHROPIC_AUTH_TOKEN=JWT_B` 原样重放。**对"嗅探+重放",任何签名(全局 HMAC 或每-worker 密钥)都无效**(o1 分歧点 3 红队已证)。而对"猜 ID 伪造",UUIDv4 的 122 位熵已彻底封死。**结论:签名在本威胁模型下是花架子;真正的两道门是 (a) UUIDv4 封死伪造、(b) OS 进程隔离封死嗅探重放。** A 的全局单密钥签名(verdict 甲-3:隔离实际靠 per-socket 闭包而非签名)恰好印证"签名是点缀"。
- **采纳 o1**:采纳 `alg:none` + UUIDv4 + UDS 物理隔离一防线 + UDS-worker_id↔JWT-worker_id 一致性二防线;**放弃全局 HMAC/RSA 签名**(o1 分歧点 3 核心倾向)。
- **驳回 o1 未主张但设计 §3.1 失效模式 A 提的"网关生成 RS256 自签"**:本次不实施(见 §三 失效模式登记)。理由同上:对当前 CLI 无必要,对真实威胁无效。
- **冻结契约**:
  1. **JWT 格式锁死 `alg:none`**:`{"alg":"none","typ":"JWT"}` . payload . (空第三段),`exp=32503680000`、`sub=ah-worker-session`、`worker_id=<UUIDv4>`。B `fake_worker_jwt:235` + `fake_jwt_worker_id:244`(严格校验三段/alg/typ/exp/sub/非空 worker_id)原样保留。
  2. **worker_id 必须 cryptographically-secure UUIDv4**——这是安全前置条件,不是实施细节。ahd 生成 worker_id 处必须用 CSPRNG UUIDv4(实施须给出生成点;测试可断言格式为 UUIDv4)。
  3. **二防线通道校验保留且必须被真实运行时覆盖**:通道 worker_id(listener 绑定,非客户端提供)≠ JWT worker_id → 403 `WORKER_ID_MISMATCH_ERROR_CODE`(B `:161-170`)。B 已有逻辑测试 `ac4_gateway_rejects_fake_jwt_from_wrong_worker_channel`;嫁接后须再有**真 UDS listener 端到端**的该断言(补 verdict 乙-7 缺口)。
  4. Base64Url 用第三方库还是 B 手写辅助(`:312`)属实施自由度。

### 分歧点 4 · master 席位与 host env 剥离 —— 裁决:**完全继承 B 的凭据彻底剥离;master 无特例,同样接入 Gateway 管道**

- **理由**:2026-07-11 WSL2 真机事故(incident 现场:`/root/.claude/.credentials.json`→`/mnt/c/.../.credentials.json` 被 `expiresAt:0` 写穿,登出 master/d1/g1/g2 **及用户本人 Windows claude**)证明:任何让席位持有指向宿主物理凭据的链都是越栈高危。verdict 甲-2 记 A 只做 Worker 门控、master 仍走共享 symlink——正是被登出的那条链。**必须斩断,无折中。**
- **采纳 o1**:绝对坚守立场(o1 分歧点 4)。master 也在内存消费 token,不留特例通道。
- **驳回 o1 反方**:"master 生命周期长、应文件拷贝独立凭据"——驳回。独立副本会与 host 侧其它进程各自触发 OAuth 刷新 → RTR 轮转冲突 → refresh token 永久封禁(o1 红队失效模式,正是本 spec 根因)。
- **冻结契约**(B 已落地,嫁接原样继承,不得回退):
  1. `PROVIDER_AUTH_WHITELIST` **删除 `.claude/.credentials.json` 项**、`link_credentials` 函数整体删除(B `home_layout.rs:21` 白名单、原 link 区块已删)——任何 claude 角色(worker 与 master)都不再产生该 symlink。
  2. `collect_spawn_env` 剥离宿主 `ANTHROPIC_API_KEY/ANTHROPIC_AUTH_TOKEN/ANTHROPIC_BASE_URL` 透传;extra_env 仅放行"合法假 worker JWT"(`fake_jwt_worker_id(value).is_ok()`)与等于沙箱 base_url 的 `ANTHROPIC_BASE_URL`(B `manifest.rs:505-513`)。**注:因分歧点 7 端口改动态,`manifest.rs:513` 现在的 `value == SANDBOX_TCP_BASE_URL` 精确匹配必须改为"host 为 localhost 的前缀/host 校验",见第五节实施边界。**
  3. **master 复活链接入 gateway**:`master_watch.rs:798-805` 已把 master 启动 env 切为 `CLAUDE_CODE_USE_GATEWAY=1` + `ANTHROPIC_BASE_URL`(改动态)+ `fake_worker_jwt(session_id)`;`master_revival.rs:143` 已有 gateway-home 缺失告警。嫁接须补:**master 专属 UDS 生命周期绑定 daemon session,不随 master revive 销毁;master 桥随每次 master 拉起重启**(o1 缓解机制)。
  4. worker 侧 host UDS 路径必须过 `validate_credential_path_not_wsl_windows_mount`(B `:270`,拒 `/mnt/c`)——沙箱根在 `~/.cache/ah/sandboxes/` 的 Linux FS 上,天然满足;此守卫是回退/迁移期的兜底,不得删。

### 分歧点 5 · invalid_grant 重试风暴 vs 永久黏死 —— 裁决:**引入 30s TTL 失败状态缓存,门控在 single-flight 锁内;不做指数退避(记为后续独立任务)**

- **理由(两个极端都错)**:A 把 `last_failure` 永久写死、进程不重启永不自愈(verdict 甲-5:即便人工重登也不恢复);B 完全不缓存失败态,持续 `invalid_grant` 下每请求都抢锁打上游 → 刷新风暴(verdict 乙-5,`wt-gw-b/...:138-145` 直接返 Err 不改状态)——恰是 requirements 根因"反复失败刷新触发账号级速率保护"。**两者是对称取舍,正确解在中间:短 TTL 失败缓存 + 单飞。**
- **采纳 o1**:30~60s TTL 失败缓存立场(o1 分歧点 5),本设计锁 **30s**。
- **驳回/延后**:指数退避本次**不做**(控制嫁接范围),记为后续独立任务;这是 o1 明确留的实施自由度。
- **冻结契约**:
  1. `GatewayCore` 增字段:失败态 `Option<(FailureKind, Instant)>`(或等价),记 `failed_at` 与错误类型。
  2. **边界齐发穿透必须被单飞吸收**(o1 红队核心失效模式):TTL 到期瞬间若积压 N 个请求,第一个抢到 `refresh_lock` 的去打上游,其余 N-1 阻塞在锁上;若再失败,持锁者更新 `failed_at=now` 并让后续读到缓存失败直接返回——**任何时刻打向上游的刷新恒为串行单飞**。即:失败缓存的读/写/再刷新全部发生在 `refresh_lock` 语义内,不得在锁外并发穿透。
  3. **自愈**:30s 内所有请求直接返回缓存的 401(不打上游);30s 后第一个请求重试刷新,若用户已在 host 侧 `/login` 写入新凭据则成功自愈,**无需重启 ahd**。
  4. 可观测:失败仍走 B 的 `RecordedCredentialEvents::record(CredentialEvent::RefreshFailed{...})`(`wt-gw-b/...:140`),缓存命中期间的"抑制上游调用"行为须可测(mock upstream 断言:TTL 内 `refresh_calls` 不增长)。
  5. **跨平台注意**:`Instant` 是安全的(o1 反方提的 WASM/Windows 兼容担忧对 `std::time::Instant` 不成立,Windows/Linux 均原生支持);无需引入外部时钟抽象。

### 分歧点 6 · 测试策略 —— 裁决:**100% 生产路径;废除一切网关测试平行副本;真起 UDS listener + 内置 Rust 桥 + MockUpstream**

- **理由**:A 的 AC-1/2/4/6 打 `worker_gateway_for_test` 副本(verdict 甲-4),"改了生产、测试全绿、真跑挂掉"的虚假安全感。B 的 454 行条条锚定可观测契约、打真 `GatewayCore`/`prepare_home_layout`、`serial_test` 隔离(verdict 乙-4,9/10)。嫁接必须把 B 的纪律扩展到新增的运行时层。
- **采纳 o1**:绝对坚守立场(o1 分歧点 6)。
- **驳回 o1 反方**:"真起 socket 易 flaky、社区惯例用内存 mock 绕过 io"——部分驳回:核心逻辑测试**继续**用 B 的 `RecordingUpstream` mock(零网络);但**新增的 listener/桥/挂载层必须有真起 UDS + 真起桥的端到端测试**补 verdict 乙-7 缺口,flaky 风险用下列机制压制,不用"绕过"回避。
- **冻结契约**:
  1. **禁止任何 `worker_gateway_for_test` 或网关测试副本**(与分歧点 1 联锁)。
  2. **测试端口动态**:真起桥的测试用 `TcpListener::bind("127.0.0.1:0")` 拿随机空闲端口(与生产同机制),严禁硬编码 `8206`,杜绝并发 `AddrInUse`(o1 缓解机制)。
  3. **强制串行**:涉及物理网络/进程拉起/全局 env 修改的集成测试加 `#[serial_test::serial]`(B 已有 `global_env` 串行,`wt-gw-b/tests/...`)。
  4. **Drop Guard**:测试创建的 UDS 临时文件(`tempfile`)与后台桥子进程必须 Drop Guard 包装,panic 也 100% 清理,防描述符/进程泄漏。

### 分歧点 7 · 沙箱 TCP 端口分配 —— 裁决:**桥自身 `bind 127.0.0.1:0` 拿内核动态端口;废除 B 的固定 8206,也不采 A 的 `port_from_slot_id` 哈希**

- **理由(第一性,超出 o1 两个选项给出更优解)**:
  - 固定 `8206`(B `SANDBOX_TCP_BASE_URL:10`)在共享 loopback 下多 worker 并发必 `AddrInUse` 崩第二个(o1 分歧点 7,已被 main `scope.rs:225` `--user --scope` 无 netns 实证)——**必须废除**。
  - A 的 `port_from_slot_id(agent_id)` 哈希(`wt-gw-a/...:174`)有碰撞概率、且端口可由 agent_id 反推(给攻击者省了扫描),o1 自己也要加"bind 失败 +1 探测 + DB 回填"补丁——**那层探测其实是在手工重造内核 `bind(:0)` 的临时端口分配**。
  - **更优解**:让**沙箱内的桥自身 `bind 127.0.0.1:0`**,由内核原子分配空闲端口,桥读回实际端口,wrapper 用它组 `ANTHROPIC_BASE_URL` 后再 `exec claude`。**这一步同时消灭了"ahd 预分配→close→桥 re-bind"之间的 TOCTOU 竞态**(端口在桥进程里 bind 后不释放,直到 CLI 连上),比 A 的哈希、比"ahd 预分配"都干净。
- **采纳 o1**:采纳"必须动态端口"的裁决(o1 分歧点 7 冻结判定)。
- **驳回 o1 的落地形态**:o1 建议"类似 A 的哈希 或 ahd 分配+DB 持久化"——**驳回,改为桥内 `bind(:0)` 原子分配**(理由如上,消除 TOCTOU + 无碰撞 + 不可反推)。
- **驳回 o1 反方**:"动态端口难 tcpdump 抓包/增加端口生命周期负担"——驳回。诊断可从 `bridge.err`/sandbox DB 读回实际端口;生命周期随桥进程自然回收(桥死端口释放),无需独立台账。
- **端口非安全边界(必须写清的机制)**:动态端口**只解决可用性(防冲突),不提供隔离**——攻击者仍能扫端口。隔离由 §三 的 UDS bind-mount + 通道/JWT 校验 + hidepid 三者提供,与端口是否动态无关。此点必须在实施与审核契约里明确,防止误把动态端口当安全措施。
- **冻结契约**:
  1. 沙箱桥端口 = 桥进程 `bind 127.0.0.1:0` 内核分配,禁止固定端口、禁止 agent_id 哈希端口。
  2. `ANTHROPIC_BASE_URL` 由 wrapper 在 `exec` 前用桥实际端口填充(**静态 env 与动态 BASE_URL 分离注入**:`USE_GATEWAY`/`AUTH_TOKEN` 由 ahd spawn 期注入,`BASE_URL` 由沙箱内 wrapper exec 期注入)。
  3. 桥 `bind` 失败(极小概率)→ 健康探测失败 → fail-fast 报错 → ahd 重试 spawn(与分歧点 2 契约 (2) 复用同一条 fail-fast 路径,不新增分支)。

---

## 三、安全机制冻结(给机制不给断言)

### 3.1 威胁模型基线(所有安全裁决的共同前提)

沙箱 = `systemd-run --user --scope`(main `scope.rs:225-227`),**无私有网络命名空间,与宿主共享 `127.0.0.1` loopback**;所有沙箱与 ahd 同 OS 用户 `sevenx`。由此推出两条硬事实:
- **F1**:worker A 能连到 worker B 桥监听的 `127.0.0.1:<dynP_B>`(端口可扫)。
- **F2**:worker A 若能读 B 的 `/proc/<pid>/environ`,可直接拿 `JWT_B` 原样重放(签名对重放无效)。

### 3.2 双防线(在 F1/F2 事实下的真实防御)

| 防线 | 机制 | 前置条件 | 失效模式与回滚 |
|---|---|---|---|
| **一防线:UDS 物理隔离** | host UDS 仅 bind-mount 进对应 worker 的 mount namespace;沙箱 A 的边界内**根本不存在** B 的 UDS 文件(设计 §3.2)。 | bind-mount 正确(每 worker 只挂自己的 UDS)。 | 若挂错(A 拿到 B 的 UDS),退到二防线拦截。 |
| **二防线:通道 vs JWT 一致性** | listener 绑定的**通道 worker_id**(非客户端提供)必须 == JWT 内 `worker_id`,不符 403(B `validate_worker_identity:150`)。 | worker_id = UUIDv4(122 位熵,封死伪造);A 无法猜到 B 的 UUID 去伪造 `JWT_B`。 | 若 A 能读 B environ(F2)拿到真 `JWT_B` 重放,则二防线被绕过 → 依赖三防线。 |
| **三防线(前置必要条件,非本次代码):OS 进程隔离** | 沙箱间不得互读 `/proc/<pid>/environ`(hidepid 挂载 / 进程用户隔离)。 | 部署形态提供 hidepid 或等价门控。 | **若未开启(如 WSL2 裸跑全共享特权用户),F2 的嗅探+重放防线失守——这是本嫁接方案的最弱区域,见第七节。** |

**冻结判定**:一防线 + 二防线在代码层落地并测试;**三防线(hidepid/进程环境读取门控)作为运行前置必要条件写入 spec**(o1 弱区强烈建议),ahd/部署文档须声明:在未开启进程环境门控的宿主上运行多 worker,存在跨租户嗅探重放的残留风险。桥必须严格仅 `bind 127.0.0.1`,绝不监听外部/公共 IP(o1 缓解机制 3)。

### 3.3 凭据面(incident 铁律落地)

- 沙箱内**零真实 token**:无 `.credentials.json`(link_credentials + whitelist 项已删,分歧点 4);仅环境层假 JWT。RCE 攻击半径锁死在当前 worker 单次 API 配额内(设计 §3.3)。
- host 侧:真实 `credentials.json` 0600 归 ahd 独占;`GatewayCore` 内存持 Access Token(< 1h),即便网关被控也窃不到 Refresh Token(设计 §3.3 失效半径分级)。
- **incident 越栈铁律**:worker 侧不得持有指向 `/mnt/c` 宿主凭据的可写链——Plan B 下 worker 无 `.credentials.json` 自然满足;`validate_credential_path_not_wsl_windows_mount`(B `:270`)守回退/迁移期。

### 3.4 JWT 失效模式登记(设计 §3.1,本次不实施但记录回滚)

- **失效模式 A(未来 CLI 强制签名校验)**:当前 CLI 接受 `alg:none` 是本方案前置。若未来 CLI 版本强制数字签名,`alg:none` 被拒 → **触发 spec 修订**(届时评估网关自签 RS256,§3.1 缓解),不在本嫁接内预置签名(理由:对当前 CLI 无必要、对 F2 重放无效)。
- **失效模式 B(CLI 限制 exp 上限)**:当前用 `exp=32503680000`(公元 3000)。若 CLI 加 exp 跨度上限 → 改注入短周期(如 24h)可轮转假 JWT(§3.1 缓解),worker 生命周期通常 < 1-2h,24h 绰绰有余。本次不实施,记为失效模式响应。

---

## 四、单飞刷新 + 失败缓存状态机(冻结)

保留 B 的 std 锁双检单飞(`wt-gw-b/valid_access_token:115-147`),**在其上叠加 30s TTL 失败缓存 + 5 分钟过期安全缓冲**两处修正:

```
[forward_messages] → validate_worker_identity(403 on mismatch)
        │ ok
        ▼
[valid_access_token]
   ┌─ read RwLock<TokenSet> ──> expires_at > now + 5min ?  ──(yes)──> return access_token
   │                                    │(no / expiring)
   │                                    ▼
   │                          acquire refresh_lock (Mutex)  ← 单飞门
   │                                    │
   │              ┌─────────────────────┼──────────────────────┐
   │        (double-check)         (failure cache)         (do refresh)
   │        token 仍有效?          failed_at 距今 < 30s?     upstream.refresh
   │           │yes                    │yes                    │
   │           return                  return cached 401       ├─ Ok → write token, clear failure, return
   │                                   (不打上游)               └─ Err → set failed_at=now, record event, return 401
   └───────────────────────────────────────────────────────────────────────────────
```

- **5 分钟安全缓冲(对 B 的修正)**:B 的 `is_expired` 是 `now >= expires_at`(`wt-gw-b/...:21`),**无缓冲**;设计 §4 伪码用 `expires_at > now + 5min`。裁决**采纳设计的 5 分钟缓冲**——否则临界点 token 在 host→Anthropic HTTPS 在途时失效导致 401。这是第一性正确性修正,须落地(可测:token 剩余 4min 时 `valid_access_token` 触发刷新)。
- **30s 失败缓存**:分歧点 5 契约,门控在同一 `refresh_lock` 内,单飞吸收边界齐发。
- **不用 tokio `watch`**:设计 §4 伪码用 `watch` 唤醒;B 用 std Mutex 阻塞等待(等待者阻塞在 `refresh_lock`),更简单且已测。裁决保留 B 的 std 锁方案(经 spawn_blocking 在 blocking 池阻塞,可接受)。已知特性:一次刷新在途时,并发 worker 短暂阻塞 blocking 线程(单真实凭据、亚秒级),非缺陷。

---

## 五、第一性实施边界(不打补丁 · 不后兼容 · Kill List)

嫁接按第一性重建,**不保留任何过渡兼容层**。以下为硬性删除/禁止项(实施与审核共同契约):

| # | 禁止/删除 | 出处锚点 | 替代 |
|---|---|---|---|
| K1 | `worker_gateway_for_test` 及任何网关测试平行副本 | A `wt-gw-a/...:383-583`(与生产 `:738` 约 180 行孪生) | listener 只翻译,测试打真 `GatewayCore` + 真 UDS(分歧点 1/6) |
| K2 | python3 heredoc 桥 + `bash -c "... & exec"` 无条件前置 | A `scope.rs:301,333,276` | `ah internal-bridge`(内置 Rust,`copy_bidirectional`)(分歧点 2) |
| K3 | 全局 HMAC 密钥 JWT 签名 | A `wt-gw-a/...:73-88,100-135` | `alg:none` + UUIDv4(分歧点 3) |
| K4 | 固定端口 `SANDBOX_TCP_BASE_URL=...:8206` | B `wt-gw-b/...:10` | 桥 `bind :0` 动态端口(分歧点 7) |
| K5 | `port_from_slot_id` agent_id 哈希端口 | A `wt-gw-a/...:174` | 同 K4,内核原子分配(分歧点 7) |
| K6 | `link_credentials` + `PROVIDER_AUTH_WHITELIST` 的 `.claude/.credentials.json` 项 | B 已删(继承,不得回退) | 无 symlink,gateway 供 token(分歧点 4) |
| K7 | 宿主 `ANTHROPIC_*` env 透传进沙箱 | B 已剥离(继承) | `collect_spawn_env` 白名单式放行(分歧点 4) |
| K8 | `last_failure` 永久黏死态 | A `wt-gw-a/...:267-269,305-309` | 30s TTL 失败缓存(分歧点 5) |
| K9 | 生产函数体内 `unsafe { std::env::set_var(...) }` | A `home_layout.rs:149-154` | **禁止**在生产路径改进程全局 env;测试隔离用 `serial_test` |
| K10 | 无缓冲 `is_expired`(`now >= expires_at`) | B `wt-gw-b/...:21` | `expires_at > now + 5min` 安全缓冲(第四节) |
| K11 | `manifest.rs` 对 `ANTHROPIC_BASE_URL` 的**固定 URL 精确匹配** | B `wt-gw-b/manifest.rs:513`(`value == SANDBOX_TCP_BASE_URL`) | 改为 host=localhost 前缀/host 校验(因端口动态,K4) |

**边界铁律**:实施者不得为"少改动"而保留上述任一项;不得引入 A→B 的兼容 shim;不得因"A 已经跑通"而直接 `git cherry-pick` A 的 commit——A 的运行时逻辑必须按本设计**重写**接到 B 核上。

---

## 六、tasks.md 结构大纲(供 c1 实施,替换现有 `tasks.md` Phase 1-3)

> 现有 `tasks.md`(`.kiro/specs/ah-per-worker-credentials/tasks.md`)是 Plan B 的抽象框架;嫁接落地需按下列结构改写。B 已完成的项标 `[x]`(继承自 `d55c26b`),嫁接新增项标 `[ ]`。

```
## Phase G0: 干净核落地(继承 B @ d55c26b,验证性搬运)
- [x] GatewayCore<U> + ClaudeUpstream trait + 单飞双检          [B src/claude_gateway.rs:91-148]
- [x] validate_worker_identity / rewrite_authorization           [B :150-195]
- [x] fake_worker_jwt / fake_jwt_worker_id(alg:none)            [B :235-268]
- [x] PROVIDER_AUTH_WHITELIST 删项 + link_credentials 删除        [B home_layout.rs]
- [x] collect_spawn_env 剥离宿主 ANTHROPIC_*                      [B manifest.rs:505-513]
- [x] master_watch/master_revival 切 gateway env                 [B master_watch.rs:798-805]
- [x] validate_credential_path_not_wsl_windows_mount(/mnt/c 守卫)[B :270]
- [ ] G0-fix1: is_expired 改 5min 安全缓冲(K10 / 第四节)
- [ ] G0-fix2: GatewayCore 增 30s TTL 失败缓存,门控在 refresh_lock 内(分歧点5)
      Test: TTL 内 refresh_calls 不增长;TTL 后自愈;边界齐发单飞吸收

## Phase G1: 运行时 UDS 服务器层(嫁接 A 拓扑→B 核,重写)
- [ ] G1-1: per-worker UnixListener + 薄翻译层(字节流↔GatewayRequest/Response)
      调 spawn_blocking(GatewayCore::forward_messages);禁止 worker_gateway_for_test(K1)
- [ ] G1-2: 裸 HTTP 读取硬上限:Header≤8KB / Body≤10MB / 15s timeout(分歧点1契约2)
      Test: 超限→400;超时→连接关闭
- [ ] G1-3: 生产 ClaudeUpstream 实现(阻塞 HTTP 客户端做 refresh + messages)
- [ ] G1-4: GatewayCore 单例 + register_worker(worker_id)(host UDS = sandboxes/{id}/tmp/ah-gateway.sock)
      Test: 真 UDS 端到端——通道 worker_id≠JWT worker_id → 403(补 verdict 乙-7)

## Phase G2: 沙箱桥 + 端口 + 挂载(嫁接 A,重写为内置 Rust)
- [ ] G2-1: ah internal-bridge 子命令(copy_bidirectional,bind 127.0.0.1:0 动态端口)(分歧点2/7)
- [ ] G2-2: wrapper——起桥→读实际端口→组 ANTHROPIC_BASE_URL→健康探测(500ms 宽限)→exec claude
      桥 stderr→{sandbox_root}/bridge.err;探测失败 fail-fast 报错不挂起(分歧点2契约2)
- [ ] G2-3: bind-mount host UDS → 沙箱 /var/run/ah-gateway.sock(A agent.rs:135 逻辑,重写)
- [ ] G2-4: env 注入拆分——ahd spawn 期注 USE_GATEWAY/AUTH_TOKEN(JWT);wrapper exec 期注 BASE_URL
- [ ] G2-5: manifest.rs BASE_URL 校验改 host 前缀匹配(K11)
- [ ] G2-6: worker_id 生成点用 CSPRNG UUIDv4(分歧点3契约2)
      Test: 端到端——真起桥+真 UDS+MockUpstream,127.0.0.1:0 动态端口,serial + Drop Guard(分歧点6)

## Phase G3: master 席位接入(嫁接,重写)
- [ ] G3-1: master 专属 UDS 生命周期绑 daemon session(不随 revive 销毁);桥随 master 拉起重启(分歧点4)
- [ ] G3-2: master 端到端——revive 后仍能经 gateway 取 token,无 symlink 无级联登出

## Phase G4: 前置条件与失效模式文档(非代码,写入 spec/部署文档)
- [ ] G4-1: hidepid/进程环境读取门控作为运行前置必要条件声明(三防线,第七节最弱区)
- [ ] G4-2: JWT 失效模式 A/B 的回滚路径登记(§3.4)

## 后续独立任务(本次不做,记录)
- [ ] 指数退避(分歧点5);沙箱私有 netns 硬隔离(第七节最弱区终极解)
```

---

## 七、验收契约(AC,可观测可测)与最弱区域

### 7.1 关键 AC(嫁接后须全绿,锚定可观测契约)

| AC | 契约 | 观测点 |
|---|---|---|
| AC-单飞 | 并发 expired worker 请求仅触发 1 次上游刷新 | mock `refresh_calls()==1 && message_calls()==N`(B 已有 `ac1`) |
| AC-零凭据 | worker home 无 `.credentials.json` 且全树无真 token 字节 | 真 `prepare_home_layout("claude",Worker)` 递归扫描(B 已有 `ac3`) |
| AC-重写 | 上游见真 token;任何 header 不含假 JWT | mock 断言(B 已有 `ac4`) |
| AC-通道隔离 | 通道 worker_id≠JWT worker_id → 403,0 上游调用 | **真 UDS listener 端到端**(嫁接新增,补乙-7) |
| AC-失败缓存 | 持续 invalid_grant:TTL 内上游刷新不再增长;TTL 后自愈 | mock `refresh_calls` 时间序列(嫁接新增) |
| AC-桥不挂死 | 桥启动失败→健康探测失败→spawn fail-fast 报错(非挂起) | spawn 返回错误 + `bridge.err` 有内容(嫁接新增) |
| AC-端口不冲突 | 多 worker 并发各得独立动态端口,无 AddrInUse | 并发 spawn 两 worker,两端口不同且均可用(嫁接新增) |
| AC-master 无级联 | master revive 后经 gateway 取 token,无 symlink | 无 `.credentials.json` symlink + revive 后可用(嫁接新增) |

### 7.2 架构最弱区域(诚实标注)

**共享 loopback + 共享 OS 用户下的跨租户嗅探重放(§3.1 F2)。** `--user --scope` 无法给每个沙箱造物理隔离网卡,`127.0.0.1:<dynP>` 在同用户所有进程间可见;签名对重放无效(分歧点3已证)。二防线(UUIDv4+通道校验)封死"猜 ID 伪造"但**不封死"读 environ 拿真 JWT 重放"**——后者唯一的门是三防线 hidepid/进程环境门控。**若部署形态(尤其 WSL2 裸跑全共享特权用户)未开启该门控,此防线失守。** 故本设计将 hidepid 锁为**运行前置必要条件**(G4-1),终极解是给沙箱私有 network namespace(需超出 `--user` 的特权,记为后续独立任务,不在本嫁接范围)。

---

## 八、执笔收敛自述(o1 反方采纳/驳回台账 + 覆盖度)

### 对 o1 七个分歧点的裁决汇总

| 分歧点 | o1 核心倾向 | 本设计裁决 | 对 o1 的处理 |
|---|---|---|---|
| 1 UDS 服务器 | B 核+薄 listener | **采纳**,加 8KB/10MB/15s 硬限 | 采纳倾向;驳回"A 扁平好补丁"反方 |
| 2 沙箱桥 | 内置 Rust 子命令 | **采纳** `ah internal-bridge` | 采纳倾向;驳回"python 热插拔"反方 |
| 3 JWT 签名 | alg:none+UUIDv4 | **采纳**,第一性重述威胁模型 | 采纳倾向;额外驳回设计 §3.1 的 RS256 自签(登记为失效模式) |
| 4 master/env | 全程 gateway 无特例 | **采纳**,继承 B 全部剥离 | 采纳倾向;驳回"master 文件拷贝"反方 |
| 5 重试风暴 | 30-60s TTL 缓存 | **采纳 30s**,门控入单飞 | 采纳倾向;指数退避延后 |
| 6 测试策略 | 100% 生产路径 | **采纳** | 采纳倾向;部分驳回"绕过 io"——核心 mock+新层真起 |
| 7 端口分配 | 动态(哈希 或 ahd+DB) | **采纳"必须动态"裁决,驳回其落地形态** | 改为桥内 `bind :0`——消 TOCTOU、无碰撞、不可反推,优于 o1 两个选项 |

**独立于 o1 的第一性增量**(执笔席主张,非照单全收):
1. 明确"同步核 + 异步肢 via spawn_blocking"的缝合契约——A/B 都没把这层写清。
2. 端口改"桥内 `bind :0`"消灭 TOCTOU(o1 只给到哈希/ahd 预分配,均留竞态)。
3. 补 B 的 5 分钟过期安全缓冲(B 无缓冲,在途失效风险)。
4. 把"动态端口≠安全边界"写死,防实施/审核误判。
5. `manifest.rs:513` 固定 URL 精确匹配随动态端口必须改前缀校验(K11,grep 实证的连带改动)。

### 覆盖度与最弱区诚实报告

- **覆盖**:7 个分歧点全部给出明确裁决,零开放问题;架构/桥/签名/端口/master/失败态/测试策略全锁死;凭据面 incident 铁律落地;tasks.md 结构给 c1。
- **最弱区**:§7.2 跨租户嗅探重放——本设计以"hidepid 前置条件 + netns 后续任务"处理,**未在本嫁接内提供代码级终极隔离**,这是诚实的能力边界,非疏漏。
- **grep 实证边界**:所有 A/B 锚点均带 worktree 路径 + file:line(A=`wt-gw-a`@7f5dc2b,B=`wt-gw-b`@d55c26b);main 无网关代码系 grep 实证;沙箱共享 loopback 系 main `scope.rs:225-227` `--user --scope` 无 `PrivateNetwork` 实证。未实跑任何构建/测试(设计任务边界:只产 markdown)。
- **待实施者复核项**:B 的 master 复活链改动(`master_watch.rs:789-806`)在嫁接后 UDS 生命周期与 revive 的交互(G3-1)需实测;`collect_spawn_env` 白名单在动态 BASE_URL 下的前缀校验(K11)需实测不误放。

---

*执笔:d1-claude(设计主笔,唯一执笔席)。本稿为冻结稿,交实施线(c1)前由 master/operator 确认;PR/合并/冻结生效归 operator。*

