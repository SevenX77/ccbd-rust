# SOP:required checks 作为 auto-merge 前置 — Requirements

Status: requirements drafted 2026-07-11(operator)。性质是 **SOP/场景包基建**,不是代码任务;落点在场景包(ah-scenario-pack)规矩文档与仓库配置核查清单。

## 背景(实锤事故)

2026-07-11,dev 仓未配 branch protection 时,`gh pr merge --auto` 对 #142/#143 均为**立即合并**(<1 分钟,CI 尚未跑完)——auto-merge 语义依赖 required checks 存在,否则等于无门禁直合。当日已给 dev 仓 main 配上 `test` check 必过。用户裁决:该配、属 SOP 一环、是编程场景包的必要条件(三问全是)。

## Requirement R1: 仓库配置核查项

编程场景(泳道产码→PR→auto-merge)启用前,必须核查目标仓库 main 分支已配 required checks(至少一个真实 CI job),否则 auto-merge 流程禁用、退化为人工等 CI 绿再合。

验收标准:
- 场景包规矩文档(kernel 层)新增 prerequisite 条目:required checks 配置核查,含 `gh api repos/{owner}/{repo}/branches/main/protection` 的核查姿势与最小配置示例(required check + 禁 force-push/删除)。
- 明确例外:**公开镜像仓(如 SevenX77/ah)不配**——会挡 dev→公开的同步 push;泄露防线由三道 leak-gate 承担,不靠 branch protection。

## Requirement R2: check 选型纪律

- required 只挂**稳定**的 check(dev 仓=Linux `test`);已知 flaky 的(macos/windows,见 #140)不挂,否则合并链会被 flake 卡死。
- flaky check 修稳后再评估是否升为 required,不自动升。

## 落点

- 场景包仓 ah-scenario-pack(dev pack/ 唯一 source of truth),下个 pack 版本随发;发布走既有 publish 脚本漂移门。
- 本 spec 完成的定义:pack 规矩文档含该 prerequisite 且随版本发布;dev 仓配置已生效(已完成,2026-07-11)。
