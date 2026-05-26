# Requirements: PR8 - 通用 Migration, Orphan FS Reconcile, 替换 parity

本文档定义了 PR8 的业务需求与技术约束，核心使命是实现 `ccbd-rust` 对生产 Python `ccb` 的无缝替换。

## 1. 替换 Parity 需求 [NEW]

### 1.1 核心命令补齐
*   **Ask 体验对齐**: 实现对 `stdin` 管道输入的自动检测与读取，支持 `echo "text" | ccb ask` 模式。
*   **状态显示对齐**: `ps` 命令的输出指标需包含对 Agent 思考状态（BUSY/IDLE）的直观呈现。

### 1.2 非破坏切换机制 (Refined: Non-Global)
*   **零全局风险**: 严禁修改全局 `ccb` 二进制 (`~/.local/bin/ccb`)，确保生产稳定性。
*   **显式 Opt-in**: 仅在包含 `ccb.toml` 的项目中，通过显式“激活”动作（如 `source` 脚本）切换至 Rust 后端。
*   **路由透明化**: 激活后，主控/Agent 的 `ccb ask` 等指令应自动路由至 `ccb-rust`，无需修改脚本中的命令名称。

---

## 2. 通用 Migration 框架需求 [NEW]

### 2.1 稳健迁移
*   **版本管理**: 使用 `PRAGMA user_version` 进行版本控制。
*   **平滑升级**: 必须能识别并接管现有的、通过碎片化补丁升级的存量数据库。
*   **事务性**: 迁移失败必须全量回滚并阻止启动。

---

## 3. 全量 Orphan FS Reconcile 需求 [NEW]

### 3.1 深度清理
*   **全路径覆盖**: 包含 `logs/`, `evidence/`, `pipes/`, `sandboxes/` 的孤儿清理。
*   **安全缓冲区**: 强制执行 24 小时修改时间（mtime）检查，防止并发误删。

---

## 4. 技术决策 (Engineering Decisions)

| 决策点 | 推荐方案 | 理由 |
|---|---|---|
| **Q1: 配置版本** | **保持 Version 1** | **重大纠正**：为了保证与存量项目的兼容性及非破坏性切换，PR8 禁止强制升级到 Version 2。 |
| **Q2: SCS 适配** | **延后至 PR9+** | 优先级调整：当前的头号任务是 parity 和替换，SCS 的架构重设计不应阻塞稳定版替换的落地。 |
| **Q3: 路由实现** | **Shadow Shim + PATH Prepend** | 采用 `source .ccb/activate.sh` 模式。比全局路由器更安全（零全局影响），比别名（Alias）更健壮（子进程可继承）。符合 Python `venv` 习惯。 |

---

## 5. 继承字段表 (Config & DB)

### 5.1 Config 继承
| 字段 | 现状 | PR8 状态 | 理由 |
|---|---|---|---|
| `version` | "1" | `[NO CHANGE]` | 维持兼容性 |
| `master.cmd` | 字符串 | `[NO CHANGE]` | 维持兼容性 |
| `agents` | 映射表 | `[NO CHANGE]` | 维持兼容性 |

### 5.2 DB Schema 继承
| 表名 | 现状 | PR8 变更建议 | 理由 |
|---|---|---|---|
| `schema_version` | 不存在 | `[NEW]` user_version | 迁移基础 |
| `agents` | 现有多列 | `[NO CHANGE]` | 维持稳定性 |

---

## 6. 关键 Open Questions (需 User 拍板)
1. **Dispatcher 安装方式**: 是通过 `install_ccb_rs.sh` 修改 `~/.local/bin/ccb` 软链接，还是提供一个独立的别名建议？
2. **Parity 覆盖度**: `trace` 和 `fault` 指令在首批替换中是否可以接受缺席？（建议：是，优先保证核心 ask/start 流程）。
