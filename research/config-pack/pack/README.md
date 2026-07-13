# ah 编程场景配置包

**版本:v0.7.0**(v0.5.0=执笔权铁律+设计管线;v0.5.1=双泳道拓扑模板 + OPERATOR.md 哨兵体系 + master 派单哨兵机制;v0.6.0=经典版拓扑模板 `classic/`——设计线+双实施+统一审核+测试席,含 d1 双角色 r2 后备审核;v0.7.0=沉淀一轮换血周期硬教训——派单前 commit worktree 依赖 / 选 PR base 前 `git fetch` / 稀缺 provider 朝富余侧替换 / 「完成」=端到端亲验 / 同根因第2轮归因修机制 / 行为保持重构双保险 / 每代疗效判决报告五级 verdict)

把「用 ah 编排多个 AI agent 做软件工程」这件事,从零散经验固化成一套**可复用、与拓扑无关**的规矩包。装进任意项目的 `.ah/rules/`,你的 master 和 workers 就天然按这套编程协作规矩工作。

## 这是什么

ah 注入每个 agent 的规则分三层:`ah 内核(编译进二进制,通用协调+安全) + bundle 层 + 你项目的 .ah/rules/<agent-id>.md 场景层`。
本包提供的是**场景层内容 + 人读指南**,不改 ah、不锁死拓扑:

- `.ah/rules/*.md` —— 各 slot 的场景规则模板,核心是每个文件的「角色定位」一节。
- `ROLES.md` —— 常见角色原型 → 推荐 provider 能力(参考表,不是强制)。
- `ah.toml.example` —— 一份示例拓扑(id+provider),**明标可自行增删改**。
- `GUIDE.md` —— SOP 协作闭环 + 设计管线(架构级课题五步换手)+ operator↔master 代理实践 + 工程纪律清单。
- `VERIFY.md` —— **项目验证档案模板(fill-once)**:编译/测试命令、迭代-收口测试分离、资源约束、按改动类型的验收矩阵(含 UI 的 agent/user 验收开关)。接入时填一次,所有 agent 查表执行,不现场重导。
- `OPERATOR.md` —— **任务推进保障体系**:三层哨兵(每单 pend 闹钟/全局停摆体检/状态翻转监听)、停摆分诊树、高危操作连带清单、闭环证据纪律。核心原则:每个等待都必须有闹钟,靠机制不靠自律。
- `OPERATOR-HANDBOOK.md` —— **operator 使用指南(从接管到跑通)**:治理链(用户 → operator=代理 CEO,监督管理 PM → master=项目经理,管辖调配全员 → workers)、接管四步(身份块/坐标实测/报到)、从零拉栈、派单文件注入 SOP、哨兵驾驶、收口与权力面。被任命为 operator 的 agent 从这里上岗。
- `dual-lane/` —— **双泳道拓扑模板(并发版,拷来即用)**:master(零裁决中继)+ 两条泳道(每条 = 1 闸门 g + 1 实施 m,m 只向本泳道 g 汇报、泳道内 g 终裁)+ o1 设计席。含 ah.toml.example + 全部 6 个 rules 文件。单泳道够用就删 g2/g2-m1;实施位首选 codex,用发散型 provider 时保留 m 文件里的"发散型附加护栏"一节。
- `classic/` —— **经典版拓扑模板(设计线+双实施+统一审核+测试席,拷来即用)**:master + 两个 codex 实施者(c1/c2)+ 设计线(o1 辩论席只辩论 / d1 主笔唯一执笔)+ r1 统一审核席 + test 测试席。**亮点=d1 双角色**:主角色设计主笔,审核饱和且 d1 空闲时由 master 显式派单切成 r2 后备审核(含自证闭环回避铁律)。含 ah.toml.example + 7 席 rules + 专属 OPERATOR.md(d1↔r2 调度协议);顶层 GUIDE/VERIFY/OPERATOR 通用部分照用。

## 三套拓扑怎么选

- **基础版(根目录 `ah.toml.example`)**:master/impl/impl2/design/audit——审计是独立角色,适合任务流较线性的项目。
- **经典版(`classic/`)**:设计发散/执笔分离(o1 辩论 → d1 执笔)+ 两个 codex 并发实施 + 一条统一 r1 审核基准 + 独立 test 席压端到端;d1 兼 r2 后备审核吸收审核峰值。比基础版多了对抗式设计线与专职测试,又不引入泳道层级——适合任务线较集中、要单一审核基准 + 独立测试席的项目。
- **双泳道版(`dual-lane/`)**:闸门下沉进泳道、审计+测试执笔合一(g)、两泳道真并发,master 退成纯中继——适合任务可拆两线并行、且要把"实施↔审收"回路压到泳道内最短路径的项目。实测:单泳道 RED→GREEN 接力 ~30 分钟一棒,泳道内事务不过 master。

## 一分钟上手

1. 把 `.ah/rules/` 拷进你的项目根。
2. 参照 `ah.toml.example` 在你的 `ah.toml` 里声明 agents(每个 = 一个 `id` + 一个 `provider`)。**几个、什么 provider、怎么排,你定。**
3. 给每个 agent-id,在 `.ah/rules/<id>.md` 里填「角色定位」——它是谁、干什么。可以直接 copy 本包的角色模板(见 `ROLES.md` 的原型)改。
4. **填 `VERIFY.md`**:把你项目的构建/测试/验收命令沉淀进去(一次性),并把关键命令同步进各 `.ah/rules/<id>.md`。
5. `ah start`。ah 会把内核 + 你的场景层拼好注入每个 agent 的 home。

## 关键约定

- **agent 用 `id+provider` 标识**(如 `a1-codex`、`review-antigravity`)。`id` 随你起,ah 按 `.ah/rules/<id>.md` 匹配。
- **不锁死拓扑**:本包所有示例(含下面的 id 命名)都只是示例,照抄或重排都行。
- **角色能力写在规则文档里**,不在这份 README 里硬编码「谁必须干什么」。

详见 `GUIDE.md`。
