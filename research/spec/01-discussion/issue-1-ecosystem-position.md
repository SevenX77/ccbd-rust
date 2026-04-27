# 议题 1：Spec-Coding 工具栈中的生态位定义

## 第 1 轮 (Round 1) - Gemini 立场

### 1. 核心立场 (Core Stance)
**ccbd-rust 必须是一个「强领域绑定（Domain-Specific）的 ccb Agent I/O 调度器」，绝不能退化为一个「通用的 API 驱动版 tmux」。** 它在生态位上属于 L3（Master Claude 等编排层）的专属基础设施，彻底切断与 IDE 或用户 GUI 的直接数据交换。

### 2. 正方推理 (Pro Arguments)

*   **论据 1：强领域绑定才能做到「精准的物理确认（ACK）」，解决投递失效盲区。**
    *   *模拟实例*：Master Claude 派活给 Codex 去修复一个 Rust 编译错误。如果 ccbd-rust 只是个通用 PTY，它只能告诉 L3「我把命令塞进管道了」。但在强领域绑定下，ccbd-rust 知道自己管的是一个 LLM CLI，它会在底层 VT100 屏幕缓冲区死死盯着 `(Codex) ✦` 或 `> ` 这种特定的提示符（Prompt Marker）。只有看到这个标记，ccbd-rust 才在 SQLite 里记一笔 `DeliveryAck`。这就根治了前述的 tmux 盲投失败问题。
*   **论据 2：生态位下沉，阻断与 IDE 的越权耦合，确保单向数据流。**
    *   *模拟实例*：假设用户在 VSCode Companion 界面里点击了「拒绝并要求重写」按钮。VSCode 绝对不能直接连 ccbd-rust 的 Socket 去发指令。正确的数据流是：VSCode 把点击事件发给 Master Claude（L3），Master Claude 根据对话上下文决定重写策略，然后 Master 组装成 `{"method": "agent.send", "params": {"agent_id": "codex-1", "text": "用户拒绝了，请换一种写法\n"}}` 发给 ccbd-rust（L2）。ccbd-rust 根本不知道 VSCode 的存在，它只认 L3 的 JSON-RPC。
*   **论据 3：生命周期与沙盒严格绑定 `.ccb` 物理结构，杜绝野进程。**
    *   *模拟实例*：L3 发起 `agent.spawn` 请求启动 `a2`。由于 ccbd-rust 认同 ccb 领域模型，它会自动去读取 `.ccb/agents/a2/agent.json`，并把 `.ccb/agents/a2/provider-runtime/` 只读挂载进 bwrap 沙盒。如果它是个通用 PTY，L3 每次启动都得传几百行的复杂挂载参数，极大增加 L3 的工程复杂度。

### 3. 反方立场及推理 (Counter Stance & Arguments)

*   **最强反方立场**：ccbd-rust 应该是一个**「纯粹的、无领域逻辑的通用 Headless PTY Multiplexer」**（即一个通过 API 控制的 tmux 替代品），所有对 agent_id、提示符解析、沙盒组装的理解都应该上浮到 L3。
*   **反方论据 1：极致的泛用性与解耦。**
    *   *模拟实例*：项目后期，用户想在后台持续跑一个 `npm run dev --watch` 的前端编译服务，并希望通过 API 获取输出。如果是通用 PTY 立场，L3 直接发命令启动一个普通的 shell 进程即可；如果是 ccbd-rust 领域绑定立场，系统会因为这个进程不是合法的 Agent 且不吐出 `(Codex) ✦` 提示符而将其判定为异常进程并 Kill 掉。
*   **反方论据 2：L2 逻辑极度收敛，不背业务锅。**
    *   *模拟实例*：Claude Code 某天更新了版本，把底部的 `Thinking...` 提示语改成了 `Processing...`。如果是通用 PTY 立场，L2 根本不解析这些字，只负责透传字节流，改代码的任务在 L3；如果是强领域立场，ccbd-rust 自身的解析器就会失效，必须发版修改 Rust 代码。

### 4. 取舍点与决策开关 (Trade-offs)

*   **如果选择我的立场（强领域绑定）：**
    *   *代价（Cost）*：失去了管理非 AI CLI 进程的能力。
    *   *代价实例*：如果 Master Claude 想要在后台跑一个耗时 10 分钟的 `cargo build` 并实时监控，它不能把这个纯 Bash 任务塞给 ccbd-rust，必须自己用 Node.js/Python 的 `child_process.spawn` 去拉起。ccbd-rust 的池子里只有 Agent。
