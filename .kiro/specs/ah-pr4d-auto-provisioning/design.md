# Design: ah PR4d Auto-provisioning (Git-based Plugins)

| 状态 | 草案 (Draft) |
| :--- | :--- |
| **日期** | 2026-05-28 |
| **范围** | ah 环境中基于 Git URL 的扩展能力自动补齐与物化 |

## 1. 目标 + 痛点对齐

PR4d 旨在实现 `ah` 环境的“自愈式部署”。当 `ah.toml` 中声明了位于远程 Git 仓库的插件（Plugins）时，`ah start` 过程将自动执行补齐动作。

- **核心目标**：实现类似 `npm install` 或 `cargo build` 的自动化体验，确保 Agent 环境在不同宿主机上的一致性。
- **解决痛点**：
    - 痛点 A：手动 `git clone` 扩展组件过程烦琐且易错。
    - 痛点 B：环境迁移时，遗忘 clone 依赖导致 Sandbox 启动失败。
    - 痛点 C：难以管理同一个扩展组件的不同版本（ref/branch）。

> **边界声明**：PR4d 只负责 git provisioning 动作本身（检查 cache → clone → symlink）。`ah up` 的指纹计算 / 变更检测 / 强制对齐 归 PR4e（后续 PR）。

---

## 2. 继承字段表 (Inherited Fields Audit)

| 类别 | 字段 / 接口 | 现状 [file:line] | PR4d 变更 [NEW/BREAKING] |
| :--- | :--- | :--- | :--- |
| **ah.toml** | `[master].plugins` | `Vec<String>` (src/cli/config.rs:35) | `[NEW]` 增强语义：支持 `name@git_url` 格式。 |
| **ah.toml** | `[agents.<id>].plugins` | `Vec<String>` (src/cli/config.rs:71) | `[NEW]` 增强语义：支持 `name@git_url` 格式。 |
| **物化接口** | `prepare_home_layout` | `src/provider/home_layout.rs:33` | `[NEW]` 内部接入 `provision_plugins` 拦截器。 |
| **物化接口** | `materialize_claude_settings` | `src/provider/home_layout.rs:407` | 无（复用 PR4c 逻辑）。 |

> **注**：本次变更为非破坏性（Non-breaking），纯 ID 形式的插件将继续按本地既有缓存逻辑处理。

---

## 3. 核心机制

### 3.1 Plugin Spec 解析
插件声明支持两种形状，解析后剥离出 **逻辑 ID (Name)** 用于物化配置。

1.  **ID-only**: `github@openai-curated` —— 映射到本地预装或已缓存目录。
2.  **Git-enabled**: 统一使用首个 `@git@` 作为 unambiguous 分隔符。
    - **Grammar**: `<name>@git@<url>[#<ref>]`
    - **解析结果**: 逻辑 ID = `name`；Fetch URL = `url`；版本 = `ref` (缺省为 `main`)。

**示例解析**:
- `rust-expert@git@github.com:foo/bar.git` -> ID: `rust-expert`, URL: `github.com:foo/bar.git`
- `web-tools@git@https://github.com/org/web.git#v2` -> ID: `web-tools`, URL: `https://github.com/org/web.git`, Ref: `v2`
- `legacy-mod@git@git@internal.com:ops/mod.git` -> ID: `legacy-mod`, URL: `git@internal.com:ops/mod.git`

### 3.2 缓存目录布局 (Cache Layout)
对齐 XDG 规范：
`$XDG_CACHE_HOME/ah/cache/git/<host>/<owner>/<repo>/<ref>/`
例如：`~/.cache/ah/cache/git/github.com/my-org/rust-skills/v1.0/`

### 3.3 Provisioning 工作流 (Pipeline)
1.  **扫描 (Scan)**：解析 `ah.toml`，提取所有含 Git URL 的插件声明，解析为 `(name, fetch_url, ref)`。
2.  **检查 (Check)**：根据解析结果计算目标缓存路径。
3.  **拉取 (Fetch)**：若目录不存在，在**宿主环境**执行 `git clone`。
4.  **物化 (Materialize)**：
    -   将对应的缓存目录 `symlink` 到沙箱内。
    -   **Claude**: 目标为 `<sandbox>/.claude/plugins/cache/<name>/`。
    -   **Codex**: 目标为 `<sandbox>/.codex/plugins/cache/<name>/`。
5.  **屏障 (Barrier)**：若拉取失败，`ah start` 报错并中断。

### 3.4 凭证复用与继承
`git clone` 子进程由 `ccbd` 继承父进程环境并在宿主环境运行：
- **环境变量**: 继承 `SSH_AUTH_SOCK` (ssh-agent 支持)。
- **物理文件**: 继承对 `~/.ssh` (known_hosts) 和 `~/.gitconfig` 的读取能力。
- **隔离性**: 敏感凭证仅留在宿主，不进入 Sandbox 内部。

### 3.5 幂等性与原子性
- **幂等性**：`ah start` 重复执行时，检测到目录存在即跳过。
- **原子性**：Clone 过程先下载到 `.tmp` 目录，成功后执行 `mv` 到最终路径。

### 3.6 ExtensionConfig Schema 改造方案 (R2)

为支持 Git Provisioning，需对配置类型进行升级，平衡解析精度与 PR4c 兼容性。

