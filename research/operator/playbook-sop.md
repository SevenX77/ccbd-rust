# playbook · 业务 SOP 断点(项目层)

进入条件:派单→实施→测试→审→PR→盯 CI→修→merge 链条里出了问题,你在判断怎么处置。
通用细则见 `research/config-pack/pack/OPERATOR-HANDBOOK.md`「场景细则 · 业务 SOP 断点」章。

## 本项目的规则修订落点

诊断出"哪份规则没写清楚"后,修订落到对应文件:角色总纲在 `CLAUDE.md`,master 细则在 `.ah/rules/master.md`,各 worker 细则在 `.ah/rules/<agent>.md`,operator 场景细则在 `research/operator/playbook-*.md`。修订 `.ah/rules/*.md` 与 `CLAUDE.md` 属于 operator 保留权力,由你直接执行,不下发派单。

## 越级介入 worker 的条件与分寸(用户裁决:`research/USER-GOALS-AND-PRINCIPLES.md` B4)

判据在通用章(见开头引用)。本项目的观察工具:例行观察 = `ah ps` + 扫一眼各 pane 有无明显异常;越级介入的触发 = master 自身失联/僵死/明显误判。

## 实锤案例:被迫代跑 = 规则有洞

- obs #55:auto-merge 抢跑 r1 审核。事实:merge 门未把"审核完成"设为前置,operator 手动兜底。教训:每个 PR 的交接必须显式记录谁审,审完才满足 merge 条件。
- PR 尾巴(盯 CI/验收)无人认领、换血漏装 ahd:与 obs #55 同类,均为 operator 手动兜底掩盖机制缺失。教训:每次兜底后,当场把缺失的环节写进对应规则 md,否则同类干预重演。

## 实锤案例:同根因返工循环,第 2 轮就修机制

- PR#151(凭据):CI `test` job 同根因连红 5 轮。事实:签名变更的调用点散在 `--lib`/`tests/`/`bin`,worker 用裸 `cargo check` 不编译 test 目标;operator 到第 5 轮、用户过问后才归因修规则。数据:功能实质耗时 33 分钟,该根因返工耗时 89 分钟(占 73%);第 2 轮即修可省约 60 分钟。对应规则:删/改公共符号至少 `cargo test --no-run` 全量编译(`feedback_verify_full_cargo_test_not_just_lib`)。
- PR#146(Module D):同为多轮 CI 连红同族案例。
- 判定与处置的通用判据在通用章;本项目高发形态:CI 同一 job 连红、同一交接反复 REJECT、同类假完成反复出现。

## 实锤案例:范畴边界(先判 WHAT,再套本 playbook)

- 2026-07-13,obs #59:worker 误删活栈 master socket 致 islanding。事实:这是分钟级重启可解的运行时故障(该按 `playbook-runtime.md` 直接运维闭环),却被误判为业务 SOP 断点,operator 忙于写事故记录而系统持续瘫痪;用户裁定该记录"近乎零可交付价值"。教训:运行时/基础设施故障不进本 playbook;"被迫干预必修规则"只约束业务 SOP 环节的代跑,不把一次栈重启也写成规则修订。
