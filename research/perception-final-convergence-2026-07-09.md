# 感知两模块最终收敛 — 发散(a3) × Deep Research 裁决(operator)

日期:2026-07-09。本文是两个低置信定制模块(感知仲裁器 ~70%、进程敏感完成判定 ~50%)的发散→研究→收敛终稿,作为「感知+控制面统一设计轮」的钉死输入。

材料:
- 发散方:`research/perception-divergence-antigravity-2026-07-09.md`(a3-antigravity,双盲)
- 裁决方:deep-research 报告(107 agent,25 claims 三票对抗验证 → 23 confirmed / 2 refuted,全一手来源;原文在 session task w3voj6dn8 输出,关键结论已内嵌本文)
- 前情:`research/architecture-assessment-converged-2026-07-09.md` §三、`research/perception-layer-first-principles.md`(T0-T3/R1-R4)

---

## 一、难题一(多信号源仲裁)裁决

### 1.1 采纳:单写仲裁 FSM(a3 候选一)——升级为高置信
a3 推荐的「监视器降级为只读信号生产者 + 单一仲裁者独占写状态」与研究结论完全对齐:

- **K8s conditions 即此形态**(3-0 confirmed):每个信号源是独立观察通道(不是状态机),聚合成权威判决是**消费侧单一 controller 的职责**——"Without further knowledge of the conditions, it is not possible to compute a generic summary"。
- **重要修正(refuted 0-3)**:K8s **并不在 API 层强制** single-writer(/status 子资源是惯例不是强制)。所以「单写」不是行业免费午餐,是**我们必须在代码里自己强制的纪律**——设计轮必须把"状态变更只有一条入口函数"做成编译期/审计可查的硬约束,而不是约定。
- a3 的 SQLite `perception_events` 折衷(monitor 只 INSERT 事件,orchestrator tick 串行消费 + 单次 CAS 落状态)采纳为实现形态:避免引入 actor 框架,天然获得**事件回放审计**(a3 理由 3)。

### 1.2 采纳:分级定责,不投票(否决 a3 候选三)
研究最强结论:**成熟系统不在异构信号间投票**。

- K8s 探针是"一信号一固定后果"(liveness→重启、readiness→摘流量),互不融合(3-0)。
- 表决文献自己也这么说(3-0×3):字级 voter 无多数时**报错而非输出**;DMR 两通道不一致必须**上抛监督者**,不许就地裁决;hybrid voter 把源健康度并入判定——文献对异构可靠性信号的答案就是"分级+冲突上抛",不是加权平均。
- a3 候选三(500ms 滑窗加权投票)**否决**:高权重信号被调度延迟挤出窗口即误判(a3 自己列的失效模式),且无生产级先例(hybrid voter 是论文不是部署)。

落到 ah:T0(OS)/T1(hook)/T2(transcript)/T3(pane) 固定优先级,每级绑定固定后果;**级间冲突 = 响亮告警 + 上抛(人/master 语义兜底),仲裁器绝不自动用低级信号改判高级信号**。

