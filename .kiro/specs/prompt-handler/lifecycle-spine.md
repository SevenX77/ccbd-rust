# Agent 生命周期主干与设计契约 (Lifecycle Spine)

> **注记**：本文档由 `/tmp/pr4-lifecycle-spine-ratified.md` 草案固化而来。§3 判据已按 a1/a2/a3 三方收敛（2026-05-25）修订，以对齐真实实现并规避任务期间的输入污染风险。
>
> **实现锚点**：
> - `src/agent_io/reader.rs:57-72`：基于输出静默窗的稳定性门控实现。
> - `src/provider/manifest.rs`：各 Provider 的 `stability_ms` 配置（Codex/Gemini/Claude 均为 300ms）。
> - `src/rpc/handlers.rs:1123`：通过 `is_meaningful_diff` 过滤 Spinner 噪点。
> - `src/prompt_handler/runner.rs:275`：`can-input` 探针在 SPAWNING 就绪门的物理确认实现。

## §1 第一性原理 (user 原话，设计契约)

> 为什么要点掉这个框，因为我们要进入到可以输入 prompt，可以做任务的 ready 阶段。所以判断是否成功点掉这个框，不是去扫描屏幕上有没有这些文字，而是确认是否可以输入，可以做任务。这就是我说的生命周期：现在在什么阶段，目标是进入哪个阶段，进入那个阶段的核心判断是什么？要去测试这个核心判断，如果没有进入，被什么卡住了？要解决掉这个卡点，目的是进入下一个阶段。

> (兜底) llm 读屏幕，记录到数据库，自己判断怎么操作，记录操作，再读屏幕查看结果，记录结果...直到出现目标状态。这样数据也记录了，操作对应的结果也记录了，也不耽误进入到正确的阶段。下一次遇到阻塞，先查数据库，有没有匹配的情况，有的话直接按照数据库记录的操作，没有的话再走一遍主控自己判断、操作、记录的流程。

## §2 Ground Truth

真实状态（`src/db/state_machine.rs`）：
`SPAWNING` / `IDLE` / `WAITING_FOR_ACK` / `BUSY` / `PROMPT_PENDING` / `STUCK` / `CRASHED` / `KILLED` / `UNKNOWN`。

## §3 生命周期主干与客观判据

判据铁律：**看结果 (Outcome)，不看症状 (屏幕文字)**。

| 当前阶段 | 目标阶段 | 客观核心判断 (Outcome Predicate) |
| :--- | :--- | :--- |
| **SPAWNING** (进程启动，CLI 初始化) | **READY/IDLE** (具备任务处理能力) | **主动探针 (can-input)**：向输入区注入无害字符（如 `x`），物理确认回显出现在输入行并成功清理。用于绕开 Banner 噪点与 TUI 重绘竞态。 |
| **IDLE** (等待任务) | **WORKING** (任务已接收) | **状态跃迁信号**：注入 Prompt 后，屏幕出现“在干活”信号（Thinking/工具调用/输出开始流动）。 |
| **BUSY/WORKING** (执行任务中) | **DONE → IDLE** (任务完成) | **被动静默信号**：输出停止抖动（维持 `stability_ms` 时间的静默窗）**且** `IdleMarker` 回归。不重跑探针以规避污染后续 Prompt 的风险。 |

### 设计修正说明：
探针的“写”属性决定了它仅适用于 `SPAWNING` 门。在 `BUSY→IDLE` 转换中，由于 Agent 可能正处于渲染收尾阶段，物理注入字符极易引发竞态或污染缓冲区。因此，任务结束的判定回归为“结果信号的最终一致性”（Quiescence + Marker）。

## §4 异常处理循环

当“客观核心判断”未通过时，系统进入 **探测 -> 诊断 -> 清障 -> 复测** 循环：
1. **探测**：判定核心 Outcome 未达成。
2. **诊断**：调用 `classify_capture`。此时“文字匹配”作为诊断手段，识别是否被已知弹窗（如 Trust Workspace）阻塞。
3. **清障**：执行 `PromptAction`（如发送 `Enter`）。
4. **复测**：立即重新回到 §3 的核心判断，直到物理确认进入目标阶段。
