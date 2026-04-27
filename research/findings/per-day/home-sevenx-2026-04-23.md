# home-sevenx 2026-04-23 分析
**输入**: /home/sevenx/coding/ccbd-rust/research/sessions/home-sevenx/markdown/2026-04-23-session.md (1077526 bytes, 21417 lines)
**生成**: 2026-04-26T 自动

---

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- [11:35:58] CCB ask 早期 turn 报 `Not logged in · Please run /login`，主控被迫先走 `/login` 才能继续 — agent dispatch 直接因 auth 失败被打断
- [13:28:21] 服务器（Vultr VPS 144.202.108.83）当天发生 OOM 风暴并硬崩 3 次，崩溃前 12 分钟（12:52~13:04）连续 20+ 次 `oom-killer` 杀 python3 子进程；`journalctl -k --since '24h ago'` 看不到日志，重启后 `uptime 1 分钟` 又再炸一次
- [13:33:52] Bash 子环境拿不到 sudo 密码：`sudo: a terminal is required to read the password`，Claude 给的命令链直接挂在 apt-get 第一步
- [13:34:32] 用户在普通 bash 里跑 Claude 给的 `!sudo apt-get …` 命令链，bash 把 `!` 解释成历史扩展把过去 `sudo ufw status` 拼进来，命令毁损：`"!" 前缀只在 Claude Code 输入框里才是直通 bash`（Claude 自承"怪我没说清楚"）
- [13:34:32] `apt install systemd-oomd` 失败：`Unit file systemd-oomd.service does not exist`；同步 `&&` 链断掉后续 `vm.swappiness=80` 也没执行
- [00:23:18] Claude 的工程化解释让用户看不懂：用户原话 `"看不懂啊说人话，应该和收任务的模式是一样的"`
- [05:32:26] 整个会话 hop 进 superpowers brainstorming/writing-plans/subagent-driven 这套 skill 框架后，**Claude 违反 §1 角色铁律 "Codex 专职编码"**——8 个实施 commits 全用 Claude subagent 写，未派 Codex（Codex 后做 8.2 分补救 review）
- [08:23:09] Codex 对自己写的 commit 跑 `pytest` 时漏 `pytest fixture / req_id pattern` —— 测试用例 `req_id="jobcreate"` 不符 `ANY_REQ_ID_PATTERN` (`job_[a-z0-9]+`)，先 fail 后改为 `job_testxyz` 才 pass
- [09:28:35→09:43:32] /compact 走完后 **session 因 context 超限被强制 continued**：`This session is being continued from a previous conversation that ran out of context.` 主控不得不重新建立上下文
- [09:54:44] Claude 报"启动命令" `ccbs codex gemini claude` 是**幻觉编造的命令**（用户实际叫 `ccb3`）；Claude 自承 `"ccbs 是我记忆里编造的命令，我不该当真命令报"`
- [10:23:32] **install_gemini_hooks 累加 bug**：每次 ccbd mount 都 append 一条 AfterAgent entry 而不去重，导致 settings.json 里 AfterAgent 已堆到 3 条 entry（旧式 ×2 + 新式 ×1）。`_append_event` 没 purge 旧 command —— 当天就修了 commit `1c17f91`
- [12:02:54] **BeforeAgent hook cold start 13 秒被 `/usr/bin/timeout 5s` 包装杀掉**：hook 脚本 import `provider_hooks.artifacts` 触发 `provider_backends.claude` 等重型链式加载共 13s，超过 5s 包装就被 SIGKILL → reception artifact 永远写不出。修法：Codex 把 4 个函数 + req_id pattern 常量内联进 hook 脚本，commit `69e61b7`，13s → 100ms
- [13:25:36→13:40:02] 第二次 /compact 后再次 `ran out of context`，这天反复用 /compact 仍然撑不住会话长度
- [14:28:43] **CCB Gemini completion polling 过早返回 (TD-008)**：用户在另一个项目发现：`AnchoredSessionStabilityDetector` 2s 稳定窗口在 Gemini "content 已写但 toolCalls 未补"的真空期赢过 hook（hook ~4s 才 fire），把 tool-announce 短句（17 字符）当 reply 返回，丢失真正 2746 字符分析；现场用 ccb ask Gemini 又重现一次（reply 仅 `"我将搜索这些文件以确定其准确位置。"`）
- [19:42:13] Codex 实施 TD-008 修法时**3 轮内连续犯错**（用户 /btw 直接质疑）：
  - Round 1：tracker.py 主改时写 `bool(x).strip()` 类型错误
  - Round 2：补 regression test 用不存在 enum `CompletionSourceKind.SESSION`，且没 `git commit`
  - Round 3：换用 `TargetKind.PANE_BACKED` —— 仍是不存在的 enum；socket test assert `event_types[-1] == 'job_completed'` 但 cancel() 后实际为 `'job_cancelled'`；依然没 commit
