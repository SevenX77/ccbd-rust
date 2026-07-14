# 换血#2 命名方案定稿 — 三件套备料 + 落点回执(2026-07-10)

用户已批命名方案定稿,**换血#2 生效,替换前条 a4w 方案(已作废,见文件名带 `.SUPERSEDED-a4w-scheme` 后缀的旧稿)**。

角色 = gatekeeper(**g**,质量门)/ code monkey(**m**,antigravity 快速实施)/ oracle(**o**,设计辩论席)/ master(不变)。格式 `<角色首字母><泳道号>`,层级分隔符 `-`(TOML 键名禁用点号,用连字符)。m 系挂靠 g 系,只向自己的 g 汇报,不向 master 汇报。

映射:a4→**g1**、a1→**g1-m1**、a5→**g2**、a2→**g2-m1**、a3→**o1**。扩容规则:g1 下第二 code monkey = `g1-m2`;新增第三泳道 = `g3`/`g3-m1`。

**落点=换血#2(模块A/B合入边界),不单独重启;本文档 + 下述文件均为 staged 状态,未应用于现行运行栈。**

## 三件套落盘路径

1. **ah.toml 草稿**:`research/blood-swap2-ah.toml-draft`(`ah config validate` 已过)。`[agents.g1-m1]`/`[agents.g2-m1]`/`[agents.o1]`/`[agents.g1]`/`[agents.g2]`,g1/g2 的 statusLine 文案改为 `"g1 · ..."`/`"g2 · ..."`。基于当前活栈 `ah.toml`(含 toolchain RO 挂载 + `RUSTUP_HOME`/`CARGO_HOME` 修复)平移,那两条不因改名而变。
2. **rules 文件**:新增 `.ah/rules/g1.md`(原 a4)、`.ah/rules/g2.md`(原 a5)、`.ah/rules/g1-m1.md`(原 a1)、`.ah/rules/g2-m1.md`(原 a2)、`.ah/rules/o1.md`(原 a3)。隶属声明已写清楚:g1-m1/g2-m1 的 `.lane-question` 收件人字段分别是 `g1`/`g2`。**换血#2 时 operator 需删除 `.ah/rules/a1.md a2.md a3.md a4.md a5.md`**(五个旧编号文件本次全部保留原样未动,新文件是平行新增,不是覆盖)。
3. **工单模板 / `.lane-question` 约定编号替换**:本轮会话内所有面向 a1/a2/a3/a4/a5 的 ad-hoc brief 已用旧编号派发,不追溯改写(模块A/B 是换血#2 的边界条件本身,理应在换血#2 前收口)。换血#2 之后,master 新写的 brief/`.lane-question` 引用一律改用 `g1`/`g2`/`g1-m1`/`g2-m1`/`o1`;新 rules 文件内的 `.lane-question` 收件人字段已经是新编号,无需二次改。

## 已知约束(写作时验证过,备落地参考)

- `provider settings`(含 `statusLine`)当前只对 `claude` provider 生效——`g1-m1`/`g2-m1`/`o1` 是 antigravity,不能在 `ah.toml` 里配 `statusLine` 字段实现"g1-m1-agy"式称呼;那种称呼只能落在 tmux pane/session 命名与日常口头/文档引用上。
- TOML 键名 `g1-m1` 等带连字符的写法已用 `ah config validate` 验证通过(bare key 允许连字符);agent id 命名校验只要求 ASCII 字母数字/`_`/`-`,未来扩容(`g1-m2`/`g3`/`g3-m1`)无阻碍。

## 顺带发现:`.ah/rules/master.md` 有编号漂移,仍未在本次改动范围内

`master.md` 仍写着换血#1 之前的单泳道措辞(只提 a1/a4,没有 a2/a3/a5),这次的 g/m/o 改名同样没有触碰它——留痕不动,换血#2 执行前请 operator/用户确认是否一并订正(连同新编号一起改)。

## 换血#2 执行清单(给 operator)

1. 应用 `research/blood-swap2-ah.toml-draft` → 替换 `ah.toml`。
2. `rm .ah/rules/a1.md .ah/rules/a2.md .ah/rules/a3.md .ah/rules/a4.md .ah/rules/a5.md`(新 `g1.md`/`g2.md`/`g1-m1.md`/`g2-m1.md`/`o1.md` 已就位)。
3. (可选,征询用户后再定)订正 `.ah/rules/master.md` 的拓扑措辞 + 编号。
4. 换血 runbook 其余步骤(优雅停栈→换二进制→前台重启→逐 agent 等 IDLE→双验 `ah ps`/`tmux list-panes`)照旧不变——本次编号变更不改变换血机制本身。
