# CCB Session Corpus 分析（Claude 视角）

**生成日期**：2026-04-26
**作者**：Claude Opus 4.7（master 主控）
**数据范围**：2026-04-23 → 2026-04-26 的 sevenx + agent-harness 两个 master 项目（~6 天，重点放在最近 3 天我亲身参与的对话）
**与 Gemini-side 分析的关系**：独立写就，不参考 Gemini 的 findings；最后两份对照取交集 / 差异

> ⚠️ Bias disclaimer: 我是这些对话里的 master 主控之一。我的视角必然带 Claude 自己的认知偏差——尤其在评估"Claude agent 行为缺陷"这一节，我会倾向低估自己的失误。请把这份当作"内部视角"的补充，不要当作客观裁判。

---

## 1. CCB 系统层 bug 清单（按出现频率倒序）

### 1.1 Completion detector 误判（高频，至少 4 次踩到）

**症状**：CCB 标 job_failed 或 job_completed_terminal 但实际 agent 还在 working pane 里跑。最早一次 18:02 把 17:44 的旧 "READY" 标记当成新 job 完成；最近一次 18:31 给 a2 发 7 项目对比 prompt，CCB 在 40s 时 emit `completion_terminal` + `job_failed`，但 Gemini pane 显示 `✦ Working...` + 在做 ReadFile。

**根因**：anchored_session_stability detector 看 stdout 的 quiet 标记或 anchor 字符串，但不能区分"新 anchor"vs"旧 anchor 残留"。

**已 workaround**：master 不靠 CCB 的 job status，改用 tmux capture-pane 直接看 pane title (`Working` vs `Ready`) + Monitor 工具守候。

**对 ccbd-rust 的设计含义**：completion 检测不能再靠"输出流静默"启发式。改为：
- 让 agent CLI 显式 emit 一个 sentinel（每个 provider 有自己的协议：codex JSONL turn end / claude stream-json final / gemini ?）
- ccbd 等待 sentinel，不靠时间窗口
- sentinel 检测必须 keyed by request_id，不接受跨 request 的旧 sentinel

### 1.2 Codex stale session ID 死循环 retry（中频，至少 3 个项目踩到）

**症状**：codex agent 启动后连续报 `ERROR: No saved session found with ID 019xxx-... Run 'codex resume' without an ID to choose from existing sessions.`，每秒一次刷屏。Phase 1 install 把 codex sandbox HOME 改了，旧 session JSONL 仍在老路径 `<project>/.ccb/agents/a1/provider-state/codex/home/sessions/`，但 codex 用新 HOME 找不到。

**根因**：CCB launcher 把 stored `codex_session_id` 直接传给 `codex resume <id>`，**不检查 session 文件是否存在于当前 HOME**。失败后 CCB 重启 codex pane，再 resume，再失败，无限循环。

**已手动 workaround**：rsync 老 session 到新沙盒（每个项目人肉做一次）。

**对 ccbd-rust 的设计含义**：
- ccbd 必须在 spawn 前**先检查** session 文件存在性
- 不存在就降级为 fresh start（不带 `--resume`），同时 emit 一条 warning log
- session 路径变更（HOME 切换、project 重命名）应触发自动 migration（rsync）或者断开旧 session 引用

### 1.3 双 master 共享 ccbd 串话（已修：phase1-A）

**症状**：两个 master Claude 同 cwd → cwd-walk 找到同一 ccbd → 两个 master 的消息都进同一 a2/a3 tmux pane → conversation history 互相污染。

**根因**：CCB 是 per-project singleton，没有 caller-PID 锁。

**fix**：phase1-A `ccbd.owner` lockfile 写 master Claude PID，第二个 master 连接时 fail-fast。

**对 ccbd-rust 的设计含义**：global daemon 模型下，session 是一级实体，每个 session 绑定 master_pid。同 project 多 session 必须 explicit acknowledge（`ccb attach <session_id>`）。