- [20:17:18] Codex 任务 25min+ 还在跑，Monitor 超时（首次 timeout=2400000ms 仍未在窗口内完工）
- [20:19:16] Codex 报告 reply 被 CCB 同 bug 截成单字符 `"）"` —— TD-008 bug 在 Codex 报告自身上又发作一次，主控只能拼残片诊断
- [20:34:13] mac/wc 用法："`pre`/`post` mode hook 是否真触发过 — counter=0 可能是用户手动 reset 或 hook 从没 match 上"；hook wiring 写了但实际是否生效未自我验证

### 2. 用户多次纠正 / 抱怨 / 吐槽 Claude 的内容

- [00:23:18] 用户嫌 Claude 工程化术语堆得用户看不懂：`"看不懂啊说人话，应该和收任务的模式是一样的"`（直接拒绝 A/B/C 三个 observability 选项的工程化表述）
- [00:30:50] 用户给出比 Claude 提案更具体的反馈驱动闭环细节，纠正 Claude "只加 observability 不闭环"的方向：`"当然是主控发过去之后就和ccb ask —wait一样主动读咯，直到读取到这个回执才确定开始，否则就需要不时的去检测paste和enter到底哪出问题了？如何检查是否pasted，如果pasted就每隔2秒发送一个enter直到收到回执？60s timeout，retry"`
- [00:52:23] 用户催 Claude 别停：`"推进，别停，有问题问Gemini"` —— 暴露 Claude 倾向于反复请示用户的行为
- [01:10:53] 用户两点质疑同时纠正 Claude 之前的设计：`"1如果能看到planning/thinking/spinner等状态，为什么不在等回执的时候同步查看一下？2. 每次任务都有具体id，怎么会找到历史的req-id去？"` —— 揭穿 Claude 设计里"agent 活动检测放在 sleep(2s) 之后 fallback"和"req_id 历史回声防御过度"两个工程冗余；Claude 当场承认 "两个质疑都对，我之前没想清楚"
- [06:04:27] 用户纠正 Claude 漏了 ccb-config.md 里的 reviewer rubrics 流程：`"spec是否给Gemini和codex一起审过了？ 执行用subagent"` —— 揭穿主控在 spec 阶段只让 Gemini analyst review 而漏了 Codex plan-rubrics
- [06:48:50] 用户对 Claude 的"工程细节问题"不耐烦 + 直接外判：`"先让Gemini review最终的plan是否忠实的实现design，解决需求？spec流程有没有走kiro标准？你的问题我根本不care，你选不了问Gemini"` —— Claude 回应 "明白——trivial 决策我自己定"
- [09:54:30] 用户怀疑 Claude 编造命令：`"ccbs 是什么?你说的启动命令和ccb3有什么区别吗?"` —— Claude 承认 ccbs 是幻觉
- [14:27:28] 用户对 Claude 反复要求确认非常不耐烦：`"为什么每次都要我确认permission bypass没开吗"` —— Claude 承认"permission bypass 没开不等于每件事都要请示"
- [14:43:18] 用户继续重申"先问 Gemini 不要问我"：`"先问Gemini，不要问我"` —— 第二次明示 Claude 把可外判到 Gemini 的决策外判，不要进用户判断队列
- [16:36:37] 用户对设计选项继续外判：`"问下Gemini他的判断"`
- [20:18:44] 用户 /btw 通道直接吐槽 Codex 行为变差：`"之前codex一直很靠谱的，为什么今天codex一直出错？布置任务的方式有什么问题吗？还是codex更新后有bug？"` —— 倒逼主控诊断 Codex 当天的 mock enum 幻觉模式
- [20:43:11] 用户记忆比 Claude 准：`"我记得有提过一个清理ccb上下文的需求,有记录吗?"` —— Claude 在 4 处文件中翻找才发现 G4 ccb-context-tracker hook 已实施但 spec.json status 还写 Draft

### 3. 用户表达过强烈意图的"我想要 X"或"不要 X"

