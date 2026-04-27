# Home-sevenx 2026-04-24 Session 提炼

> Session ID: `1d514898-da1f-411f-ab86-892ef1520fb4` | Project: `/home/sevenx`
> 跨度: 00:24 → 16:27 (≈16h)
> 时间锚点按 markdown 转录中的 `[HH:MM]` 标注。

---

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- **[00:35]** Claude 的 awk 正则错把 `tmux .* -S` 多塞了一个空格,导致清理脚本输入为空、`xargs -r kill` 静默 no-op,造出"清干净了"的假象。原话:"我上一版给你的 awk 正则有问题,根本没匹配到进程。"
- **[00:24-00:43]** 持续观察到 pytest 测试套件每跑一次泄漏一批 tmux server (pytest-52/57/58 累积 25+),teardown 没有 kill-server,孤儿活几小时。这是 CCB 自身测试套件 (`test/test_v2_phase2_entrypoint.py`) 的设计缺陷,`test/conftest.py` 全文 0 个 `kill-server` / `addfinalizer`。
- **[21:17 → 22:01]** Codex `--wait` 30 分钟超时,`updated_at` 完全不变。`tmux capture-pane` 看到 Codex 实际 2m39s 跑完但 CCB tracker 没捕获 reply,只在 pane 里打印结果。
- **[22:01 → 07:46]** 同一 plan review job 卡 6 小时:Codex 检测到自动升级,执行 `npm install -g @openai/codex` 后 pane 死 (status 0),CCB 仍报 `health: healthy / pid_alive=true / runtime_state=busy`,观测层与 pane 真死状态完全不一致,提交的消息进了无主队列。原话:"runtime_state: busy / health: healthy / 但 agent pane 层的真实死亡 CCB 没升级到顶层状态。"
- **[07:50]** Codex 升级后新版 v0.124.0 起来,处理排队的 plan review 时 spawn worker 触发 Rust panic `WouldBlock: Resource temporarily unavailable`(EAGAIN at `rustlib/src/rust/library/std/src/thread/functions.rs:131`),**因为整个 CCB scope 撞到 TasksMax=150 上限**。
- **[01:33]** ws transport 不稳:`Falling back from WebSockets to HTTPS transport. stream disconnected before completion: websocket closed by server`。
- **[09:53-10:04]** 发现 11h+8h 两个 `npm install -g @google/gemini-cli@0.39.0` 卡死的孤儿进程,以及一个 11h 的 `provider_backends.codex.bridge` pytest-52 残留进程。说明 CCB 启动 gemini agent 时如果触发 npm 升级 + registry 卡住,会无限 block。
- **[14:30]** 在新 session 里 `echo $CCB_PROJECT_DIR` 返回空,说明 fork PR #190 的 `~/.claude/shell/ccb.sh` export 在 Claude Code Bash tool 的非交互 non-login shell 里**根本没生效**,workaround 失效。
- **[14:35]** `command ccb ask` 在 `/home/sevenx/coding/claude_code_bridge` 目录下报 `project ccbd is unmounted`,因为 cwd-walk 找到的是 fork 仓库的 `.ccb`,需要显式 `--project /home/sevenx`。
- **[01:30-08:00]** Claude 误以为本机和 VPS 是两台机器,实际 `hostname=vultr-sever-sv` 就是 VPS。Claude 自己的 memory `user_vps_workload.md` 把"本机+VPS"信息写错。原话:"我的上下文认知错了,memory 里的'本机 + VPS'(或已经过时)。"
- **[10:43-10:46]** Claude 误推断 `--remote-control-session-name-prefix` flag 控制 RC session 名;实际 RC session 名由会话内容自动摘要生成,这个 flag 在当前实现里基本无效。原话:"我上一轮以为 `--remote-control-session-name-prefix` 是控制'session 名主体'的开关,看 flag 名字想当然推断的——推断错了。"
- **[15:43]** Claude 看到一个 3 小时的 `test-mvp` 陌生 tmux,做了完整的"横查竖比对扒进程猜归属"流程,差点把不属于自己的 scope 当孤儿清掉。

### 2. 用户纠正 / 抱怨 / 吐槽 Claude 的内容

