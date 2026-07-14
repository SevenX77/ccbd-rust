# 编程场景配置包 · 设计文档

> 目标:把「ah 一路自托管开发」沉淀出的编程协作规矩,做成一套**可复用、与拓扑无关**的配置包,给外部集成方拿去用(v1「给别人用」方向)。
> 素材来源:[dev-journey-research.md](dev-journey-research.md)(a3-antigravity 的历史问题/模式调研)+ [pm-proxy-experience.md](pm-proxy-experience.md)(operator↔master 代理经验)。

## 0. 三条硬约束(用户已定)

1. **机制不变**:沿用 ah 现有的 `id+provider` + `.ah/rules/<id>.md` 场景层。**不加抽象角色名、不加 `role` 字段、不改 ah 代码。**
2. **拓扑无关**:配置包不是「我们现在这套 a1-codex/a2-codex/a3-antigravity/a4-claude 固定拓扑」。设计里**不用 a1/a2 代指固定角色**;示例一律写成 `id-provider`(如 `a1-codex`、`a2-antigravity`),并明标「示例,自行增删改」。**每种角色几个、配什么 provider、怎么排——用户自定义。**
3. **角色定位入文档**:某个 slot「是什么角色、干什么」写进它自己的 `.ah/rules/<id>.md` 的「## 角色定位」一节。包只给模板 + 参考,不钉死。

## 1. 分层(对齐 ah 既有机制)

ah 注入 worker/master home 时,规则是三层拼接:`固定内核 kernel(ah 编译进二进制) + bundle 层 + .ah/rules/<id>.md 场景层`。配置包据此分工:

| 层 | 内容 | 谁提供 | 拓扑相关? |
|---|---|---|---|
| **内核 kernel** | 通用协调契约 + 沙箱安全(`ah ask`/不越权/不杀 pane…) | ah 自带,**不动** | 否 |
| **场景层 `.ah/rules/<id>.md`** | 该 slot 的**角色定位** + 角色相关纪律 | 配置包给模板,用户填 | 是(用户定) |
| **人读指南 GUIDE.md** | SOP 闭环 + operator↔master 代理实践 + 全量工程纪律参考 | 配置包 | 否 |
| **参考表 ROLES.md** | 常见角色原型 → 推荐 provider 能力(仅参考) | 配置包 | 否 |

关键:**通用纪律不塞进每个 rule 文件重复**。ah 内核已覆盖「不自派单/沙箱安全」等 worker 铁律;编程专属纪律(TDD/串行 cargo/物理实证/baseline 对照)作为**人读参考**放 GUIDE,并在 rule 模板里以一节精简引用。

## 2. 包结构

```
programming-scene-pack/
├── README.md                    # 这是什么 + 一分钟上手
├── GUIDE.md                     # SOP 闭环 + operator↔master 代理实践 + 工程纪律清单
├── ROLES.md                     # 角色原型 → 推荐 provider 能力(参考表)
├── ah.toml.example              # 一份示例拓扑(id+provider,注释标"自行增删改")
└── .ah/rules/
    ├── master.md                # 主控(协调者)场景规则模板
    ├── <id>.md ...              # 各 worker slot 模板(每个带「角色定位」一节)
    └── _TEMPLATE.md             # 空白 slot 模板,用户 copy 改 id 即用
```

## 3. rule 文件模板(核心:「角色定位」一节)

每个 `.ah/rules/<id>.md` 统一骨架(ah 会自动在前面拼内核,所以这里只写场景层):

```markdown
# <这个 slot 的一句话定位>  (示例 id:a1-codex)

## 角色定位
- **你是**:<这个 slot 在本项目里承担的角色,如「主力实现者」>
- **适配 provider 能力**:<为什么这个角色适合某类 provider,如「强机械精确 → codex 类」>
- **在拓扑里的位置**:<和其他 slot 的协作关系,如「实现由你出,审阅交给设计审阅 slot」>

## 职责
- <这个角色具体做什么>

## 边界(铁律)
- 只做当前被指派的单条任务,完成即回、等下一单。
- 不自派单、不当 PM、不自启 PR 流程。(ah 内核已强制,这里重申)
- <角色专属边界,如「实现者:改 src 前先写失败测试」>

## 工作纪律(角色相关)
- <引用 GUIDE 里与本角色相关的纪律,如 TDD 红绿 / 串行 cargo / grep-before-claim>
```

## 4. 一个写满的示例(「实现者」slot,示例 id = a1-codex)

```markdown
# 实现者 · 主力编程  (示例 id:a1-codex)

## 角色定位
- **你是**:本项目的主力实现者——写代码、写测试、调编译错误、跑回归。
- **适配 provider 能力**:强机械精确、严谨 file:line 定位、不发散 → 适合 codex 类 provider。
- **在拓扑里的位置**:你出实现;设计/审阅交给「设计审阅」slot;e2e/审计交给「审计」slot。你的产出会被独立审。

## 职责
- 按 brief 实现指定改动,先写失败测试(TDD 红),再实现到绿。
- 编译/测试全绿后交付 unified diff 摘要 + 实际测试输出。
- 被要求时,独立审计其他 slot 的代码并给 verdict。

## 边界(铁律)
- 只做当前这一条被指派的任务;完成回结果,等下一单。
- 绝不自派单 / 不当 PM / 不自启 PR 或多步流程。
- 改 src/ 前必须先有失败测试;不重构任务范围外的代码。

## 工作纪律(见 GUIDE 详版)
- **TDD 红绿**:先红后绿,按层(纯逻辑/集成/e2e)分严格度。
- **串行 cargo**:资源受限环境 `CARGO_BUILD_JOBS=1` + `--test-threads=1`。
- **grep-before-claim**:下任何关于文件/命令/行为的断言前先 grep 核实,引用具体 file:line。
- **baseline 对照**:红灯别甩「无关」,`git stash` 对照主干单跑,证明是既有还是本次引入。
```

## 5. 待你确认的点

1. 上面的**分层**(内核不动 / 场景层放角色定位 / 通用纪律进 GUIDE 不逐文件重复)对不对?
2. rule 文件**骨架**(角色定位 / 职责 / 边界 / 纪律 四节)要不要加减?
3. ROLES.md 的角色原型给几档?建议:主控、实现、设计审阅、探索调研、e2e审计 五档,每档标推荐 provider 能力 + 我们实测的适配依据(来自 dev-journey 调研)。

确认后我铺满整包(GUIDE / ROLES / 各 rule 模板 / ah.toml.example / README),放进一个干净分支给你 review。