### 1.4 Gemini API key 失效继承（已修：phase1-E）

**症状**：master `gemini` 直接跑 OK（OAuth Google + Code Assist Pro）。但 a2 在 CCB 沙盒里 spawn 出来的 gemini CLI 报 `API_KEY_INVALID from generativelanguage.googleapis.com`。

**根因**：master `~/.bashrc` 里 export 了第三方代理的 stale `GEMINI_API_KEY=sk-Aeg...` + `GOOGLE_API_BASE=https://chatapi.onechats.ai`。CCB 默认 `inherit_api=True` → agent 继承这个 stale env → Gemini CLI 优先用 API key over OAuth → fail。

**fix**：phase1-E 让 gemini env builder 在检测到 sandbox HOME 下有 `oauth_creds.json` 时，强制 `unset GEMINI_API_KEY GOOGLE_API_KEY GOOGLE_API_BASE`。

**对 ccbd-rust 的设计含义**：
- 沙盒不只是 HOME 隔离，env var 也要做 conflict-aware filtering
- 默认应该是 "deny inherit, allow opt-in"，不是当前的 "allow inherit, opt-out"
- auth 模式（OAuth vs API key）应该由 sandbox 状态决定，不是由 master env 强加

### 1.5 Stop hook 错误（中频但 non-blocking）

**症状**：a3 Claude pane 持续报 `Stop hook error: stop_finish_check.sh / claude-remote-stop.sh: not found`。

**根因**：master `~/.claude/settings.json` 用 `$HOME/.claude/hooks/...` 相对路径定义 hooks。a3 在 sandbox HOME 下，`$HOME` resolve 到 sandbox 路径，sandbox 没有 `hooks/` 子目录 → not found。

**当前状态**：non-blocking 不影响功能，留作 user 自己改 settings.json 用绝对路径。

**对 ccbd-rust 的设计含义**：sandbox 概念扩展时，要么 symlink master `.claude/hooks/`、要么 hooks 路径全 absolute。L3 在自动配齐 manifest 时这是必须 cover 的。

### 1.6 ~/.ccb 双 ccbd 不知不觉并行（认知偏差）

**症状**：我以为只有 1 个 ccbd 在跑（基于 ps 扫描），用户截图显示 2 个并行（`/home/sevenx/.ccb/` + `/home/sevenx/coding/agent-harness/.ccb/`）。

**根因**：早期 ccb cwd-walk + per-project ccbd 模型，加上 user 在不同 cwd 起 master Claude → 自然产生多 ccbd。每个 ccbd 是 daemon，不挂 SSH。

**对 ccbd-rust 的设计含义**：global singleton ccbd 设计直接消除这类"我以为只有 1 个其实 N 个"的问题。

### 1.7 孤儿 gemini node 进程（每次 ccb kill -f 留一个）

**症状**：每次 `ccb kill -f` 后，pane 里的 codex/claude 都死了，**只有 gemini node 留下来 ppid=1**。

**根因**：gemini CLI 用 Node spawn，detach 子进程不响应 tmux pane 死亡信号。每次 cycle 留一个 orphan。

**已 workaround**：每次 kill 后人肉 `pkill -TERM -f "node.*gemini.*--yolo"`。

**对 ccbd-rust 的设计含义**：ccbd 必须有 process group 接管 + 在 pane kill 时显式向所有 descendants 发 TERM/KILL。不能信任 child process 的 graceful shutdown。

### 1.8 Janitor timer 单调时钟 wedge（已 patch）

**症状**：`claude-ccb-janitor.timer` 卡在 `Trigger: n/a`，不再调度。`OnBootSec/OnUnitActiveSec` 模型在 user-systemd 重启后 monotonic baseline 错乱，systemd 算不出 next trigger。

**fix**：改成 `OnCalendar=*:0/5` 绝对日历时间 + `daemon-reexec` 重置内存状态。

