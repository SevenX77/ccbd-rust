# PM 代理（operator ↔ master）工作经验总结

> 作者：PM 代理（operator）本人，基于 ah 自托管开发全过程的实操沉淀。
> 用途：作为「编程场景配置包」使用指南的一部分——它描述的是**人的代理（operator）**这一层怎么和 ah 托管的 master 协作。这层经验不在仓库任何设计文档里，是操作侧的隐性知识。

## 0. 三层拓扑与角色边界

```
用户（需求/目标层）
   ↕  只在需求与方向上参与
operator（PM 代理 = 我）
   ↕  转达目标、盯结果、管 gh/CI/发布
master（ah 托管的 PM）
   ↕  ah ask 分派 / ah pend 收结果
workers：a1 codex(主力编程) · a2 codex(设计+审) · a3 antigravity(调研/探索) · a4 claude(e2e+审计)
```

- **operator（我）**：用户的代理。把用户的目标翻译成可执行 brief 转给 master；持续盯到真出可信结果；独占 `gh`/CI/PR/合并/发版权。**substantive 代码审阅归 master**，我做独立抽检。
- **master**：项目 PM。规划、分解、分派 workers、审阅、收敛。不直接改 `src/`/`tests/`。
- **worker**：只执行当前被指派的单条任务，完成即回、等下一单。**绝不自派单、绝不自命 PM**。

分工铁律（每次派活都要兜底声明，否则 agent spawn 无身份易越权当 master）：
- 严谨机械操作（grep / file:line / N 章节全覆盖 / 参数化小改）→ codex（a1/a2）。
- 独立判断/推理/综合 → 分析型（历史是 gemini，现为 codex a2 或 operator 自己）。
- 探索性只读 sweep / 结构化提取 → antigravity（a3）。
- e2e / 审计 / a1 忙时分担 → claude（a4）。

## 1. 不信状态，信盘面（头号纪律）

ah 的 `status`/`title`/`wait`、job 的 `COMPLETED`、agent 的 `IDLE` **都可能撒谎**。`COMPLETED` ≠ 真完成，`BUSY→IDLE` 2 秒可能是存了垃圾 reply。

每次判断都要物理验证：
- `tmux -L <socket> capture-pane -p -t <pane>` 看 pane 真实内容。
- `git diff` / `git status` / `grep` 看代码真落盘。
- agent 报「完成/已写 11 个文件」后必 `ls`/`wc`/`head` 实证（subagent 会把 INVENTORY 标 ✅ 但实际 0 个）。

## 2. in-loop 盯到结果，不干等靠机制

派活的那一刻 = 我的任务开始，不是结束。持续 monitor 直到真出可信结果，**不 finish turn 等 background 通知**。
- 用后台 watcher poll DB（job 状态 / agent 状态）或 pane，条件满足才回。
- 声称「继续盯」必须配一个 in-flight 信号（后台 watcher / wakeup），不靠自律空等。
- master 的 status 也会撒谎：忙时也要每 ~60s 亲自 capture-pane，别无脑重启。

## 3. 投递文本的安全铁律

**绝不用 `printf`/`echo` 双引号传待发文本**——bash 双引号里的反引号是命令替换会真执行。曾因此误启一整个 rogue ah 栈（spawn a1-a4，OOM 险）。

正确姿势：
- `Write` 工具把文本写到 scratchpad 文件 → `tmux load-buffer -b X 文件` → `tmux paste-buffer -b X -t <pane> -d` → `send-keys Enter`。
- 经 CLI 传：`ah ask a3 "$(cat 文件)"`——`$(cat)` 的输出作为参数不会被二次求值，文件里的反引号是字面量，安全。

## 4. 派新任务前先重置上下文

master 攒 token 到某量级（如 150k），派**新**任务前先对它 `/clear`，否则旧任务上下文污染新任务判断 + 白烧 token。重置后重新 brief（含角色、落点、纪律、为什么急）。

## 5. claude 周池是共享的——池紧时的省法

operator（我）、master、a4 **共用同一账号的 claude 周池**；a1/a2 是 codex、a3 是 antigravity，各自独立池。池见底时：
- 审阅只走 a2（codex），a4（claude）二审留到发版前一次性跑。
- 别让 master 开 claude Explore/Task 子代理做 recon——给精确 file:line，让 codex worker 自己在码里找。
- 独立、可并行的工作流**直接 `ah ask <worker>`** 绕开 master，省 master 的 claude 编排开销。
- 我自己也少来回、批量做。

## 6. 共享 git 工作树约束决定并行度

沙箱只隔离 config/home，**不隔离 git 工作树**：master + 所有 worker 的 cwd 是同一棵仓库。
- 两个 worker **不能同时开分支/commit**（即使改的文件不重叠也互踩）→ 需要 git 落盘的任务串行。
- **纯 markdown 设计/调研可与 git-active 工作并行**（a3 写研究文档 ∥ a1/a2 跑 T1/T2 的代码分支，互不干扰）。
- 据此排布：能并行的（不同池 + 不碰 git）尽量并行，碰 git 的串行。

## 7. 工程纪律（转达给 worker 时要钉死）

- **串行 cargo 防 OOM**：VPS 上并行多个 cargo 会 OOM 杀主控 + 崩 ahd。`CARGO_BUILD_JOBS=1` + 测试 `--test-threads=1`。**但 CI 默认并行 `cargo test --all-targets` 绿才是并发安全真验收**（串行会掩盖全局 static 竞态 bug）。
- **红绿 TDD**：先写失败测试再实现，按层分严格度（纯逻辑/集成/e2e）。
- **baseline 对照抓真回归**：别放过「红灯说无关」，`git stash` 对照 main 单跑，证明是既有还是本分支引入。
- **删/改公共符号跑全量**：`cargo test --no-run` 至少确认集成测试全量编译，别只 `--lib`。

## 8. 直连 live 栈的通道（operator 侧）

operator 的 `ah` CLI 默认指向 `~/.local/state/ah/default`，**连不到 live daemon**。要连 live 栈：
```
AH_STATE_DIR=/home/sevenx/.local/state/ah/<hash> ah ps      # 看拓扑
AH_STATE_DIR=... ah ask <agent_id> "$(cat brief)"           # 直接派 worker
```
- 指挥 **master** 用 `ah tell master "<text>"`（1.3.0+ 特性，异步不阻塞）——**但要 live 栈的二进制 ≥1.3.0**；若 running 栈是旧二进制（`ah tell` 无法识别），退回 tmux paste 指挥 master。
- **dogfood 自测 ah 时反而要 `env -u AH_STATE_DIR`**，否则外层栈的 AH_STATE_DIR 泄漏进被测进程致假失败。

## 9. 目标闭环自驱，只在方向层 escalate

用户只在**需求/目标层**参与。实施全链路（research→design→impl→e2e→goal verify→不闭合再回 research）由 operator + master + agents 自驱跑完。
- 抓到的 blocker bug：第一动作是修（派 a1），不包装成「放哪个 PR / 要不要修」抛给用户等拍板；commit 落点是工程细节自定。
- 只有**真的产品方向选择**才 escalate 给用户。
- 阶段性任务完成时**停下作三段报告**（现状/根因/下一步），让用户决下一步方向——这不算违反「不准停下来问」。

## 10. 报告风格

说人话、不夹术语、不堆表格。三段结构（现状 / 根因 / 下一步）。针对具体问题第一句直接给答案，再展开。
