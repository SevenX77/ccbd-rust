# o1 辩论备忘录 · 模块 D 网关嫁接设计
> 创建时间: 2026-07-11
> 角色定位: 设计辩论席 (只发散/辩论/红队，不执笔最终收敛设计)
> 目标路径: [o1-debate-memo-graft-2026-07-11.md](file:///home/sevenx/coding/ccbd-rust/research/ab-experiment-gateway/o1-debate-memo-graft-2026-07-11.md)

---

## 导言：红队视角与核心安全断言

本备忘录针对 `REVIEW-gateway-ab-verdict.md` 提出的“将 A 的运行时管道嫁接到 B 的干净核心之上”的指导方针进行发散性对抗辩论。

我们始终遵循：**“安全关键点给机制不给断言”**。以下辩论将逐一剖析 6 个关键分歧点及新发现的第 7 个关键分歧点，提供正反双方最强硬的论据与失效模式分析，并明确标注哪些决策必须在“冻结设计”中锁死。

---

## 分歧点 1：UDS 服务器的落地方式

### 背景与现状
- **实现 A (`ARM-A-diff.patch:1637`)**：在 `ClaudeGateway` 直接实现 `register_worker`。该函数在 `tokio::spawn` 异步循环中处理网络连接，手动用 `read_http_request` 和 `write_response` 读写原始 TCP/HTTP 字节流，并在 `spawn_blocking` 中使用 `ureq` 发起上游请求。该实现与测试副本 `worker_gateway_for_test` 存在约 180 行高度重复。
- **实现 B (`ARM-B-diff.patch:884`)**：定义了 `GatewayCore<U: ClaudeUpstream>` 泛型骨架，将鉴权、Token 刷新与重写剥离为纯粹的结构化处理器，但无任何运行时 Socket 监听代码。

### 辩论与取舍

#### 核心倾向
**强烈推荐：在 B 的 `GatewayCore` 骨架上实现一个薄的 per-worker UDS listener 层，将 A 的网络服务器逻辑彻底降解。**

#### 论据与设计优势 (Pros)
1. **彻底消除代码重复 (DRY)**：通过将 Socket 监听与 HTTP 请求的读取/写入封装进独立的 listener，使其只负责“字节流 <-> `GatewayRequest`/`GatewayResponse`”的翻译，核心逻辑完全交给 `GatewayCore::forward_messages` 处理。测试时直接 mock `ClaudeUpstream` 即可，不再需要 `worker_gateway_for_test` 这种孪生测试副本。
2. **强类型错误传播与异常安全**：A 的原始处理函数在网络故障或 panic 时直接消亡，缺乏集中式错误追踪。降解为 `GatewayCore` 之后，所有的 `GatewayError` (`ARM-B-diff.patch:834`) 能够统一映射并安全返回给客户端，并支持落入 B 已经建立好的 `RecordedCredentialEvents` 观测日志。

#### 反方/最强反对意见 (Cons / Adversarial View)
*“引入 `GatewayCore` 泛型抽象层和 Upstream Trait 增加了调用链路的复杂性，且强行拆分了网络读写与核心逻辑。A 的单文件扁平设计虽然臃肿，但在出现网络并发挂起或粘包等底层问题时，更容易直接在单处函数中打补丁定位。”*

#### 红队失效模式与安全防线机制 (Mitigation)
- **失效模式：裸 HTTP 解析器的健壮性缺陷。** A 的 `read_http_request` 在没有第三方 HTTP 框架的情况下手写了解析逻辑 (`ARM-A-diff.patch:1490`)。如果 Worker 沙箱内的客户端恶意发送无上限的 Header 字节，或故意延迟发送 `content-length` 指定的 Body（Slowloris 攻击），该 parser 会无条件循环等待，导致对应的 Tokio 协程永久挂起，并耗尽宿主机内存。
- **缓解机制（必须锁死在设计中）**：
  1. 必须在 UDS 监听读取流中，硬编码 **Header 最大字节限制（如 8KB）** 和 **Body 最大字节限制（如 10MB）**，超限立即抛出 400 Bad Request。
  2. 必须为 UDS stream 的读写操作注入 **Tokio `timeout` 机制（如 15 秒）**，防止慢连接死锁。

#### 冻结判定
- **必须锁死**：必须在设计中确定使用 B 的 `GatewayCore` 架构加薄 Listener 翻译层的方案。禁止直接复制 A 的 `register_worker`。
- **实施自由度**：Listener 具体的 HTTP 字符解析（如何切分请求行与 Head）可以由开发人员在代码级实现时进行重构优化，不需要在设计 spec 中锁死细节，只要满足 Header/Body 的大小限制即可。

---

## 分歧点 2：沙箱桥接方式

### 背景与现状
- **实现 A (`ARM-A-diff.patch:832`)**：在 `scope.rs` 中使用内联 Python3 Heredoc 脚本 (`build_python_bridge_script`) 作为 TCP↔UDS 转发桥。利用 `bash -c "python3 -c '...' & exec \"$@\""` 的后台任务形式拉起。
- **设计原文 §3.2**：建议使用 `socat` 或“内置轻量级转发器”。

### 辩论与取舍

#### 核心倾向
**强烈推荐：在 `ah` CLI 二进制中直接内置一个轻量级 Rust 转发子命令（例如 `ah internal-bridge --port <PORT> --uds <PATH>`），通过 `std::env::current_exe()` 动态获取当前可执行文件路径并在沙箱中拉起。彻底消灭对外部 `python3` 或 `socat` 的运行依赖。**

#### 论据与设计优势 (Pros)
1. **零外部运行时依赖**：沙箱内部无需安装 `python3` 或 `socat`。尤其是在极简沙箱或 alpine-like 环境下，外部脚本和工具极易因缺失解析器而运行失败。
2. **高并发与内存安全**：利用 Tokio 的 `tokio::io::copy_bidirectional` 能够实现极其高效、零拷贝、完全异步的双向数据流转发，其并发性能和稳定性显著优于 Python 粗暴的 `recv(4096)` 并发多线程循环。
3. **强可观测性与生命周期绑定**：内置的 Rust bridge 可以捕获进程信号，在异常退出时将 Panic 信息或 stderr 重定向至沙箱日志目录（如 `{sandbox_root}/bridge.err`），使 daemon 能感知并判定“桥梁已死”，避免 CLI 发生无响应挂起。

#### 反方/最强反对意见 (Cons / Adversarial View)
*“Python3 bridge 是解释型代码，直接以字符串内联在 `ah` 代码中，修改转发逻辑不需要重新编译二进制，开发和现场热插拔排障非常灵活。如果在 `ah` 内部实现子命令，每次网络转发逻辑的微小修改都需要重构并重新编译发布 `ah`。此外，如果 `std::env::current_exe()` 得到的路径在沙箱中没有执行权限，会导致转发器直接启动失败。”*

#### 红队失效模式与安全防线机制 (Mitigation)
- **失效模式：桥后台启动默默挂掉导致的 CLI 挂起。** 不管是 Python 还是 Rust 转发器，只要是用 `&` 在后台运行，一旦因为端口被占用或 UDS 无权限而启动失败，前台的 Claude CLI 并不知道，会继续向 `localhost:PORT` 发起请求，最终表现为不可恢复的超时挂起。
- **缓解机制**：
  1. **主动健康检查**：前台启动脚本在 `exec` 真实的 Claude CLI 之前，必须先在 loop 中用 `nc` 或简单的 socket 探测 `127.0.0.1:PORT` 的连通性，给桥转发器留出 500ms 的启动宽限期，若探测超时则立即终止进程并报错，防止 silently hanging。
  2. **输出流捕获**：将桥进程的 `stderr` 重定向到沙箱指定的日志文件，而非废弃，保证事故现场可追溯。

#### 冻结判定
- **必须锁死**：必须在设计中锁死“沙箱内使用内置 Rust bridge 子命令还是外部依赖 (Python3/socat)”这一决策，因为这直接决定了 `wrap_command` 的命令组装逻辑及 `ah` 二进制的功能范围。
- **实施自由度**：健康探测的具体超时阈值（如 500ms 还是 1s）和探测重试次数可留给实施阶段调试决定。

---

## 分歧点 3：Fake JWT 签名方案

### 背景与现状
- **实现 A (`ARM-A-diff.patch:73-88`)**：手写了全局 HMAC 密钥对 Fake JWT 进行签名。但真正的租户隔离逻辑其实只依赖 Socket 连接绑定的 `worker_id` 和解码 claims 的比对，签名仅作点缀。
- **实现 B (`ARM-B-diff.patch:1028`)**：忠实实现设计原文 §3.1 的 `alg: none` 签名规范（不含第三段签名，仅 `header.payload.`）。B 的核心代码中虽然写了 `validate_worker_identity` 的应用层对比，但因为缺失运行时服务器，并未得到实际测试覆盖。

### 辩论与取舍

#### 核心倾向
**推荐：采纳设计 §3.1 的 `alg: none` 无签名方案，配合 high-entropy UUIDv4 组成的 `worker_id`，在 per-worker UDS 物理隔离的第一防线上，辅以 UDS-worker_id 与 JWT-worker_id 一致性匹配的第二防线。放弃全局 HMAC/RSA 签名。**

#### 论据与设计优势 (Pros)
1. **免去密钥分发的复杂性**：如果采用签名，不管是全局共享密钥（安全形同虚设）还是每 Worker 独立密钥（增加 `ahd` 与沙箱间分发及存储密钥的负担），都会增加系统复杂度。`alg: none` 能极大保持假凭据的轻量与纯净。
2. **物理隔离与应用层校验双保险已足够**：通过 systemd bind-mount 机制，沙箱 A 物理上无法触及沙箱 B 的 UDS file。

#### 反方/最强反对意见 (Cons / Adversarial View)
*“网络命名空间是共享的。同宿主机下的沙箱 A 虽不能打开沙箱 B 的 UDS，但能够扫描并连接到沙箱 B 内部转发桥监听的 TCP 端口 `localhost:8207`。如果 Fake JWT 是无签名的 `alg: none`，且 Worker ID 可以被轻易猜到（例如递增的 ID），那么沙箱 A 的攻击者就能轻易伪造沙箱 B 的 Fake JWT，并通过 TCP 端口向网关假冒沙箱 B，从而窃取其真实 Token。”*

```
[Sandbox A] --(猜到 Worker B ID)--> [Connect to 127.0.0.1:8207 (B's Bridge)]
                                                 |
                                                 v (Bridge blindly forwards)
                                        [UDS-B (Gateway)] <-- (Saw JWT-B & UDS-B Match!)
                                                 |
                                                 v
                                        [Authentication Bypass!]
```

#### 红队失效模式与安全防线机制 (Mitigation)
- **失效模式：同用户下的进程环境嗅探与重放攻击。** 
  如果沙箱 A 的攻击者能够读取宿主机的进程树环境（因为它们以相同 OS 用户 `sevenx` 运行），攻击者不仅可以猜到 ID，还能直接读取沙箱 B 的环境变量 `ANTHROPIC_AUTH_TOKEN`。此时，即便该 JWT 是用高级密钥签名的，攻击者也可以**完整复制并重放该 Token** 发送至 `localhost:8207`。这意味着，**数字签名根本无法防御 Token 被嗅探后的重放攻击**。
- **缓解机制（必须锁死在设计中）**：
  1. **高熵 Worker ID**：Worker ID 必须采用 cryptographically secure UUIDv4，彻底堵死猜测攻击路径。
  2. **OS 级进程与环境隔离**：系统启动 Worker 时，必须使用 systemd 的进程隔离特性（如 `DynamicUser=yes` 或配置特定的用户隔离，或者挂载隐藏 PID 的 `/proc`），确保沙箱 A 绝对无法读取沙箱 B 的 `/proc/<pid>/environ` 环境变量，阻止 Token 被嗅探。
  3. **Bridge 绑定限制**：桥接转发器必须严格仅绑定在 `127.0.0.1` 环回口，绝不允许监听任何外部或公共 IP。

#### 冻结判定
- **必须锁死**：必须在设计中锁死 Fake JWT 的格式（`alg: none`）与 Worker ID 生成必须为 UUIDv4。
- **实施自由度**：具体在 Rust 中进行 Base64Url 编解码是用第三方库还是沿用 B 的手写辅助函数，由实施人员自行决定。

---

## 分歧点 4：master 席位与 host env 剥离

### 背景与现状
- **实现 B 的安全防线 (`ARM-B-diff.patch:1264, 1336`)**：
  1. 彻底从 `PROVIDER_AUTH_WHITELIST` 中删除 `.claude/.credentials.json`，在 `prepare_claude_overrides` 中删除 `link_credentials`（彻底根绝了 WSL2 下 symlink 指向宿主物理凭证被盖写的物理路径）。
  2. 在 `collect_spawn_env` 中屏蔽了宿主 `ANTHROPIC_API_KEY`、`ANTHROPIC_AUTH_TOKEN`、`ANTHROPIC_BASE_URL` 等环境变量的向内透传。
  3. 修改了 `master_watch.rs`，将 Master 的启动环境变量也切换为 `CLAUDE_CODE_USE_GATEWAY=1` 并注入假 JWT。

### 辩论与取舍

#### 核心倾向
**绝对坚守：必须完全继承 B 的凭证彻底剥离设计（物理上不创建任何凭据 symlink / 彻底清理 Env Passthrough）。同时，必须在实施阶段将 Master 席位也接入 UDS 网关与沙箱桥接逻辑中，不能有特例通道。**

#### 论据与设计优势 (Pros)
1. **防止事故重现**：WSL2 真机 symlink 登出事故证明了任何让沙箱持有物理凭据链接的方案都是高危的。将 Master 同样接入 Gateway，使得 Master 也在内存中消费 token，完全斩断了 Master 的泄露和写穿向量。
2. **统一的安全架构**：Master 和 Worker 在 API 访问层采用完全相同的机制（UDS + 桥接 + Fake JWT），减少了特例代码，使得安全审计更为清晰。

#### 反方/最强反对意见 (Cons / Adversarial View)
*“Master 进程是整个会话的控制器，生命周期远远长于单个任务 Worker，且在会话生命周期内可能多次重启（Revive）。如果将 Master 也套进 UDS 网关和 bridge 机制，一旦网关进程本身发生重启，Master 的连接就会断开，导致主控制链崩塌。因此 Master 应该通过文件拷贝的方式持有独立的凭据副本，而不是走网关。”*

#### 红队失效模式与安全防线机制 (Mitigation)
- **失效模式：Master 独立凭证副本导致刷新冲突。**
  如果 Master 采用“独立凭证文件拷贝”来规避网关依赖，由于 Master 进程和 Host 侧的 `ah` 其他进程是并发运行的，它们会在不同的时间点检测到 Token 过期并各自发起 OAuth 刷新。这将极易触发 Anthropic Upstream 的 OAuth **“Authorization Code Reuse” 或 “Refresh Token Rotation (RTR) 冲突”**，导致用户的 Refresh Token 永久失效被封禁。
- **缓解机制**：
  1. **Master 专用 UDS 寿命绑定**：网关为 Master 分配的 UDS 必须在 `session` 启动时建立，且其生命周期与整个 daemon 会话绑定，不随 Master 进程的重启（Revive）而销毁。
  2. **快速自愈桥**：Master 的桥在每次 Master 进程拉起时重新随同启动，确保连接建立。

#### 冻结判定
- **必须锁死**：必须在设计中锁死“彻底移除 `link_credentials` 和 WHITELIST 常量项”，以及“Master 同样接入 Gateway 管道”。不允许退回到文件拷贝或保留 symlink 的折中方案。

---

## 分歧点 5：invalid_grant 重试风暴 vs 永久黏死

### 背景与现状
- **实现 A (`ARM-A-diff.patch:1706`)**：一旦发生 `invalid_grant`，错误会被写入全局 `last_failure`，后续所有请求直接返回该错误，**进程不重启永不恢复（永久黏死）**。
- **实现 B (`ARM-B-diff.patch:925`)**：发生刷新失败时仅返回 `Err`，不缓存失败状态。高频并发请求下，每个请求在读锁判定过期后都会依次尝试抢占刷新锁并打向上游，带来**上游刷新风暴**风险。

### 辩论与取舍

#### 核心倾向
**推荐：在网关的核心 `CredentialsManager` 中引入一个带 30 秒 TTL（生存时间）的失败状态缓存。不在此次嫁接中引入复杂的指数退避（以控制任务范围），但必须将指数退避记录为后续独立任务。**

#### 论据与设计优势 (Pros)
1. **阻断并发风暴**：一旦上游返回 `invalid_grant`（说明 Refresh Token 彻底失效，如用户在别处登出），网关在内存中记录 `failed_at = Instant::now()` 和错误类型。在接下来的 30 秒内，所有 Worker/Master 的请求直接返回该缓存的 401 错误，不再打向上游。
2. **支持自动愈合**：30 秒的 TTL 足够短。如果用户在 Host 侧重新运行了 `/login` 写入了新凭证，网关在 30 秒后再次尝试刷新时就能成功自愈，无需重启 `ahd` 进程。

#### 反方/最强反对意见 (Cons / Adversarial View)
*“30 秒的 TTL 缓存依然会在边界点产生瞬间的并发请求（例如 30 秒过后的那一瞬间，若有 50 个请求积压，会同时抢锁并可能产生重试请求）。此外，在内存中引入时间戳和 `Instant` 增加了状态管理的复杂度，增加了在 WASM/Windows 跨平台编译时的兼容性风险。”*

#### 红队失效模式与安全防线机制 (Mitigation)
- **失效模式：并发在缓存失效时刻的齐发穿透。**
  当 30 秒 TTL 到期的一瞬间，如果积压了大量请求，它们会同时发现缓存已失效，并尝试重新刷新。
- **缓解机制**：
  结合 B 已有的 **Single-flight Mutex 刷新锁** (`ARM-B-diff.patch:916`)。当 30 秒过去，第一只抢到 `refresh_lock` 的协程去执行实际的 Upstream 刷新，其余 99 个协程会阻塞在 `refresh_lock` 处。如果刷新再次失败，该协程更新 `failed_at = Instant::now()` 并广播失败，其余 99 个协程被唤醒后直接读取缓存失败，从而确保**任何时候打向上游的刷新都是串行单飞的**。

#### 冻结判定
- **必须锁死**：必须在设计中锁死“网关必须具备失败状态缓存且其 TTL 在 30~60 秒之间”这一机制。
- **实施自由度**：是否实现更复杂的指数退避（Exponential Backoff）可留给后续迭代，本次实施仅限固定 TTL 缓存。

---

## 分歧点 6：测试策略

### 背景与现状
- **实现 A (`ARM-A-diff.patch:31-33`)**：绝大多数 AC 测试均调用 `worker_gateway_for_test` 测试专用的网关副本，而不是真正的生产网关 `register_worker`。改动生产网关不会引起测试爆红，破坏了测试保障效力。
- **实现 B (`ARM-B-diff.patch:56-60`)**：所有的单测和集成测试都直接打在生产代码 `GatewayCore` 与 `prepare_home_layout` 上，使用 `RecordingUpstream` 对上游进行 Mock，并用 `serial_test` 进行环境隔离。

### 辩论与取舍

#### 核心倾向
**绝对坚守：必须废除任何测试专用的“网关副本代码”，所有的集成测试必须通过真正的生产网关进行。通过将 UDS Listener、内置 Rust bridge 和 MockUpstream 在测试中真实拉起，以 100% 的生产路径测试端到端契约。**

#### 论据与设计优势 (Pros)
1. **测试即出货**：任何生产代码的改动（如 Header 重写规则改变）都会在测试中被即时捕捉，彻底杜绝 A 组“改了生产、测试全绿、但在真实运行时直接挂掉”的虚假安全感。
2. **测试用例的高可信度**：真实测试 UDS 的挂载、文件是否存在、网络代理状态码等物理边界，而不是只在内存里对 Mock 函数断言。

#### 反方/最强反对意见 (Cons / Adversarial View)
*“在 Cargo 的并发测试环境下，真实启动 Socket 监听和桥进程极易因为端口冲突、异步等待时序不一致或者系统描述符耗尽而发生偶发性测试闪红（Flaky Tests）。为了测试的绝对稳定性，使用内存 Mocks 并绕过网络 io 是 Rust 社区集成测试的普遍最佳实践。”*

#### 红队失效模式与安全防线机制 (Mitigation)
- **失效模式：并发集成测试下的 TCP 端口冲突。**
  如果在测试中硬编码了 TCP 端口（如 `8206`），并发运行多个 AC 测试时，它们会因为同时争抢绑定该端口而引发 `AddrInUse` 失败。
- **缓解机制**：
  1. **测试内动态端口分配**：测试用例启动 Rust bridge 时，必须通过向 OS 绑定 `127.0.0.1:0` 获取临时随机空闲端口，提取该端口号后关闭连接并传给 bridge 使用。
  2. **强制测试串行化**：对所有涉及物理网络、进程拉起和全局环境修改的集成测试，强力附加 `#[serial_test::serial]` 装饰器，强制其单线程串行执行。
  3. **生命周期 Drop Guard**：测试中创建的 UDS 临时文件（利用 `tempfile`）和启动的后台 bridge 子进程必须通过 Drop Guard 机制进行包装，保证测试不论成功还是 Panic 挂掉，相关物理资源与进程都能被 100% 自动清理。

#### 冻结判定
- **必须锁死**：必须锁死“测试必须使用生产代码，禁止保留 `worker_gateway_for_test` 或任何网关的测试平行副本”。

---

## 分歧点 7（新发现）：沙箱 TCP 端口分配策略

### 背景与现状
- **实现 A (`ARM-A-diff.patch:1651`)**：在 `register_worker` 中，每个 Worker 都会根据其 `agent_id` 通过哈希映射算法计算出一个专属于该 Worker 的动态端口（`port_from_slot_id`），桥转发器与环境变量会动态绑定该端口。
- **实现 B (`ARM-B-diff.patch:858`)**：在 `fake_gateway` 代码中，固定将 `SANDBOX_TCP_BASE_URL` 设置为 `http://localhost:8206`，假设所有沙箱都共享同一个固定端口。

### 辩论与取舍

#### 核心倾向
**强烈推荐：必须在设计中锁死“使用动态 TCP 端口映射策略”（类似于 A 的 `port_from_slot_id` 算法，或在会话启动时由 `ahd` 动态分配并持久化在 DB 中的空闲端口），彻底废弃 B 的固定端口方案。**

#### 论据与设计优势 (Pros)
- **防范端口冲突崩溃**：由于 `systemd-run --user` 在 WSL2/Linux 宿主机中默认共享主机的 Loopback 网络接口（非独立的 network namespace），当多个 Worker 沙箱或 Master 同时并行运行时，如果全部绑定 `8206` 端口，第二个及后续拉起的 Worker 将因端口占用直接崩溃。动态端口是保障多实例并发安全性的唯一可行机制。

#### 反方/最强反对意见 (Cons / Adversarial View)
*“动态端口意味着沙箱内的 `ANTHROPIC_BASE_URL` 在每次启动时都是随机且不可预测的。这增加了进程环境的诊断和抓包难度（无法通过固定端口做 tcpdump 过滤）。此外，系统必须维护端口的生命周期（分配与释放），增加了主控制平面的逻辑负担。”*

#### 红队失效模式与安全防线机制 (Mitigation)
- **失效模式：动态端口的冲突与哈希碰撞。**
  基于 `agent_id` 哈希生成的端口号存在理论上的碰撞概率，或者该哈希端口刚好已被宿主机上其他无关进程占用，导致 Worker bridge 启动 Address In Use 失败。
- **缓解机制**：
  1. **探测备选机制**：计算出的哈希端口在绑定前，bridge 必须先进行短暂的 bind 尝试，如果失败，则顺延 `+1` 探测下一个端口直到成功，并将最终绑定的真实端口作为 metadata 回填写入 Sandbox DB，不再是纯静态映射。
  2. **网关动态配置分发**：沙箱启动时读取 DB 中最终确认的端口号注入 `ANTHROPIC_BASE_URL`。

#### 冻结判定
- **必须锁死**：必须在设计中锁死“沙箱桥端口必须为动态分配模式”。

---

## 辩论备忘完整度与最弱区域报告

### 覆盖范围
本备忘录全面覆盖了模块 D 嫁接设计涉及的全部七个关键技术分歧：
1. **UDS 服务器架构设计**（`GatewayCore` 翻译层隔离）
2. **沙箱桥实现介质**（Rust 内置子命令 vs 外部 Python）
3. **假凭据签名方案与租户隔离安全边界**（`alg: none` + UUIDv4）
4. **Master 席位安全防线对齐与卷入**（排除 symlink / 全程 Gateway）
5. **异常状态处理的重试风暴与挂起防范**（30s TTL 失败缓存）
6. **测试策略的生产线契约断言**（消除 Twin Copy 测试副本）
7. **多进程网络环境下的端口冲突防御**（动态端口映射）

### 架构最弱区域 (Weakest Link)
本设计目前最薄弱的安全区域在于：**在 `--user` 非 Root 权限下，系统级沙箱的网络命名空间共享问题。**
由于无法对每个 systemd-run 容器创建物理上完全隔离的私有网络网卡，导致 `127.0.0.1:PORT` 事实上在宿主机同用户下的所有进程（包括其他沙箱）间是可见的。为了防止跨租户越权窃取，设计完全依赖于：
1. `worker_id` 的高熵 UUIDv4 特性。
2. systemd-run 环境对 `/proc` 进程环境变量读取权限的物理限制（防止 Token 泄露被重放）。
如果在极特殊的 WSL2 配置下未能开启进程环境读取门控（例如所有 Worker 均裸跑在完全共享特权的用户下），此防线将面临被嗅探并重放的风险。我们强烈建议 d1 在执笔冻结设计时，必须在 specs 中将 **“OS 级进程权限门控/hidepid 挂载配置”** 作为嫁接方案运行的前置必要条件予以锁死。