#### 3.6.1 类型定义 (Type Signatures)
```rust
// 逻辑 ID 与 物理路径 的中间承载
pub struct ResolvedPlugin {
    pub name: String,         // 逻辑 ID，用于配置 Key
    pub cache_path: PathBuf,  // 物理缓存/预装路径
}

// 扩展后的插件声明
pub enum PluginSpec {
    IdOnly(String),           // "github@openai-curated"
    Git(GitUrlSpec),          // name, url, ref
}

pub struct GitUrlSpec {
    pub name: String,
    pub url: String,
    pub reference: String,    // branch/tag/sha
}
```

#### 3.6.2 解析与物化流
1.  **Parse 时机**: 保持 `ExtensionConfig.plugins: Vec<String>` 不变（保持 ah.toml 反序列化兼容），但在 **Materialize 启动初期** 执行批量解析。
2.  **转换逻辑**: `Vec<String> -> Vec<PluginSpec> -> provision_all() -> Vec<ResolvedPlugin>`。
3.  **兼容性 (PR4c Bridge)**: 
    -   不含 `@git@` 的字符串视为 `IdOnly`，`cache_path` 指向既有本地路径。
    -   含 `@git@` 的字符串走新 `Git` 路径，`cache_path` 指向 `$XDG_CACHE_HOME/ah/cache/git/...`。
4.  **物化接入点**: 修改 `materialize_claude_plugins` 与 `materialize_codex_plugins` 接口，接收 `&[ResolvedPlugin]`。

---

## 4. 现有代码兼容性

- **逻辑 ID 重写**：在物化阶段（`settings.json` 的 `enabledPlugins` 或 `config.toml` 的 `[plugins]`），必须使用解析出的 `name` 作为 Key，而非原始声明串。
- **物化层接入**：`materialize_claude_settings` 等函数不再直接接收 `String` 列表，而是接收已 Resolved 的 `(name, cache_path)` tuple 列表。

---

## 5. PR 范围 + 实施切片

### 5.1 实施切片
1. **M1 (Spec & Type)**: 实现 `PluginSpec` 解析逻辑与单元测试。
2. **M2 (Git Client)**: 封装 `Command::new("git")` 子进程调用，处理 clone 与 checkout。
3. **M3 (Provisioner)**: 实现缓存路径计算与原子下载逻辑。
4. **M4 (E2E Integration)**: 接入物化流水线，实现从 `ah.toml` 到沙箱 symlink 的完整闭环。

### 5.2 估算
- **LOC**: 约 400-600 LOC。
- **文件**: `src/provider/extensions.rs`, `src/provider/provisioner.rs` (NEW), `src/provider/home_layout.rs`。

---

## 6. 验收场景 (Tests-First)

### 场景 1: 本地 ID 插件 (Regression)
- **输入**: `plugins = ["github@openai-curated"]`
- **预期**: 无 Git 动作，直接 symlink 本地预装路径。

### 场景 2: Git 插件初次安装
-   **输入**: `plugins = ["my-plugin@git@github.com:foo/bar.git"]`
-   **预期**: `$XDG_CACHE_HOME/ah/cache/git/github.com/foo/bar/main/` 被创建，内含仓库文件；沙箱内可见对应 symlink。

### 3. 缓存命中 (Idempotency)
-   **步骤**: 运行场景 2 后再次启动。
-   **预期**: `ccbd` 日志显示 "Plugin cache hit"，无网络开销。

### 4. Clone 失败 (Barrier)
-   **输入**: 非法 Git URL。
-   **预期**: `ah start` 报错并退出，沙箱不物化。

### 5. 私有库验证
-   **输入**: 需要 SSH Key 的内网 Git 地址。
-   **预期**: 只要宿主能 `git clone`，`ah` 亦能自动补齐。

---

## 7. 风险 + 待 PM 拍板

| 议题 | 描述 | 影响 | 推荐 |
| :--- | :--- | :--- | :--- |
| **议题 7.1: Git 版本锁定** | 是否支持 `@v1.0` 或 `#branch` 这种复杂 tag？ | Medium (M) | **推荐支持**。映射到缓存目录的最后一级，实现简单的多版本并存。 |
| **议题 7.2: 沙箱逃逸风险** | `git clone` 过程中的 hook 是否会执行恶意代码？ | High (H) | **配置封锁**。执行 clone 时增加 `-c core.hooksPath=/dev/null` 禁用远程仓库自带的 hook。 |
| **议题 7.3: 网络超时** | 若 Git 仓库极大，会导致 `ah start`挂起。 | Medium (M) | **设置超时**。给 git 子进程设置合理超时（如 60s），并在超时后报错。 |

---

## 8. 审计实证记录 (Evidence Trail)

1.  `grep -n "plugins" src/cli/config.rs`: 确认了现有 schema 物理位置。
2.  `ls -d $XDG_CACHE_HOME/ah/cache`: 验证了 ah 根缓存目录的可选性（若不存在则创建）。 fallback 逻辑：若 `$XDG_CACHE_HOME` 未设，则使用 `$HOME/.cache/ah/cache/`。
3.  `git clone --help`: 确认了 `-c core.hooksPath` 是有效的安全防御手段。

