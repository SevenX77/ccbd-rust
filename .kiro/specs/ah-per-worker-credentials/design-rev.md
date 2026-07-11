# ah Per-Worker Credentials — Design Revision (Plan B: Fake Gateway)

**Status**: Published (Design Oracle Revised)  
**Target Path**: [.kiro/specs/ah-per-worker-credentials/design-rev.md](file:///home/sevenx/coding/ccbd-rust/.kiro/specs/ah-per-worker-credentials/design-rev.md)  
**Date**: 2026-07-10  

---

## 一、 决策与取舍 (Verdict & Choice)

在 Phase 0 Spike 实证了 **“无法通过原生配置重定向 Claude CLI 的 OAuth Token 刷新请求”** 这一结论后，本设计对原方案进行了重构。我们明确排除 Plan A (HTTPS_PROXY MITM)，**选定并实施 Plan B: Fake Gateway (本地 HTTP 网关 + 凭证重写) 方案**。

### Plan 对比与选择理由：

| 维度 | Plan A: HTTPS_PROXY MITM | Plan B: Fake Gateway (已选定) |
| :--- | :--- | :--- |
| **网络层侵入度** | **极高**。需拦截 HTTPS 流量，生成自签名根证书，并在 Worker 沙箱内通过 `NODE_EXTRA_CA_CERTS` 强制注入以让 CLI Node 运行时信任，配置复杂且易引发安全性阻碍。 | **极低**。CLI 模型调用地址直接配置为本地 HTTP 网关（`http://localhost:GATEWAY_PORT`），**完全免去 TLS/HTTPS 证书注入**。网关在宿主机发起对官方端的 HTTPS 请求。 |
| **凭证暴露面** | **中等**。Worker 仍需持有某种凭据文件或触发刷新，且沙箱内存在与真实 OAuth 刷新服务器交互的协议影子，需在 Proxy 侧深度模拟 OAuth 刷新响应体。 | **零暴露**。Worker 内部物理上**不持有任何真实 Token**（无论是 Access 还是 Refresh Token）。仅持有环境变量层面的 Fake JWT，从源头上斩断了凭据在沙箱内被盗的风险。 |
| **实现复杂度** | **极高**。需要编写一个完整的 TLS 中间人代理（MITM），涉及动态证书生成、TLS 握手处理、连接劫持。 | **极低**。仅需实现一个普通的 HTTP 反向代理，将请求 Header 里的 Fake Bearer Token 替换为 Host 侧有效的 Real Access Token。 |
| **RTR 风险防范** | **一般**。若 Proxy 模拟 OAuth 刷新返回了不一致的凭证，CLI 可能因状态失常触发 RTR 警告导致全局 Session 被封禁。 | **绝对免疫**。Worker 从不发起 refresh token 刷新，所有真实刷新完全收拢在宿主机单例中，绝对杜绝 RTR 并发冲突。 |

---

## 二、 核心架构与工作流程 (Core Architecture)

### 1. 架构拓扑图

```text
+---------------------------------------------------------------------------------------------------+
| Host System (宿主机)                                                                               |
|                                                                                                   |
|  +--------------------+             (0600 file)                                                   |
|  | ahd Daemon         | <------------------------------> /home/sevenx/.config/ah/credentials.json |
|  | (持有唯一真实凭据) |                                                                           |
|  +--------------------+                                                                           |
|        |                                                                                          |
|        | (内存分发最新 Access Token)                                                              |
|        v                                                                                          |
|  +-------------------------------------------------------------------------+                      |
|  | Host-side HTTP Gateway (网关单例进程 / ahd 内置任务)                     |                      |
|  |  - 监听多个 Worker 独占 of Unix Domain Sockets                            |                      |
|  |  - 维护 Single-flight 刷新锁与 Token 缓存                               |                      |
|  +-------------------------------------------------------------------------+                      |
|        ^                                             ^                                            |
|        | (Unix Domain Socket)                        | (Unix Domain Socket)                       |
|        | /tmp/ah-worker-A.sock                       | /tmp/ah-worker-B.sock                      |
|        v                                             v                                            |
|  +-----------------------------------+         +-----------------------------------+              |
|  | Worker A Sandbox (沙箱 A)          |         | Worker B Sandbox (沙箱 B)          |              |
|  |                                   |         |                                   |              |
|  | [socat / TCP-to-UDS Bridge]       |         | [socat / TCP-to-UDS Bridge]       |              |
|  | TCP 8206 <--> UDS (bind-mounted)  |         | TCP 8206 <--> UDS (bind-mounted)  |              |
|  |         ^                         |         |         ^                         |              |
|  |         | (HTTP / Plaintext)      |         |         | (HTTP / Plaintext)      |              |
|  |   Claude CLI 进程                 |         |   Claude CLI 进程                 |              |
|  |   - USE_GATEWAY=1                 |         |   - USE_GATEWAY=1                 |              |
|  |   - BASE_URL=http://localhost:8206|         |   - BASE_URL=http://localhost:8206|              |
|  |   - AUTH_TOKEN=FAKE_JWT_A         |         |   - AUTH_TOKEN=FAKE_JWT_B         |              |
|  +-----------------------------------+         +-----------------------------------+              |
+---------------------------------------------------------------------------------------------------+
                                           |
                                           | (HTTPS / Authorization: Bearer <REAL_ACCESS_TOKEN>)
                                           v
                             +---------------------------+
                             | Anthropic Upstream API    |
                             | (https://api.anthropic.com|
                             +---------------------------+
```

### 2. 调用时序 (Sequence Flow)

1. **Worker 初始化**：
   - `ahd` 启动 Worker 沙箱时，在宿主机上为该 Worker 生成专用的 UDS 文件 `/tmp/ah-worker-{worker_id}.sock` 并将该 UDS 以只读或读写挂载到沙箱内 `/var/run/ah-gateway.sock`。
   - `ahd` 在沙箱内启动一个极轻量的桥接转发进程（例如 `socat` 或内部小型转发器），将沙箱内 `127.0.0.1:8206` 的 TCP 请求转发到 `/var/run/ah-gateway.sock`。
   - `ahd` 为 Worker 进程注入环境变量：
     - `CLAUDE_CODE_USE_GATEWAY=1`
     - `ANTHROPIC_BASE_URL=http://localhost:8206`
     - `ANTHROPIC_AUTH_TOKEN=<FAKE_JWT>`（其中包含 Worker 唯一的 ID）
2. **API 调用发起**：
   - CLI 发起模型请求，检测到 `CLAUDE_CODE_USE_GATEWAY` 被设置，因此不会去读取 `.credentials.json`，而是直接将包含 `Authorization: Bearer <FAKE_JWT>` 的请求发送到 `http://localhost:8206`。
3. **网关处理与重写**：
   - 请求通过 UDS 到达宿主机的 HTTP Gateway。
   - 网关接收到连接，首先根据 **“物理套接字来源”** 和 **“Fake JWT 中的 worker_id”** 双重判定发送方身份为 Worker A。
   - 网关读取当前有效的 Real Access Token。如果 Token 已过期，网关在宿主机发起 Single-flight 刷新（见下文），获取新 Token。
   - 网关重写 Header，将 `Authorization` 字段替换为 `Bearer <REAL_ACCESS_TOKEN>`，并发起 HTTPS 请求转发至官方的 `https://api.anthropic.com`。
4. **响应返回**：
   - 官方返回响应，网关将响应体原封不动通过 UDS 路由回沙箱内的 CLI。

---

## 三、 安全机制设计 (Security Mechanisms)

为了贯彻 **“安全关键点必须给机制，不许给断言”** 的铁律，以下对四大核心安全边界进行机制层面的定义。

### 3.1 机制一：Fake JWT 构造与失效模式

#### 1. 结构构造
我们利用 CLI 内部仅解析 JWT Payload 中 `exp` 字段的特性，构造一个 `alg: none` 且带有极长过期时间的 Fake JWT：
- **Header**:
  ```json
  {"alg":"none","typ":"JWT"}
  ```
  Base64Url 编码为：`eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0`
- **Payload**:
  ```json
  {
    "exp": 32503680000, 
    "sub": "ah-worker-session",
    "worker_id": "worker-A-uuid-550e8400"
  }
  ```
  *(注：`exp` 设置为 32503680000，即公元 3000 年，确保 CLI 的 P6o() 校验直接放行且永远不会触发本地刷新逻辑。)*  
  Base64Url 编码为：`eyJleHAiOjMyNTAzNjgwMDAwLCJzdWIiOiJhaC13b3JrZXItc2Vzc2lvbiIsIndvcmtlcl9pZCI6Indvcmtlci1BLXV1aWQtNTUwZTg0MDAifQ`
- **Signature**: 空字符串（因为 `alg` 是 `none`）。
- **最终拼接的 JWT**：
  `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJleHAiOjMyNTAzNjgwMDAwLCJzdWIiOiJhaC13b3JrZXItc2Vzc2lvbiIsIndvcmtlcl9pZCI6Indvcmtlci1BLXV1aWQtNTUwZTg0MDAifQ.`
  *(注意末尾的 `.` 必须保留，以符合 JWT 的三段式格式)*

#### 2. 失效模式 (Failure Modes) 及防御缓解：
1. **失效模式 A (强制 Signature 校验)**：如果未来版本的 Claude CLI 升级，在客户端强制对 JWT 进行数字签名校验（防止网关伪造 token），导致 `alg: none` 直接被拒绝。
   - *缓解机制*：宿主机网关在启动时，生成一对临时的自签名密钥对（如 RS256），并用私钥对 Fake JWT 进行签名。只要 CLI 的 `CLAUDE_CODE_USE_GATEWAY` 模式不需要在本地硬编码校验特定根证书公钥（事实上作为通用的私有网关配置，它不可能硬编码特定公钥），自签名 JWT 即可完美绕过校验。
2. **失效模式 B (过期时间上限校验)**：如果 CLI 增加安全策略，限制 `exp` 的最大跨度（例如不能超过 7 天），当检测到公元 3000 年的 token 时会抛出异常。
   - *缓解机制*：网关将 Fake JWT 的过期时间设置为较合理的短周期（如当前时间往后推 24 小时）。由于每个 Worker 沙箱的生命周期通常小于 1-2 小时，24 小时的有效期已绰绰有余。若 Worker 存活时间确实超过 24 小时，网关可通过更新沙箱环境变量或重置桥接端口的方式重新注入，而无需修改持久化配置。

---

### 3.2 机制二：多租户物理隔离与请求鉴别 (Multi-Tenant Isolation)

网关作为宿主机共享的单例服务，必须保障 **“Worker A 无法以任何手段获取或劫持 Worker B 的凭证/流量”**。

1. **第一防线：UDS 物理隔离 (Physical Isolation)**
   - 网关**绝不在公共 TCP 端口上监听**（如 `0.0.0.0` 或公共 `127.0.0.1:PORT`，因为这样同宿主机下的沙箱可通过扫描端口尝试连接，存在串扰风险）。
   - 网关只监听由 `ahd` 动态创建的 Unix Domain Sockets。每个 Worker 在宿主机上有唯一的套接字路径：
     `/home/sevenx/.cache/ah/sandboxes/{worker_id}/tmp/ah-gateway.sock`
   - 由于沙箱的文件系统是强隔离的，`ahd` 仅将该 worker 专属的 UDS 文件 bind-mount 到其沙箱内的 `/var/run/ah-gateway.sock`。
   - 沙箱 A 的物理边界内**根本不存在**沙箱 B 的 UDS 文件。从操作系统内核级别（Unix 文件权限及挂载命名空间）保证了 Worker A 物理上无法向网关的 Worker B 通道发送任何数据。

2. **第二防线：应用层 Token 签名校验 (Cryptographic Check)**
   - 网关在收到连接后，读取 HTTP Header 中携带的 Fake JWT。
   - 网关提取其中的 `worker_id`，并使用在 `ahd` 内存中注册的该 worker 的对称密钥（在 Worker 启动时随机生成并注入）验证该 Fake JWT 的签名。
   - 网关同时校验：**当前连接 of UDS 文件描述符关联的 Worker ID，必须与 HTTP Header 中的 `worker_id` 完全一致**。
   - **双防线失效分析**：即使因宿主机文件挂载配置出错（如错误的 bind-mount 导致 Worker A 拿到了 Worker B 的 UDS），Worker A 发送的请求中也只能携带其自身的 Fake JWT A。网关在校验时会发现：连接来自 UDS-B，但 Token 属于 Worker A，判定身份冲突，立即阻断连接并返回 403，触发告警。

---

### 3.3 机制三：凭据最小化暴露与 Host 侧泄露面限制 (Credential Exposure Minimization)

1. **沙箱零泄漏面**：沙箱内部绝对不存放任何真实的 credentials 文件。由于沙箱通常会执行用户不受信任的项目代码，一旦沙箱被攻破（例如通过 RCE 漏洞），攻击者只能在内存中拿到 `FAKE_JWT`，且其网络只能通过 UDS 发送模型调用。**攻击者无法获取宿主机的 Refresh Token，也无法获取宿主机或其他沙箱的任何凭证，攻击半径被严格锁定在当前 Worker 沙箱的单次 API 配额内**。
2. **网关无状态设计限制泄露面**：
   - 真实的持久化凭据（`credentials.json`，内含长期有效的 Refresh Token）以 `0600` 权限保存在宿主机 `ahd` 独占的目录中。
   - `Host-side HTTP Gateway` 作为独立的网络暴露进程，**不直接读取该持久化文件**。它完全在内存中维护由 `ahd` 核心通过进程间安全通道传递给它的 `access_token`。
   - **失效半径**：即使网关进程本身因缓冲区溢出等高危漏洞被完全控制，由于网关进程的物理内存中只有当前的 `access_token`（有效期 < 1 小时），**攻击者也只能窃取到这枚临时 Access Token，无法窃取到能够用来维持长期控制的 Refresh Token**。这实现了凭据在 Host 侧的安全防线分级隔离。

---

## 四、 单例刷新锁机制 (Single-Flight Refresh Lock)

在高并发场景下（如多个 Worker 并发启动或并发发送大量模型请求，且 Access Token 刚刚失效），必须保证有且仅有一次真实的 refresh 请求被发送至 Anthropic Upstream。

### 1. 刷新锁状态机

```text
[Gateway Receive Request]
           |
           v
   +---------------+
   | Token Valid?  | --(Yes)--> [Forward Request with Access Token]
   +---------------+
           |
          (No/Expiring)
           v
   +------------------------------------+
   | acquire refresh_mutex (try_lock)  |
   +------------------------------------+
       |                            |
    (Locked)                    (Busy/Unlocked)
       v                            v
[I am the Active Refresher]   [I must Wait]
       |                            |
       | (Post to platform...)      | (Subscribe to watch channel / Condvar)
       v                            |
[Update Token Cache]                v
[Broadcast Success Signal] ----> [Wakeup & Fetch New Token]
       |                            |
       v                            v
[Forward Request]             [Forward Request]
```

### 2. 具体实现机制 (Rust pseudo-code)

网关内部定义一个全局单例 `CredentialState`，基于内存锁实现 Single-flight 刷新：

```rust
use std::sync::Arc;
use tokio::sync::{RwLock, Mutex, watch};
use chrono::{DateTime, Utc, Duration};

struct TokenCache {
    access_token: String,
    expires_at: DateTime<Utc>,
}

pub struct CredentialsManager {
    // 保护 Token 缓存的读写锁，支持并发读取
    cache: RwLock<Option<TokenCache>>,
    // 串行化刷新操作的互斥锁
    refresh_mutex: Mutex<()>,
    // 广播刷新状态更新的通道 (Single-flight 唤醒机制)
    update_notifier: watch::Sender<bool>,
}

impl CredentialsManager {
    pub async fn get_valid_token(&self) -> Result<String, AuthError> {
        loop {
            // 1. 尝试获取读锁读取缓存
            {
                let cache_opt = self.cache.read().await;
                if let Some(ref cache) = *cache_opt {
                    // 保留 5 分钟的安全缓冲，避免临界点网络请求在途失效
                    if cache.expires_at > Utc::now() + Duration::minutes(5) {
                        return Ok(cache.access_token.clone());
                    }
                }
            }

            // 2. Token 已失效或即将失效，尝试抢占刷新权
            let mut rx = self.update_notifier.subscribe();
            if let Ok(_guard) = self.refresh_mutex.try_lock() {
                // 双重检查锁定 (Double-checked locking)，防止排队期间已被上一个抢占者刷新成功
                {
                    let cache_opt = self.cache.read().await;
                    if let Some(ref cache) = *cache_opt {
                        if cache.expires_at > Utc::now() + Duration::minutes(5) {
                            return Ok(cache.access_token.clone());
                        }
                    }
                }

                // 3. 执行真正的 Upstream Refresh 操作
                match self.perform_real_refresh().await {
                    Ok(new_token) => {
                        let mut cache_write = self.cache.write().await;
                        *cache_write = Some(TokenCache {
                            access_token: new_token.access_token.clone(),
                            expires_at: Utc::now() + Duration::seconds(new_token.expires_in),
                        });
                        // 广播通知所有正在等待的并发请求
                        let _ = self.update_notifier.send(true);
                        return Ok(new_token.access_token);
                    }
                    Err(e) => {
                        // 刷新失败，释放锁并抛出错误
                        return Err(e);
                    }
                }
            } else {
                // 4. 未抢到刷新锁，说明已有刷新在途，监听通知通道等待被唤醒
                // 当 notifier 发生变化时，循环会重新尝试获取读锁
                let _ = rx.changed().await;
            }
        }
    }
}
```

- **实现介质**：因为网关与 `ahd` 同属宿主机单进程上下文（由 `ahd` 以异步 Task 形式直接拉起，或通过单例进程管理），该锁**完全运行在内存中**。相比文件锁或数据库行锁，内存锁（`tokio::sync`）没有任何死锁或残留文件锁的风险，性能为纳秒级，保障了极高的鲁棒性。

---

## 五、 Phase 1 边界修正与 Tasks.md 调整计划

### 1. 边界修正说明

由于从“重定向 OAuth”转向了“HTTP Gateway 反向代理”，原设想的 Worker 侧集成方式已不存在。
- **不再**对沙箱挂载 `.credentials.json` 物理凭证文件。
- **改在**沙箱启动时，挂载 UDS 套接字文件，并在沙箱内部拉起一个桥接进程，且注入 `CLAUDE_CODE_USE_GATEWAY` 等环境变量。
- 验收标准中，原“模拟 credentials.json 文件变动”测试，替换为**“验证 API 请求 Header 在网关层成功重写，且在 Refresh Token 变化时其他 Worker 的 TCP 流量转发不受影响”**。

### 2. tasks.md 调整内容

为确保实施落地，需将原 `tasks.md` 的内容进行改写，明确定义 Plan B 下的技术任务。

```diff
- ## Phase 1: Token Proxy Core
- 
- - [ ] Host-side proxy service (ahd-owned or sidecar) holding the one real seed credential.
- - [ ] Single-flight refresh logic (one refresh in flight system-wide, regardless of concurrent worker requests).
- - [ ] Test: concurrent simulated worker requests against a fake upstream trigger exactly one real refresh call.
- 
- ## Phase 2: Worker-Side Integration
- 
- - [ ] Point worker sandboxes' Claude CLI auth at the proxy instead of materializing a real `.credentials.json` with a refresh token.
- - [ ] Test: worker A triggering a refresh through the proxy does not disrupt worker B's concurrent requests (request-layer isolation, not file-layer).

+ ## Phase 1: Token Gateway Core (Plan B)
+ 
+ - [x] Phase 0 Spike: 已由 Spike 报告验证原生 OAuth 无法配置，确定转向 Plan B Gateway。
+ - [ ] 实现宿主机本地 HTTP Gateway 服务（作为 `ahd` 的内嵌任务或 UDS 监听进程），持有唯一的真实持久化凭据。
+ - [ ] 实现 `/v1/messages` 等模型调用接口的反向代理转发，以及 Header 的重写逻辑（将 Fake JWT 重写为真实 Access Token）。
+ - [ ] 实现基于内存 `RwLock` + `Mutex` + `watch` 的 Single-flight 刷新锁，保护并发请求下的单次 upstream refresh。
+ - [ ] 测试：利用 mock upstream 验证多进程并发请求网关，网关仅发起一次真实刷新，且后续流量转发正常。
+ 
+ ## Phase 2: Worker-Side Integration (Plan B)
+ 
+ - [ ] 在 `ahd` 的沙箱启动模块中，取消对 `.credentials.json` 的挂载；改为挂载专用 UDS 套接字文件：
+       `/home/sevenx/.cache/ah/sandboxes/{worker_id}/tmp/ah-gateway.sock` -> `/var/run/ah-gateway.sock`
+ - [ ] 在沙箱启动脚本中，实现微型 TCP-to-UDS 桥接（例如使用内置轻量级转发逻辑或 socat，将本地 `127.0.0.1:8206` 的 TCP 请求桥接至 `/var/run/ah-gateway.sock`）。
+ - [ ] 注入环境变量 `CLAUDE_CODE_USE_GATEWAY=1`、`ANTHROPIC_BASE_URL=http://localhost:8206`、`ANTHROPIC_AUTH_TOKEN=<FAKE_JWT>`。
+ - [ ] 测试：在沙箱内运行 `claude` CLI，其命令流能够通过本地网关正常透传并完成 API 交互，无凭证文件依赖。
```

---

## 六、 结论与后续路径

1. **核心结论**：全面转向 **Plan B: Fake Gateway**。本方案完全摆脱了对 Claude CLI 原生 OAuth endpoint 的配置依赖，避开了高复杂度的 TLS MITM，且在安全隔离性上达到了最高标准（沙箱内凭证零暴露）。
2. **设计冻结**：本修订稿即为最终收敛稿，建议立即冻结设计。
3. **分单建议**：由于此方案网络拓扑与集成方式较原草案有较大改变，建议将 `tasks.md` 按上述修订版进行更新，并交由 Code Monkey (g1-m1/g2-m1) 独立进行 Phase 1 的开发工作。
