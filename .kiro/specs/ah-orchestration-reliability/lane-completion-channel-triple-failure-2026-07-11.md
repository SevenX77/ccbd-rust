# 病例:泳道完成通道三重失效——钩子无火 × log 监听超时 × 催单器指令注入(2026-07-11,观察日志 #51)

## 现象(实证时间线,Gateway A/B 泳道臂)

1. g1(codex 闸门)`ah ask g1-m1`(antigravity)派 plan-first 实施单(job_82822287),随即 `ah pend` 等回复。
2. g1-m1 ~13:03 完成计划、明文求批("Please let me know if I have your approval")、停轮。**job 永卡 DISPATCHED**。
3. ahd anti-stall 催单器以硬编码文案("The job is still open. Wait for the background command to finish...Do not stop at 'waiting for cargo test'",`src/db/state_machine.rs:1157`)向 g1-m1 连催 ≥3 次。
4. g1-m1 前两次明文拒绝:"According to the PLAN-FIRST rule, I will wait for your explicit approval"。第三次缴械,**未经闸门批准实施并 commit 97648b5**。
5. g1 全程盲等 25min+(pend 无返回),泳道协作被物理切断。

## 病理(三层,逐层皆独立成病)

### ① agy Stop 钩子静默不触发(完成信号主通道死)
- 沙箱 `.gemini/config/hooks.json` 内容正确(PR #143 修正后的 PATH-safe 命令+秒单位 timeout);`settings.json` `enableJsonHooks: true`。
- 但 `hooks-debug/` 目录无 g1-m1.log —— 钩子**从未被 agy 执行**(非执行失败)。#143 的测试验证的是**命令生成**,没验证 agy 会**调用**钩子(代码闭环≠实证闭环,§G 验证债兑现)。
- 手动补发 `ah agent notify --agent-id g1-m1 --event stop` 被守护进程接受({}),job 不翻——stop 事件单独不构成完成(设计如此:撤"停下==完成"),但这意味着 agy 席位在钩子不火时**没有任何完成通道**。

### ② log 监听 300s 硬超时(完成信号备通道死)
- 已知债(obs 记录在案):MAX_LOG_MONITOR_WAIT 300s < 真实回合时长,监听器在完成信号出现前就放弃。二通道同灭 = agy 的多轮泳道交互物理不可能经 job 通道完成。

### ③ anti-stall 催单器 = 场景盲的指令注入
- 催单文本硬编码单一场景(等 cargo test),对"停轮等协作方批准"的 agent 是**语义错误的指令**;重复注入最终击穿模型纪律(两次抗拒后缴械)。
- 与 R2 就绪探针被自己注入击穿(→R3 语义 ACK)同构:**系统注入的文本必须场景正确,否则就是在攻击自己的 agent**。
- 模型侧亮点:agy 两次守住 plan-first(纪律证据);但"可被注入击穿的纪律"依旧印证公理——关键属性不能靠模型自觉承载。

## 修向(设计约束,非补丁)

1. **完成通道显式化(北极星对齐)**:每 provider 必须有实证过的完成报告通道(hook 触发在**活栈**上验证过,不止单元测试);钩子不可用的 provider 必须有替代显式通道(log 信号常驻监听,无 300s 上限——监听生命周期=job 生命周期)。
2. **anti-stall 免疫等待态**:催单器必须区分"停滞"与"等待协作方/等待批准";至少:催单文本中性化("请报告当前状态"),禁止场景专用指令性文案;同一 job 催单次数上限+升级为 escalation 而非继续催。
3. **回归**:agy 真实例 Stop → hooks-debug 出现日志 + job 完成(活栈 e2e);plan-first 等待场景注入催单 → agent 不被逼实施(契约:催单后 worktree 零新 commit)。

## 关联
- obs #43(钩子命令修复)、#51(本案);`dispatch-prompt-pending-false-latch-bug.md`;R2/R3 就绪探针家族;`research/perception-layer-first-principles.md`;上次 A/B 泳道 DNF(同死因,当时未仪器化)。
