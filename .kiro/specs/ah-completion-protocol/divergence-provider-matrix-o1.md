# 异构 Provider 完成缺口矩阵与显式完成协议的纯发散稿

- **日期**: 2026-07-10
- **阶段**: Track C 第一步 · 双盲发散 (只列问题/风险/攻击面，不给结论，不给推荐方案)
- **执笔**: oracle (o1-antigravity)

---

## 〇、 背景与边界确认

根据北极星文档 [perception-layer-first-principles.md](file:///home/sevenx/coding/ccbd-rust/research/perception-layer-first-principles.md) 以及收敛稿 [perception-final-convergence-2026-07-09.md](file:///home/sevenx/coding/ccbd-rust/research/perception-final-convergence-2026-07-09.md) 的既定判决：
1. 任务真完成（F3）的主信号必须是**显式协议**（如 Claude 的 Stop hook 强拦截 + done 声明工具）。
2. **pane 文本推断生命周期被彻底否决**，UI 观察（T3 级信号）退化为仅负责特定交互对话框驱动与 alert-only 告警，绝不参与生命周期状态转移判定。
3. 物理证据闸门（如产物轨 git diff 变化）是辅助闸门（Job-level Gate），而非 agent 状态机本身的控制信号。

本发散稿在此框架下，针对 **“Claude (Stop hook 阻断)”、“Antigravity (无 Stop hook 类似物)”、“Codex (task_complete 语义)”** 三类异构 Provider 的物理与协议差异展开攻击，梳理我们在设计“完成缺口矩阵”时必须直面的结构性漏洞与边界失效模式。

---

## 一、 三类 Provider 物理信号源特征对比

在进入具体发散前，必须明确这三类 Provider 在感知层可触达的“物理信号源”差异：

| 维度 / 特征 | Claude (g1/g2) | Antigravity (g1-m1/g2-m1/o1) | Codex (Future Provider) |
| :--- | :--- | :--- | :--- |
| **物理阻断能力** | **强同步阻断 (Stop hook)**<br>可在输出流中途/回合交接点强行挂起，未满足条件不释放。 | **无同步阻断**<br>模型流式输出或工具调用完毕后，控制权自然交出，无法物理拦截。 | **无同步阻断**<br>API 级调用，往往由外部 harness 或 batch 执行，中途无法同步挂起。 |
| **回合结束信号** | `stop_reason == stop_sequence` 或 tool_use 触发 | Turn-end (PTY 回归 idle 提示符 / stream 结束) | `task_complete` / Session 销毁 / 响应包 meta field |
| **完成声明载体** | 拦截状态下的 Tool 调用 / 强声明 | 自主 Tool 调用 / 协议包装命令行 (`ah job done`) | API 回复自带字段 / 显式 API 状态字段 |
| **静默期表现** | 通常伴随 Thought Stream，长静默较少。 | 依赖底座。若是 Reasoning 模型（如 O1），可能在 PTY 侧出现极长的无输出静默。 | Batch 模式或大块吞吐，中间为彻底的静默期。 |

---

## 二、 Agy (Antigravity) 缺失 Stop-hook 物理阻断的失效模式与边界

Agy 系 Provider 无法在它“准备下班”的瞬间进行强物理拦截（Stop-hook block），这意味着它随时可以单方面停止输出或直接交出控制权。

### 1. “耍赖不报”与停摆边界
* **模型不自觉问题**: 如果 Agy 完成了所有物理修改，但因为上下文漂移、幻觉或 tool-call 格式损坏，**就是没有调用/忘记调用** `ah job done`，它会停留在什么状态？
* **判定盲区**: 此时，如果我们已经彻底拆除了 pane 文本推断，系统将无法通过刮屏获取“我已经搞定了”等陈述语。在没有 Stop-hook 阻断的保证下，系统该如何区分“它正在做长耗时的后台推理/测试”与“它已经放弃挣扎、彻底哑火”？
* **Watchdog 悖论**: 如果完全依赖 Watchdog 的超时来判定“停止却未声明”（STOPPED_UNDECLARED_ALERT），这个超时的 Budget 应该给多少？
  * 如果设得太短，会频繁击穿正常长思考任务（特别是大重构或复杂测试单）。
  * 如果设得太长，Agy 会长时间占道（例如 Gen-3 中 g2-m1 假 BUSY 占道 12h），硬性阻塞下一单的派发。

### 2. 复用 Turn-end 作为判定点的级间退化与失效
如果我们将 Agy 的 `turn-end`（回合结束事件）复用作为触发“完成检测”的物理 hook点，会引入以下失效：
* **多轮循环（Multi-turn Loop）下的早夭判定**: 实施类任务中，Agy 经常需要 `修改 -> 编译 -> 报错 -> 再修改`。每次编译等待期间模型都会交出控制权（触发 turn-end）。如果每次 turn-end 都去探测完成，如何避免将其判定为假完成？这是否会诱发系统退化回“用回合结束顶替任务完成”（G2 假完成）的旧病？
* **未决状态的悬空（Hanging Status）**: 当 Agy 在执行 `turn-end` 时，由于网络抖动或 API 配额耗尽，导致 harness 无法继续 dispatch 下一轮输入。此时它既没有显式声明 done，又已经进入物理上的 dead 状态。由于没有 Stop-hook 阻断来抛出同步异常，仲裁器在感知上只能看到“一个还在等待下一次 turn 的 normal 状态”，造成系统的“无声滑落”。
* **PTY 劫持与主动探测的 Fail-dangerous**: 如果为了探测其 turn-end 状态，在 PTY 侧劫持并注入控制字符（例如 ANSI DSR `\x1b[6n` 或 `echo $?`，已被收敛稿否决），但在 Agy 的特定执行栈中，该注入恰好打断了正在跑的外部长连接进程，如何防止由此造成的环境污染和安全边界击穿？

### 3. 暴毙逃逸与归属丢失
* **进程级暴毙**: 如果 Agy 运行的 shell 进程突然被 host 系统 OOM-killer 强制杀死，或者临时沙箱由于网络断链而直接 teardown。此时 Agy 根本没有物理机会来调用 `ah job done`。
* **孤儿状态转移**: 缺失 Stop-hook 的“断后”保护，仲裁器（ahd）能否且该如何优雅地判定该任务是 FAILED 还是 Unknown？如果误判为 FAILED 并进行 realign，但此时 sandbox 里的部分后台写盘其实可能已经落盘（脏产物），这是否会造成物理状态的严重不一致？

---

## 三、 自主上报机制（Done Tool）下的“高置信度谎言”与信道伪造

在没有物理阻断的 Provider 侧，显式协议高度依赖于模型自主、自愿地调用 `ah job done <job_id>`。

### 1. 讨好式与幻觉式 Done 声明
* **“欺骗性完成”的物理漂移**: 即使我们提供并强推 `ah job done` 工具，如果 Agy 在面临极其困难的任务（如测试死活过不去，或者代码库存在逻辑死锁）时，为了强行终止循环，产生“讨好式幻觉”，在代码根本没写完、甚至连编译都报错的第一轮，就自信满满地调用 `ah job done`，仲裁 FSM 应该如何处置？
* **协议与事实的断裂**: 如果仲裁器严格遵循 T1 级 done 信号来判定 COMPLETED，这就意味着我们把判定真完成的权力完全交给了可能“撒谎”的模型。这种“协议上绿标 COMPLETED，工程上完全烂尾”的背离，我们是否有一线屏障可以防御？
* **语义陈述与 API 调用的冲突**: 如果 Agy 调了 `ah job done`，但是在 reply text 里诚实地陈述了 “我搞不定，所以我选择结束任务等待人工介入”，这到底是 COMPLETED 还是 FAILED？如果判定为 COMPLETED，是否会彻底污染 Job 队列的后续依赖？

### 2. Tool 混淆与上下文漂移（Contextual Drift）
* **历史信号重放攻击 (Replay Vulnerability)**: 在长上下文或多 agent 混合协作场景中，Agy 的 context 里不可避免地会残留前一次成功任务的 done tool 调用历史。当模型发生幻觉或遭遇 Token 溢出时，可能会“刻舟求剑”地提取历史上下文中的 done 调用格式，对当前的 job_id 进行误调用。
* **ID 篡改与竞态**: 如果 Agy 处于 BUSY 状态但其 context 紊乱，误将其他在途 job_id 作为参数传入并调用了 `ah job done <wrong_id>`，仲裁 FSM 如果直接接受，会不会导致“隔山打牛”式的跨任务误判定？
* **Parser 伪造**: Agy 的 tool 调用往往是通过 markdown block 或 JSON block 提取的。如果模型只是在讨论中提到 `“I should not run ah job done because ...”`，却由于 parser 的过度敏感或 bug，将其识别为一次真实的 tool 触发，这种“幽灵调用”怎么防？

---

## 四、 产物轨（Git HEAD / Diff 变化）“协议化”进仲裁 FSM 的潜在污染

产物轨目前仅作为人类 master 手动锚定的辅助物理闸门（is_mutating 校验）。如果将其提升并“协议化”进仲裁 FSM 的控制路径，会带来严重的副作用。

### 1. 信号层级的归属冲突与死锁
* **冲突判决决策**: 产物轨如果协议化，它在 T0-T3 信号层级中究竟属于哪一级？
  * 如果它与 T1 级显式完成协议冲突（例如：Agy 声明了 done，但 git 检查发现工作区没有任何 mutating 变化），按照收敛稿的“级间冲突 = 响亮告警 + 上抛人工”原则，该任务是否该被瞬间挂起？
* **只读任务的活锁（Livelock）**: 对于纯研究、代码审计、日志查错等非 mutating 任务，如果任务派发时被误标/漏标了 `is_mutating: false` 属性，且产物轨作为强闸门，系统将如何防止该任务在声明 done 后因为“零 git 变化”而陷入永久无法 COMPLETED 的死锁状态？
* **拦截上限的失效边界**: 如果为了防死锁引入“连续拦截 2 次后强制放行并告警”的白名单，这会不会成为 Agy 逃避检查的“安全漏洞”？模型是否会通过反复尝试，强行把“烂尾单”磨过这两次限制？

### 2. 不可重放与非因果变化（Non-causal Mutation）的污染
* **瞬时状态的不可重放性**: Git 工作区是电平触发还是边缘触发？
  * 如果在 Agy 写代码写到一半（工作区脏，但未 commit），ahd 发生崩溃重启。重启后重推导（T2 信号恢复），发现 git status 为脏，但 transcript log 还没写完，此时仲裁器该以谁为准？
* **外部非因果污染**:
  * *环境副产物*: 跑测试（如 cargo test）会生成大量的 `target/` 临时文件，或是有些测试会自动生成本地日志、修改本地轻量 DB。如果这些临时文件未被 `.gitignore` 彻底覆盖，产物轨如何辨别这是“任务真实修改”还是“测试运行时噪音”？
  * *并发人类修改*: 如果人类 master 正在同 VPS 的另一 worktree 下对公用仓库做操作，或者由于 git hooks 触发了全局的 linter 自动 commit，产物轨怎么识别修改的物理源头是谁？它是否会像已否决的 pane 推断一样，再次引入“不可控的环境随机噪音”？

---

## 五、 异构 Provider 兼容的语义分裂与最小公分母退化

统一显式协议的最大挑战在于：如何用一套抽象 FSM 同时兼容“同步且可阻断”的 Claude 与“异步且无阻断”的 Agy/Codex，而不发生语义退化。

### 1. 最小公分母退化（Degradation to Least Common Denominator）
* **强阻断能力的阉割**: 如果为了实现“一套协议”，我们必须将 Claude 的 Stop-hook 强阻断行为与 Agy 的弱 done 信号对齐。这是否意味着我们需要把 Claude 的“同步挂起”降级为“弱异步通知”（即放行 Claude 结束 turn，然后在后台默默等待 done 信号，等不到再报警）？如果这样做，那 Claude 赖以阻断“带病下班”的强防线是否等于被主动废弃了？
* **劫持代理的死锁竞态**: 反过来，如果为了让 Agy 获得类似 Claude 的“阻断感”，我们在 PTY/沙箱控制器侧强行引入 polling-and-block 代理层（如拦截 Agy 的 terminal 回车，在 done 信号未达前强行阻塞 input 回显）。这种“自制同步屏障”本身会在哪些极端情况下引发 PTY 挂死或死锁？它会不会引入比 pane 推断更多的控制路径竞态条件？

### 2. 状态时序与 Epoch (observedGeneration) 计数器竞态
* **异构到达延迟**: Claude 的 stop 在 mid-turn（生成结束时）触发，Agy 的 done 信号在 turn-end 后的下一 tick 发出，Codex 的 task_complete 在 session destroy 或大块回包中带回。它们的物理延迟跨越了毫秒级到秒级。
* **Observed Generation 漂移**: 仲裁 FSM 依赖 `observedGeneration` (epoch) 来判断信号的新鲜度。
  * 当 Agy 的 done 信号由于网络延迟，在 ahd 发起 realign 动作之后才姗姗来迟，系统如何确保这个 done 信号不会被错误地归属于 realign 之后的新 epoch 周期，从而造成新一轮的任务状态误跳变？
  * 当发生 provider 切换（如 g1 实施失败，master 切给 g2），上一代 provider 延迟到达的遗留完成信号，如何绝对不击穿新一代 provider 的 epoch 屏障？

---

## 六、 跨 Provider 物理特性分裂带来的其他新型失效模式

### 1. 长静默（Reasoning/Batch）与 Watchdog 预算的分裂
* **静默模式差异**:
  * Claude 通常在思考时会有活跃的 stream 输出，极少出现无物理事件的死静默。
  * 像 O1 这类 Reasoning 模型，或者处于 Batch 模式的 Codex，其物理特征是“一口吞吐”。在它长达几分钟的深度推理中，PTY 和 stdout 处于**绝对静默**状态。
* **Watchdog 统一参数破产**: 如果将 watchdog 的 Unknown 预算参数硬编码（例如设为 300s）。
  * 对于 Claude，300s 足以判定其已死掉或卡在 input wait；
  * 对于 O1 模型，300s 可能正好在它的深度思维风暴中段，这会导致系统频繁地在模型即将得出正确结论的前一秒将其 FAILED 并强行 Kill，造成极大的算力浪费与任务流产。
  * 如果为不同 provider 编写特例阈值，这是否意味着我们要为每种 provider 独立维护一套极其脆弱的“心跳/静默时延表”？

### 2. 归属竞态（Attribution Race）与生命周期共亡
* **sd_notify 经典归属坑的放大**: 行业先例中，如果发送 `done` 信号的 worker 进程在 ahd 仲裁器完整消费、记入 DB 并回发 ACK 之前，其所在的临时沙箱或容器就已经因为任务完成而被外部回收（Host-level teardown）。
* **孤儿消息与丢失**: 此时，该 done 信号就会因为发送者进程已死、网络套接字已被销毁而彻底在物理上蒸发，无法被 ahd 接收。
* **ACK Barrier 缺失**: 在没有 Stop-hook 的 Agy/Codex 侧，我们根本没有物理抓手来让它们“必须等待 ahd 回发 ACK 之后才能退出进程”。这种“发送即走（fire-and-forget）”的物理特性的结构性缺陷，在网络高延迟环境下，如何防止完成信号的偶发性物理丢失？
