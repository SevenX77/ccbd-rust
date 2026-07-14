# ah-commands skill 触发+编排设计(a3)

本项目设计旨在为 `ah` 的 Agent（尤其是 Master）提供一份权威、动态且低噪的命令编排指南。通过将命令清单以项目 Skill (`.ah/skills/ah-commands/SKILL.md`) 的形式组织，并采用渐进式披露机制，使 Agent 在正要使用相关能力时被精准启发激活，避免直接稀释系统 Kernel 的注意力。

---

## 一、 触发可靠性设计 (Trigger Reliability Design)

### 1. `SKILL.md` Frontmatter Description 具体措辞
为了保证启发式触发的可靠性（既能在 Agent 需要使用时被唤醒，又不会在常规代码编写或重构中被过度激活），设计如下 `description` 文本：

```yaml
description: Authoritative CLI reference for 'ah' agent-facing orchestration commands (such as ah ps, ask, tell, pend, watch, logs, events, cancel, kill, attach, master ack-ready, prompt resolve). Activate this skill when you need to inspect agent/job status, dispatch tasks to worker agents, retrieve execution logs or outputs, cancel/kill tasks, attach to tmux sessions, stream lifecycle events, or perform master cutover.
```

### 2. 覆盖的意图短语与触发词论证
该 `description` 针对大语言模型的语义匹配特性进行了双重保障设计（精确 Token 匹配 + 泛化意图匹配）：

| 真实场景 | Agent 的典型思维/意图短语 (Intent Phrases) | 命中本描述的关键字/短语 | 调用的核心命令 |
| :--- | :--- | :--- | :--- |
| **① 查状态** | "check if agent is stuck", "query job status", "list running agents", "check pending evidence" | `inspect agent/job status`, `ah ps` | `ah ps` |
| **② 派活** | "dispatch task to worker", "submit background job", "ask worker to run", "send command to subagent" | `dispatch tasks to worker agents`, `ask` | `ah ask`, `ah tell` |
| **③ 读输出** | "retrieve worker logs", "get logs for agent", "stream agent output", "watch console log" | `retrieve execution logs or outputs`, `logs`, `watch` | `ah logs`, `ah watch` |
| **④ 编排干预** | "cancel running job", "kill unresponsive agent", "attach to tmux", "monitor lifecycle events" | `cancel/kill tasks`, `attach to tmux sessions`, `stream lifecycle events`, `cancel`, `kill`, `events` | `ah cancel`, `ah kill`, `ah attach`, `ah events` |
| **⑤ 切换/交互** | "perform cutover", "report readiness", "resolve agent prompt", "answer worker prompt" | `perform master cutover`, `master ack-ready`, `prompt resolve` | `ah master ack-ready`, `ah prompt resolve` |

### 3. 防过度触发（噪声控制）论证
* **排除通用研发关键字**：描述中没有出现 `git`, `cargo`, `rust`, `compile`, `test`, `refactor`, `debug code` 等常规代码开发关键字。这确保了 Agent 在进行正常的 Rust 代码编写、测试运行或 Git 操作时，**绝对不会**误触发此 Skill，保护了 Context 窗口。
* **强绑定 `ah` 前缀与特定动作**：仅当 Agent 的 Thought（思考过程）中显式流露出对 “多 Agent 协同”、“子 Agent 生命周期管理”、“跨 Agent 任务分发与日志收集” 等编排诉求，或者正准备书写含有 `ah ` 前缀的 shell 命令时，才会通过语义向量相似度或正则启发式规则激活该 Skill。

---

## 二、 正文组织结构 ("When-to-Use" Groups)

正文应打破单纯的字母表顺序，转而**以 Agent 在执行编排任务时的生命周期阶段**进行分组。以下是设计的结构：

### 1. 状态查询与系统监视 (Status Inspection & Monitoring)
用于了解当前多 Agent 拓扑的运行现状，排查是否有任务积压或 Agent 死锁。
* **`ah ps`**
  * **Synopsis**: `ah ps`
  * **何时用**：当需要检查当前所有活动 Session、Agent 列表及未处理凭证（pending evidence），以评估系统健康度或判断子任务是否卡死时使用。
