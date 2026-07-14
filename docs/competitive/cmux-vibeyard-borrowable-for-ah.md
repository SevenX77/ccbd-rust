# cmux / vibeyard 可借鉴清单 —— 给 ah PM

> 调研日期:2026-06-13 · 调研人:ccbd-rust master PM · 状态:实证(已读两库源码/文档,非凭印象)
>
> **一句话目的**:从两个流行的"AI 编程 agent 编排工具"里,挑出对 **ah(Agent Hypervisor)当前阶段** 真正有用、能直接落地的设计,逐条标注对应 ah 哪块工作 + 建议 + 优先级 + 证据强度。读完能直接排进 backlog。

---

## 0. 两个库是什么(一句话)

| 库 | 是什么 | 体量 | 协议 | 跟 ah 的关系 |
|---|---|---|---|---|
| **cmux** (`manaflow-ai/cmux`) | 为 AI 编程 agent 打造的**原生 macOS 终端**(Swift/AppKit + libghostty,非 Electron) | 21.8k★ · 很活跃 · 商业化 | **GPL-3.0**(双授权,⚠️见 §1) | 同赛道的"人机交互/通知"前端 |
| **vibeyard** (`elirantutia/vibeyard`) | 为 AI 编程 agent 打造的**桌面 IDE/仪表盘**(Electron + TS) | 1.1k★ · 活跃 | **MIT**(宽松) | 同赛道的"产品化工作台" |

**它们跟 ah 是不同层次,不是直接竞品**:这俩是**面向人**的前端(GUI / 终端);ah 是**无头(headless)的编排后端**,由 master PM 驱动。ah 完全可以是这类前端底下的后端。

---

## 1. ⚠️ 先看授权风险(PM 决策前必读)

借鉴分两种,法律后果完全不同:

- **借"想法/机制/设计范式"**(看人家怎么解决问题,自己重新实现)——**两个库都随便借,零风险**。
- **直接拷源码进 ah**:
  - **vibeyard 是 MIT**:可以拷代码,基本无约束(保留版权声明即可)。
  - **cmux 是 GPL-3.0(传染性 copyleft)**:**严禁把它的源码拷进 ah**。一旦拷,ah 整个就被"传染"成必须开源 GPL。**只能读它学思路,不能抄它的代码。**

> **给 PM 的硬规则**:下面所有来自 cmux 的条目,落地方式一律是"**理解机制 → ah 自己重新实现**",不是"复制粘贴"。来自 vibeyard 的可以更直接参考代码。

---

## 2. ah 的护城河是什么(决定"不要照抄什么")

这两个库**都没有自己造一条"跨厂商 agent 消息总线"**。它们的"多 agent"只有两种:
1. 一堆独立会话并排放(只解决"人怎么看/怎么管");
2. 借底层 CLI 自带的能力(如 Claude Code 原生子 agent / teammate 模式)——**还是同一家、同一个脑子、临时的、互相不主动说话**。

而 ah 的 `ask a1/a2/a3` 是一条**自己造的、跨厂商的双向消息总线**:能让 codex / claude / antigravity 这些**不同公司的真 agent** 互相发消息、互审、辩论收敛。**这是两个库结构上都做不到的,是 ah 的核心差异化。**

**结论给 PM**:借鉴只补"人机交互/通知/资源管理"这些 ah 的短板,**绝不要反过来把 ah 的跨 agent 协作总线换成人家那种"单进程子 agent"**。别被人家功能名("Team")唬住。

---

## 3. 可借鉴清单(按对 ah 当前阶段的价值排序)

### 🟢 借鉴点 1:Hook 推送式"完成/求关注"信号(确定性主动喊话)