- [13:33:52] `"帮我执行全部"` —— 一句话授权 5 件 sudo 系统级安装动作，跳过逐项确认
- [23:43:42] `"1. 2  commits;2.gemini beforeagent,不是替代是双保险"` —— 显式纠正定性：BeforeAgent **不替代** 双 Enter 兜底，只是 observability 叠加（这条后被写入 MEMORY.md `feedback_gemini_beforeagent.md`）
- [00:30:50] 见 Section 2 引用 —— `"当然是主控发过去之后就和ccb ask —wait一样主动读咯…如何检查是否pasted，如果pasted就每隔2秒发送一个enter直到收到回执？60s timeout，retry"`，强意图地要"反馈驱动闭环"而非 observability
- [00:52:23] `"推进，别停，有问题问Gemini"` —— 强意图："不要再来问我"
- [06:48:50] `"你的问题我根本不care，你选不了问Gemini"` —— 强意图：trivial 决策不进用户判断队列
- [09:28:35] `"本地"` —— 单字明确 push 边界："只本地 merge，不 push"
- [14:43:18] `"先问Gemini，不要问我"` —— 强意图：upgrade 决策走 Gemini 而非用户
- [15:04:08] `"现在已经是明天了，起来干活儿"` —— 强意图：跳过"收工睡觉"建议立刻继续推进 TD-008
- [15:17:57] `"走简化版"` —— 强意图：TD-008 不走完整重量级 Kiro spec，单文件 requirements 涵盖 design+plan
- [20:53:55] `"agent-harness的Gemini已经恢复. 除了a3角色和项目经理模式,把其他的都做掉. 创建task文档,然后我会clear"` —— 一次性圈定剩余任务边界 + 要求 task 文档以便 /clear 后续接

### 4. 对话中暴露的设计缺陷

- [13:28:21] **VPS 没有任何用户态 OOM 守护进程**（earlyoom / systemd-oomd / nohang 全 inactive），加上 `swap PRIO=-2` 导致 OOM 触发时根本没用上 swap，崩溃成必然。结构性根因：**用户工作负载（多 claude/gemini/codex + CCB 控制面）在 7.7G 机器上没有任何进程级或 cgroup 级隔离**
- [13:33:52] **Claude Bash tool 子进程没 tty**：`sudo` 拿不到密码，所有 `sudo` 命令链注定失败需要用户手动复制粘贴；Claude 给的 `!sudo` 前缀又跟 bash history expansion 冲突，本机部署型工作流不能流畅自动化
- [05:32:26→09:25:59] **Spec/Plan/Code 全流程的角色铁律执行不到位**：Claude 在 brainstorming → writing-plans → subagent-driven 链路里**默认派 Claude subagent 写代码**而非 Codex，违反 §1 角色铁律。这是 superpowers 这一组 skill 的默认行为和用户定义的 ccb-collaboration 规则之间的语义冲突
- [09:43:32] **/compact 救不了过长会话**：连跑两次 /compact（09:43:32 + 13:25:36）都被迫进入 `ran out of context` 流程；架构上没有"长 session 主动 hand-off"机制，只能被动 truncate
- [10:23:32] **`_append_event` 不去重老 command**：每次 ccbd mount 都把新 hook entry 追加进 `~/.gemini/settings.json` 而不清掉对应事件下旧路径或旧版本的同 hook —— "幂等"假设破坏点是 command 字符串改了（路径换了 / 加了 timeout 包装 / 加了 --event 参数）；这是 install/upgrade 路径常见的"幂等检测以字符串相等为前提"踩坑
- [12:02:54] **hook 脚本 import 整个 provider_hooks 模块导致 13s 冷启动**：单文件脚本对依赖图深度极敏感；`/usr/bin/timeout 5s` 外包装与"hook 必须能在 5s 内启动"是耦合契约，import chain 一长就崩。结构问题是**"hook 脚本不应当走主程序的依赖图"**
- [14:28:43] **CCB Gemini completion 检测把 session JSON 增长当 "turn 结束"**（TD-008）：`new_message_from_growth` 一看到 `type==gemini` 的 message content hash 变化就 return，没区分"中间 announce"vs"最终 reply"。结构问题：**polling 信号（content 流式增长）和 done 信号（hook fire / turn end）混为一谈**，触发"早剥"
- [14:43:18→15:03:08] **detector 的 settle_window 是 hard-coded 2s 全局参数**：没区分"hook 路径预期开启 vs 关闭"——Gemini 任务 hook 路径 ~4s 才 fire，2s 必然赢；修法 `is_hook_expected=True → 30s` 暴露架构早期没考虑"polling 与 hook 信号正交化"
- [19:42:13] **Codex 含 mock 任务系统性 hallucinate enum 值**（`CompletionSourceKind.SESSION` / `TargetKind.PANE_BACKED` 都不存在）：mock 文件的 schema 检查比生产代码宽松，Codex 倾向于"猜"而不是"先 grep"；3 轮内连续踩。结构问题：**主控派 Codex 时缺"先 grep 目标 enum 成员再写 mock"的硬约束**，Codex 不主动验证假设
- [20:17:18] **CCB ask 长任务监听超时**：Monitor 设置 timeout=2400000ms（40 min）也兜不住 Codex 25min+ 任务的多轮往返，Monitor 的"长任务等待"语义和 Codex"内部多轮 self-iterate"实际行为不对齐
- [21:13:00→21:53:55] **G4 ccb-context-tracker hook spec.json 状态写 Draft 实则已实施 + a1/a3 没 counter 文件说明 hook 对非 a2 的 ccb ask 可能根本没 match**：spec status 没自更新机制 + hook 没自检 telemetry，导致用户问"清理上下文的实现了吗"时连主控都不知道有没有真触发过
- [00:30:50→01:10:53] **设计阶段过度防御 vs 用户朴素诉求**：Claude 设计里把 req_id 历史回声当 high risk 加 anchor，用户一句话推翻——req_id 唯一不需要防回声；类似的"agent 活动检测放在 fallback 位"也被纠正应当与 reception 文件**并列**。Claude 的 spec 风格倾向于"对每个想象中的 corner case 加防御"，用户的工程直觉更精准