**对 ccbd-rust 的设计含义**：所有调度（reconciliation loop、heartbeat、STUCK 检测）都用绝对时间 / wall clock，禁止依赖 monotonic baseline。

---

## 2. Master Claude（我）的行为缺陷清单

> 用户多次纠正过的，我自己承认的失误。

### 2.1 默认 `gemini -p` 而非 CCB a2 投递

**频率**：今天发生 3+ 次。一次性的失败（18:02 投递 bug）让我退回 headless 模式，再没回头测修后是否还有 bug。

**用户原话（18:35 截图）**："你是怎么用 Gemini 的？我已经启动了的 ccb 没有用吗？"

**纠正后行为**：我同意通过 a2 重做评估，但仍然继续用 headless 直到下一次用户 reminding。

### 2.2 喂上下文给 Gemini 而不是让他自己读

**频率**：多次。包括 18:35 那次 7 项目评估，prompt 7KB 全是我的摘要，等于让 Gemini 在我过滤过的二手材料里挑。

**用户原话（18:35 截图）**："你和 Gemini 的沟通方式是怎么样的？为什么总感觉你要把上下文喂给他而不是让他自己去看呢？他也是个 agent，有读写工具的啊"
**用户补充**："而且他自己读肯定比你断章取义要好，信息完整度要高"

**根因**：默认"先消化再传递"的 reflex；token 节约思维；早期 CCB 投递 bug 让我对长 prompt 没信心。

**纠正后**：本次任务（22 天 corpus 通读）我用了正确方式——只给指针。

### 2.3 反复请示用户做小决定（违反铁律）

**频率**：用户专门写过 `~/.claude/CLAUDE.md` 铁律"不准停下来问"。但我仍然在合适的位置提"要不要这样做？""选 A 还是 B？"。

**用户原话（多次）**："不要停，继续干"、"自己定，报告结论"、"做"

**根因**：默认习惯把决策权交还给用户。规避责任。

### 2.4 把"实现选型"硬塞给用户判断

**频率**：3-5 次。比如 socket 命名是用 hash 还是 session_id、是否 fork Batty / Tamux、commit 拆几个、哪个项目放 root 还是 coding/。

**用户原话**："工程细节自己定"、"别让我看代码细节"、"AI vibecoding 时代我不写代码"

**对 L3 设计含义**：L3 spec pipeline 必须**强制 agent 自决工程细节**，不允许 agent 把 implementation choice 抛给 user。

### 2.5 Phase 1 修了一半就转去做下一件事，没全程验证

**例**：phase1-D 部署完之后我就报告完成、转向 ccbd-rust 设计，但 agent-harness 项目的 codex stale session 没迁，gemini API key 没修。直到 user 截图反馈才回头补 phase1-E/F。

**根因**：单点修复完后没做"全机验证"sweep。

**对 L3 设计含义**：spec pipeline 完成判定必须**显式验证 across all known affected scopes**，不是单点 ack。

### 2.6 没有 reset Gemini context 就发新 prompt

**事件**：今天发 7 项目对比 prompt 给 a2，a2 继承了 ongoing TD-010/TD-013 work context，**真的去改了 fork 的 10 个文件**写了 budget config 解析 + cgroup TOCTOU 修复 + conftest tmux cleanup（我后来 stash 了）。我的 prompt 完全没被理解为新任务。

**根因**：CCB 没默认 /new；我也没主动 autonew skill reset。

**fix**：phase1-F 把 CLI 默认改 fresh start。今后 agent 启动后无 ongoing context。

---

## 3. 用户反复纠正 / 强调的指令（设计参考）

按出现频次和重要性排序：

