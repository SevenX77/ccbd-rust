# Design: PR8 - 通用 Migration 框架 + Orphan FS Reconcile + 渐进式替换

本文档定义了 PR8 的技术实现，核心目标是支持 `ccbd-rust` 稳定、非破坏性地替换生产环境 Python `ccb`。

## 1. 通用 Migration 框架设计 [NEW]

### 1.1 基于 `user_version` 的版本管理
*   **存储机制**：利用 SQLite 内置的 `PRAGMA user_version` 维护当前 Schema 版本号。
*   **脚本组织**：在 `src/db/migrations/` 目录下存放 SQL 脚本，通过 `include_str!` 在编译期嵌入二进制。
    *   `V1__initial_schema.sql`: 初始核心表结构（基线）。
    *   `V2-V5`: 逐步收编原有的硬编码补丁（`sub_state`, `cancel_requested`, `status`, `master_pane_id`）。

### 1.2 升级路径
*   **幂等执行**： Rust 层迁移执行器在执行每个版本前会先检测目标列是否存在（通过 `PRAGMA table_info`），确保存量数据库平滑升级而不报错。
*   **废弃补丁**：原有 `src/db/mod.rs` 中的 `migrate_xxx` 手动补丁将被移除。

---

## 2. 全量 Orphan FS Reconcile 设计 [NEW]

### 2.1 孤儿判定与清理
*   **SoT (事实来源)**：以 DB 中的 `sessions` 和 `agents` 表为准。
*   **清理目标**：`logs/`, `evidence/`, `pipes/`, `sandboxes/` 下无 DB 引用且修改时间超过 24 小时的孤儿文件/目录。
*   **接入点**：启动期同步执行 + 运行期长周期异步 Tick。

---

## 3. 按项目非破坏切换设计 (Shadow Shim) [NEW]

### 3.1 路由劫持机制
通过 shell 的 `PATH` 优先级实现按项目的局部路由，而非修改全局 `ccb` 二进制。

*   **物理组成**:
    *   `.ccb/bin/ccb`: 局部影子脚本，直接 `exec ccb-rs "$@"`。
    *   `.ccb/activate.sh`: 激活工具，执行 `export PATH="$PWD/.ccb/bin:$PATH"`。
*   **操作流**: 
    1.  执行 `ccb-rs project-init` 初始化影子目录。
    2.  执行 `source .ccb/activate.sh` 激活当前会话。
*   **优势**: 100% 零全局风险。若脚本出现问题，仅影响已激活的局部会话，且 `ccb` 命令语义保持不变。

### 3.2 共存保障
*   **State Root**: 使用 `~/.local/state/ccb-rs/`，与 Python 版 `.ccb/ccbd/` 完全隔离。
*   **Tmux Socket**: 使用 8 位摘要前缀 (`ccbd-<8位hash>`)，与 Python 版的 12 位摘要前缀天然区分，防止 Tmux 服务器冲突。

---

## 4. Master 配置重设计 (Reframed: 非破坏性)

### 4.1 兼容性优先
*   **保持版本 1**: 继续使用 `version = "1"`，**禁止**在 PR8 引入强制性的 `version 2` 升级。
*   **SCS 适配延迟**: 将原计划的 `provider_specs` 和 `role` 声明式框架**推迟至 PR9+**，以优先保证对现有 Python `ccb` 项目的平滑兼容。

### 4.2 渐进式扩展
*   **可选字段**: 若需增加 Master 配置（如 `restart_policy`），必须以 `Option` 形式新增，不填写则使用现有默认行为。
*   **[DEFERRED to PR9+]**: 彻底移除 `home_layout.rs` 硬编码逻辑的目标顺延。

---

## 5. 范围分期 (Phasing Plan)

### 5.1 PR8 目标 (Parity & Stability)
*   **Migration**: 完成基于 `user_version` 的稳健迁移框架。
*   **Reconcile**: 完成 24h 缓冲区支撑的孤儿资源清理。
*   **Switching**: 确立基于 `ccb.toml` 的项目级切换规范。
*   **Parity**: 修复 `ask` 管道输入检测等关键体验缺口。

### 5.2 PR9+ 目标 (SCS & Advanced Features)
*   **SCS Integration**: 引入职级映射、声明式 Provider Specs。
*   **Full Parity**: 补齐 `trace`, `fault` 等剩余低频指令。

---

## 6. 关键 Breaking Changes
*   **[NONE]**: 本设计严禁引入任何破坏存量 `ccb.toml` 或导致无法与 Python `ccb` 并存的变更。