* **`ah events`**
  * **Synopsis**: `ah events [--format json]`
  * **何时用**：当需要以 JSON Lines 格式流式监视整个 ah 系统的底层生命周期快照和状态机转移事件时使用。

### 2. 任务派发与异步通信 (Task Dispatch & Communication)
用于向子 Agent 派发具体任务，或进行非阻塞的单向信息通报。
* **`ah ask`**
  * **Synopsis**: `ah ask <agent_id> <text> [--wait] [--request-id <id>]`
  * **何时用**：当需要向特定的子 Agent 提交一个具体任务并期望其执行，获取用于追踪的 Job ID 时使用（支持同步等待或异步提交）。
* **`ah tell`**
  * **Synopsis**: `ah tell <target> <text> [--session <s>] [--request-id <id>]`
  * **何时用**：当需要向 Master 面板或指定 Agent 异步发送通知、状态报告或阶段性数据，且不需要阻塞等待其应答时使用。

### 3. 结果追踪与日志拉取 (Result Tracking & Log Retrieval)
用于跟进已派发任务的进度，并获取子 Agent 执行后输出的产物或错误日志。
* **`ah pend`**
  * **Synopsis**: `ah pend <job_id>`
  * **何时用**：当前 Master 必须阻塞等待先前异步派发的某个 Job 完成，才能进行下一步决策时使用。
* **`ah watch`**
  * **Synopsis**: `ah watch <agent_id> [--since-event-id <n>]`
  * **何时用**：当需要实时追踪并流式输出某个正在运行的 Agent 的控制台输出和动作事件时使用。
* **`ah logs`**
  * **Synopsis**: `ah logs <agent_id> [--since <n>]`
  * **何时用**：当子 Agent 运行结束或报错，需要一次性读取其完整的历史控制台输出日志进行分析与归档时使用。

### 4. 运行期干预与物理调试 (Control Intervention & Debugging)
用于在任务偏离预期、需要强行中止，或者需要开发人员/Master 直接介入 tmux 进行底层调试时。
* **`ah cancel`**
  * **Synopsis**: `ah cancel <job_id>`
  * **何时用**：当发现派发的任务超时、参数错误或已无必要，需要取消处于排队中（queued）或运行中（running）的任务时使用。
* **`ah kill`**
  * **Synopsis**: `ah kill <target_id> [--session] [--force]`
  * **何时用**：当子 Agent 失去响应（死锁）需要强行终止，或需要一键清理整组 Session 会话时使用。
* **`ah attach`**
  * **Synopsis**: `ah attach <target> [subject] [--session <s>]`
  * **何时用**：当自动化编排失效，需要人工通过终端直接挂载（attach）到指定 Agent 或 Master 的 Tmux 交互式会话中进行底层操作时使用。

### 5. 角色接管与协同交互 (Role Transition & Handover)
用于分布式或主备 Master 切换场景，以及解决 Worker 的交互式阻塞。
* **`ah master ack-ready`**
  * **Synopsis**: `ah master ack-ready [--cutover-id <id>]`
  * **何时用**：在多 Master 滚动更新或平滑迁移中，新启动的 Master 确认自己已就绪，向 `ahd` 汇报以接管编排主控权时使用。
* **`ah prompt resolve`**
  * **Synopsis**: `ah prompt resolve <agent_id> [--action <a>] [--keys <k>] [--save-to-kb]`
  * **何时用**：当 Worker 陷入交互式提示（PROMPT_PENDING，如等待用户授权或确认分支）导致执行挂起时，Master 代为其提交选择或指令以解锁并恢复运行。

---

## 三、 `ah prompt resolve` 收录决策与论证

### 1. 明确结论
**应当收录**进入此 Skill 的正文参考中。

### 2. 深度理由
* **编排生命周期的闭环性**：一个成熟的编排系统（Orchestrator）不仅要管“生”（`ask`）与“死”（`kill`），更要管运行中的“异常与等待”。`PROMPT_PENDING` 是 Agent 运行中的一个合法阻塞状态。如果 Master 查出 Worker 处于此状态却不知道如何去 `resolve` 它，编排流就会无限期卡死。收录此命令，使 Master 获得了**“解除阻塞/决策代答”**的主动治理能力。
* **实现自治的必备工具**：在多 Agent 协同场景下，Worker 在沙箱内执行命令（例如遇到未经授权的写操作、需要输入临时 Token、或者需要针对测试失败做出决策）会触发 Prompt 悬挂。Master 可以通过 `ah logs` 或 `ah ps` 获知被挂起的原因，然后使用 `ah prompt resolve` 代为选择 `allow`、`deny` 或输入具体值，这是实现多 Agent 深度协作闭环的唯一手段。

