# ah 自我知识 skills — Tasks

依赖:#108 已合 main(内建 skills 机制在)。全部实施走 master 派 codex,TDD,operator 过目后 PR。

## T1 ah-config + ah-runtime-state SKILL.md(R1)
- 内容按 design R1,锚点逐条对 `research/ah-internals-anchors.md`。
- 注册表加两条 MasterOnly(src/provider/builtin.rs,纯数据)。
- 测试:master 三家 provider 物化、worker 三家无、frontmatter 合法、关键字段抽查(如 RuntimeSnapshot 字段名与 struct 一致)。

## T2 kernel skill 索引(R3)
- master_kernel.md 指针行扩为三 skill 索引,保留 --help 兜底。
- tests/builtin_skills.rs 扩:三 skill 名齐 + ask/ack-ready 锚不丢。

## T3 ah-operate skill(R6)
- 正文按 design R6 六条 playbook,provider-中立。
- 注册 MasterOnly + worker-absent 断言。

## T4 README 外部安装表 + #107 模板修复(R4 基线 + R5)
- README:三 provider 复制目标表(antigravity=.agents/skills 或 ~/.gemini/config/skills)+ 内建 skills 说明。
- 模板修复 4 处(公开仓安装命令 / 私货打标 / close-out 如实 / builtin skills 提示)。

## T5 plugin 一键安装(R4 增强)
- 公开仓 claude plugin(marketplace 声明,官方 schema 实施时核实)。
- antigravity plugin.json bundle。
- codex 核实无 plugin 则文档写明复制安装。
- 验收:claude 一家真装通(`/plugin marketplace add` → skills 可见)。

## T6 发布同步
- 合 main 后同步公开仓 SevenX77/ah(防泄漏剪裁 + 三道 leak-gate 照旧)。

顺序:T1/T2/T3 可并行(同分支串行 commit);T4 随后;T5 可拆第二 PR;T6 收尾。
