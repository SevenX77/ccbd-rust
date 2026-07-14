# ah 编程开发场景配置包设计问题、张力与未决选择 (a3-design-questions.md)

本文件由 a3 worker (explore/深度调研角色) 主导编写。本轮任务旨在深度挖掘「要做好编程场景配置包」必须先想清楚的真实设计问题、系统张力与未决选择，为后续与用户进行高质量方向对齐提供素材。**本报告聚焦于暴露问题和展示选项空间，不预设任何最终结论。**

---

## 维度 1 · 教什么：配置包的知识边界与承载实体

我们要把「协作规矩」做成一个可复用的配置包，但最核心的知识载体和交付边界在哪里？

### 真实张力与问题
* **系统配置、人读指南与智能体规矩的纠缠**：
  我们自托管开发中沉淀的大量教训，并不全属于注入给 Agent 的规则。
  * **出处 A**：`CLAUDE_CODE_OAUTH_TOKEN` 的环境变量透传与 credentials 拷贝 ([dev-journey-research.md:L19](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L19))。这本质是系统级/Daemon 层的配置（`ah.toml` 与运行期 env）。
  * **出处 B**：投递长文本时，绝不能在宿主 Shell 用 `printf`/`echo` 双引号传反引号文本以防命令替换执行 ([dev-journey-research.md:L20](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L20))。这本质是**人的操作纪律**，在沙箱里运行的 Worker 无法限制外部人如何拼 Shell 字符串。
  * **出处 C**：Worker 角色兜底与不准越权 ([dev-journey-research.md:L45](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L45))。这本质是**注入给 Agent 的 Prompt 规矩**。
* **设计焦点**：如果把这些混在一起，配置包会变得臃肿且分工不明；如果只保留给 Agent 的规则，又会漏掉系统环境搭建和人的操作安全网（这恰恰是容易出大事故的地方）。

### 选项空间
* **选项 A【严格按受众隔离】**：
  * `ah.toml` 承载系统参数（环境隔离、UDS 连接、安全白名单）；
  * `.ah/rules/*.md` 仅承载注入给 Agent 的思维模式、角色定位与行为边界约束；
  * `GUIDE.md` 仅作为人类 Operator 的操作手册（讲解 Shell 避坑、双重验证步骤、SOP 流程控制）。
* **选项 B【统一的 Playbook 范式】**：
  不以受众隔离，而是以「场景阶段/SOP」为轴心。在每个 SOP 环节（如“实施阶段”）内，将“Agent 必须做什么（如 TDD）”、“人类必须验证什么（如物理实证）”与“工具怎么配置”平铺写在一起，形成场景闭环的手册。

---

## 维度 2 · "打过仗"怎么落纸：事故证据的通用化与颗粒度

既然本包的核心价值在于「带疤（真实事故为证）的纪律」，那这些真实事故应该以什么颗粒度呈现在配置包里？

### 真实张力与问题
* **真实性 vs 通用性的冲突**：
  * **出处 A**：`session_watch` 级联删除抢跑， wipe 掉了 `master_watch` 刚复活并准备 continue 的 worker home ([dev-journey-research.md:L14](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L14))。
  * **出处 B**：`MAX_LOG_MONITOR_WAIT` 硬编码 5 分钟超时，导致 Codex 深度 review 长任务被 health_check 误判为 STUCK ([dev-journey-research.md:L25](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L25))。
* **设计焦点**：这些事故的诊断包含大量 `ah` 源码细节（如 `master_watch.rs`、`db/system.rs` 里的状态转换）。
  * 如果原封不动地写进去，非本项目的外部使用者（甚至我们自己未来换了别的编排器）会感到晦涩、充满无关噪音；
  * 如果抽象为“Watchdog 重叠时需做状态前置检查”、“复杂任务需要动态超时”，这又变成了“正确的废话”，失去了“真刀真枪打过仗”的真实感与警示效果。

### 选项空间
* **选项 A【彻底去项目化（DISTILL）】**：
  将事故高度抽象，提取为软件工程中通用的分布式状态或编排器反模式。例如：将“realign 删旧建新非原子”抽象为“动态拓扑重整的原子性与事务回滚”，仅引用原理。
* **选项 B【保留战绩附录（PROVEN EVIDENCE）】**：
  在规则文档中只保留简洁的「铁律」，但在附录中为每条铁律附带一个「真实案例库」，以“Post-Mortem 报告”形式记录当年的 Commit、真实日志和 pane 卡死内容。用具体的细节来证明纪律不是拍脑袋想出来的。

---

## 3. 通用 vs 具体：如何实现拓扑无关又不空洞

