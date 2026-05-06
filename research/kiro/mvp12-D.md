# Kiro Design: MVP 12 (Python ccb 1:1 翻译)

## 1. 总览
MVP 12 的设计哲学是**坚决向 Python Ground Truth 降级**。对于 TUI 交互、布局控制、Provider 启动侦测等与外部环境深度耦合的领域，废弃 Rust 侧凭直觉发明的“优雅”但脆弱的机制，1:1 翻译 Python 侧已经稳定运行的代码。保留 Rust 侧已证明正确的基础设施红利（如 Systemd BindsTo、SQLite 级 CAS 保护）。

**6 个 R 的核心修复架构关系图：**
```mermaid
graph TD
    subgraph M12.2: Send/Reply (R-1, R-2)
        W[writer.rs] -->|无条件 Enter| Tmux
        Orch[orchestrator/mod.rs] -->|complete_job| Disp[Orchestrator as Dispatcher]
        Disp -->|prepare_reply_deliveries / ack_reply| DB[(Events/Jobs DB)]
    end
    subgraph M12.3: Layout (R-3)
        Lay[layout.rs] -->|build_split_layout| Tmux
    end
    subgraph M12.4: InitProbe (R-4)
        IP[provider/init_probe.rs] -->|S1-S3 detect| Tmux
    end
    subgraph M12.5: Sandbox/Home (R-5)
        HL[provider/home_layout.rs] -->|materialize_trusted_folders| SandboxFS
    end
    subgraph M12.6: Reconcile (R-6)
        Rec[db/system.rs] -->|reattach SessionWatch| Systemd[Systemctl]
    end
    
    Tmux --> IP
```

**模块变动清单：**
| 新增模块 | 改写模块 | 废弃模块 |
|---------|---------|---------|
| `src/provider/init_probe.rs` | `src/agent_io/writer.rs` | `src/marker/startup_engine.rs` |
| `src/provider/home_layout.rs` | `src/orchestrator/mod.rs` | Manifest 中的伪装字段 |
| | `src/tmux/layout.rs` | |
| | `src/sandbox/bwrap.rs` | |
| | `src/db/system.rs` | |
| | `src/provider/manifest.rs` | |

---

## 2. 共性: ProviderManifest 重构

废弃所有与“猜”相关的静态交互字段。新增针对 Provider 特化行为的标志位（供后续统一调度器或路由使用）。

```rust
pub struct ProviderManifest {
    pub provider_name: &'static str,
    pub command: &'static [&'static str],
    pub env_passthrough: &'static [&'static str],
    pub injected_env_vars: &'static [(&'static str, &'static str)],
    pub readiness_timeout_s: u32,
    
    /// 新增：是否需要在 bwrap 前执行 materialize_trust 补丁 (对应 home.py)
    pub requires_home_materialization: bool,
    
    // 废弃: startup_sequence 
    // 废弃: interactive_prompt_handlers 
    // 废弃: marker_pattern (字面值)
}
```
*Python 来源映射*：`requires_home_materialization` 取代了按 provider name 硬编码的判断，对应于 `provider_backends/*/launcher_runtime/home.py` 的存在与否。

---

## 3. R-1 设计: writer 无条件 Enter

### 3.1 数据结构
无新增数据结构。

### 3.2 函数签名
修改 `src/agent_io/writer.rs`:
```rust
pub async fn send_text_to_pane(
    tmux: Arc<TmuxServer>,
    agent_id: &str,
    pane: TmuxPaneId,
    text: String,
) -> Result<(), CcbdError>
```

### 3.3 控制流
1. 调用 `tmux.load_buffer` 写入数据。
2. 调用 `tmux.paste_buffer` 发送文本。
3. **无条件**发送一次独立的 Enter：`tmux.send_keys(pane, "Enter")`。
4. 清理 buffer。
*(注：第二重 Enter (second_enter_delay) 和 Verify 机制属于高级特性，本阶段先对齐最基础的无条件单次 Enter，移除阻碍发信的 `ends_with` 判定。)*

