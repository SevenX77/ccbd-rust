# Research: ccbd-rust core-fixes

本文档针对 `core-fixes` spec 进行深度调研,旨在为 `design.md` 提供物理机制论证、关键参数依据及决策支持。

## 1. R1: "One Tmux Session per CLI" 架构论证

### 1.1 tmux 隔离机制对比

| 特性 | 单 Session 多 Pane (当前) | One Session per CLI (目标) |
| :--- | :--- | :--- |
| **物理隔离** | 共享 Session 状态、环境变量、缓冲区 | 彻底隔离,独立的 PTY 实例 |
| **生命周期** | 需要精确跟踪 Pane ID,清理依赖 `kill_pane` | 直接 `kill-session`,清理其下所有衍生进程 |
| **PTY 尺寸** | 多个 Pane 共享 Window 尺寸,受外部 Attach 干扰严重 | 每个 Session 拥有独立的虚拟尺寸,不受跨 Session 干扰 |
| **可观察性** | 一眼看全 (Layout 比例切分) | 需要切换 Session 查看 (或使用 `attach` 命令) |
| **焦点管理** | 易因 Agent 刷新导致 Master 焦点丢失 | 物理分离,Agent 行为绝不干扰 Master 焦点 |

### 1.2 "PTY 尺寸锁" 第一性原理
*   **物理根因**: VT100 解析器 (如 `vt100-rust`) 强依赖 PTY 的宽度。如果实际输出超过宽度,tmux 会插入硬换行 (`\n`)。这会导致 `MarkerMatcher` 无法匹配跨行的标志位。
*   **合理值论证**: 
    *   **Claude Code**: 默认渲染宽度通常为 100-120 字符,在显示长路径或代码对比时,150 宽能保证绝大多数输出不折行。
    *   **Codex**: 类似,但在嵌套日志模式下可能更宽。
    *   **推荐方案**: **150 宽 / 60 高**。这个值平衡了“显示完整性”与“内存占用”。 [证H 影H 置A]
    *   **锁定机制**: `tmux new-session -d -x 150 -y 60` 结合 `set-option -g window-size manual` 可确保即使外部设备以不同尺寸 attach,后台 PTY 仍保持 150 宽。 [证H 影H 置A]

### 1.3 历史反向点 (Reversal of 6739f6a)
*   **反向理由**: `6739f6a` 追求的“一眼看全”在 Agent 数量 > 2 时会导致单 Pane 宽度极窄 (如 40 宽),强行触发 LLM CLI 的自适应缩放或折行,直接导致 `MarkerMatcher` 失效。
*   **可观察性补充**: 反向后,推荐通过 `ccbd-rust attach <agent_id>` 临时进入对应 Session,或者使用 `ccbd-rust tail` 流式查看日志。注: mvp15 commit `957dbf5` 已实现 `ccb-rust attach` 子命令,反向后可直接复用。

### 1.4 业界先例
*   **tmuxinator**: 通过配置声明 Session,通常 1-app-1-session,方便整体生命周期管理。
*   **overmind**: 1-process-1-window (in 1 session),利用 tmux 作 PTY 容器,通过监控 PID 树实现存活检测。
*   **abduco/dtach**: 极致简化的 session 管理,1-session-1-PTY,不带窗口管理逻辑,适合作为 agent 容器的物理层。
*   **启发**: 1v1 纯后台隔离是实现“可靠清理”的最佳路径,只需 `kill-session` 即可清理整个进程组。

### 1.5 生命周期继承与修正
*   **继承 mvp15**: 应复用 `src/monitor/master_watch.rs:7-53` 的 master pidfd 监控,确保 Master 退出时触发 `cascade_kill_session_agents`。 [证H 影M 置A]
*   **顺手修**: `src/tmux/scope.rs:55` 使用 `ccbd-rust.service` 而 `src/sandbox/systemd.rs:29` 使用 `ccbd.service`,导致 `BindsTo` 逻辑在特定环境下失效,应统一。 [证H 影H 置A]

**与 v1 差异**: 增加了 1.4 业界先例、1.5 生命周期继承;补充了 mvp15 attach 实现说明、BindsTo 命名冲突修复及三轴标注。

---