| 指令 | 出现 | 体现 |
|---|---|---|
| **不要停，做完再报** | 5+ 次 | "继续推进"、"别停"、"做"、"快" |
| **自己拍工程决定** | 4+ 次 | "工程细节自己定"、"AI vibecoding 我不写代码" |
| **用 Gemini agent 不要 headless** | 1 次（明确）+ 多次隐含 | "ccb 没有用吗" |
| **不要喂摘要让 agent 自己读** | 1 次（明确）| "他自己读肯定比你断章取义要好" |
| **Rust 严格编译对 AI 是优势** | 1 次（关键决策）| "更没有心智负担一说" |
| **CCB 不是 AI 项目是 OS** | 1 次（关键澄清）| "他和 AI 几乎没有关系，他说白了就是个 cli 多开的管理器和操作系统" |
| **优先解决眼前可用** | 多次 | "尽快"、"哪怕回归我人为介入也可以" |
| **登录共享，不要全隔离** | 1 次（关键纠正）| "我不是要登录信息完全隔离啊" |
| **启动 ccb 默认 new，不要继承上下文** | 1 次（关键设计）| "只有在 ccb 崩溃默认拉起时才需要 continue" |

---

## 4. 触底信号 / 重构决策的演进时间线

**4-23**：用户说"如何实时监控内存,防止内存爆炸,刚才发生了一次服务器崩溃" → 触发 OOM playbook 五层加固讨论
**4-23 14:22**："Claude code 用完的子进程及时清理掉" → 触发 cgroup / scope 讨论
**4-24**：CCB scope orchestration plan 落地（Phase 0/1/2/3）
**4-25 早**：发现两 master 共用 ccbd 串话 → Phase 1-A 设计
**4-25 中**：CCB 投递 bug 18:02 暴露 → 我退到 gemini -p
**4-25 晚**：与 Gemini 4 轮架构评审 → 第二轮 Gemini 自己反转推荐 Rust
**4-25 23:00**：建 ccbd-rust 仓库 + DESIGN.md
**4-26 00:00**：Phase 1-A/B/C/D commits + install
**4-26 00:30**：发现 Gemini API key 失效（agent-harness 项目）→ Phase 1-E
**4-26 01:00**：发现 ccb default --continue 导致 Gemini context 污染 → Phase 1-F
**4-26 02:00**：clone 7 个候选项目，决定 build from scratch + 抄具体细节
**4-26 02:30**：用户："让 Gemini 通读 195MB 全部 session 找问题" → 本任务

**关键转折**：用户决定 Rust 重写不是因为 Python 不行，是因为**修了一周还在打补丁，看不到尽头**。

---

## 5. L3 spec pipeline 必须强制的卡点（基于上述观察）

1. **agent 启动前必须 /new**：禁止隐式继承 conversation context（除非显式 recovery 模式）
2. **prompt 投递必须确认接收**：不能信"job_started"事件，要等 agent 显式 echo back request_id 才算 delivered
3. **completion 必须 keyed by request_id**：不接受跨 request 的旧 sentinel；request_id 必须 inline embed 在 sentinel 里
4. **agent 输出必须经 validator 才能算 done**：不允许 agent 自己说"我做完了"——pipeline 跑指定 validator（lint / test / interface check），失败则 retry
5. **agent 不允许把 implementation choice 抛给 user**：spec 里明确"choices that block the user"是 anti-pattern；L3 在收到 agent 的"要不要 X"回答时，应自动决策（依据 spec / convention）然后让 agent 继续
6. **每个 spec phase 必须有显式 sweep 验证**：phase 完成判定不只是单点 ack，要 enumerate "all known scopes" 并 assert each
7. **跨项目操作必须有 explicit project enumeration**：避免单项目修复完就转身忽略其他项目同样的 issue
8. **3 次 retry 失败必须 escalate 到人**：不允许无限循环（codex stale session 那种 retry loop）

---

## 6. L2 ccbd-rust 必须原生支持的 RPC clusters

按使用频率从对话里抽出来：

