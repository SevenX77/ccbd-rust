# Logic Explained: Agent 生命周期与探针就绪机制 (PR4a)

本文档解释了 PR4a 落地后的 Agent 生命周期判定逻辑。该设计优化了“纯屏文匹配”的传统路径，引入 **can-input 探针** 作为 **READY 终判门**，通过物理确认 Agent 是否真正具备任务处理能力，解决了 TUI 环境下视觉信号与物理状态脱节的痛点。

## 1. 核心判定逻辑：看结果，物理确认

在旧思路中，Agent 是否就绪完全取决于屏幕上是否有特定的 Banner 或 Prompt 字符（看症状）。这在 TUI 重绘延迟或 Scrollback 残留时会导致误判。

新架构（PR4a）采用两阶段判定：
- **阶段一：屏文初步分类（症状初步诊断）**：利用现有的正则库、Marker 分类器识别屏幕候选情况。
- **阶段二：探针物理确认（结果终判）**：当屏幕显示“看似就绪”时，通过向终端发送一个无害字符（如 `x`），观察回显并清理，物理验证输入链路的通达性。

## 2. 数据流与函数调用链

判定过程由 `init_probe_task` 驱动，通过多级过滤最终决定是否进 `IDLE`。

### A. 触发层：`run_init_probe_task`
- 定时抓取屏幕，调用 `probe.detect()`（如 `CodexInitProbe`）。
- **定位变化**：`detect()` 现在降级为 **Prefilter（预过滤）** 或触发器。即使它返回 `false`（例如 Banner 还没消失），系统也会尝试执行下一级扫描，以防被未知弹窗挡住。

### B. 判定层：`scan_startup_prompt` (于 `init_probe_task.rs`)
- 调用 `scan_prompt_and_apply_outcome`。
- 如果返回 `ReadyConfirmed`，则累加连续成功计数，达到阈值（STEADY_COUNT=2）后调用 `mark_idle_after_probe` 进 `IDLE`。
- 如果返回 `HandledOrClear`（表示刚点掉一个已知框），则重置计数继续探测。
- 如果返回 `Pending`（探针未确认 / 未知框），则经 `scan_prompt_and_apply_outcome` → `mark_prompt_pending_and_emit_unknown`（`integration.rs`）把 agent 置为 `PROMPT_PENDING` 并终止启动任务。这正是探针 gate 的核心：看似 ready 但探针没确认，不再被吞成“无事发生”，而是升级为 `PROMPT_PENDING` 等主控介入。

### C. 执行层：`handle_prompt_chain` (于 `runner.rs:108`)
这是 `can-input` 探针的实际执行点：
1. **分类**：调用 `classify_capture`。如果命中已知 Prompt，执行 Action。
2. **就绪校验**：如果命中 `IdleMarker`（触发就绪候选），则强制调用 `confirm_can_input`。
3. **探针逻辑 (`confirm_can_input`)**：
   - 调用 `is_input_candidate` 检查屏幕是否有符合输入特征的行。
   - 发送探测字符 `x`。
   - 调用 `probe_echoed` 检查 `x` 是否出现在预期的输入位置（锚定输入行，不使用全屏 contains）。
   - 发送 `BSpace` 清理探针。
   - 只有回显成功且清理成功，才返回 `CanInputProbe::Confirmed`。

## 3. Provider 结构化判据

探针不再使用模糊的全屏 `contains`，而是锚定特定 Provider 的输入行特征：

| Provider | `is_input_candidate` (前置条件) | `probe_echoed` (确认条件) |
| :--- | :--- | :--- |
| **Codex** | 存在以 `›` 开头的行 | 存在以 `› x` 开头的行 |
| **Gemini** | 存在包含占位符文本的行 | 该行占位符消失，且内容恰好为 `x` |
| **Claude** | 存在模型标记 (Sonnet等) 且存在 `❯` 空行 | 存在以 `❯ x` 开头的行 |

## 4. 为什么取代了旧 §10 (HandledSet)？

- **旧 §10 (纯症状去重)**：试图通过记录点过哪些框的指纹（HandledSet）来防止重复点击。它依然在猜测“框还在不在”，极易受重绘噪点干扰。
- **新架构 (探针终判)**：将“屏文匹配”定位为“寻找候选”，将“探针回显”定位为“确认就绪”。只要探针没确认“能输入”，系统就认为还没 Ready，持续执行“诊断并清障”循环。

**结论**：生命周期重画通过引入物理探针这一“硬判据”，从根源上消解了重绘时延和字符残留导致的死循环问题，屏文匹配则回归其作为高效分类器的本质角色。

