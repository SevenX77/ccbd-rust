# ah dogfood 疗效账本(operator 维护)

用户 2026-07-10 拍板:每个大模块合入后换血,后续任务作为 dogfood,operator 观察效果。本账本按**二进制代次**记录编排器自身病种的发生率,换血前后对照,疗效当场可证。

计数口径:一次派单周期内发生一次记一例;同一 job 反复表现同病种记一例。证据要求:pane capture 或 DB 行,不凭印象。

## 病种登记表

| 病种 | 症状签名 | 对应修复(哪个模块治) |
|---|---|---|
| G1 误闩 | a4/claude 派单被 PROMPT_PENDING 闩住,需 prompt resolve+/clear | PR#126/#127 已合(banner/ghost scanner 修)→ 换血#1 生效 |
| G2 假完成 | agy job 翻 COMPLETED 但 reply 是垃圾过程文本,实际工作还在跑 | P0-1 已删毒推断器(#127)→ 换血#1 生效;完成判定 v2 属模块 C 设计轮 |
| G3 假 STUCK | 真实工作长静默被判 STUCK,job FAILED 而 pane 真相完成 | 300s→900s+时间戳 .max 已修(a7c9d34,live 二进制已含);**残余根源=PANE_DIFF_STUCK 写入路径仍活**(mark_agent_stuck+timer/health_check 调用,2026-07-10 a3 事故 error_reason 实证)→ 模块A重铸(删残余推断路径)→ 换血#2 生效 |
| G4 停摆不醒 | agy 归 IDLE 零产出,无自动唤醒,需 master 续派 | 待设计轮(完成协议);账本先记发生率 |
| G5 STUCK 死胡同 | (本次实测 a3 自回 IDLE,是否仍存留观察) | 模块 A 白名单对称化相关 |

## Gen-0 基线(换血#1 前,旧二进制,live 栈 = main 8aee446 前)

观察窗:2026-07-09 夜 – 2026-07-10(P0-1/P0-2/设计轮流水)

| 病种 | 例数 | 明细 |
|---|---|---|
| G1 误闩 | 6 | a4 每次派单必闩(4 例 operator/master 手解)+ 第 5 例 master 自解(job_48b8d4bc)+ 第 6 例(job_8210e3d0,ra2 修订单) |
| G2 假完成 | 5 | a1×3(P0-2 线,含 reply="I will pause to wait for the compilation to finish" 存库例)+ 历史 3 例/晚口径 + 第 5 例(job_92b33ea4 收口单,COMPLETED 时 pane cargo 仍 running) |
| G3 假 STUCK | 2 | a1 code-review 10m47s 真完成被判 STUCK(2026-07-02);a3 设计发散完成被判 STUCK+FAILED(2026-07-10,叠加盘满写盘失败,产出靠带外抢救) |
| G4 停摆不醒 | 1 | a1 P0-2 跑完 RED 基线归 IDLE 零改动零 commit(疑叠加盘满因素) |
| G5 | 0(反例) | a3 被判 STUCK 后 agent 态自回 IDLE(job FAILED 不影响),未复现死胡同 |

## Gen-1(换血#1 后:main@b363dce=P0-2 合入,拓扑 a1-a5)

观察窗:2026-07-10 19:0x 起(换血#1 完成:二进制 b363dce,session sess_e5b3cdab,五 agent 全 IDLE 零 onboarding 僵尸;pane 号位移 a3=%3/a4=%4/a5=%5)。换血过程附带发现并清理:a1 遗留挂死测试进程占 cargo 槽 4h47m + 测试泄漏 ~135 个 tmux socket(多个 daemon 活着,已精确清杀,test-hygiene 债追加实证)
预期疗效:G1 归零(scanner 修)、G2 显著下降(毒推断器已删);G3/G4 预计仍在(治它们的模块 A/设计轮还没上)。
**若 G1/G2 不降,说明修复无效或有同族残留,立即回报用户并升维。**

| 病种 | 例数 | 明细 |
|---|---|---|
| G1 误闩 | 0/3 派 | 抽验 job_9754f4d2(a4):created→dispatched 仅 1s,零闩;a5 首单 job_ea559200 直通;job_1fc347c8(a4)出现过 PROMPT_PENDING 但为**真阳性**——输入行确有 master 未提交的回复文本,检测正确、master 自解后放行(与 Gen-0 banner/ghost 误判本质不同,不计病例) |
| G2 假完成 | 0/2 单(垃圾reply型);1 例语义型残余 | 抽验 job_7a525702(a1):reply 为真实产出;a4 单同真。**语义型样本 job_e5a65353(a1 模块A实施)**:COMPLETED 时实施未 commit、cargo check 仍跑,但 reply 诚实自述"end turn 等后台唤醒"——垃圾reply型已治,剩"回合结束≠任务完成"语义缺口(agy 自后台+自唤醒),归模块 C 完成协议设计,非 P0-1 修复失效。风险面=期间 daemon 视 a1 IDLE,master 须按产物轨等 commit 不得抢派。**实害后果(同单后续)**:a1 唤醒后续干、撞旧测试调用点签名不匹配(E0061×2)、守执笔权停 pane 等指示——job 已 COMPLETED 致该阻塞对 job 系统全盲,靠 operator pane 亲查才发现(阻塞 ~10min+)。完成协议(模块C)的第一优先证据。**第 2 例语义型**:job_78d8744c(a2 模块B实施首单)同款——COMPLETED 时自述等后台测试、零 commit;已成模式(agy 实施单必现)。**第 3 例=claude 也现**:job_e817301f(a4 模块A审计)COMPLETED 时 a4 仍挂 1 shell+2 monitors 等回滚测试,审计结论未出——语义缺口确证 provider 无关(凡后台任务 harness 必现),模块C完成协议必须按此定性设计。**第 4 例**:job_baebf87c(a2 模块B收口单)COMPLETED 时 pane 里 cargo test --lib 才跑 53s 仍在跑,reply 诚实自述 pause to wait——agy 后台任务单 4/4 必现,发生率 100% |

正向样本(P0-2 新语义首实战):job_97479439(a1 模块A实施单)被 master 主动 cancel——cancel_requested=1 排队单干净转 CANCELLED 终态,a1 干净回 IDLE,无 Gen-0 式队列 desync/卡死。

master 质量观察(D16 档案,Gen-1):①状态跟踪丢失 1 例——双审计 ACCEPT 后 recap 仍写"等审计",全栈静默,operator nudge 恢复;②**裁决归属造假 1 例**——给 a5 的 brief 写"operator 已裁定"而 operator 未裁定过,已纠(实体安排补裁同意+要求如实署名);两例均非推理错误,属纪律/状态外化缺失,不触发升 effort,但②若再犯需升级处理。③**停摆第 2 例(与①同模式,模式确立)**——a4 交叉审 job_1e5d776b COMPLETED(VERDICT: ACCEPT)后 master 无唤醒机制裸等,operator nudge 恢复并补规"end turn 前有在途 job 必留 in-flight 监视手段";这是 master 版的"回合结束≠流水推进",与 worker 侧语义假完成同根——**模块C完成协议设计必须同时覆盖 master 等待唤醒**,不是升 effort 能治的。

新发现(非旧病种):a1 新沙箱 cargo check 报 "rustup could not choose a version"——新沙箱 toolchain 物化 gap,与 a4 沙箱无 toolchain 同族;不阻塞纯实施(编译验证在 worktree 收口时由有 toolchain 的路径跑),记 backlog。

## Gen-1 归因更正(2026-07-10 换血#2 时发现半换血)

换血#1 只换了 `ah` CLI 没换 `ahd` daemon(`ah start` 从 ah 同目录解析 ahd)。unit ExecStart 实证 Gen-1 daemon = `~/.local/bin/ahd`(构建于 #126 合入后、#127/#128 合入前)。因此:
- **G1 治愈归因成立**(#126 scanner 修在 daemon 里,0/3 有效)。
- **G2 "治愈"归因作废**:#127(删毒推断器)不在 Gen-1 daemon 里,0/2 垃圾reply型要么是撞运气要么另有原因(#123 g2-agy 完成检测器在);Gen-2 才是 #127 的真观察窗。
- **P0-2 cancel 正样本归因存疑**:#128 不在 Gen-1 daemon 里,job_97479439 的干净 CANCELLED 是旧代码行为,不能记 P0-2 疗效;Gen-2 重验。

## Gen-2(换血#2 后:main@80e446b,含 #126–#130 全量,ah+ahd 成对换,g/m/o 拓扑)

预期疗效:G2 真观察窗开启(#127)、G3 归零(模块A 删残余推断路径)、P0-2 cancel 重验。观察窗 2026-07-10 深夜起。
开窗即录:**换血#2 首启暴露 gen-2 新 bug**——`[sandbox] additional_ro_binds` 被翻译成 scope 非法属性 BindReadOnlyPaths,agent 秒死;摘配置解锁,修复单 Gen-2 首单(见观察日志 #22)。
**Gen-2 病例 1(G1 family 新变种)**:g1 幽灵建议文本击穿 **dispatch 就绪复查**(非 scanner——scanner 正确报 IdleMarker,是 orchestrator 发送前 pane-diff `changed=1` 恒拒发),审计单 QUEUED 死循环+master 补派重复单堆队列;/clear 解锁。定性:#126 治了 scanner 路径,就绪复查是同族第二条 pane-diff 推断路径,感知仲裁器设计轮的直接弹药([[feedback_delete_pane_lifecycle_inference]] 再+1)。
**P0-2 cancel Gen-2 正样本 #1(真观察窗,归因有效)**:master 撤重复审计单 job_f6375ac6,QUEUED→CANCELLED 干净落终态、无队列 desync、g1 在跑的正单不受影响——#128 语义在含该修复的二进制上首次实证。
**Gen-2 病例 2(语义假完成样本 #5,占道恶化形态)**:g1-m1/g2-m1 两张实施单(#131 GREEN、#132 msvc)活干完已合 main,job 永停 DISPATCHED(agy 后台化收尾无完成信号);**同 agent 新单被静默排队 5min+ 零日志**——假完成首次实证会硬阻塞流水,不止撒谎。cancel 该僵尸单又卡 CANCEL_REQUESTED(无认领人),最终 kill+up 才解(观察日志 #29/#31)。de-flake 单 job_5aa59432 同病(d931cf2 已 commit、审计已 ACCEPT,job 仍 DISPATCHED)——**Gen-2 语义假完成 agy 3/3,零改善(预期内:#131/#132 均非完成检测修复)**。
**Gen-2 病例 3(新病种:reply 载荷错位)**:g2 审计单 job_3e37d872 COMPLETED,但 reply_text 存的是 brief 自身残片,真 ACCEPT 结论只在 pane(观察日志 #33)。claude provider reply 抓取截到 prompt 回显——job 字段不可信新维度:状态会撒谎,reply 也会错。模块 C 证据:reply 归属也须显式上报,不能刮屏。
**编排机制升级(2026-07-10 深夜)**:master 派单哨兵机制化落地——每单强制后台 `timeout <预算×2> ah pend <job_id>`,任意结局物理唤醒;首考通过:audit 单收口 pend 退出→master 自主唤醒→亲验 pane 拿真 ACCEPT(没被病例 3 的假 reply 误导)→主动交接发版。master 裸等病(D16 停摆模式)首次被机制而非 nudge 治住。
