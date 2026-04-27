# 议题 3：与原版 Python CCB 的边界继承关系

## 第 1 轮 (Round 1) - Master Claude 立场

Master Claude 的核心立场是：**完全替代（complete replacement），但保留 `.ccb/` 目录配置格式向后兼容。不共存、不依赖、不做数据迁移。** 具体包含 5 个关键判断：

1.  **判断 1：完全替代，不共存**
    *   *说明*：ccbd-rust 是为了解决 Python 版投递盲区等结构性 Bug 而生的，如果允许两者在同一个目录并存运行，必然导致 PID 和 Socket 的抢占。直接 Stop 旧进程、Start 新进程是最优解。
    *   *实例*：不支持灰度测试。如果用户想做对比测试，他必须在 `project_A/` 里跑 Python 版，在完全隔离的 `project_B/` 里跑 Rust 版。同一个项目目录下绝对不能有两个 Daemon 同时抢资源。
2.  **判断 2：保留 `.ccb/ccb.config` 格式，对用户零感知**
    *   *说明*：完全兼容原有的一行流配置（如 `cmd, a1:codex, a2:gemini, a3:claude`），用户升级无需重写配置文件。
    *   *实例*：用户卸载 Python 版后安装 Rust 版，他的原有工作流毫无影响，Agent 名称依然叫 a1/a2/a3。
3.  **判断 3：坚决不做状态或 SQLite 迁移**
    *   *说明*：Python 版没有 SQLite 概念，状态散落在 json 文件中。Rust 版的 SQLite 数据库必须全新创建，不提供 `import-from-python` 这种低性价比工具。
    *   *实例*：用户切换前，如果 Python 版里还有未完成的 `job_xxxx.json` 遗留，Rust 版直接无视。旧任务的清理责任交给用户，而不是把这些结构混乱的残渣导入到严谨的 SQLite 事件表中。
4.  **判断 4：Systemd Unit 名字物理隔离**
    *   *说明*：Python 版用 `claude-code-bridge.service`，Rust 版用新名字如 `ccbd-rust.service`。
    *   *实例*：用户可以用 systemd 同时管理两个配置，启用一个时必须 disable 另一个，互不干扰。
5.  **判断 5：提供兼容的 CLI 命令行接口**
    *   *说明*：由于用户习惯了 `ccb ask` 等命令，因此系统必须提供一组与旧版用法完全对齐的命令行前端。
    *   *实例*：用户输入 `ccb ask a2 "hello"`，这实际上触发的是 Rust 编写的客户端进程，它把 "hello" 打包成 JSON-RPC 发给 UDS 的守护进程，体验和旧版一致。

## 第 2 轮 (Round 2) - Gemini 评估与收敛

我完全认同 Master Claude 追求「极简替代、无缝迁移」的设计哲学。不迁移旧数据和保持 `.config` 兼容是非常务实的架构决策。在此基础上，我提出两点工程层面的微调：

### 对判断 3 的补强：增加明确的警告（Explicit Warning）
我不反对“不迁移”，但如果我们“静默忽略”旧版 `.json`，可能会导致用户以为自己的任务丢失了。
*   *实例*：如果新系统启动时发现 `.ccb/mailboxes/` 目录下有遗留文件，Rust 守护进程应该在标准错误（stderr）或日志中输出黄色的明确警告：`WARN: Legacy Python mailboxes found in .ccb/mailboxes/. ccbd-rust does not migrate old states. Please clean them up manually.`。这既坚持了“不做迁移”的底线，又照顾了开发者的可观测性体验。

### 对判断 5 的补强：采用单二进制多模式（Single Binary, Multi-mode）
Master Claude 提议将 daemon (ccbd) 和客户端 (ccb) 拆分为两个独立的 binary。从 Rust 编译分发的角度看，这会增加构建和版本对齐的复杂度。
*   *建议*：采用 Cargo 工作空间中经典的「Fat Binary」策略，即编译出一个统一的二进制文件。运行时根据命令名或者参数决定模式（例如，执行 `ccbd` 或者是通过 `ccb daemon` 启动为服务端；执行 `ccb ask` 则作为短生命周期的客户端发 RPC）。这能让安装脚本变得最简单。
*   *实例*：用户下载的 release 只有一个文件 `ccb`。在 systemd 脚本里，他写的是 `ExecStart=/usr/local/bin/ccb daemon`；在日常敲击键盘时，他输入的是 `ccb ask a2`。一切都在一个二进制包内，但通过入口参数完成了 L2 守护进程与 L3 客户端的物理隔离。

### 最终共识结论
**决议结果：通过优化后的 5 条边界继承关系。**
ccbd-rust 对旧版 Python 版执行“断代式”的替换：不共存、不迁数据（但提示遗留）、启用新 Systemd 名字，但在文件配置和 CLI 用户接口上保持完全兼容。打包策略上采纳单二进制模式以简化分发。
