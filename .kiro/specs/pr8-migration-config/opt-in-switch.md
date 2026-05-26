# Design: PR8 - 按项目 Opt-in 切换机制 (Non-Global)

本文档定义了 `ccbd-rust` 在不干扰全局 Python `ccb` 环境的前提下，实现按项目粒度显式 Opt-in 的切换机制。

## 1. 核心原则
*   **零全局破坏**: 严禁修改 `~/.local/bin/ccb` 或全局环境变量。
*   **显式 Opt-in**: 只有显式通过特定操作的项目才使用 Rust 版。
*   **透明集成**: 切换后，主控/Agent 的 `ccb ask` 等指令应在无感知的情况下路由至 `ccb-rust`。

## 2. 推荐方案：Shadow Shim + PATH Prepend

### 2.1 方案概述
通过在项目目录下建立临时的“影子目录”，利用 shell 的 `PATH` 优先级机制实现局部路由劫持。

### 2.2 物理结构
1.  **影子目录**: 在项目根目录下创建 `.ccb/bin/`。
2.  **Shadow Shim (`.ccb/bin/ccb`)**:
    一个极简的 Bash 脚本：
    ```bash
    #!/bin/bash
    exec ccb-rs "$@"
    ```
3.  **激活脚本 (`.ccb/activate.sh`)**:
    ```bash
    #!/usr/bin/env bash
    # CCB-RS Opt-in Activator
    export PATH="$PWD/.ccb/bin:$PATH"
    echo "ccb-rs activated for this session."
    ```

### 2.3 操作流程 (Opt-in)
1.  **初始化**: 在目标项目运行 `ccb-rs project-init` (PR8 新功能)。
    *   该指令在 `.ccb/bin/` 下生成 `ccb` shim。
    *   生成 `.ccb/activate.sh`。
2.  **激活**: 用户/Agent 在项目根目录下运行 `source .ccb/activate.sh`。
3.  **验证**: 运行 `which ccb` 应指向项目内的影子目录，而 `ccb ping` 应打到 Rust Daemon。

## 3. 为什么这是最佳方案？

| 维度 | PATH Shim 方案 | 其它方案 (如 Shell Alias) |
|---|---|---|
| **安全性** | 仅影响当前 Session，不改全局文件。 | 全局 Alias 风险大，易破坏 fallback。 |
| **集成度** | 子进程/子 Agent 均能继承 `PATH` 变更。 | 别名 (Alias) 不被子 Shell 继承，易失效。 |
| **可控性** | `source` 是显式的、符合 CLI 惯例的行为。 | 自动探测 (Direnv) 依赖外部工具安装。 |
| **回滚** | 关闭终端即恢复，或运行 `unset PATH`。 | 极其容易。 |

## 4. 实现细节 (Integration)

### 4.1 CLI 子命令 `ccb-rs project-init`
*   检查当前目录是否存在 `ccb.toml`（若无，提示先初始化配置）。
*   创建 `.ccb/bin` 目录及 `ccb` 执行文件。
*   生成 `.ccb/activate.sh`。

### 4.2 路由容错
Shadow shim 内部硬编码调用 `ccb-rs`。若 `ccb-rs` 不在 PATH 中，则报错并提示安装，而非盲目降级回 Python（防止环境混淆）。

## 5. 共存性验证 (Confirm)
*   **State Root**: Rust 版固定使用 `~/.local/state/ccb-rs/`，与 Python 版的 `.ccb/ccbd/` 或 `/tmp/` 路径完全隔离。
*   **Tmux Socket**: Rust 版使用 `ccbd-<16位hash>` 命名，Python 版使用 `ccbd-<12位hash>`，两者不会在 Tmux 后端产生命名冲突。
*   **Tmux Session**: Rust 使用 `agent_<id>` 和 `master_<proj>`，Python 使用 `ccb-<slug>`，物理会话完全独立。