## 2. R2: WAITING_FOR_ACK 状态机防抖

### 2.1 竞态物理根因
*   **残留标志位**: 终端 scrollback 缓冲区中保留着上一条指令的 `=== DONE ===`。
*   **解析延迟**: 当 `send_text` 发出后,`MarkerMatcher` 立即开始扫描。如果此时 PTY 尚未接收到新字符,扫描器读到的依然是旧缓冲区的末尾,导致“秒回”假象。

### 2.2 防抖策略与现有逻辑协同
*   **实质性更新定义**: 
    1.  `PaneDiffWatcher` 检测到内容 Hash 变化。
    2.  光标位置发生移动 (特别是下移)。
    3.  接收到至少 1 字节的新输出 (非 ANSI 颜色转义)。
*   **推荐方案**: **引入显式 `WAITING_FOR_ACK` 状态**。 [证H 影H 置A]
    *   **与现有逻辑关系**: 
        *   **替代**: 取代 `src/rpc/handlers.rs:1010` 中硬编码的 `spawn_new_capture_seed` (5s 轮询)。
        *   **整合**: 将 `src/agent_io/reader.rs:53-94` 的 `stability_ms` (300ms) 逻辑作为 `WAITING_FOR_ACK` 态下的退出条件之一(即稳定无新字节输入)。
        *   **协同**: 维持 `is_prompt_only_reply` 启发式校验 (`src/db/state_machine.rs:43`) 作为最后一道防线。
*   **时间窗论证**: **500ms 最小强制窗口** OR **检测到实质性内容更新**。 [证M 影H 置A]
    *   **典型延迟 (P50)**: Claude API 响应通常在 300ms-800ms。
    *   **本地 PTY 回显 (P99)**: < 50ms。

### 2.3 业界先例
*   **expect**: 核心逻辑是 `expect { prompt { ... } timeout { ... } }`,在发送命令后有明确的等待期,不匹配历史缓冲区。
*   **pexpect**: 继承自 expect,提供 `searchwindowsize` 限制匹配窗口,避免匹配到旧的 scrollback。
*   **tmux-pipe-pane / vhs**: 通过监控 PTY 输出流的“静止期” (Quiescence) 来判定输出结束,而非仅靠 Marker。
*   **启发**: 必须将“发出指令”与“开始匹配”在时间轴上物理拉开,防抖态是工业级 PTY 自动化的标配。

**与 v1 差异**: 补充了与 `stability_ms`/`capture_seed` 现有代码的整合逻辑、2.3 业界先例及三轴标注。

---

## 3. R3: CWD/沙盒路径绝对校准

### 3.1 物理根因分析
*   **Master 飘逸**: `src/rpc/handlers.rs:146` 错误地将 `session.project_id` (basename) 直接当做 Path 使用: `let master_cwd: PathBuf = session.project_id.clone().into();`。
    *   **证据**: `project_id` 源自 `cli/start.rs:55-60` 的 `project_root.file_name()`,仅为目录名而非绝对路径。tmux 拿到相对路径后按 `ccbd` 自身的 CWD 解析。 [证H 影H 置A]
*   **Agent 飘逸**: `src/rpc/handlers.rs:282-356` 中使用 `session_dir` (即 sandbox 目录) 作 tmux `-c` 参数,而非工程绝对路径。
*   **bwrap 缺陷**: `src/sandbox/bwrap.rs` 未显式传递 `--chdir /workspace`,导致进程在沙盒内 CWD 继承自外部(通常是 sandbox state 目录)。 [证H 影H 置A]

### 3.2 绝对校准方案
*   **激活 Dead 通道**: 启用 `session.absolute_path` 字段。该字段已在 DB 中存在 (`src/db/schema.rs:2-6`),但目前仅供 UI 显示 (`src/rpc/handlers.rs:206`)。
    *   **方案**: 在 `handle_session_spawn_master_pane` 和 `handle_agent_spawn` 中反查该绝对路径并传给 tmux `-c`。 [证H 影H 置A]
*   **bwrap 修正**: 
    1.  **参数对齐**: 修正 `src/provider/home_layout.rs:33` 的命名误导,确保 `prepare_home_layout` 接收正确的 `project_root`。
    2.  **强制 Chdir**: 在 `src/sandbox/bwrap.rs` 中加入 `--chdir /workspace` 参数。 [证H 影H 置A]
