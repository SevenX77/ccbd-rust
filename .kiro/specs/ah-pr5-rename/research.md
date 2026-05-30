# Research: ah 全流程 Grand Tour PR-5 (ccbd → ah rename)

## §1 Cargo.toml 命名拓扑

当前 `Cargo.toml` 包含三层命名：
- **Crate Name**: `ccbd` (line 2)。决定了代码中 `use ah::...` 的空间名。
- **Daemon Binary**: `[[bin]]` 名为 `ccbd` (line 46)，路径为 `src/bin/ccbd.rs`。
- **CLI Binary**: `[[bin]]` 名为 `ah` (line 50)，路径为 `src/bin/ah.rs`。

**约束分析**:
- 允许 Crate 名与 Binary 名相同。若 Crate 改名为 `ah`，则代码变为 `use ah::...`。虽然已有名为 `ah` 的 binary，但 Cargo 处理 library 和 binary 命名空间是隔离的，不会产生编译冲突。
- 为了语义清晰，建议 Daemon Binary 遵循用户建议改名为 `ahd`，以区分 CLI 工具 `ah`。

## §2 "ccbd" 引用面分类

全案搜索共发现约 512 处 `ccbd` 引用（排除 `.git` 和 `target`），分类如下：

1.  **代码空间 (Namespace)**: `use ah::...` 约 388 处。这是最广泛的引用。
2.  **二进制文件名**: `src/bin/ccbd.rs`，`src/bin/ahd_test_helper.rs`。
3.  **持久化文件名**: `ahd.sqlite`。
4.  **Systemd 单元**: `ahd.service`, `ahd-session-{session_id}.service`, 属性 `BindsTo=ahd.service`。
5.  **资源命名前缀**: Tmux Socket (`ccbd-`), Tmux Buffer (`ccbd-buf-`), Cgroup Slice (`ccbd-agents.slice`)。
6.  **环境变量**: `AH_STATE_DIR`。
7.  **安装脚本**: `scripts/install_ah.sh` 中的变量与 Wrapper 名 (`ahd`)。
8.  **文档与注释**: `README.md`, `CLAUDE.md`, 历史 spec 记录。

## §3 区分 CCB Framework 与 ah Project

必须严格区分“脚手架”与“产品底座”：
- **CCB Framework (保留)**: `.ccb/` 目录、`CCB_ENV` 环境变量、`ccb ask` 命令。这些属于外部 Agent 框架（Claude Code Bridge），不应改名，否则会导致现有 Agent 无法识别项目环境。
- **ah Project (Rename)**: `ccbd` daemon、`ahd.sqlite`、`use ccbd` 引用。这些属于本项目（Agent Hypervisor）的核心实现，是本次 Rename 的主体。

## §4 Daemon Binary Rename 影响

将 `ccbd` 重命名为 `ahd`：
- **二进制重生物化**: `src/bin/ccbd.rs` -> `src/bin/ahd.rs`。
- **调用点同步**: `ah.rs` 中通过 `current_exe()` 寻找 daemon 路径的逻辑需更新。
- **Systemd 联动**: `ahd.service` 必须更名为 `ahd.service`，且代码中所有 `BindsTo` / `PartOf` 引用必须同步。
- **安装 Wrapper**: `install_ah.sh` 应生成 `ah` 和 `ahd` (取代 `ahd`) 的软链。

## §5 GitHub 仓库 Rename 影响

`SevenX77/ccbd-rust` -> `SevenX77/ah`：
- **URL 重定向**: GitHub 会自动重定向旧 URL，但建议更新 README 中的 Badge 链接、CI 配置文件中的路径。
- **本地 Remote**: 开发者需执行 `git remote set-url origin ...`。

## §6 本地路径 Rename 影响

`~/coding/ccbd-rust/` -> `~/coding/ah/`：
- **风险**: 正在运行的 Claude Master Session 的当前工作目录 (CWD) 会失效。
- **缓解**: 建议在 Rename 后立即退出并重新进入 Session，或者使用 `cd` 手动同步。
- **脚本硬编码**: 需检查 `install_ah.sh` 等脚本中是否含有 `/ccbd-rust/` 的硬编码路径（经查确实存在 `default_home` 硬编码）。

## §7 核心风险

1.  **PR-6 协同冲突**: PR-6 (Recovery/Resume) 涉及约 300 行代码修改，且大量使用 `use ah::`。若 PR-5 先合入，PR-6 将面临巨大的 Rebase 压力。
2.  **全量替换风险**: `use ccbd` 变为 `use ah` 是“牵一发而动全身”的改动，必须通过 `cargo check` 严密验证。
3.  **持久化兼容**: 已存在的 `~/.local/state/ah/ahd.sqlite` 是否需要更名为 `ahd.sqlite`？为了彻底性，建议更名并提供自动迁移逻辑。

## §8 实施分批策略推荐

建议采取 **“小步快跑，大步收尾”** 的策略：
1.  **第一阶段 (PR-5a)**: 完成 Crate Name (`ah`) 和 Binary Name (`ahd`) 的重命名，更新所有代码引用。
2.  **第二阶段 (PR-5b)**: 更新 Systemd、脚本、文档以及 GitHub 仓库设置。
3.  **顺序约束**: 强烈建议在 **PR-6 (Functional logic) 合入后**立即启动 PR-5，以减少重复劳动。