### 3.4 Python parity 表
| Rust 行为 | Python file:line | Python 函数 |
|-----------|-----------------|-------------|
| paste-buffer 发文本 | `terminal_runtime/tmux_send.py:131` | `_paste_via_buffer_legacy` |
| Enter delay 0.5s | `terminal_runtime/tmux_send.py:134-136` | `_paste_via_buffer_legacy` |
| send-keys Enter 无条件 | `terminal_runtime/tmux_send.py:137` | `_paste_via_buffer_legacy` |

---

## 4. R-2 设计: dispatcher 完成信号模型

### 4.1 架构错位回应
在 MVP11 中，Rust 的 `src/db/state_machine.rs` (`mark_agent_idle_matched_sync`) 在探测到 marker 时，顺手修改了 job 的状态为 COMPLETED。这破坏了 Dispatcher 的单向事件流，导致缺乏统一的“收尾”动作（如主动触发回信交付 `ack_reply`），从而引起挂起。
**解决方案**：强化 `src/orchestrator/mod.rs` 作为全局 Dispatcher 的角色。
- 废除 `reader.rs` 扫描到 marker 后直接调用 `mark_agent_idle_matched_sync` 内嵌的 `mark_job_completed` 逻辑。
- 引入明确的 `orchestrator::complete_job` 函数。
- 扫描器仅负责发出 `state_change` (BUSY -> IDLE) 事件。Orchestrator 在 `run_once` 的循环中，若发现 Agent 从 BUSY 转为 IDLE，并且其对应 Job 是 DISPATCHED 状态，则由 Orchestrator 负责调用 `complete_job`。
- `complete_job` 内部串联起状态更新，并模拟 Python 的 `prepare_reply_deliveries` 和 `ack_reply`。由于 ccbd-rust 目前是同步返回给 CLI，这里的 `ack_reply` 对应于解锁 `handle_job_wait` 的 pending 状态。

### 4.2 数据结构
无新增数据结构，复用现有 DB Job 和 Event 记录。

### 4.3 函数签名与控制流
在 `src/orchestrator/mod.rs` 新增：
```rust
pub async fn complete_job(ctx: &Ctx, job_id: &str, reply_text: &str) -> Result<(), CcbdError> {
    // 1. 标记 DB 中的 job 为 COMPLETED (对应 persist_terminal_completion)
    crate::db::jobs::mark_job_completed(ctx.db.clone(), job_id, reply_text).await?;
    
    // 2. 模拟 ack_reply / prepare_reply_deliveries (通知挂起的 CLI 请求)
    crate::orchestrator::pubsub::notify_job_update(job_id);
    
    Ok(())
}
```

### 4.4 Python parity 表
| Rust 行为 | Python file:line | Python 函数 |
|-----------|-----------------|-------------|
| 显式 complete_job 入口 | `ccbd/services/dispatcher.py:112` | `complete` |
| 标记终端状态 | `ccbd/services/dispatcher_runtime/finalization_runtime/service.py:17` | `persist_terminal_completion` |
| 通知交付/应答 | `ccbd/services/dispatcher_runtime/finalization_runtime/service.py:28-29` | `prepare_reply_deliveries` / `ack_reply` (通过 message_bureau) |

---

## 5. R-3 设计: grid layout 序号绑定

### 5.1 数据结构
无新增持久化结构，在 `src/tmux/layout.rs` 引入临时的树形表示。

### 5.2 函数签名与控制流
改写 `src/tmux/layout.rs::apply_layout`：
```rust
pub async fn apply_layout(
    server: &TmuxServer,
    window_target: String,
    mode: LayoutKind,
    panes: &[TmuxPaneId], // 传入当前 window 下按 Provider 排序好的 pane 列表
) -> Result<(), CcbdError>
```
**控制流 (1:1 翻译 `build_split_layout`)**:
1. 获取 root pane。
2. 根据 panes 的数量 (2, 3, 或 4) 执行精确的 `tmux split-window -t <parent> -h/-v -p <percent>`。
3. 弃用 `select-layout tiled`。
4. 将对应的 agent 进程用 `tmux respawn-pane -t <新切出的 pane_id> ...` 绑定过去（或者在 spawn agent 之前先分配好 pane 拓扑）。

