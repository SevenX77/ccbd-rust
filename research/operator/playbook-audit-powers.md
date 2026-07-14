# playbook · 常设审计 / 保留权力(项目层)

进入条件:设计 gate 把关、发版、公开仓同步、发凭据、换血、同步 main、资源分配——保留权力事项,天然归你,不走 SOP。
通用细则见 `research/config-pack/pack/OPERATOR-HANDBOOK.md`「场景细则 · 审计与保留权力」章。

## 需求追溯审计(O1)· 项目落点

- 总账:`research/REQUIREMENT-LEDGER.md`(每需求:用户原话/spec/PR/验证状态)。spec 需求按 kiro 规范写在 `.kiro/specs/<spec>/requirements.md`,带用户原话出处。
- 设计 gate 第一项对账:requirements.md 是否完整承载用户层需求;有没有被静默削/推迟的需求;变更记录里的削减原则是否站得住;"后续/可选"是否进了登记簿并有 owner。发现静默削需求,替用户追问"为什么改需求"到决策原因,不放行。
- 实锤:hook spec 曾无 requirements.md,用户的"任务开始 hook"需求静默蒸发。教训:需求断链本身就是结构病,gate 对账从 requirements.md 是否存在查起。

## PR 疗效审计(O2)· 项目落点

- 台账:`research/pr-efficacy-ledger.md`。每 PR merge 当场登记预期效果 + 实证计划;状态四值:实证闭环 / CI 绿仅代码闭环 / 验证债 / 未观测;出 bug 能反向定位引入 PR。
- 实锤:一次全量盘点 45 个 PR,仅约 20% 实证闭环;PR#146(Module D)CI 全绿,到换血才发现网关从未激活。
- 与报告禁令分账:本条管台账状态;对用户措辞的硬门在 `playbook-report-escalate.md`。

## 保留权力 · 项目细则

- 发版 / 公开仓同步:tag 后必同步 dev→公开仓 SevenX77/ah;过三道 leak-gate;公开仓 commit 身份用 `SevenX77 <107291361+SevenX77@users.noreply.github.com>`,不带 Claude session-URL trailer。参考:`project_ah_release_sync_dev_to_public_repo`。
- 凭据发放:worker 沙箱只挂鉴权 + 二进制(隔离红线见 `playbook-runtime.md`);master 的 gh 凭据是沙箱级注入的 copy,换血即丢,spawn 路径注入是已登记欠账(`project_ah_master_gh_injection_mechanism_gap`);全局 ah.toml `[env]` 禁放 token。
- 换血:已获用户预授权,执行时知会;每个大模块合入必换血;换血后必出阶段性疗效判决报告(`research/gen-efficacy-reports.md`,预期→实际→verdict 五级),未观测不许算治愈。

## 同步 main(O6)· 项目实锤

选 PR base 前必 `git fetch`,以 origin/main 为准。实锤:master 依据陈旧的本地 main 判断分支关系,把 fix 合进了落后于 main 的死分支(`project_ah_stale_local_main_wrong_pr_base`)。教训:过时的局部认知做决策与 current_exe 案是同族病,凡涉分支基线先同步再判断。

## 资源分配(O9)· 项目实锤

- 方向:某 provider 成为约束时,把活往不吃该资源的席位挪——设计发散挪 o1/antigravity(免费),审查挪 codex 闸门;不砍免费席位、留稀缺 claude 继续烧。
- 实锤:凭据任务中 operator 声明"claude 周额度仅剩 4%"却做反向分配(跳过免费 o1,保留 claude 的 d1+r1);r1 随后在 PR#151 烧 6 轮 claude 额度做复审,而该类 bug 只有编译器/测试抓得到,claude 的质量优势无用武之地,双重浪费。
- 自查判据(用户裁决:`research/USER-GOALS-AND-PRINCIPLES.md` B11):每次分配席位前核对"这一步与已声明的稀缺约束一致吗?稀缺资源是否被放在便宜资源能顶的位置上?"——分配与约束冲突时,暂停该次分配,先复核再执行。