*   **如果选择反方立场（通用 PTY）：**
    *   *代价（Cost）*：L3 (Master Claude) 变得极度臃肿且易错（重蹈 Python 版 CCB 覆辙）。
    *   *代价实例*：为了判断 Codex 是否处于 idle（空闲）状态，L3 必须自己建立 WebSocket 长连接，一帧一帧地把 ccbd-rust 透传上来的原始 VT100 字符流在 L3 内存里跑一遍终端模拟器（Terminal Emulator），一旦 Node.js 的事件循环卡顿，就会误判 idle，导致新指令投递到半空闲的面板上发生错乱。

### 5. 给用户的对话邀请 (Call to Action)

为了推进议题 1 到结论阶段，请针对以下两个决策点进行二选一（或给出您的第三种方案）：

*   **决策点 A：关于 L2 是否应该理解 Agent 协议。**
    *   [ ] **选项 A1（Gemini 推荐）**：ccbd-rust 必须理解特定 CLI（如 Claude Code / Gemini CLI）的空闲提示符特征，由 L2 来负责输出 "Agent is Idle" 的确定性事件。
    *   [ ] **选项 A2（反方立场）**：ccbd-rust 只负责暴露原始的字节流，到底 Agent 卡没卡住、有没有 idle，全由 L3 读取流后自己做正则匹配判断。
*   **决策点 B：关于非 Agent 进程的接管。**
    *   [ ] **选项 B1（Gemini 推荐）**：ccbd-rust 拒接一切不包含在 `.ccb/agents/` 体系下的普通命令启动请求，保持血统纯正。
    *   [ ] **选项 B2**：ccbd-rust 应该留一个后门接口（比如 `pty.spawn_raw`），允许 L3 把耗时的纯 bash 编译命令也扔给 L2 托管，复用 L2 的崩溃拉起能力。

## 第 2 轮 (Round 2) - 共识达成

### 1. Master Claude 核心论据摘要
针对决策点 A（L2 是否理解 Agent 协议）与决策点 B（L2 是否接管非 Agent 进程），Master Claude 表达了完全的赞同（选择 A1 + B1），其核心论据如下：
*   **关于 A1 的认同**：
    *   **DRY 原则（不重复造轮子）**：VT100 解析必须在 L2 收敛。如果让 L3 各自解析，会导致 Node/Python/Go 多种客户端观测同一 Agent 状态不一致（例如 Node 事件循环卡顿引发误判）。
    *   **LLM 物理约束**：L3 作为 LLM，物理上无法维护屏幕缓冲区和实时响应字节流，这是此前“主控空等误判”的根因。
    *   **解耦策略**：协议变化的代价可以通过外置 `.toml` 配置（如 `.ccb/providers/claude-code.toml`）来软化，避免修改 Rust 源码。
*   **关于 B1 的认同**：
    *   **YAGNI 原则**：当前无真实需求。
    *   **防止 Scope Creep**：接管纯 bash 命令意味着要新增 RPC、独立的 raw bytes 流表和进程生命周期表，属于过度设计。Bash tool 已经能满足 master 跑 `cargo build` 的需求。

### 2. Gemini 评估与补充
我完全认同 Master Claude 的论据，这极大补强了强领域绑定的合理性。在此基础上，我补充两点现实约束与工程视角的评估：
*   **对 A1 TOML 外置方案的补强（默认回退约束）**：
    *   *评估*：将 Marker 提取到 `.toml` 极具工程价值。但必须注意边界情况——如果用户初次安装环境，尚未生成 `.ccb/providers/` 目录该怎么办？
    *   *实例*：新用户执行 `ccbd-rust` 并通过 L3 拉起 Codex。如果 L2 强依赖外部 `.toml`，就会因找不到配置而 Panic 或无法解析 Idle 状态。因此，**Rust 二进制内部必须使用 `include_str!` 等宏硬编码一份主流 Agent（Codex, Claude Code, Gemini CLI）的 Default Marker 配置作为 Fallback（托底）**。外部 `.toml` 只做覆盖（Override）之用。
*   **对 B1 “防扩展锁死”的评估**：
    *   *评估*：坚持 B1（拒接普通 Bash）并不会阻断未来的扩展可能。因为在架构上，如果未来真的需要接入通用进程，完全可以设计一张与 `agents` 表平行的 `processes` 表，并暴露独立的 `process.spawn` RPC。
    *   *实例*：如果未来 Master Claude 强烈需要监控后台 10 分钟的编译任务，我们在 SQLite 中新增表结构，且旧的 Agent 状态机完全不受干扰。当前的「拒绝」是为了保持当下核心业务的纯粹性，并不是物理层面的锁死。

### 3. 最终共识结论
**决议结果：A1 + B1**。
ccbd-rust 确立为「强领域绑定的 ccb Agent I/O 调度器」。它必须在底层直接解析特定 Agent 的 VT100 Idle Marker（结合 TOML 软配置与内置 Fallback），并且坚决拒接非 Agent 体系的普通 Bash 命令（如 `cargo build`），保持极简边界。