*   **安全暴露面**:
    *   禁止 bind `$HOME`。
    *   `.git` 目录应按需 bind (通常只读),除非 Agent 需要操作 git。
    *   敏感目录 (`.ssh`, `.config`) 通过 `bwrap` 的 `--dir` 掩盖或不映射。 [证M 影L 置B]

**与 v1 差异**: 使用 inventory 提供的 file:line 锚点重写根因;明确了“激活 dead 通道”方案及 bwrap `--chdir` 缺失问题。


---

## 4. R4: CLI 启动命令完整透传

### 4.1 物理路径分析
*   **执行链**: `src/sandbox/systemd.rs:42-61` `master_command` 将 `cmd` 字符串原样封装入 `["sh", "-lc", cmd]`。
*   **事实**: 这种通过 `sh -lc` 执行的方式,天然支持空格分词、引号处理和环境变量展开。Bug B 并非代码无法解析多参数,而是 `ccb.toml` 默认配置仅写了 `"claude"`。 [证H 影H 置A]

### 4.2 Claude 参数语义分析
目标命令: `claude --dangerously-skip-permissions --continue /remote-control`
*   **`--dangerously-skip-permissions`**: Argv Flag。Claude Code 特有,用于跳过 Y/N 确认,属于启动参数。
*   **`--continue`**: Argv Flag。恢复上一个 Session,属于启动参数。
*   **`/remote-control`**: **Slash Command**。
    *   **深度分析**: 在 Claude Code 中,`/remote-control` 是启动后在交互界面输入的指令。若直接作为 argv 传递,取决于 Claude Code 是否支持通过 CLI 预加载 slash 指令。若不支持,则需通过 `ccbd-rust send` 路径在启动后注入。
    *   **结论**: R4 应通过修正 `ccb.toml` 解决启动参数透传;若 `/remote-control` 必须在交互态输入,则需配合 `agent.send` 自动化逻辑。 [证M 影M 置B]

### 4.3 推荐方案
*   **无需代码修改**: 维持现有 `sh -lc` 逻辑。 [证H 影L 置A]
*   **配置驱动**: 在 `ccb.toml` 中将 `[master] cmd` 更新为完整带参字符串。
*   **兼容性**: 确保 `src/rpc/handlers.rs:156` 的标题截取逻辑 (`cmd.split_whitespace().next()`) 依然能正确显示 `master (claude)`。

**与 v1 差异**: 推翻了 v1 的 `shell-words` 推荐(属于过度设计),确认现有 `sh -lc` 链条已支持多参数,Bug B 核心在于配置与参数语义理解。

---

## 5. 跨 Req 横向分析

### 5.1 依赖与冲突
1.  **R1 是基础**: 必须先改掉“单 Session 多 Pane”逻辑,否则 R3 的 `new-session -c` 会跟现有的 `split-window` 逻辑产生路径冲突。
2.  **R2 提升稳定性**: 在 R1 铺好的纯净 PTY 上,R2 的防抖更具确定性。

### 5.2 推荐实施阶段
*   **Phase 1 (Core)**: 实现 R1 + R3。确保 Master 和 Agent 都能在正确的目录下、独立的 Session 里拉起来。
*   **Phase 2 (State Machine)**: 实现 R2。解决“秒回”和状态机死锁。
*   **Phase 3 (UX)**: 实现 R4 及配套的 `attach` 观察命令。

## 6. 开放问题

1.  **R1 后遗症**: 多个独立 Session 会增加 `tmux ls` 的视觉负担,是否需要为这些 Session 统一打上 `ccbd-` 前缀并由 daemon 定时 reconcile 孤儿 Session? [证L 影L 置B]
2.  **R2 极端情况**: 如果模型输出极慢 (如每秒 1 字符),`PaneDiffWatcher` 的阈值是否需要动态调整? [证L 影M 置C]
3.  **R3 多级目录**: 如果 `Project Root` 下有嵌套的 `.ccb/`, 路径自动探测逻辑在 daemon 模式下是否稳健? [证M 影M 置B]