### 5.3 Fallback 决断
> **MVP12 决断**：目前暂不复刻 `cli/services/runtime_launch_runtime/tmux_panes.py:179` 针对 `split-window failed` 异常而 fallback 到 detached session 的逻辑。假设开发者的 tmux 显示器尺寸充足。Rust 侧代码中留下 `// TODO(mvp13): handle 'no space for new pane' fallback to detached session` 注释。

### 5.4 Python parity 表
| Rust 行为 | Python file:line | Python 函数 |
|-----------|-----------------|-------------|
| 放弃 tiled，采用树形拆分 | `terminal_runtime/layouts_split.py:20-63` | `build_split_layout` |
| Right/Bottom 百分比切分 | `terminal_runtime/layouts_split.py:66-78` | `assign_pane` |

---

## 6. R-4 设计: init_probe S1-S3 状态机

### 6.1 废弃迁移
彻底删除 `src/marker/startup_engine.rs` (废弃 `StartupSequenceEngine` 伪装按键逻辑)。

### 6.2 新模块 `src/provider/init_probe.rs`
引入特定于 Provider 的多阶段内容侦测（而非简单的正则）。
```rust
pub trait InitGateProbe {
    fn detect(&self, capture: &str) -> bool;
}

pub struct ClaudeInitProbe;
impl InitGateProbe for ClaudeInitProbe {
    fn detect(&self, capture: &str) -> bool {
        banner_gone(capture) && prompt_present(capture) && steady_marker_present(capture)
    }
}
// 同理提供 GeminiInitProbe
```

### 6.3 控制流
在 `src/monitor/agent_watch.rs` 或类似 Agent 状态监控循环中：
1. 定期 (如 500ms) 使用 `tmux capture-pane -p` 抓取屏幕可见文本 (不含 scrollback)。
2. 将文本喂给当前 provider 对应的 `InitGateProbe::detect()`。
3. 若连续 N 次 `detect()` 返回 true (对齐 Python 的 `steady_count`)，则宣告 readiness，状态由 `SPAWNING` 转入 `IDLE`。

### 6.4 Python parity 表
| Rust 行为 | Python file:line | Python 函数 |
|-----------|-----------------|-------------|
| Claude 3 段式侦测 (banner, prompt, steady) | `provider_backends/claude/init_probe.py:100` | `ClaudeInitProbe.detect` |
| Gemini 2 段式侦测 (banner, prompt) | `provider_backends/gemini/init_probe.py:65` | `GeminiInitProbe.detect` |
| 仅捕获可见屏幕 (-p 无 -S) | `provider_backends/gemini/init_probe.py:75` | `_capture_visible` |

---

## 7. R-5 设计: home_layout materialize

### 7.1 新模块 `src/provider/home_layout.rs`
负责在拉起 `bwrap` 之前，在宿主文件系统上准备并修改目标沙盒内的配置文件。

### 7.2 差异化处理
- **Gemini**: 读取 `~/.gemini/trustedFolders.json`，合并沙盒的 `/workspace` 路径，写回到沙盒隔离映射目录。
- **Claude**: 复制 `.claude.json` (Trust 标志)，并 Symlink `.claude/.credentials.json` 到目标隔离目录。
- **Codex**: 准备 `sessions` 文件夹路径，并作为环境变量挂载，不涉及复杂的 JSON 合并（与 Python 中 Codex 无 `home.py` 而是通过 env var 控制行为一致）。

### 7.3 函数签名
```rust
pub fn materialize_home_layout(provider: &str, project_root: &Path, sandbox_root: &Path) -> Result<(), CcbdError>
```

