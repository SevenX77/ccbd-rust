# ah-per-worker-credentials Phase 0 Spike: Claude CLI Base URL / Proxy Support Investigation

**Status**: Completed (Design Oracle Verdict)  
**Target Path**: [research/credentials-phase0-spike.md](file:///home/sevenx/coding/ccbd-rust/research/credentials-phase0-spike.md)  
**Date**: 2026-07-10  

---

## 一、 核心结论 (Verdict)

经过对当前系统上运行的 Claude CLI 生产版本二进制包 (`/home/sevenx/.local/share/claude/versions/2.1.206`) 的逆向分析和代码追溯，结论如下：

**结论：不存在可行的原生机制将 Claude CLI 的 OAuth token 刷新请求直接指向自定义的本地 proxy / 替代 Base URL。**

`design.md` 中关于 *"配置 Worker 的 Claude CLI 将 auth-bearing requests 指向本地代理"* 的**承重假设在原生配置级别是不成立的**。

### 关键证据摘要：
1. **本地开发分支已在构建期被剪除**：虽然二进制代码中包含读取 `process.env.CLAUDE_LOCAL_OAUTH_API_BASE` 并将其解析为本地地址 (`http://localhost:8000`) 的 `uzf()` 函数，但决定环境的 `u8a()` 函数在生产包中被**硬编码**为 `function u8a(){return "prod"}`。因此，CLI 运行时永远只会走 `prod` 生产环境分支，无法通过设置 `USE_LOCAL_OAUTH=1` 等环境变量进入 `local` 分支。
2. **自定义 OAuth 域名存在极其严格的硬编码白名单限制**：虽然可以通过环境变量 `CLAUDE_CODE_CUSTOM_OAUTH_URL` 来覆盖 OAuth 的各种 URL，但代码中存在严格的成员校验。若该环境变量的值不在白名单数组 `eEn` 中，CLI 运行时会直接抛出致命错误 `"CLAUDE_CODE_CUSTOM_OAUTH_URL is not an approved endpoint."` 并退出。而白名单数组 `eEn` 在二进制中硬编码为：
   `["https://beacon.claude-ai.staging.ant.dev", "https://claude.fedstart.com", "https://claude-staging.fedstart.com"]`。
3. **Gateway 覆盖模式不覆盖 OAuth 刷新本身**：虽然 CLI 提供了 `CLAUDE_CODE_USE_GATEWAY` 结合 `ANTHROPIC_BASE_URL` 覆盖模型 API base URL 的机制，但这属于静态 JWT 注入模式。在此模式下，OAuth 刷新逻辑被完全绕过；一旦 token 到期，CLI 将直接报错退出，并不具备自动向代理刷新 token 的能力。

---

## 二、 源码证据深度解析 (OAuth Endpoint Logic)

以下是从生产二进制包 `/home/sevenx/.local/share/claude/versions/2.1.206` 中提取的、负责初始化和获取 OAuth 终端配置的关键混淆函数（已美化排版）：

### 1. `ds()` 函数：获取当前 OAuth endpoints 配置
此函数负责汇总最终的 OAuth endpoints 设置，包含对自定义 OAuth URL 及 Client ID 环境变量的处理：
```javascript
function ds() {
  // 1. 通过 u8a() 获得当前运行环境，并获取对应的基本 endpoints 映射
  let e = (() => {
    switch (u8a()) {
      case "local": return uzf();
      case "staging": return czf ?? c8a;
      case "prod": return c8a;
    }
  })();

  // 2. 检查是否存在自定义 OAuth 覆盖
  let t = process.env.CLAUDE_CODE_CUSTOM_OAUTH_URL;
  if (t) {
    let n = t.replace(/\/$/, "");
    // 关键拦截：必须在 approved 域名白名单 eEn 内，否则直接抛错崩溃
    if (!eEn.includes(n)) {
      throw Error("CLAUDE_CODE_CUSTOM_OAUTH_URL is not an approved endpoint.");
    }
    e = {
      ...e,
      BASE_API_URL: n,
      CONSOLE_AUTHORIZE_URL: `${n}/oauth/authorize`,
      CLAUDE_AI_AUTHORIZE_URL: `${n}/oauth/authorize`,
      CLAUDE_AI_ORIGIN: n,
      TOKEN_URL: `${n}/v1/oauth/token`,
      API_KEY_URL: `${n}/api/oauth/claude_cli/create_api_key`,
      ROLES_URL: `${n}/api/oauth/claude_cli/roles`,
      CONSOLE_SUCCESS_URL: `${n}/oauth/code/success?app=claude-code`,
      CLAUDEAI_SUCCESS_URL: `${n}/oauth/code/success?app=claude-code`,
      MANUAL_REDIRECT_URL: `${n}/oauth/code/callback`,
      OAUTH_FILE_SUFFIX: "-custom-oauth"
    };
  }

  // 3. 检查自定义 OAuth Client ID 覆盖
  let r = process.env.CLAUDE_CODE_OAUTH_CLIENT_ID;
  if (r) {
    e = { ...e, CLIENT_ID: r };
  }
  return e;
}
```

### 2. `u8a()` 函数：已被硬编码折叠
在生产客户端二进制中，环境判断函数 `u8a()` 的动态逻辑已被编译期常数折叠：
```javascript
function u8a() {
  return "prod";
}
```
**推论**：任何尝试通过设置 `USE_LOCAL_OAUTH=1` 或其它动态环境变量的方法，都**无法**使 `u8a()` 返回 `"local"`，从而无法触发本地模式分支 `uzf()`。

### 3. `eEn` 数组：限制自定义 OAuth 的硬编码白名单
```javascript
eEn = [
  "https://beacon.claude-ai.staging.ant.dev",
  "https://claude.fedstart.com",
  "https://claude-staging.fedstart.com"
];
```
**推论**：`CLAUDE_CODE_CUSTOM_OAUTH_URL` 只能设为上述三个域名之一，无法配置为 `http://localhost:XXXX` 或任何本地代理服务器 IP。

### 4. `uzf()` 与 `c8a`：开发与生产环境配置对比
```javascript
// 本地开发环境预置映射（已被 u8a 返回 prod 废弃）
function uzf() {
  let e = process.env.CLAUDE_LOCAL_OAUTH_API_BASE?.replace(/\/$/, "") ?? "http://localhost:8000",
      t = process.env.CLAUDE_LOCAL_OAUTH_APPS_BASE?.replace(/\/$/, "") ?? "http://localhost:4000",
      r = process.env.CLAUDE_LOCAL_OAUTH_CONSOLE_BASE?.replace(/\/$/, "") ?? "http://localhost:3000";
  return {
    BASE_API_URL: e,
    CONSOLE_AUTHORIZE_URL: `${r}/oauth/authorize`,
    CLAUDE_AI_AUTHORIZE_URL: `${t}/oauth/authorize`,
    CLAUDE_AI_ORIGIN: t,
    TOKEN_URL: `${e}/v1/oauth/token`,
    API_KEY_URL: `${e}/api/oauth/claude_cli/create_api_key`,
    ROLES_URL: `${e}/api/oauth/claude_cli/roles`,
    CONSOLE_SUCCESS_URL: `${r}/buy_credits?returnUrl=/oauth/code/success%3Fapp%3Dclaude-code`,
    CLAUDEAI_SUCCESS_URL: `${r}/oauth/code/success?app=claude-code`,
    MANUAL_REDIRECT_URL: `${r}/oauth/code/callback`,
    CLIENT_ID: "22422756-60c9-4084-8eb7-27705fd5cf9a",
    DESIGN_CLIENT_ID: "00000000-0000-4000-8000-000000000000",
    OAUTH_FILE_SUFFIX: "-local-oauth",
    MCP_PROXY_URL: "http://localhost:8205",
    MCP_PROXY_PATH: "/v1/toolbox/shttp/mcp/{server_id}"
  };
}

// 默认生产环境映射 (最终被 ds() 采纳)
c8a = {
  BASE_API_URL: "https://api.anthropic.com",
  CONSOLE_AUTHORIZE_URL: "https://platform.claude.com/oauth/authorize",
  CLAUDE_AI_AUTHORIZE_URL: "https://claude.com/cai/oauth/authorize",
  CLAUDE_AI_ORIGIN: "https://claude.ai",
  TOKEN_URL: "https://platform.claude.com/v1/oauth/token",  // <-- OAuth refresh 发往此处
  API_KEY_URL: "https://api.anthropic.com/api/oauth/claude_cli/create_api_key",
  ROLES_URL: "https://api.anthropic.com/api/oauth/claude_cli/roles",
  CONSOLE_SUCCESS_URL: "https://platform.claude.com/buy_credits?returnUrl=/oauth/code/success%3Fapp%3Dclaude-code",
  CLAUDEAI_SUCCESS_URL: "https://platform.claude.com/oauth/code/success?app=claude-code",
  MANUAL_REDIRECT_URL: "https://platform.claude.com/oauth/code/callback",
  CLIENT_ID: "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
  DESIGN_CLIENT_ID: "59637612-477b-4836-a601-b0589eda7704",
  OAUTH_FILE_SUFFIX: "",
  MCP_PROXY_URL: "https://mcp-proxy.anthropic.com",
  MCP_PROXY_PATH: "/v1/mcp/{server_id}"
};
```

---

## 三、 Model API Base URL 覆盖机制分析 (`CLAUDE_CODE_USE_GATEWAY`)

二进制中同样存在供模型调用请求拦截的 **Cloud Gateway** 模式（用于企业私有代理/网关）。我们对其可行性进行了梳理：

### 1. 运行时初始化逻辑
在应用加载阶段，CLI 会执行以下逻辑（`OTi()` 函数）：
```javascript
async function OTi() {
  if (ut(process.env.CLAUDE_CODE_USE_GATEWAY)) {
    let e = process.env.ANTHROPIC_BASE_URL,
        t = process.env.ANTHROPIC_AUTH_TOKEN;
    if (e && t) {
      let r;
      try {
        r = UBn(e); // 校验 URL 格式
      } catch (o) {
        throw Error(`CLAUDE_CODE_USE_GATEWAY is set but ANTHROPIC_BASE_URL is invalid: ${ne(o)}`);
      }
      let n = BHr(t); // 解析 jwt 获取过期时间
      xBe({
        url: r,
        jwt: t,
        expiresAt: n !== null ? n * 1000 : Number.MAX_SAFE_INTEGER,
        unpinned: true
      });
      return;
    }
    w("CLAUDE_CODE_USE_GATEWAY is set but ANTHROPIC_BASE_URL or ANTHROPIC_AUTH_TOKEN is missing; ignoring", { level: "warn" });
  }
  // ... (下接读取本地 settings 的 gatewayTrust 证书指纹匹配流程)
}
```

在构造 HTTP 请求客户端时，若 `provider === "gateway"`，则执行以下逻辑：
```javascript
if (_ === "gateway") {
  await xYe();
  let x = o_(); // 获取上述注册的 url/jwt 结构
  if (!x || P6o()) {
    throw Error(
      ut(process.env.CLAUDE_CODE_USE_GATEWAY)
        ? "Cloud gateway token expired — refresh ANTHROPIC_AUTH_TOKEN and restart."
        : "Cloud gateway session expired — run /login to reconnect."
    );
  }
  let { rest: I } = qbt(b.defaultHeaders);
  return new iK({
    ...b,
    defaultHeaders: { ...I, Authorization: `Bearer ${x.jwt}` },
    apiKey: null,
    baseURL: x.url,
    authToken: x.jwt,
    ...X5() && { logger: PXe() }
  });
}
```

### 2. 对 Token Proxy 设计的影响
*   **不覆盖 OAuth 刷新**：该机制**仅用于拦截对话及模型调用请求**，根本不包含 OAuth token 的刷新路由。
*   **生命周期静态化**：如果 `CLAUDE_CODE_USE_GATEWAY=1` 处于激活状态，CLI 自身的自动 refresh token 逻辑会被完全短路。一旦 `expiresAt` 到期（即 jwt 过期），CLI 进程会直接报错并终结。
*   **依赖外部刷新与重启**：在此模式下，任何 token 的刷新责任必须完全由外部管理器（如 `ahd`）承担。管理器在刷新后必须为 Worker 注入新的静态 `ANTHROPIC_AUTH_TOKEN` 并重启 CLI 实例。

---

## 四、 替代拦截路径可行性评估 (Alternative Interception Paths)

由于原生配置层无法绕过白名单，如要实现 **“消除多个 Worker 间的 RTR (Refresh Token Rotation) 冲突并维持单实例刷新”**，可以采用以下两种网络层/架构层替代方案：

### 方案 A：利用 `HTTPS_PROXY` 进行 TLS 劫持拦截 (MITM Proxy)
因为 Claude CLI 的 Node.js 运行时支持系统的 `HTTPS_PROXY`（及 `HTTP_PROXY`），可通过在 Sandbox 内设置此环境变量将所有请求强制路由到本地 Token Proxy。
*   **实现原理**：
    1. 在 Sandbox 启动时注入 `HTTPS_PROXY=http://127.0.0.1:TOKEN_PROXY_PORT`。
    2. 本地 Proxy 监听请求，如果检测到发往 `https://platform.claude.com/v1/oauth/token` 的 token 刷新请求，则由 Proxy 统一代理，使用宿主机持有的单例 refresh token 完成刷新，再返回构造的正确相应。
    3. 其它普通的 API 流量（发往 `https://api.anthropic.com`）直接透传（或在 Proxy 层做转发）。
*   **痛点与风险**：
    *   **根证书信任建立**：由于目标是 HTTPS 协议，Proxy 必须做 MITM 证书劫持。Worker 进程必须信任 Proxy 的自签名 CA，需要在 Sandbox 内配置 `NODE_EXTRA_CA_CERTS=/path/to/proxy_ca.crt` 注入 Node 运行时。
    *   **环境侵入度高**：需要在 Sandbox 文件系统或环境变量内注入证书文件，增加了实施负担。

### 方案 B：利用 `CLAUDE_CODE_USE_GATEWAY` 建立本地 Fake Gateway (推荐)
通过原生 Gateway 覆盖机制，将 Worker 对外发出的所有模型调用直接路由到本地代理服务，在代理服务层重写 Authorization Header。
*   **实现原理**：
    1. 启动 Worker 时，注入：
       ```bash
       CLAUDE_CODE_USE_GATEWAY=1
       # 指向本地代理，因为是自定义地址，甚至可以直接走 HTTP 协议，避开 TLS
       ANTHROPIC_BASE_URL=http://localhost:GATEWAY_PORT
       # 随便注入一个格式合法的 JWT，并设置一个极长的过期时间（绕过 CLI 的过期校验）
       ANTHROPIC_AUTH_TOKEN=FAKE_LONG_LIVED_JWT
       ```
    2. 此时，Worker 完全不需要持有任何真实的 OAuth token。所有的对话和 API 请求都会直接发往 `http://localhost:GATEWAY_PORT`。
    3. 本地 Gateway 代理接收到请求后，将 `Authorization: Bearer FAKE_LONG_LIVED_JWT` 头部剥离，替换为由宿主机 `ahd` **最新鲜、有效、经单例刷新** 的真实 OAuth Access Token，再转发给官方生产端 `https://api.anthropic.com`。
*   **方案优势**：
    *   **完美规避 RTR 冲突**：Worker 物理上不进行任何 OAuth 操作，整个 Sandbox 内连真实的 Refresh Token/Access Token 都不存在。
    *   **避开 TLS 劫持**：因为 `ANTHROPIC_BASE_URL` 指向本地 HTTP，不需要配置任何 CA 证书，免去了 `NODE_EXTRA_CA_CERTS` 的配置。
    *   **架构简单**：本地 Gateway 仅是一个普通的 HTTP 反向代理和 header 重写器，开发和运维难度极低。
*   **限制条件**：
    *   Worker 在此模式下将只能进行模型调用（`/ask` 等操作），无法执行任何需要在 Sandbox 内真正调用官方控制台的 OAuth 行为（但这完全符合 specs 中 Worker 仅做单项任务的设定）。