- **来源**:cmux(`docs/agent-hooks.md`、`docs/notifications.md`、CLI `cmux notify`)
- **机制(大白话)**:agent 主动喊话**不靠大模型自己"想起来喊"**(那不可靠),而是靠 **hook(钩子)**——CLI 运行框架里预埋的触发器。cmux 用 `cmux hooks setup` 把触发器写进每个 agent 的配置文件(codex→`~/.codex/hooks.json`、gemini→`~/.gemini/settings.json`、claude→包装注入),在**确定的生命周期事件**上自动跑一条命令把信号推出来:
  - `Stop`(这一轮答完)→ 标记 idle
  - `PreToolUse`(要调工具)→ 标记 running
  - `PermissionRequest` / `Notification`(要权限/要拍板)→ 标记 needsInput
- **为什么"确定性"**:决定喊不喊的是**程序代码(事件处理器)**,不是大模型的判断。框架在 `Stop` 事件上**必然**触发 hook,就像按开关灯必亮。
- **对应 ah 当前工作**:ah 现在的完成检测是**"拉"(pull)**——主动读 agent 写的日志找 `task_complete`(codex)/ `stop_reason`(claude)(见 `project_ah_completion_v2_log_signal_verified`)。cmux 是**"推"(push)**——hook 在事件发生瞬间主动推。
- **建议**:在 ah 现有"读日志"通道之外,**加一条"hook 推送"通道**作为补强/备选。推比拉更准、更省(不用一直 tail 日志、不用猜时机、能避开你们踩过的"claude 跨 tick armed guard 失效"那类拉模式固有坑)。这套 hook 是**跨厂商通用**的(cmux 靠它支持 15 个 agent),ah 不用造轮子,接各家现成 hook 即可。
- **优先级**:🟡 中高(完成检测已可用,这是"更稳的第二条腿")
- **证据强度**:High(已读 cmux 官方文档的集成表 + hook shape)
- **✅ 已核实:三个 provider 全覆盖**(2026-06-13 在本机 `agy v1.0.7` 二进制 + 配置目录实证):
  - **codex / claude**:cmux 已直接支持(`~/.codex/hooks.json`、claude 包装注入),hook 事件齐全。
  - **antigravity(agy,ah 目标 provider,见 `project_gemini_deprecated_antigravity_target`)**:cmux 的 15 个 agent 列表里虽没列它,但 **antigravity 自带完整 hook 引擎**——`agy` 二进制里有 `jsonhook.JSONHookSpec`、`PreInvocationHook` / `PostInvocationHook` / `StopHook` / `PreToolHookArgs` / `PostToolHookArgs` / `HookSystemMessage` / `EXECUTOR_TERMINATION_REASON_TERMINAL_CUSTOM_HOOK` 等符号,事件模型跟 Claude 同构(Pre/PostTool + Stop + Pre/PostInvocation)。配置入口在 `~/.gemini/antigravity-cli/settings.json`(默认无 `hooks` 键,需 opt-in 加),另可 `agy plugin import` 从 claude/gemini 导入带 hook 的插件。
  - **结论**:hook 推送式完成信号**对 ah 当前全部三个 provider 都适用**,不是只对 codex/claude。这条的适用面比 cmux 文档表面看起来更广,落地无 provider 短板。

---

### 🟢 借鉴点 2:"Agent 休眠"的安全杀进程判定逻辑(直击 OOM 决策点)

- **来源**:cmux(`docs/agent-hooks.md` → Agent Hibernation 章节)
- **机制(大白话)**:cmux 把**闲置的后台 agent 进程直接 `SIGTERM` 杀掉**省内存/CPU,等你切回那个标签页,再用各家**原生 resume 命令**把会话恢复回来。关键是它"**该不该杀**"的多重门控写得很克制:
  > 只在**全部满足**时才杀:① 有可恢复的 session ② agent 处于 idle(不是在干活也不是在等输入)③ 在后台不可见 ④ 活着的 agent 数 **超过上限**(默认 12)⑤ 已静默够久(默认 5s)⑥ 杀之前还有 **~60s 确认窗口**,期间一有新输出/新动静/PID 变化就**取消杀**。
  >
  > 杀法:对该 workspace 的**进程组**发 `SIGTERM`(精确范围,不误伤);恢复:跑各家原生 resume + 保存的 session id。