协作规矩包宣称「与拓扑无关」，但在不同项目中，Agent 的数量、所用的大模型（Provider）和分工都是千差万别的。

### 真实张力与问题
* **模型特异性与规则通用性的张力**：
  * **出处 A**：严格角色分工 ([dev-journey-research.md:L39](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L39))。我们实证得出“Gemini 搞分析，Codex 搞严谨操作（如精确定位引用和修改）”。
  * **出处 B**：Claude 空闲 banner 与幽灵占位符误判 unknown_prompt ([dev-journey-research.md:L18](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L18))。
* **设计焦点**：这些教训在物理上完全受制于特定 Provider CLI（例如 Claude CLI 的 UI 渲染特性、Codex 在特定长度下的精确度）。如果我们的配置包写死“Codex 负责实现，Gemini 负责分析”，当用户没有 Codex 或 Gemini 权限，全用 Claude 时，这套包直接失效。但如果写成通用理论（如“分析型模型负责方案”，不提具体名字），又无法指导用户在 `ah.toml` 里怎么去选型。

### 选项空间
* **选项 A【角色原型映射（PROTOTYPE ROLING）】**：
  配置包不定义任何具体的 `a1`、`a2`，而是定义“实现者 (Implementer)”、“设计审阅者 (Architect-Reviewer)”、“探索者 (Explorer)”等虚拟角色原型，并给出这些角色对应的「推荐模型能力指标（如强推理 vs 强机械精确）」。由用户在 `ah.toml` 里将具体 slots（如 `a1`, `impl`) 绑定到实际 provider，然后 ah 自动映射规则。
* **选项 B【固定推荐阵型（RECOMMENDED SQUAD）】**：
  直接提供一套经过我们真实验证的“最佳黄金阵型”（如 Claude Master + Codex Worker + Claude Auditor），并明确声明这套包在这套阵型下表现最好。其它自由排布作为高阶玩家的自定义选项，不作为开箱即用保障。

---

## 维度 4 · 给谁用：目标受众的诉求冲突

配置包的受众可能大相径庭，我们要优先满足谁的诉求？

### 真实张力与问题
* **受众诉求的张力**：
  * **外部集成方（如 Graph Agent Studio 开发者）**：希望配置包能作为一套“安全与协调内核”，保证调度不冲突、不超期、授权不越权。他们需要明确的技术契约、RPC/UDS 环境规范。
  * **我们自己跨项目复用**：希望快速把 ccbd-rust 的成功经验无缝拷贝到其它私有项目，需要最简化的 onboarding 步骤。
  * **教学新人**：需要详尽的 SOP 讲解、四问解释、甚至要解释为什么 `printf` 传双引号不安全这类 Linux 基础知识。
* **设计焦点**：试图用一套文件同时满足集成方的架构规矩、个人的快速配置和新人的教学，极易导致“对集成方来说废话太多，对新人来说门槛太高”。

### 选项空间
* **选项 A【以集成方为先（API-First）】**：
  配置包主干为技术白皮书和严格的配置文件模板，注重机制（BindsTo、Socket 路径、环境隔离、`CLAUDE_CODE_OAUTH_TOKEN` 透传等）。人文的 SOP 和避坑指南作为 secondary 文档或直接剪裁掉。
* **选项 B【以开发实战为先（Playbook-First）】**：
  配置包以实战演练手册为主体，突出 SOP-08 流程和 operator 的隐性操作知识（不信二次信号、in-loop 盯盘、TDD 流程），将技术细节视作该实战流程的技术支撑。

---

## 维度 5 · 形态：纯静态规则文档 vs 可执行护栏

如何确保这套配置包里的纪律是「被严格执行的」，而不是写完就落灰的字面规则？

### 真实张力与问题
* **软约束与硬机制的张力**：
  * **出处 A**：TDD 红绿循环 enforce 落地 ([dev-journey-research.md:L42](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L42))。我们虽然在文档里要求“先写失败测试，再写实现”，但在实际开发中，Worker（包括 Codex 和 Claude）经常图省事一气呵成把代码和测试写完，跳过了红灯阶段。
  * **出处 B**：Worker 越权当 master ([dev-journey-research.md:L45](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L45))。我们必须在 CLAUDE.md 中通过 `CCB_CALLER_ACTOR` 作结构防护，并在 Prompt 头部做硬声明。
* **设计焦点**：纯文字规则（.md）对于大模型和人类来说都是“软的”。大模型会遗忘或忽视，人类也会因习惯而犯错。如果配置包里只有静态的 `.md` 规则，它的防护效果极差。

### 选项空间
* **选项 A【纯静态知识包（Declarative Config）】**：
  仅提供 `.ah/rules/` 模板和 `GUIDE.md`。依靠人类 Operator 或主控 Master 在运行时通过审计来发现违规。优点是极其干净，不包含任何外部脚本依赖。