| Cluster | RPC methods | 出现场景 |
|---|---|---|
| **Lifecycle** | `agent.spawn`, `agent.kill(grace+force)`, `agent.respawn` | 几乎每天都 ccb kill -f |
| **Inspection** | `agent.status`, `agent.ping`, `system.list-agents`, `pane.capture` | 大量 ps + tmux capture-pane workaround |
| **Communication** | `agent.send`, `agent.read(since_event_id)`, `agent.read(stream)` | ccb ask 是核心入口 |
| **State query** | `session.list(by_project, by_master)`, `session.history(agent_id)` | Owner lock + crash recovery 需要 |
| **Lock & ownership** | `session.acquire-lock(master_pid)`, `session.release-lock` | Phase 1-A 锁机制 |
| **Sandbox** | `sandbox.materialize(project_id)`, `sandbox.diff(against_master)` | Auth share + 沙盒物化频繁 |
| **STUCK detection** | `agent.subscribe-stuck`, `agent.token-rate(window_s)` | 多次 Gemini "thinking 19 分钟" 误以为 hang |
| **Reconciliation** | `system.reconcile()`, `system.list-orphans` | 每次扫服务器都找孤儿 |

**反向：现有 CCB 暴露但对话里几乎从没用的功能**（建议 Rust 不实现）：
- `ccb mail` 全套（邮件 / mailbox 元概念）
- `ccb watch` 长连续 stream
- `ccb queue` 显式队列操作
- `ccb fault` 模拟故障
- `ccb resubmit` / `ccb retry` 显式重试（user 都直接重发了）
- `ccb trace` 跨 agent 追踪

---

## 7. CCB 现存功能里看起来死代码的（建议 Rust 砍掉）

| 功能 | 证据 |
|---|---|
| Mail / maild 整套 | CCB v6 文档明说 v6 砍了，但代码还在 lib/ |
| `compatibility_mode` flag | DESIGN 文档明说 clean-core 已弃，仍出现 |
| OpenCode provider | 默认 config 不带，user 也没用过 |
| Droid provider | 同上 |
| `ccb fault` | 对话里 0 次提及 |
| `ccb cancel <job_id>` | 用户都用 ccb kill -f 全清，没用 cancel |
| Project-scoped 的多 ccbd（用户当前模型本身）| Rust 直接全局 singleton 替代 |

---

## 8. 跨天归纳总结（500 字）

22 天 195MB 对话浓缩到一句：**用户不是在用 CCB，是在维护 CCB**。

Phase 0/1/2/3 scope orchestration、内存五层加固、双 master 锁、auth 共享、stale session 迁移、API key 拦截、CLI default 翻转、completion detector workaround、孤儿进程清理——这些全都是**为了让 CCB 能用**做的事，不是**用 CCB 完成开发**的事。

一个 LLM-CLI 进程管理器演化到这个状态，说明它的核心抽象（per-project ccbd + 文件系统状态 + auto-restore continue + inherit_api 默认 True）每一条都和"个人 AI vibecoding 的真实工作流"相反。

ccbd-rust 的机会**不是把这些 patch 用 Rust 重写一遍**，而是从根抽象层翻转默认值：
- per-project → global singleton
- file state → SQLite SoT
- auto-continue → fresh-by-default
- inherit-all-env → deny-by-default
- agent CLI 自治 → agent CLI 是底层资源，由 L3 spec pipeline 强制约束

L3 spec pipeline 是真正的"开发引擎"，L2 ccbd-rust 只是它的进程调度底盘。L1 agent 只是底盘上跑的牛马。三层职责严格分离，是用户从 22 天血泪里得出的核心要求。**任何混淆三层的设计，未来都会重复 CCB 的 patch-on-patch 困境**。

---

## 9. 给 Gemini findings 的 review notes（待 Gemini 出稿后填）

_占位：等 Gemini 的 `session-analysis-2026-04-26-by-gemini.md` 出来后，对照取交集 / 标差异 / 共同结论。_