- **对应 ah 当前工作**:这正好打在 ah **唯一未决的设计点** 上——`project_ah_product_delivery_phase` 里 "master-OOM vs 反孤儿级联杀"(待 a2 设计)+ Step3 并发峰值 OOM smoke 未跑。cmux 这套"**只杀最老的、可恢复的、确认没动静的**"是现成的**安全杀范式**,可直接喂给 a2 当设计输入。
- **建议**:把这套"多重门控 + 60s 确认窗口防误杀 + 进程组精确 SIGTERM + 原生 resume 恢复"作为 a2 设计 OOM 自愈策略的**参考范式**。尤其"确认窗口"思想能避免你们最怕的"误杀一个其实在 long-thinking 的 agent"。
- **优先级**:🔴 高(对应当前阶段的真决策点)
- **证据强度**:High(已读官方文档完整判定逻辑 + 默认参数)

---

### 🟡 借鉴点 3:Session 恢复的"会话↔窗口"映射文件 + launch 命令脱敏

- **来源**:cmux(`docs/agent-hooks.md` → session restore)
- **机制(大白话)**:cmux 的 hook 把每个会话记进 `~/.cmuxterm/<agent>-hook-sessions.json`——存:agent session id、workspace id、surface id、cwd、PID、生命周期状态、**一条脱敏后的启动命令**。重启 app 时按这张表用各家原生 resume 命令逐个恢复。**脱敏**很关键:保留 model/sandbox/config/cwd 相关 flag,**丢掉 prompt、凭据、旧 session selector**,这样恢复的是"续上会话"而不是"重开新任务"或泄漏密钥。
- **对应 ah 当前工作**:ah Step2 已做 "resume 续断点"。cmux 这张映射表 + **脱敏规则**是一个具体可对照的落地范式,尤其"恢复时只续不重开、不泄密"这条值得对照 ah 现有实现查漏。
- **建议**:对照 ah 的 resume 实现,检查是否也做了"启动命令脱敏 + 只续不重开"的保护;若没有,补上。
- **优先级**:🟡 中
- **证据强度**:High(官方文档明确写了存什么 + 脱敏什么)

---

### 🟡 借鉴点 4:结构化"求拍板"信号(对应 PM escalation / 决策点)

- **来源**:cmux 的 **Feed**(`docs/feed.md`,Vibe Island 风格的侧边栏内联审批);各家 hook 的 `PermissionRequest` 桥
- **机制(大白话)**:agent 要权限/要你拍板时,通过 hook 把一个**结构化的"审批请求"**(带 workspace/surface/标题/正文)推出来,人在侧边栏点"批准/否决"就回去了,不用切进那个终端翻。
- **对应 ah 当前工作**:ah 是无头后端,PM 是 master Claude。但"agent 把决策点变成一个**结构化信号主动推给 PM**"这个思路,正好对应你们 SOP 里的 escalation / 决策点。现在 PM 靠 60s 轮询 + capture-pane 才发现"agent 卡在等输入";如果 agent 能在 `PermissionRequest` 事件上主动推一个结构化信号,PM 就不用一直 poll。
- **建议**:把"决策点 = agent 主动推结构化信号"纳入 ah 的 escalation 通道设计(跟借鉴点 1 同一套 hook 机制,顺带做)。
- **优先级**:🟡 中(跟点 1 共用机制,边际成本低)
- **证据强度**:Medium(读了 Feed 文档摘要,未读实现细节)

---

### ⚪ 借鉴点 5:每会话的成本/上下文/工具调用可观测性(PM 监督升级)

