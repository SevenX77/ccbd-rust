# Design brief for a3 (antigravity) — ah-commands skill 的触发策略 + 命令编排

你是 a3(设计/领域分析,不写实现码)。这是**设计判断**任务,产出一份 markdown 设计分析,别改代码、别 grep 实现细节(锚点 master 已核实,直接用下方事实)。

## 背景(master 已 grep 核实的事实,直接采信)
- ah 的 agent(尤其 master)不知道 ah 有哪些命令,一直靠猜。要给一份权威命令参考,走**渐进式披露**:不塞进 kernel(注意力稀释),做成一个 skill。
- 机制:一个 `SKILL.md`,frontmatter 的 `description` 字段驱动**启发式触发**(agent 在"正要用某能力"那刻,靠 description 与当下意图的匹配决定是否激活 skill)。正文 = 命令参考。
- 载体是**项目 skill**(`.ah/skills/ah-commands/SKILL.md`),会物化进 claude/codex/antigravity 三家沙箱。
- **命令范围已定**:只放 agent-facing 编排子集,**不放**运维命令(start/stop/up/doctor/setup/config/bundle —— master 不该跑)。

## 已核实的 agent-facing 命令清单(带精确 synopsis,来自 src/bin/ah.rs clap 定义)
| 命令 | synopsis | clap about |
|---|---|---|
| `ah ps` | `ah ps` | List sessions, agents, and pending evidence |
| `ah ask` | `ah ask <agent_id> <text> [--wait] [--request-id <id>]` | Submit an ask job to an agent |
| `ah tell` | `ah tell <target> <text> [--session <s>] [--request-id <id>]` | Asynchronously deliver text to the master pane |
| `ah pend` | `ah pend <job_id>` | Wait for a submitted job to finish |
| `ah watch` | `ah watch <agent_id> [--since-event-id <n>]` | Stream agent output events |
| `ah logs` | `ah logs <agent_id> [--since <n>]` | Print stored output for an agent |
| `ah events` | `ah events [--format json]` | Stream runtime lifecycle snapshots as JSON lines |
| `ah cancel` | `ah cancel <job_id>` | Cancel a queued or running job |
| `ah kill` | `ah kill <target_id> [--session] [--force]` | Kill an agent, or a whole session with --session |
| `ah attach` | `ah attach <target> [subject] [--session <s>]` | Attach to an agent/master tmux session (target=master/agent/legacy-id) |
| `ah master ack-ready` | `ah master ack-ready [--cutover-id <id>]` | Report successor master readiness to ahd |
| **候选** `ah prompt resolve` | `ah prompt resolve <agent_id> [--action <a>] [--keys <k>] [--save-to-kb]` | Send an action to a PROMPT_PENDING agent |

## 你要设计/回答的
1. **触发可靠性(成败关键)**:设计 SKILL.md 的 `description` 字段文本(1-3 句)。它必须让 skill 在这些真实场景被激活:①master 想查 agent/job 状态(某 agent 卡没卡、job 完没完);②想派活给 worker;③想读某 worker 的结果/输出;④想编排 ah(取消/杀/attach/看事件流)。给出**具体 description 措辞**,并列出你覆盖的**触发词/意图短语**,论证为什么这些词能命中上述场景而不过度触发。
2. **正文组织**:正文按"何时用"组织(不是字母序)。给出分组结构(如"查状态 / 派活与等结果 / 读输出 / 干预控制 / cutover")+ 每组放哪些命令 + 每条一句"何时用"。
3. **`ah prompt resolve` 收不收进来**:它处理"worker 卡在交互 prompt、master 去替它作答"——算 agent-facing 编排吗?给判断 + 理由。
4. **边界自检**:确认你没把运维命令(start/stop/up/doctor/setup/config/bundle)混进来;说明为什么这些不该给 master(安全边界:master 只编排、不运维)。
5. **触发验收怎么测**:给一个 dogfood 实测触发的验收设计——怎么证明"master 正要跑 ah 命令那刻 skill 真被激活"(不是写完就算)。

## 产出
落一份 markdown(标题 `# ah-commands skill 触发+编排设计(a3)`),结构对应上面 5 点。不写实现码,不碰 git。