### 7.4 Python parity 表
| Rust 行为 | Python file:line | Python 函数 |
|-----------|-----------------|-------------|
| Gemini 信任目录 JSON 合并 | `provider_backends/gemini/launcher_runtime/home.py:109` | `_materialize_trusted_folders` |
| Claude 凭据 symlink 与 Trust 标记 | `provider_backends/claude/launcher_runtime/home.py:108` | `_materialize_trust` / `_symlink_credentials` |
| Provider Auth 文件根目录挂载白名单 | `launcher/sandbox_home.py:16` | `PROVIDER_AUTH_WHITELIST` |

---

## 8. R-6 设计: reconcile SessionWatch reattach

### 8.1 差异说明
Python 版本使用 `workspace/reconcile.py` 通过对比持久化配置和当前运行配置来进行 `git worktree` 级别的核对。
Rust 版本的生命周期是由 Systemd Anchor (`BindsTo`) 保证的，所以 Daemon 崩溃重启时，仅需针对目前 DB 记录的存活 Session 重新挂接轮询器。

### 8.2 TODO 实施与控制流
修改 `src/db/system.rs:200` 处的 `reconcile_startup_sync_with_state_dir`：
1. 查询 DB 获取所有 `status = 'ACTIVE'` 的 Session ID。
2. 针对每个 Session，检查对应的 systemd 单元 (`ccbd-session-<sid>.service`) 是否仍然存活 (`systemctl is-active`)。
3. 若已消亡，调用现有的 `cascade_kill_session_agents`。
4. 若存活，调用 `spawn_session_watch_task` 重新拉起后台探测 Task。

### 8.3 Python (思路借鉴) parity 表
| Rust 行为 | Python file:line | Python 函数 |
|-----------|-----------------|-------------|
| 启动时探测悬空实体 | `workspace/reconcile.py:67` | `reconcile_start_workspaces` |
| 重新接管状态 | `ccbd/keeper_runtime/loop.py:65` | `reconcile_once` |

---

## 9. 测试矩阵

| Req | Unit Test | Integration (真 Spawn) | E2E |
|-----|-----------|------------------------|-----|
| R-1 | `writer.rs` 测试无 `\n` 情况下的 Tmux 模拟序列 | `tests/mvp12_writer_real_tmux.rs` | 对接 AC2 (真实命令下发) |
| R-2 | 测试 Orchestrator 探测 IDLE 后触发 `complete_job` | 无 | 对接 AC2 (不会超时阻塞) |
| R-3 | 测试给定 pane 个数输出的切分树结构是否正确 | 无 | 对接 AC6 (Tiled 布局确认) |
| R-4 | 对各 Provider 送入典型的静默和干扰 TUI 文本块测试 | 无 | 对接 AC1 (真 CLI 能够度过启动弹窗) |
| R-5 | 创建 mock Host JSON，验证生成的目标 JSON 合并路径正确 | `tests/mvp12_home_layout_real.rs` | 对接 AC1 (不弹 Trust 对话框) |
| R-6 | Mock Systemctl 返回状态测试 Reattach 逻辑 | 无 | 对接 AC5 (跨 Daemon 存活与死亡回收) |

---

## 10. 风险与未决

1. **Grid Layout 失败 (no space for new pane)**
   - *方案*: 暂不处理 (**Deferred to mvp13**)。根据用户要求，目前假设显示器足够大。已在代码设计中留 `TODO`。
2. **bwrap 路径伪装冲突**
   - *方案*: 已通过严格翻译 `materialize_trusted_folders` 并合并沙盒挂载点 (`/workspace`) 的路径予以缓解 (**Mitigated**)。但跨文件系统的深层 symlink 仍需观察。
3. **Regex 引擎跨语言差异**
   - *方案*: Rust 侧实现 `InitGateProbe` 时，采用字面字符串比对（如 Python 的 `_banner_gone` 使用 `.lower() in capture_lower` 和 `.startswith()`）而不是复杂正则，以最大程度避开 Regex 兼容问题 (**Mitigated**)。
4. **InitGate 捕获竞争**
   - *方案*: Rust 侧的状态机探测频率（Tokio Interval）设置为 500ms（对齐 Python `init_gate.py` 的慢速轮询），允许 TUI 渲染缓冲，降低由于屏幕未刷完引发的假阴性 (**Mitigated**)。