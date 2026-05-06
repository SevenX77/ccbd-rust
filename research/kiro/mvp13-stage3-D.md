# Kiro Design: MVP 13 Stage 3 (严格状态机与输出 Diff 兜底)

## 1. 架构图: 3 层兜底与 STUCK 状态流转

```text
[Tmux Pane]
     │ (PTY Output Stream)
     ▼
[Reader (Events) + Parser] ───(BUSY->IDLE)──► (M12 原生 Marker 匹配，保留，理想路径)
     │
     ▼ (Periodic Capture 每 30s)
[PaneDiffWatcher] (主兜底路径 M13 新增)
     │
     ├─ 判定 "实质内容" ──► 重置假活计数器
     │
     └─ 判定 "假活/无变化" ──► 累计假活 > 5 min ──► 标 STUCK
                                                     │ (通知用户)
                                                     ▼
                                            [Job Dispatcher 暂停该 Agent]
                                                     │
                                                     ├─ (User Cancel) ──► CRASHED / 回收
                                                     └─ (User Retry/Ping) ──► 恢复 BUSY

[Auxiliary: Marker Token] (防假阳性)
     └─ 如果 M12 原生 Marker 误判 IDLE，但 distill_reply 找不到 [CCB-job-{id}-END]
        ──► 拒绝切 IDLE，留在 BUSY 交给 PaneDiffWatcher 处理。

[Ultimate Deadline: 3h] (极端防线)
     └─ 仅在 PaneDiffWatcher 线程崩溃 / DB 死锁时，强行标 STUCK。
```

## 2. 关键决策对比

### 2.1 PaneDiffWatcher 的触发架构
* **选项 A (Per-Agent 专属 Task)**: 在 dispatch job 时 `tokio::spawn` 一个定时任务盯着这个 Agent。
* **选项 B (全局 Watcher 轮询)**: 类似现有 Orchestrator，全局一个 Task 扫 `agents` 表找 `BUSY` 的，定期 capture。
  - *推荐*: **选项 B**。
  - *理由*: 资源消耗稳定，防止多任务带来的 SQLite 并发锁问题。Watcher 只需要每 30 秒做一次全局 `SELECT id FROM agents WHERE state = 'BUSY'`，然后串行 capture + diff，开销极低。

### 2.2 Diff 判定机制 (实质内容 vs 假活)
* **选项 A (Raw Bytes Diff)**: 纯对比两次 capture 的 byte hash。
* **选项 B (Parsed Text Diff 抛弃装饰符)**: 用 `vt100` parser 获取屏幕纯文本，去除 Spinner 字符和行末时间戳后对比。
  - *推荐*: **选项 B**。
  - *理由*: Agent 的 TUI 常带有“思考中转圈 (⠋⠙⠹)”、“耗时 (3.2s)”、“光标闪烁”等。Raw bytes 会被这些 ansi escape 和时间戳污染，导致永远认定在“干活”。我们必须解析为文本后，运用过滤算法提取“知识增量”。

### 2.3 STUCK 的出口设计
* **选项 A (自动强杀)**: STUCK 后立刻走 kill 流程。
  - *缺点*: 长耗时任务被杀，用户发疯。用户明确要求“不傻等也不强杀”。
* **选项 B (隔离与介入)**: 状态挂起，前端 UI 亮红灯。
  - *推荐*: **选项 B**。
  - *理由*: STUCK 状态下，Orchestrator 不再向其 dispatch 队列中的新 Job。用户此时如果看到其实已经做完了（只是由于 TUI 漂移没识别出 IDLE），可以用新增的 RPC (`agent.mark_idle` 或带 ID 的 `cancel`) 介入，或者由用户决定强杀重启。这符合“看到 STUCK 自己决定”的 Input。

## 3. 数据结构 + DB schema 改动

### 3.1 状态扩充
在 `src/db/schema.rs` 中的 `agents` 表 `state` 字段契约增加 `STUCK`。
(无需改 SQLite schema 本身，因为 state 是 TEXT)。

### 3.2 新增 `pane_diff_state` 内存/持久化结构
在 DB 中新增表或在内存中维护（推荐内存，因为 Daemon 重启后重新识别不影响大局）：
```rust
struct AgentDiffState {
    last_meaningful_text: String, // 上次被认为是“实质内容”的过滤后文本
    last_meaningful_at: Instant,  // 上次实质性更新的时间
}
```