- **来源**:vibeyard(README + `CLAUDE.md`:cost & context tracking、session inspector、AI Readiness Score)· **MIT,可直接参考代码**
- **机制(大白话)**:vibeyard 给每个会话实时显示花了多少钱、用了多少 token、上下文窗口还剩多少、各工具调用了几次,做成一个"会话检查器";还有个"AI Readiness Score"扫项目缺什么配置。
- **对应 ah 当前工作**:PM 现在监督 agent 靠肉眼 capture-pane(见多条 memory)。把"每会话成本/上下文/工具调用"做成结构化可观测信号,比肉眼看 pane 强,也能喂给 PM 做"agent 是否健康/是否该 /clear"的判断。
- **建议**:作为 ah 后续可观测性增强的参考(非当前阶段重点)。vibeyard 是 MIT,可直接读它的实现。
- **优先级**:⚪ 低(锦上添花,非当前阶段)
- **证据强度**:Medium(读了 README/CLAUDE.md 描述,未深入实现)

---

## 4. 明确"不要借鉴 / 不适用"的(防止误导)

| 项 | 为什么不借 |
|---|---|
| **GUI 仪表盘 / 看板 / swarm 网格**(vibeyard) | ah 是无头后端,不同层次。这些是前端长处,不是 ah 要补的短板。 |
| **Claude 子 agent "团队"**(vibeyard)/ **claude-teams**(cmux) | 本质是借 Claude **单厂商原生**子 agent,不能跨厂商、不能互相主动喊话。ah 已有**真·跨厂商协作总线**,这是降级不是升级。 |
| **cmux 的浏览器/SSH/cookie 导入** | 面向人的交互功能,跟 ah 的编排后端定位无关。 |
| **直接拷 cmux 源码** | GPL-3.0 传染,见 §1。只能学思路。 |

---

## 5. 给 PM 的落地建议(排序)

1. **🔴 立刻可用**:把 §3 借鉴点 2(Agent Hibernation 安全杀判定)作为输入,喂给 a2 设计 "master-OOM vs 反孤儿级联杀" —— 这是 ah 当前**唯一未决设计点**,人家有现成安全范式。
2. **🟡 排进近期**:借鉴点 1 + 4(hook 推送式完成/求拍板信号)合并成一个"hook 信号通道"调研项,补强现有"读日志"完成检测,顺带做 escalation 推送。**三个 provider 的 hook 系统已确认齐备**(codex/claude/antigravity 全有,见借鉴点 1),无 provider 短板,可直接进调研。
3. **🟡 查漏**:借鉴点 3 对照 ah 现有 resume 实现,补"脱敏 + 只续不重开"。
4. **⚪ 备选**:借鉴点 5 留作可观测性增强 backlog。

---

## 附录:源码/文档位置索引(可复查)

- cmux hook 机制:`manaflow-ai/cmux` → `docs/agent-hooks.md`(集成表 + 各 agent 配置文件路径 + Hibernation 判定)、`docs/notifications.md`(`cmux notify` 用法 + hook JSON shape)、`docs/feed.md`(内联审批)
- cmux 授权:`manaflow-ai/cmux` → `LICENSE`(GPL-3.0-or-later + 商业双授权)
- vibeyard 架构 + Profile 隔离:`elirantutia/vibeyard` → `CLAUDE.md`(三进程架构 + `CLAUDE_CONFIG_DIR` profile 隔离 + macOS Keychain 串号坑)
- vibeyard "团队/子 agent" 真相:`src/renderer/components/team/agent-markdown.ts`(写 `~/.<cli>/agents/<slug>.md`)、`src/shared/team-config.ts`、`src/renderer/state/team-state.ts`
- vibeyard 授权:`LICENSE`(MIT)

> 补充背景(对 ah 设计的旁证):vibeyard 独立地也走了 `CLAUDE_CONFIG_DIR` env-var 隔离(跟 ah 从 bwrap 转向 env-var 隔离同路,见 `project_ah_isolated_orchestrator_pivot`),且记录了 **macOS 上旧版 Claude(≤2.1.19)共用一个 Keychain 凭据条目导致跨 config 目录串号** 的坑——ah 若将来上 macOS 隔离,这是现成前车之鉴。