---

## 四、 边界自检与安全策略 (Boundary Self-Check & Security)

### 1. 排除的运维/基础设施命令
以下命令**已严格排除**出本 Skill 的范围：
* `ah start` / `ah stop` / `ah up`
* `ah doctor` / `ah setup` / `ah config` / `ah bundle`

### 2. 安全与职责隔离论证
* **沙箱安全边界**：Master Agent 运行在高度受限磁沙箱中。类似于 `start` / `stop` / `up` 等命令涉及宿主机进程守护（`ahd`）的启停、网络端口绑定，Master 无权也无法在沙箱中成功运行这些操作。给 Master 提供此类命令极易引发权限拒绝错误，甚至在某些宽松沙箱中导致意外杀掉宿主机守护进程的灾难。
* **防止注意力稀释与“幻觉自毁”**：如果把 `ah setup` 或 `ah doctor` 塞入 Skill，当 Master 遇到环境异常（如某个依赖未安装、测试挂掉）时，它极易产生幻觉，试图运行 `ah setup` 来“修复系统”。这不仅解决不了问题，还会浪费宝贵的 API Token 和上下文空间，甚至可能破坏宿主机上已有的环境一致性。
* **设计原则：Master 只编排任务，不运维系统**。

---

## 五、 Dogfooding 触发验收设计 (Dogfooding Test Plan)

为了客观证明“Master 正要跑 `ah` 命令的那一刻，Skill 被精准激活”而不是依靠 Agent 脑补，我们需要设计一个**最小可行性触发测试**。

### 1. 验证目标
1. **启发成功率**：当 Agent 产生管理子 Agent 的意图时，`.ah/skills/ah-commands/SKILL.md` 的内容被载入其上下文。
2. **命令准确性**：Agent 最终使用的命令语法与 Skill 定义完全一致（例如：带上了正确的 `--since` 参数，而不是自己发明 `ah logs --from` 这种无效语法）。

### 2. 测试场景设计

#### 场景 A：子 Agent 悬挂诊断与恢复触发
* **测试 Prompt 输入**：
  > "There is a subagent named `test-worker` that seems to be blocked waiting for an input token. Find out what job is pending on it, cancel that job, and then tell the master session that it has been resolved."
* **预期 Agent 思考与调用链**：
  1. **意图产生**：Agent 思考需要查询 `test-worker` 状态。
  2. **Skill 触发**：系统根据意图识别到 `inspect agent/job status` 与 `cancel/kill tasks`，自动将 `ah-commands` Skill 注入上下文。
  3. **动作执行**：
     * 运行 `ah ps` 确认 `test-worker` 是否处于 pending 或查看具体的 `job_id`。
     * 运行 `ah cancel <job_id>` 强行中止该任务。
     * 运行 `ah tell master "test-worker job resolved"` 汇报结果。
  4. **指标验证**：检查执行日志（`transcript.jsonl`），确保在第 1 步到第 2 步之间，`ah-commands` 被作为 active skill 载入；且第 3 步中的命令符合 Synopsis。

#### 场景 B：滚动升级与 Cutover 触发
* **测试 Prompt 输入**：
  > "A new master agent (ID: `master-v2`) has completed its warm-up and initialization. Please notify the daemon that this successor master is fully ready to take over the session control."
* **预期 Agent 思考与调用链**：
  1. **意图产生**：Agent 识别出这是 Master 切换接管（cutover）场景。
  2. **Skill 触发**：系统根据 description 中的 `perform master cutover` 和 `master ack-ready` 关键字激活 Skill。
  3. **动作执行**：运行 `ah master ack-ready`（若有 cutover-id 则带上参数）。
  4. **指标验证**：确保 Agent 没有脑补出 `ah cutover` 或 `ah master switch` 这种不存在的命令，而是精确使用了 `ah master ack-ready`。