## 4. 算法：Diff 性质判定 (实质内容 vs 假活)

**伪代码**：
```rust
fn is_meaningful_diff(old_text: &str, new_text: &str) -> bool {
    let clean_old = sanitize_for_diff(old_text);
    let clean_new = sanitize_for_diff(new_text);
    
    // 如果过滤了假活字符后，文本有超过 N 个字符或词的实质增长或结构变化
    let distance = strsim::levenshtein(&clean_old, &clean_new);
    distance > MIN_MEANINGFUL_DISTANCE || clean_new.len() > clean_old.len() + MIN_LENGTH_GROWTH
}

fn sanitize_for_diff(raw: &str) -> String {
    let mut cleaned = String::new();
    for line in raw.lines() {
        // 1. 过滤行尾的时间戳，如 "(3.4s)" 或 "12:34:56"
        let line = RE_TIMESTAMP.replace_all(line, "");
        // 2. 过滤常见 Spinner 字符 (Braille patterns, -, \, |, /) 紧跟 "Thinking..." 
        let line = RE_SPINNER.replace_all(&line, "");
        // 3. Trim 空白
        cleaned.push_str(line.trim());
    }
    cleaned
}
```
**边界 Case**：
* Agent 疯狂清屏重绘（如 Gemini 进度条）：由于我们对比的是最终显示的 `clean_new` 和 `clean_old`，进度条字符的改变不会带来长文本增量，从而被识别为假活。
* 假活持续 5 分钟 (300 秒) 阈值触发：转入 `STUCK`。

## 5. 跟现有模块衔接

* **原生 marker_pattern (BUSY→IDLE)**: 
  * 保持作为**最优第一路径**。如果 Agent 乖乖打出了 `> `，系统依旧亚秒级响应。
* **Marker Token 集成 (防误判)**:
  * 发送阶段: `Orchestrator` 将 Job Prompt 包装为 `[CCB-job-{id}-START] \n {prompt} \n [CCB-job-{id}-END]` (只针对特定 Provider，因为有些 Provider 如 Gemini 对结构化 Prompt 敏感，需评估。推荐仅对 Codex 使用)。
  * 拦截阶段: 当原生 `marker_pattern` 匹配准备切 IDLE 时，调用 stage 4 的 `distill_reply`。如果拿不到 `END` token，说明遇到了“假 prompt” 回显（比如代码块里正好有一行 `> `），此时**拒绝状态转换**，留在 BUSY。
* **极宽 Deadline 兜底**:
  * 改造现有的 `marker::timer::spawn_marker_timer_task`。将 `BUSY_TIMEOUT` 从 5s (mvp1 时代) 调宽至 **3 小时**。触发后，由 `UNKNOWN` 改为 `STUCK`，防止强杀。
* **`UNKNOWN` 的定位调整**:
  * `UNKNOWN` 留给“PTY 进程物理失联但 Systemd 还在”或“内部断言规则失败”等系统级崩溃异常。
  * `STUCK` 专指“进程活得好好的，屏幕也有（假）输出，但就是没推进业务逻辑”的业务级挂起。

## 6. 关键风险 + 反向契约盲点

1. **Diff 算法的过度拟合 (Overfitting) 风险**:
   依靠 Levenshtein 距离和 Regex 剔除来判断“干活”，极容易因为 Provider 的一次 TUI 升级（比如加入了一种新的动画字符）而失效，把真活当成假活。
   *防范*: 允许动态下发或在 `ccb.toml` 中配置 `spinner_patterns`。

2. **长串输出与内存压力**:
   保存 `last_meaningful_text` 可能意味着保存 200x50 的字符串矩阵（~10KB）。全局 Watcher 对并发量的内存开销完全可控，但需要注意不要过度抓取 Scrollback。必须仅使用 `tmux capture-pane -p` (可见区域)。

3. **Marker Token 对 Provider 模型质量的污染**:
   给所有的 LLM Prompt 头尾强加 `[CCB-job-xxx]` 可能会影响 Agent 的 Few-shot 推理或者污染写回的代码文件。
   *防范*: 仅作为辅助选项。若不适用，则纯依赖 PaneDiffWatcher。MVP13 中必须评估各 Provider 对此 Token 的容忍度。