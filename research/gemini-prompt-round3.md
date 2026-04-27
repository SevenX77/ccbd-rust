请用中文回答。

# 任务 Round 3：重写 by-gemini.md（A 类 #1 补料）

延续 Round 2。你 Round 2 的 verdict 把 by-gemini.md 重写定级为 A 类必补——现在专门执行这件事，**不做其他评估**。

## 任务

读取下面两个目录里所有 session markdown 文件（按天的 master Claude session 数据）：
- `research/sessions/home-sevenx/markdown/`（主控 master，18 天）
- `research/sessions/agent-harness/markdown/`（同期 agent-harness master）

通读后，按下面 8 个 skeleton 章节填充实质内容，**用 Write tool 直接覆盖** `research/findings/session-analysis-2026-04-26-by-gemini.md`。

## 章节（已存在的 skeleton 结构）

1. **CCB 系统层 bug 清单**：技术细节级别的 bug（mailbox state machine race / detector misjudgment / paste failure / completion 漏报 / orphan 进程 / etc.），每条 finding 必须带 `research/sessions/<project>/markdown/<file>.md:<line>` 引用回原文
2. **Master Claude / agent 行为缺陷清单**：master 反复犯的同类错（停下来问、摘要喂 Gemini、单样本下结论、表演性参与、装聋作哑），每条带具体例子和引用
3. **用户反复纠正 Claude 的指令**（高优先级）：用户怒斥的原话 + 后续是否被吸收成 CLAUDE.md 铁律
4. **触底信号 / 重构决策的演进**：哪些事件促成了"必须 Rust 重写"的决定（比如 Phase 1 多次补丁、Gemini 35/100 评分、4-26 .bashrc 事故 etc.）
5. **L3 spec pipeline 必须强制的卡点**：哪些 master 行为应该被 L3 在 contract 层强制约束（不依赖 agent "尽量"）
6. **L2 ccbd-rust 必须原生支持的接口**：基于上述痛点反推 L2 必备 IPC method / state model / event 类型
7. **CCB 现存的死代码 / 不被使用的能力**：观察到但用户从未启用 / 看不到效果的 CCB 功能
8. **跨天归纳总结**：18 天的整体趋势、关键转折点

## 输出要求

- **中文 markdown**
- 每条 finding **必须有引用**：`research/sessions/<project>/markdown/<file>.md:<line>` 或 `<file>.md:<approximate-line>`
- 篇幅参考 `by-claude.md`（268 行），目标 200-400 行
- 每章节都有实质 findings，**不要空标题**
- **关键差异化**：你是 Gemini，要给"独立于 Claude"的视角——`by-claude.md` 是主控 Claude 自己整理的，会有"自我视角偏见"；你应该挑出 Claude 没看到 / 不愿看到的盲点
- 用 Write tool 完成文件覆盖（你有写权限）

## 完成后回复

回复**一行**：`by-gemini.md 重写完成，<总行数> 行，<finding 数> 条 findings`。然后停手。

**不需要做 verdict / 评估 / Step A-F 那一套——这次只做单任务，不做评估。**

## 协作铁律（不变）

- 不接受"我相信"/"通常"——所有 finding 必须带引用
- 不恭维（包括对 by-claude.md 的盲目认同）
- 读不到就说，不绕