### 1.3 采纳:三态 + 缺席=Unknown + 异常真极性(响亮降级的词汇表)
- K8s 约定强制三态 True/False/**Unknown**,**信号缺席必须解读为 Unknown**(不是 False、不是忽略),首次 reconcile 就该写出 Unknown 让消费者看见信号源存在(3-0)。
- kstatus 的 `Stalled` 条件(3-0):**"卡死"是被显式设置的异常真信号**,与 busy/done 三分,不是超时后的静默推断——这正是我们 STUCK 语义该有的形态(对照现状:STUCK 是死胡同终态 + 靠超时推断,两点都反模式)。
- `observedGeneration`(3-0)= a3 候选二的 epoch 思想的生产级形态:每条状态判决必须标注"基于第几代观测",陈旧信号自然作废。**吸收候选二的 epoch/版本化,抛弃其"三证据槽齐全才转移"的重屏障**(a3 自己承认 hook 永久丢失时活锁)。

### 1.4 采纳:高可靠信号缺席的处置 = watchdog 语义(修正 a3 §3 的一半)
- systemd watchdog(3-0×4):**keep-alive 缺席本身就是权威失败判决**(failed + SIGABRT),不是退回低级信号的理由。"进程活着"不算健康证明。
- 对 ah:hook 静默超过预算 → 显式 FAILED/告警(响亮),而不是退化到 pane 文本推断。**每类信号的 Unknown 预算是设计轮必须拍的参数**(研究开放问题 #4:K8s 对 Unknown 无限期 no-op re-check vs systemd 到时转权威失败——hook 静默/日志静默/pane 陈旧各自该用哪种,按信号类分别定)。
- **否决 a3 §3.2 主动探测**(向 PTY 注入 `echo $?` / ANSI DSR `\x1b[6n`):①违反已拍板的运行铁律(不对干活 agent 投键、pane 生命周期推断整体删除);②a3 自己列出 antigravity 的 Esc=打断推理风险,这是 fail-dangerous;③研究未给任何生产先例背书。**连带否决难题二替代机制三(同一机制)**。

### 1.5 采纳:level-based 重导出 + pane 文本的第一性排除
K8s reconcile 是电平触发(3-0):状态从**当前可重读的观测**重导出,漏事件天然可容忍——但此保证**只覆盖可重读信号**(cgroup 状态、transcript 文件、DB 行);pane 文本转瞬即逝、不可重读,**在原理上就不在容错范围内**。这给「pane 降级为 alert-only」补上了行业级第一性依据,不再只是我们的事故归纳。

---

## 二、难题二(任务真完成)裁决

### 2.1 a3 对"查 pane 子进程"的四路攻击:两路被内核原语化解,两路成立
研究确认 **cgroup v2 `cgroup.events` populated 字段**是内核权威答案(3-0×4,man7+kernel docs,verifier 在 5.15 内核 + transient scope + disown 子进程下实测复现):子树内有任何存活进程=1,**僵尸不计入**,翻转产生 inotify IN_MODIFY / poll POLLPRI,官方用例就是"子树全退后触发清理"(systemd 收割空 scope 用的正是它)。

| a3 攻击 | 裁决 |
|---|---|
| ① 双 fork 守护化逃逸进程树 | **被化解**:双 fork 逃的是父子链,逃不出 cgroup——systemd transient scope 按 cgroup 记账,populated 仍见它。这正是"进程树遍历"与"cgroup 枚举"的本质差距,a3 攻击的是前者,结论反而支持后者 |
| ④ 僵尸残留假 BUSY | **被化解**:populated 语义明确排除僵尸 |
| ② 阶段间隙(build&&test 交接的瞬时 0 子进程) | **成立但改形态**:shell/agent CLI 还活着 → populated 恒 1,不会误判 0;真正的坑在反方向(见 2.2)。若用子 cgroup 方案,间隙期确实会瞬时 populated=0 → 需 a3 提的静默确认防抖(300-500ms 持续才判) |
| ③ 常驻辅助进程(gpg-agent/rust-analyzer)假 BUSY | **成立**:它们让 populated 恒 1。解法见 2.2 的子 cgroup 划分 + 完成协议主信号(常驻进程不该阻塞完成判定,它们的存在由 GC/审计管) |

### 2.2 已知坑与结构解法(设计轮必答)
研究点名的 populated 使用坑:**agent CLI 自己还在 scope 里跑时 populated 恒为 1**——它回答的是"scope 全空没有",不是"agent 闲了但子进程还挂着没有"。研究开放问题 #2 给出结构方向,采纳为设计轮必答题:

> **把 agent CLI 放父 scope,其 spawn 的 shell/子进程放委托子 cgroup**——子 cgroup 的 populated 单独回答"有无非 agent 进程存活",零启发式、零 comm 名单。不可行时退而求其次:cgroup.procs 枚举 + 白名单(自制成分回升,置信降)。

### 2.3 完成判定的主信号:显式协议(sd_notify 先例),OS 原语做兜底
- **sd_notify 是替代 pane 推断的最强生产先例**(3-0×2):被监督者显式单报文自报状态(READY/STOPPING/WATCHDOG),管理者只认显式报告、不认输出启发式。这为已定的 R2「hook 起止双信号为主」补上行业背书。
- **必须从第一天设计归属竞态**(3-0):sd_notify 文档明示——发消息进程若在 PID 1 处理前退出,消息可能丢失且无法归属(即使 NotifyAccess=all);上游解法是 sd_notify_barrier / pidfd 归属。ah 的 hook 上报通道(hook 进程发完即退,正是文档点名的高危形态)必须带 job-cookie/文件落盘等不依赖发送者存活的归属机制——**这直接解释了我们 hook 信号偶发丢失的一类事故,不是玄学**。
- 研究开放问题 #1 仍开放:codex/gemini 系 harness 的原生完成协议语义没有 claim 存活到 confirmed 集;agy 无 Stop-hook 等价物是已知缺口(G1/G4 hook 可靠化归设计轮)。

### 2.4 a3 三替代机制裁决
| 机制 | 裁决 |
|---|---|
| ① cgroup cpu.stat/io.stat 静默度 | **降为二线辅助**:研究零验证;a3 自认纯等待态(sleep/curl/网络)CPU=0 即盲;不得做权威信号,可做告警佐证 |
| ② 物理证据屏障(mutating job 无 git diff 不放行) | **采纳为 job 级闸门**(非 agent 状态机的一部分):与既有 EVIDENCE_DENY、外部锚定验收同族。必须带 a3 自查出的两护栏:任务派发时静态标注 is_mutating;连续拦截上限(2 次 nudge 后第三次放行+上抛人工),防只读任务误标或权限不足时死循环 |
| ③ PTY DSR 主动探测 | **否决**(同 1.4:违反运行铁律 + fail-dangerous + 无先例) |

a3 复合推荐的"第一级 UI 稳定提示符"**否决**:pane 文本永不造状态是已拍板铁律(1.5 给了第一性依据);UI 层只保留 alert-only。

### 2.5 tcgetpgrp:承认存在,列为二线
2-1 medium×2:语义真实(无前台组时返回哨兵值而非报错,须探测 pgid 存活;挂断/跨 PID ns 可能返回 0),但**部署约束硬**:外部 daemon 打不开 pane pty(ENOTTY),TIOCGPGRP 只在 pty master 上通用而 master 在 tmux 手里。严格弱于 cgroup populated,设计轮不作依赖,仅记录。

---

## 三、置信度结论(回答"多少是既有最佳实践")

| 模块 | 收敛前 | 收敛后 | 剩余定制成分 |
|---|---|---|---|
| 感知仲裁器 | ~70% | **~90%** | 仅剩:各信号类的 Unknown 预算参数、事件表 schema。骨架(单写仲裁+分级定责+三态缺席=Unknown+Stalled 异常真+epoch 版本化+watchdog 缺席即判决)全部有大规模生产先例背书 |
| 任务真完成 | ~50% | **~85%** | 仅剩:父/子 cgroup 委托布局的落地验证(研究开放问题 #2,需 PoC)、agy 完成协议缺口(G1/G4)。主结构(显式协议为主 + populated 内核兜底 + 证据闸门)全部有先例;自制启发式(查进程树/comm 名单/CPU 静默)全部退出权威路径 |

**两条 refuted 红线**(设计轮不得引用):K8s 探针缺席的乐观/悲观非对称默认值(0-3);K8s API 层强制 /status 单写(0-3)——单写靠我们自己的代码纪律,不是平台赠品。

## 四、移交
本文 + a3 发散报告 + 一手来源清单(K8s probes/api-conventions、kpt/kstatus、sd_notify/watchdog man、cgroups(7)/cgroup-v2 kernel docs、TMR/DMR 文献)一并作为「感知+控制面统一设计轮」的输入;设计轮执笔按执笔权铁律归严谨 agent,a3 保留辩论席位。设计轮必答四题:①单写入口的硬约束形态;②各信号类 Unknown 预算;③父/子 cgroup 委托布局 PoC;④hook 上报的归属竞态机制。