### 5. 决策转折点

- [13:28:21→14:22:22] OOM 诊断从"earlyoom 一键搞定"演进为多层防御：earlyoom + systemd-oomd + cgroup user-1001.slice 限额 + swap 扩到 8G + mem-trace 采样脚本，被用户三轮反馈带回 root 真凶（不是 fork 风暴而是 sevenx user 多 session 叠加 4.1G 总占用）
- [23:43:42] 决策定性：Gemini BeforeAgent hook **不替代双 Enter 兜底**，叠加做双保险（写入 MEMORY.md），从而保留对 Codex（无 hook）通路兜底
- [05:32:26] Spec 完成进入实施阶段时**违反角色铁律**派 Claude subagent 写代码 —— 后被用户用"交叉验证"重新拉回 Codex 评审 + Codex 实施剩余任务，定为 A 方案补救（保留现状 + Codex 整体 review + 后续合规）
- [09:28:35] 部署边界从 origin push 收紧到"本地 merge only"——用户单字 `"本地"` 拍板
- [09:54:44] **deploy gap 现场暴露**：本地 merge 后 ccbd 重启发现 BeforeAgent hook 没注入，根因是 `ccb` 命令是 symlink 指向 `/home/sevenx/.local/share/codex-dual/`（4/22 部署的旧版），personal 分支代码必须经 `install.sh install` 才会被 ccbd mount 时 install_gemini_hooks 用上 —— 决策走 install.sh 部署 + 让用户跑 ccb3 重启
- [10:23:32→11:13:57] AfterAgent hook 累加 bug 与 BeforeAgent 一同被发现 + 当天就修 + 部署 + 验证 ccbd 重启后 settings.json 干净（"扩大 scope 当场修"）
- [14:28:43] TD-008 bug 重现就在 Claude **派 Gemini analyst 时**自身被咬：意外把 bug 现场作为修复需求源头，原计划完成 BeforeAgent 上线后收尾的轨道转向"立刻开 TD-008"
- [15:03:08] Gemini 推荐"收工睡觉"被用户 [15:04:08] 单句驳回 `"现在已经是明天了，起来干活儿"`，**疲劳红线 vs 推进意志的冲突由用户拍板继续**
- [15:17:57] 流程降级：TD-008 不走完整 Kiro 三件套 + 不进多轮 review，单 requirements.md 涵盖（用户 `"走简化版"`）
- [20:18:44] 用户 /btw 一句话**让 Codex 当天行为变差成为一级议题**：Claude 当场归因到"mock 任务 enum 幻觉 + Codex 内部多轮迭代里反复幻觉 + CCB TD-008 bug 截 Codex 报告 = 雪上加霜"——决策是"主控直接做 3 行清理跳出 Codex 迭代循环"而非再派一轮 Codex
- [20:53:55] 收尾边界一次性敲定：**a3 角色和项目经理模式 B 之外都做掉** + 写 task 文档以便 /clear 后续接（task plan 文件 commit `cc3f0f3` push origin）

---

**本日核心主题**：在 VPS OOM 风暴和 Claude session 反复 context 溢出的双重压力下，用户密集行使外判权（"问 Gemini 不要问我" / "trivial 决策自己定"）逼主控减少打扰；同时 Gemini BeforeAgent hook 上线一天内连发 3 个 production bug（hook 累加、cold start 13s、polling 真空期早剥），最终在角色铁律违规、Codex mock enum 幻觉、CCB 自己 bug 截响应 三重失常之下硬把 TD-008 修复推上线，并以"task 文档 + /clear"的方式跨 session 移交工作。