- **[00:35]** "为什么还有"——直接质问,Claude 才发现自己的清理脚本是错的(awk bug)。
- **[21:14]** "你决定吧,我判断不了你说的,看都看不懂. 开始执行clear之前的那些任务,有问题问Gemini,不要停"——明确要求工程细节自己拍板,不再向用户提问。
- **[21:04]** "清理一下所有的spec状态,不要再发生明明已经做完了,还告诉我要做的情况"——抱怨 spec.json 状态与实际进度脱钩(已实施但仍标 `requirements-generated`)。
- **[09:51]** "我怎么不记得我加过agent4和agent5了?默认一直应该是ccb3,3个agent,现在ccb3是启动5个agent吗?还是说这5个是你拉起ccb时默认启动的?"——质问 agent 数量来源。
- **[09:51]** "btw,你一直说的pr到底是什么意思,听不懂,你说的很多术语我都听不懂,能在说的时候顺带解释一下吗??"——要求术语第一次出现就解释。
- **[15:17]** "1. /exit 重开 claude → 新 session 第一行 echo $CCB_PROJECT_DIR 验证 TD-009 。. 请告诉我这句话确切的意思,你说话太简略了,我没办法确认你的确切意思. 是我新开claude 第一行应该自己跳出来 echo...呢?还是新开claude我应该把这行代码打到Claude中呢?"——抱怨 Claude 表达过度简略产生歧义。
- **[15:47]** "不是说要主控维护一个list,哪些scope是自己拉起来的,做完任务可以主动清吗? 不需要再这么横调查竖调查,还容易删错"——批评 Claude 用 `ps`/路径猜 scope 归属的"investigative"做法。
- **[15:49]** "这个工作习惯应该要写进ccb工作的规范里面吧? 然后继续你的任务,不要停"——要求把这条纪律写进规则文件,继续执行不要停。
- **[09:53]** "你的方案都治标不治本"——批评 TasksMax 调高方案是治标。
- **[09:53]** 原话(详细列五点根治需求):"我希望能够实现的最终形态: 1.主控agent能够开启独立的scope,在独立的scope里运行ccb; 2.主控能够随时回收自己创建的独立scope; 主控创建的scope不共享主控自己的scope线程数; 4.每一个任务主题,Claude自己拉起一个ccb scope, 一个任务结束后, claude主动销毁这个scope, 清理线程,清理上下文..."
- **[08:15]** "你的方案都治标不治本,我可以先临时加到500或者1000没问题,但是我希望能够实现的最终形态..."——明确不接受 workaround 当解决方案。
- **[10:46]** "rc的session名…如果快速改不了的话就算了这个不重要"——隐含吐槽 Claude 在不重要的事上花太多时间。

### 3. 用户强意图 "我想要 X" 或 "不要 X"

- **[09:53]** "我想中的状况应该是Claude+ccb3这是最开始启动时的干净状态,之后每个任务+测试做完后都应该主动回到这个状态。必须从战略层根治"——强意图:任务结束必须主动回到干净基线,战略根治非可选。
- **[09:53]** "未来的模式,agent一个一个单独拉,不要一下子启动多个"——禁止默认启 N 个 agent,要按需启动。
- **[09:53]** "严格禁止主控Claude自己batch发起测试进程,测试必须在ccb中的codex或者Claude完成. 如果遇到大型e2e测试,单独开一个测试scope给e2e跑测试,每个e2e一个scope,测试完成就立刻回收"——铁律级要求,主控 Claude 不能自己跑测试 batch,大测试必须独立 scope。
- **[09:15]** 原话(决策细则):"以任务为边界必须refresh context, 在主动重启ccb时加上new session的启动配置,如果是ccb奔溃重启,加continue,继承context"——要求区分两种重启路径。
- **[21:14]** "有问题问Gemini,不要停"——明确不准把工程问题升级给用户,要先过 Gemini。
- **[15:49]** "继续你的任务,不要停"——铁律级,要求按规则推进。
- **[16:08-16:10]** (compact 后)"继续"——同样的不停指令。
- **[09:15]** "这一部分的重大更细,是不是可以提交给ccb库的maintainer?...策略上,我不关心maintainer用不用,不管怎么样我都会按照自己的想法在自己fork的库里实现(做好自己库的更新),如果他们能用就用,我也做点贡献,不能用,开个issue也OK"——明确 fork-first 策略,upstream 接受与否不是关键。
- **[10:18]** "kill ccb, 然后编辑一段话, 让我重启后复制给你"——要求 Claude 提前写好接力消息。

### 4. 对话中暴露的设计缺陷(结构性问题)