* **选项 B【带可执行看门狗的混合包（Active Playbook）】**：
  配置包除了包含 `.md` 外，还自带一组辅助工具/可执行脚本。例如：
  * 一个 Git Pre-commit 钩子：检查在修改 `src/` 时，测试目录下是否确实存在新加的且能运行失败的代码；
  * 一个自动检测脚本：在 `ah start` 启动时，自动校验沙箱 `CLAUDE_CONFIG_DIR` 是否为绝对路径，若为相对路径直接报错拒绝启动。

---

## 维度 6 · 理想 SOP 与现实执行的巨大差距

我们总结的“SOP 闭环”（调研→设计→实施→审阅→e2e→目标验证→回炉）是一个完美的闭环，但我们自己平时是怎么“破功”的？

### 真实张力与问题
* **理想化 SOP 经常在压力下妥协**：
  * **出处 A**：in-loop 盯到结果 vs 傻等 ([dev-journey-research.md:L37](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L37))。主控经常发出 "I will monitor in the background..." 后便进入空等状态，直到被用户敲打或超时。
  * **出处 B**：抓到 Blocker 立即修 vs 抛给用户 ([dev-journey-research.md:L46](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L46))。遇到 blocker 时，主控有时会把细节问题抛出，询问用户“是否需要在这个 PR 里修复”。
  * **出处 C**：物理实证 vs 盲信 ([dev-journey-research.md:L36](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L36))。主控曾因为 glob 路径没写对，就盲目汇报“实测没有 transcript 日志，这是一个 block 级别的 bug”，结果被用户用 `grep -r` 打脸。
* **设计焦点**：如果把这些真实的“妥协与破功”掩盖起来，写出一套完美无瑕的 SOP，这套配置包就失去了“真实打过仗”的灵魂。我们是否应该把“防御性设计”和“防破功机制”直接写进 SOP 规矩？

### 选项空间
* **选项 A【标准 SOP 宣言】**：
  仅记录完美的协作流程。将所有“破功”定义为操作失误，由外部机制（如 code review、独立审计）去惩罚和纠正。
* **选项 B【防御性防破功 SOP】**：
  直接在 SOP 中写明「常见破功点与反制机制」。例如：在“实施”小节，不仅写“先红后绿”，还要特意加粗写上：“⚠️ 注意：实现者会极力试图一次性提交代码与测试以假装跑过 TDD。作为审计者/主控，你必须查看 Git 历史，验证是否存在‘只有测试变更且测试失败’的独立 Commit 记录，否则一律驳回。”

---

## 维度 7 · 我们自己都还没想清楚的几个核心未决点

有些设计张力属于我们当前的技术债或架构未决点，配置包应该在这些“灰色地带”如何表态？

### 1. 共享 working tree 与串行构建的物理瓶颈
* **张力事实**：所有 slot (master+workers) 物理上共享同一个 cwd (仓库) 磁盘目录 ([pm-proxy-experience.md:L66](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pm-proxy-experience.md#L66))。这导致多个 worker 绝对不能同时开分支或 commit，且并行跑 `cargo test` 会因为锁 fd 被继承而发生 flock 死锁。
* **张力问题**：这在物理上直接掐死了“高并发多智能体协同”的上限。配置包是否该妥协并固化“必须串行构建、纯 md 任务才能并行”的铁律？还是说应该明确指出这只是临时现状，真正的解法是 `ah` 未来应该为每个 worker 独立分配 `git worktree`？

### 2. 误判 STUCK 后的状态自愈死胡同
* **张力事实**：当前 health_check 把 agent 标为 `STUCK` 后，STUCK 是一个没有任何自愈路径的死胡同 ([dev-journey-research.md:L26](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L26))。
* **张力问题**：配置包的 GUIDE 应该怎么教用户处理这个问题？是教他们接受现状，一旦 stuck 就无脑运行“`ah cancel` -> `ah kill` -> `ah up`”（当前的临时运维 SOP）；还是在配置包设计上推动 `ah` 内核层将“CAS transition 允许 STUCK -> IDLE”的硬性修完？

### 3. 超时判定（300s）的一刀切缺陷
* **张力事实**：当长任务真实执行超过 5 分钟（如 codex 做深度代码 review），系统会主动放弃日志监听并退化为 UI 扫描，极易引发假 STUCK 误判 ([dev-journey-research.md:L25](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L25))。
* **张力问题**：配置包是应该将“5分钟限制”作为项目纪律让用户把任务裁剪得极小；还是在设计中提议弃用固定硬编码，推动基于 pane 输出活动的动态续期机制？
