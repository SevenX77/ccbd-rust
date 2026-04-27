# 任务：补充 codex-evidence-pack.md（追加，不重写）

## 上一轮的问题

`research/findings/codex-evidence-pack.md` 类 2 的 10 个 observations 全是 agent-harness 项目的工程细节（OAuth scope / thread_id / trace_path / .run_id 等）—— 跟 ccbd-rust 设计**几乎无关**。ccbd-rust 是 multi-CLI agent 调度 daemon，关心的是 CCB 消息投递 / completion 检测 / sandbox / lifecycle 等。

## 追加任务（**追加到 codex-evidence-pack.md 末尾**，不重写已有内容）

### 类 2-补：ccbd-rust 设计相关的 corpus observations

重点抽以下主题（用 grep -rn 真搜命中）：

**主题 1: CCB 消息投递行为**
关键词搜：`delivering` / `mailbox_state` / `paste-buffer` / `send-keys.*Enter` / `ACK`
目标：至少 5 条 observation

**主题 2: completion / stuck 检测**
关键词搜：`completion_detector` / `anchor_seen` / `READY 探针` / `stuck` / `Thinking` / `runtime_state=busy`
目标：至少 5 条

**主题 3: pane / 进程生命周期**
关键词搜：`pane.*死` / `pane.*alive` / `orphan` / `reconcile` / `restart_count` / `recover_succeeded`
目标：至少 5 条

**主题 4: sandbox / 隔离边界**
关键词搜：`bashrc` / `~/.bashrc` / `~/.claude` / `bwrap` / `sandbox` / `cgroup` / `MemoryMax`
目标：至少 5 条

**主题 5: 用户对 master Claude 行为的反复纠正**（这些 observation 直接定 ccbd-rust 的 RPC / hook 设计）
关键词搜：`stupid question` / `不要停` / `不要问` / `视野太窄` / `听人话` / `不允许`
目标：至少 5 条

**主题 6: master Claude 自身崩溃 / OOM**
关键词搜：`SIGKILL` / `OOM` / `崩溃` / `oom_kill` / `ScheduleWakeup` / `idle timeout`
目标：至少 3 条

**目标合计**：至少 28 条新 observation 追加到 codex-evidence-pack.md 类 2 段后面。

每条格式同前：
```
### O-XX <一句话描述>
- **类别**: A1 数据一致性 / A2 并发 / A3 沙盒 / A4 协议 / A5 lifecycle / A6 观测
- **引用**: `<file:line>`
- **原文**: > <grep 输出粘贴原文片段>
```

### 类 3-补：7 候选项目 code reference 加深

当前 codex-evidence-pack.md 类 3 每个项目只列了 3-5 条。请每个项目深读关键源码 + 加到 8-12 条具体引用。

按主题加细分类：
- PTY 接管（tamux / batty / agent-orchestrator 重点）
- SQLite / mailbox 持久化（overstory 重点）
- IPC 协议（看每个项目用 UDS / TCP / mqueue 等）
- lifecycle / 进程管理（spawn / kill / heartbeat）
- sandbox / 隔离（ccswarm / tamux 重点）
- health monitoring（batty 重点）

## 工作流（同前）

1. grep / Read 真命中后才追加
2. 每条带原文片段
3. **追加到 codex-evidence-pack.md 末尾的"## 类 2 续"和"## 类 3 续"段**，不重写已有
4. 完成后回复一段（人话）：
   - 类 2-补 多少条
   - 类 3-补 每项目多少条
   - codex-evidence-pack.md 总行数变化

## 铁律

1. 只做事实摘录，不做"分析"
2. 每条带 grep 命中的真原文片段
3. 关键词搜不到的主题必须明确写"未命中"，不空跳过
4. **不重写**已有内容，只追加