- **[00:24-00:43]** **CCB 测试套件根因泄漏**: `test/test_v2_phase2_entrypoint.py` 的测试用 `_run_ccb([...])` spawn 子进程,子进程拉起的独立 tmux daemon 不与测试生命周期绑定;`conftest.py` 完全没有兜底清理。每次 pytest 跑都泄漏一批,长期累积。
- **[07:50-08:00]** **CCB 观测层与实体层不一致**: pane 死了 6 小时 CCB 仍报 `health: healthy / pid_alive=true`。这是 CCB 设计文档 `ccbd-lifecycle-stability-plan.md` 第 2.1 节自己承认的"authority split"问题——CCB 把"漏进程"识别为"权限分散"问题,但**没考虑过"我作为守护进程应该有独立资源边界"**。
- **[08:31-09:53]** **CCB 完全继承 caller 的 cgroup**: `cat /proc/<keeper_pid>/cgroup` 显示 keeper 在 `claude-XXX.scope` 里。CCB 没有"我自己应该独立 scope"的设计意识。CHANGELOG 里 `Retry transient tmux fork failures` 只是遇 fork 失败重试,scope 满时重试同样会失败,治不了根。
- **[09:53]** **CCB 缺"按需启动单个 agent"的 CLI**: 必须改 `.ccb/ccb.config` 文件后 `ccb -n` 才生效,不能 `ccb start agent3`。
- **[14:23-14:30]** **`shell/ccb.sh` 的 `export CCB_PROJECT_DIR="$HOME"` 只在交互 shell 生效**: Claude Code 的 Bash tool 是非交互 non-login,根本不读 `~/.bashrc`。fork PR #190 的 workaround 在主 use case 下完全失效。
- **[15:43-15:47]** **orchestrator `cmd_cleanup` 只扫两类孤儿**: systemd-tracking 差运算两方向。漏第三类——PROJECTS_DIR 下 project_dir 存在但 tracking 和 systemd 都没登记的纯文件系统残留(本次 test-mvp 就是)。
- **[14:30]** **TASK-PLAN 与 spec.json 状态脱钩**: TD-001 `claude-config-visibility-bootstrap` Phase 1 实际 2026-04-22 已完成,但 spec.json 仍写 `phase=requirements-generated, approved=false`。文档元数据没有自动跟踪机制。
- **[16:10]** **Claude 自己缺 atexit cleanup / scope tracking 持久化**: 主控死了或 scope OOM-killed 时,自己起的 sibling scope 不会被回收(orphan,占 500 task + 4G mem,且下次 Claude 没有 tracking 文件接续)。
- **[10:46]** **Claude Code RC session 名机制不一致**: CLI flag `--remote-control-session-name-prefix` 名字暗示控制 session 名主体,实际由会话内容自动摘要覆盖。`--help` 文档没说清。
- **[01:30]** **claude-sandbox 不是真沙箱**: 名字暗示安全沙箱(bwrap/Apple Sandbox),实际只是 systemd resource 配额,无文件系统/网络隔离。命名误导。

### 5. 决策转折点

- **[00:24-00:43]** 从"pytest 跑出 25 个孤儿 tmux"问题逐步推到"这是 CCB 测试套件 conftest 没兜底",催生 P0-1 conftest 修复 (PR #189)。
- **[00:43-00:59]** 用户确认"自己的 fork,可以直接改",从此后所有改动走 fork personal 分支 + upstream PR。
- **[01:30]** Claude 触发 `Cannot fork`,挖出 `TasksMax=150` 是用户自己加的硬限制,从"诊断"转向"系统性架构问题"。
- **[08:15]** 用户拍板"5 大需求 + 必须从战略层根治",CCB scope orchestration 设计正式立项。
- **[09:15]** 用户给 7 条具体决策(任务边界 refresh、agent 按需启、e2e 独立 scope 等),scope orchestration 方案锁定;同时确认 fork-first 策略,不依赖 upstream 接收。
- **[10:18]** 用户决定 kill ccb + 落盘 + 清 context 重启,引入"接力文档"工作模式(`TASK-PLAN-2026-04-23-RESUME.md`、`TASK-PLAN-2026-04-24-handoff.md`)。
- **[14:23-14:30]** 通过 Gemini 确认 `export CCB_PROJECT_DIR` 比 `--setenv` 更稳,改 `/usr/local/bin/claude-sandbox` 走 export 路径,TD-009 收尾。
- **[15:25-15:43]** TD-009 验证通过(`echo $CCB_PROJECT_DIR=/home/sevenx`),TD-011/012 commit + push 到 fork origin/personal,P0 三项全部闭环。
- **[15:43-15:49]** Claude 试图清陌生 test-mvp tmux 时被用户当面纠正,承认 "investigative" 做法错误,产出 `~/.claude/rules/ccb-orchestration.md` "Scope 归属纪律"章节 + 永久写入 MEMORY 的 `feedback_scope_ownership_tracking_only.md`。这是最重要的纪律内化转折。
- **[15:57-16:08]** Gemini 给 janitor 补丁 3 条具体反馈(symlink 防御、5min mtime grace、错误不静默),全部接受并定型。然后 context 紧到 `/compact`,开新 turn 接力 A.impl。

---

## 核心主题

**这一天围绕一个问题展开:Claude 的工具链(claude-sandbox + CCB + 主控行为)长期假设"我和我启动的所有进程共享一个资源边界",这个假设在多 agent + 测试场景下崩溃,催生从临时止血(TasksMax 1000)到战略根治(per-task sibling scope orchestrator + 任务边界纪律)的完整重设计;过程中用户多次纠正 Claude 的"决断越权"和"investigative 越界",把"主动维护 scope 列表 + 不靠 ps 猜"和"任务边界即清理"上升为铁律。**
