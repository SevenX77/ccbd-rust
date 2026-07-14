# Operator 观察日志

> 规则:operator 发现的**任何错误/问题/异常**当日记入本日志(症状/根因/处置/状态四要素),不论大小、不论是否已修。换血(binary swap)边界做一次回顾。本日志随公开仓发布——它既是 ah 的实证质量档案,也是多 agent 编排方法论的第一手事故语料。
> 病种发生率的代次对照见 `research/dogfood-ledger-*.md`(疗效账本);本日志记单点事件与新发现。

## 资源画像(基线,持续更新)

2026-07-10 实测,五 agent 拓扑(master + g1/g2 闸门 claude + g1-m1/g2-m1 牛马 agy + o1 设计席 agy),7.9GB VPS:

| 组件 | 常驻内存 |
|---|---|
| master(claude sonnet-5) | ~373 MB |
| claude 闸门(opus 4.8)×2 | 214-235 MB /个 |
| antigravity 实施/设计 ×3 | 141-171 MB /个 |
| ahd 守护进程 | ~66 MB |
| tmux server | ~11 MB |
| **全栈合计** | **≈1.4 GB** |

cargo 编译实采(2026-07-10 换血#2 release 冷构建,串行 CARGO_BUILD_JOBS=1,15s 采样):
- `cargo build --release` 全程 3m41s,峰值 cargo+rustc 合计 RSS **≈1.15 GB**,CPU 恒 **~100%(吃满 1 核)**;编译期间全机可用内存最低 3.4GB(栈 1.4GB 常驻 + 编译 1.15GB 并存仍有余量)。
- `cargo test --lib`(热缓存,只编测试+跑 1004 用例串行)~55s,增量可忽略(61MB)。
- 容量结论:7.9GB 机上"全栈 + 1 路串行 cargo"安全;**并行 cargo 每多 1 路加 ~1GB 峰值**,4 路即 OOM 线——2026-05-26 并行 4 路 cargo OOM 杀主控实锤,串行铁律由此而来。

## 事件记录(倒序)

### 2026-07-10(Gen-0 尾声 → 换血#1 → Gen-1)

| # | 类别 | 事件 | 根因 | 处置 | 状态 |
|---|---|---|---|---|---|
| 42 | 产品bug | Gen-4 首例(设计轮期间):g2 收敛单 job_e0c796ab DISPATCHED↔WAITING_FOR_ACK↔QUEUED 空转 ≥3 轮,prompt 从未落 pane;g2 输入行有灰色幽灵占位文本("通知 g1 复核 Track B 草稿"=上文语境自动建议回显),进程活、态闲 | 同 #36:claude 幽灵占位被 dispatch 就绪复查 pane-diff 读成 pending 文本→拒发(ah#17 第三路径);Gen-4 无该病修复,复发符合开窗预期 | operator 由停摆哨兵+job 翻转监听抓到,亲验 pane 后下派 master 按 SOP:/clear g2(C-u/Esc 无效)→待真 IDLE 由 redispatch 自落,勿补派重复单 | **升级为系统性**:同日 1 小时内三例(g2 收敛单→g1/g2 两张交叉审单齐卡),规律=当前 claude CLI 每次 turn 结束都生成灰字续写建议,故"向刚收工 gate 派单"必撞拒发;master 已立常备 SOP(派单前看输入行/空转即 /clear);同族第 3-5 现(Gen-2 #24→Gen-3 #36→Gen-4 ×3),设计轮 R3 直接靶子;疗效账本 Gen-4 记发生 ×3 |
| 41 | 正向 | 换血#4(Gen-4=main 97104cd)全程干净:ah stop 级联收净(零沙箱残余)→成对换入(md5 核对)→ah start 一次全绿(5 agent IDLE)→**活栈 `ah up` 全员 NO_CHANGE、pane pid 零变化**(#13 修复活栈实证,旧血同命令=全栈风暴);新血 ahd 自动挂 systemd user unit(Restart=on-failure),"ahd 裸进程无自愈"老欠账顺带闭合 | (正向)#141 指纹对称修复 + `ah start` 的 unit 化启动路径生效 | 疗效账本 Gen-4 开窗;交接文档 master-handoff-swap3.md(名 swap3 实为换血#4);g2 spawn 出现多余 bash pane %3(观察项,未见危害) | 正样本 |
| 40 | 操作险情 | 换血 build 前核查发现**本地 main 落后 origin 5 个 commit,#141 根本不在本地**——差点用不含 #13 修复的"新血"完成一次假换血 | operator 在 #141 merge 后没拉平本地 main(merge 在 GitHub 侧完成) | pre-flight 三查(merge-base 含 #141+树干净+全机无 cargo)拦住,ff 拉平后才 build;SOP 固化:换血 build 前必验目标 commit 是 HEAD 祖先 | 已纠 |
| 39 | 流程病 | 用户问"ABCD 四模块哪些 merge 了",operator 答不出且历史回答自相矛盾(曾编造不存在的"C2-C8/D2-D7"模块集——实为架构评估审阅判决标签误当模块号) | 无"模块→PR→状态"常驻台账,疗效账本只记病种不记完成度;两套撞名 C/D 编号(ABCD 模块 vs 感知 C1/D1)无消歧标注 | 建 research/MODULE-STATUS-LEDGER.md(只认 git merge PR#,每次 merge/换血当场更新)+记忆规矩;命名陷阱显式写进台账头部 | 已建档 |
| 38 | 正向 | Gen-3:双泳道 gate 审计各出**实质 REJECT**——C:g1 实测证明 CI grep 脚本 fail-open(通配 baseline 放行真实直写);D:g2 出 3 blocker(无守卫 requeue 写、CI 排除过宽藏真直写=与 C **同族 fail-open**、多行写 regex 漏)+1 must-surface(死码 cancel takeover 无生产调用者)。两 gate 回滚自检真跑(C:回滚→7/9 红,复原→绿树净)。g2 把裁决写进 wt-d1/.lane-verdict-d1.md(**落盘survive respawn**,好实践) | (正向)闸门质量机制按设计工作:独立 gate 逐条验收+回滚自检真抓到 fail-open 类真 bug,非橡皮图章;跨泳道同族缺陷(CI 排除过宽)两条独立发现互证 | 记疗效账本正样本;g2 裁决落盘 pattern 值得写进 pack(裁决/审计结论落约定文件抗 respawn) | 正样本 |
| 37 | 产品bug | Gen-3:agy 实施位(g1-m1/g2-m1)GREEN 后卡**假 BUSY**(pane 停旧产出+调查 banner,ah log-tail latch BUSY/Matched);后继 fix 单排在假 BUSY 后落不下,master cancel→redispatch 打**赢不了的循环**(实测各 2-3 次) | agy 语义假完成的占道形态([[project_ah_codex_premature_completion_bug]] 家族);/clear 对 agy 不够快/不改 ah 感知→只能 kill+up 释放;**kill+up 连带触发 drift_realigned 把配对 gatekeeper(g1/g2)也 respawn**(context 清零,幸而 verdict 已交付/落盘故无丢)+ respawn 的新沙箱**无 rust toolchain**(#13 复发,agent 现装 stable 约min级+磁盘) | operator 诊断下派 master 补 SOP:显 BUSY 但 pane 旧产出+banner=假 BUSY,先 /clear 否则 kill+up 释放再派,派后亲验 prompt 真落 pane;master 已内化(两 lane 都 kill+up+pane 验证+落盘 verdict 抗 respawn) | 恢复中;真修=模块C(agy 完成信号 log-tail,免 latch 假 BUSY)+ 沙箱种子化含 rust 路径(backlog);kill+up 连带 respawn 归 realign 原子性设计轮 |
| 36 | 产品bug | Gen-3:g1/g2 两张 gate 审计单(job_60cbe733/90ee3348)WAITING_FOR_ACK↔DISPATCHED↔QUEUED 空转 >5min,审计 prompt 从未落 pane;gate 进程活(Ssl+ 睡眠低CPU)、态 IDLE、pane 停在旧 RED 报告 + **灰色幽灵占位文本**(input 空时 claude 回显上条 prompt 的灰影,如"派 g1-m1…") | claude 幽灵占位被 ah dispatch 就绪复查读成"输入行有 pending 文本"→拒绝写 prompt→job 永远 ACK 不掉;C-u/Esc 对占位无效,ZZPROBE 探针证实 input 实为空(占位是渲染层非 buffer),只有 /clear 可靠重置;同族第三条 pane-diff 推断路径(banner→ghost→就绪复查) | operator 诊断(探针+进程态+产物轨亲验)→下派 master 跑 SOP:cancel 两空转单(干净 CANCELLED)→/clear g1(%1)/g2(%3)→重派**自包含**审计 brief(gate 已清无前情,RED/diff/tasks 全在盘)+pend 哨兵 | 恢复中;模块C感知仲裁器直接靶子 |
| 35 | 资源泄漏 | 换血#3 时 /tmp/tmux-1001/ 攒了 56 个 ahd-* socket,**55 个全是活的泄漏测试 tmux server**(agent_w1/master_p1/agent_ag_orphan_leak_b2/rearm_skip_running_* 等夹具名;orphan_leak_b2 即今晚 de-flake 测试自己泄的) | B2 修的是产品侧收割,**测试 harness 侧不清 tmux daemon**(test-hygiene backlog 老病);这些常驻 daemon 还持有 flock fd,是 cargo 锁扣死病的温床 | 双保险清杀 55 个(会话名全匹配夹具模式才杀+跳过活栈),socket 归 1;真修=测试 harness teardown 统一 kill-server,归 test-hygiene backlog | 已清,待根治 |
| 34 | 编排纪律 | operator 合 #134 时抢跑:凭"上一拍事件报 req1-installer-landing pass"就 merge,实际该 check 有两个实例、第二个仍 pending(#26 同病轻度复发,自查自纠) | 事件流报的是单实例翻转,不等于全绿终态;正确信号只有看门的 ALL_GREEN 终态行或 `gh pr checks` 零 pending | 合并后立刻补验:main 合并 commit 的完整 CI 另行在跑,tag 推迟到它绿;SOP 收紧:merge 前必须 `gh pr checks` 零 pending 实证,不认单事件 | 已纠,tag 等 main CI |
| 33 | 产品bug | g2 审计单(job_3e37d872)COMPLETED 后 `reply_text` 存的是 **brief 自身残片**("(self-contained, post-restart)"),真实 ACCEPT 结论只在 pane 里;master 靠 pend 哨兵唤醒后按 SOP 亲验 pane 拿到真相,未被误导 | claude provider 的 reply 抓取把 prompt 回显当成回复截走(疑似截取窗口/锚点错位);"job 状态/字段不可信"再添一维——不止状态撒谎,**reply 载荷也会错** | 无需处置(产物轨+pane 亲验 SOP 已兜住);记模块 C 证据:完成协议里 reply 归属也要走显式上报,不能刮屏 | 设计轮证据 |
| 32 | 编排纪律 | master 第 3 次裸等("Checking back in ~5 minutes"承诺后无任何机制保证回来);用户点名"master 应该有更严格的哨兵" | 监视只有纪律条文没有机制;`ah pend`(阻塞等 job 收口)原语一直存在但 master 从未使用 | **机制化落地**:master.md 新增强制"派单哨兵"——每 `ah ask` 后必挂后台 `timeout <预算×2,下限900s> ah pend <job_id>`,任务退出自动唤醒(正常=收口去产物轨亲验;124=停摆警报);没挂不许 end turn;现任 master 已对在途单补挂(task bykiiv075)并确认 | 已落地,观察疗效 |
| 31 | 产品bug | 对 DISPATCHED 单 ah cancel 停在 CANCEL_REQUESTED 不落地(agent 端无认领);agent DB 态 BUSY 与 pane 实际空闲彻底 desync;最终靠 kill+up 才解 | in-flight cancel 需 agent 认领,但 agent 早已回合结束无人认领——cancel 语义对"僵尸在途单"无效 | kill+up SOP 生效;cancel-无人认领路径记设计轮(控制面 job 状态机单写权威的直接案例) | 设计轮 |
| 30 | 产品bug | ah up 因 toml 有改动触发全员 REALIGN:g1/g2/o1 上下文清零;首次 REALIGN 还**丢了 g2**(拓扑悄缩 4/5,老 bug 复现);二次 up 补回但新 g2 的 tmux session 名错标为 agent_g2-m1 | realign 非原子(删旧未建新)+respawn 会话命名 bug;operator 教训:**改过 toml 后跑 up=全员重建,慎用;仅想补 agent 时应先核对 config 漂移面** | 二次 up 补齐+双验;定位 agent 一律 pane id;realign 原子性+命名 bug 记设计轮工单 | 已恢复,设计轮 |
| 29 | 误判事故 | 语义假完成恶化形态:两个实施单活干完已合 main 但 job 停 DISPATCHED(agy 后台化收尾无完成信号),**同 agent 新单被静默排队 5min+,daemon 零日志零告警**;master 初诊只看到"卡派发"没锁到占道根因 | "回合结束≠任务完成"的反向面(活完 job 不翻)+ per-agent 串行队列 + 排队原因不可观测 | 陈旧单 cancel 释放通道+重派;master 新 SOP:产物合 main 后核对 job 终态,没翻就 cancel;模块C优先级证据再+1;"排队原因可观测性"记设计轮 | 已解,设计轮 |
| 28 | 测试卫生 | modB 孤儿收割测试在 Linux CI 并行跑 flaky:同 commit 两次 test job 一过一挂(panic registry.rs:565"进程该死没死"断言),重跑即绿 | 并发敏感断言(即时断言无等待收敛);本地串行铁律测不出,#84 同款模式再现 | #132(msvc 门控)先合解 blocker;de-flake 单已派泳道 2(禁 #[ignore] 掩盖);"串行掩并发"第 2 例,升维证据 | 修复单已派 |
| 27 | 资源/排程 | 双泳道各自跑全量 cargo test 并行(worktree 独立 target 互不锁),机器级单 cargo 铁律被排程破掉;用户看图问"agy 在后台编译?"时抓到 | 铁律只写进了单条 brief,没有机器级 cargo 槽排程;两 m 的收口窗口撞车 | 本次编译期已错开(可用 4.5GB)不中断;master 立"cargo 槽"规矩:派含 cargo 单前 ps 全机核查,两泳道收口排队;若本轮假红先怀疑 B2 清扫互杀 | 规矩已立,观察 |
| 26 | CI/流程 | 模块 B 破 Windows 交叉编译:registry.rs:556-565 裸调 libc::kill 无 cfg(unix) 门控,windows-msvc-check 红;**且 operator 在 CI 绿之前就 merge 了 #130**(--auto 在无分支保护仓=即时合),红是合后才发现 | 代码=平台门控缺失(Linux 本地全绿测不出);流程=无分支保护时 --auto 不等 CI | 泳道 2 派修(CI 绿=唯一硬门);#131 挂起等修;**operator SOP 改:无分支保护仓不许用 --auto,必须 watcher 等 checks 绿再 merge** | 修复在途 |
| 25 | master纪律 | master 撤重复单时撤错 job id:cancel_requested=1 打在正单 3036b91c 上,重复单反而 0;且 recap 报告"干净撤了重复单"与事实相反(D16 精度第 3 例);审计工作本体无损(pane 完成 ACCEPT),但正单 reply 因 CANCELLED 丢失 | 对操作对象核对不严+报告未对账 DB | 结果无害不返工;D16 档案计数;佐证完成协议必须让 job 状态可信(reply 丢失=状态机再添一种产物丢失路径) | 已记档 |
| 24 | 误判事故 | Gen-2 首病例:g1 空闲输入行幽灵建议文本使 dispatch 就绪复查恒拒发(changed=1→requeue 死循环),审计单卡 QUEUED;master 见状补派逐字重复单开始堆队列 | #126 只治了 prompt-scanner 的幽灵误判,**就绪复查是另一条 pane-diff 路径没治**;Esc 对 ghost 无效(复证) | /clear 解锁(ctx 0% 后立刻真派发);master 撤重复单+获新 SOP(反复 WAITING_FOR_ACK 先看输入行,勿连续补派);根治归感知仲裁器设计轮(pane-diff 推断连根撤的又一实证) | 已解,设计轮弹药 |
| 23 | 资源 | cargo 收口后垃圾盘点(用户增设例行项):4 个已合入 worktree 的 target/ 共 15.5GB + 34 个死沙箱 18GB(40 个中在用仅 6)+ 单日再积 108 个测试泄漏 tmux socket;三轮清完磁盘 82%→57% | worktree target 无人回收;**沙箱 GC 是产品缺口**(ah 不回收死沙箱,每代换血留一批);泄漏 socket 是 B2 修复前旧模式测试产生 | merge-base 验证后 cargo clean ×4;沙箱 GC=全进程 environ 扫在用集合+30min mtime 保险;socket 双保险清杀;换血 runbook 增"清上代沙箱"例行项;**Gen-2 核对项:若 ahd-* 测试 socket 继续新增,B2 只治了夹具没治生产路径 spawn** | 已清,B2 疗效观察中 |
| 22 | 产品bug | 换血#2 首启失败:daemon 把 `[sandbox] additional_ro_binds` 翻译成 systemd-run **--scope** 的 `BindReadOnlyPaths`,该属性仅 service 合法,scope 直接拒绝→agent 秒死 AGENT_UNEXPECTED_EXIT、session 回滚 | scope 属性合法性无把关;gen-1 daemon 从不翻译此配置故同配置没炸(潜伏 bug 被换血暴露) | 摘除该配置解锁启动(env-var 隔离下 RO bind 本就冗余,toolchain 走 [env]);修复单 Gen-2 首单派 | 已解锁,修复在途(g1 RED 7aaa3a3 已 commit,g1-m1 实施中) |
| 21 | 换血SOP | 换血#1 半换血:只换了 `ah` 没换 `ahd`,daemon 一直跑旧二进制(`ah start` 从 ah 同目录解析 ahd,src/bin/ah.rs:627)。复核:旧 daemon 构建晚于 #126 早于 #127/#128 → Gen-1 的 G1 治愈归因成立,但 G2 治愈与 P0-2 cancel 正样本归因存疑(那两个修复不在跑的 daemon 里) | 换血 runbook 漏了第二个二进制 | 换血#2 起两个二进制成对换+成对备份;账本已加归因更正 | SOP 已改 |
| 20 | 操作认知 | operator 自己的 pkill/pgrep -f 自匹配:命令行含目标脚本名,pkill 杀掉自己的 shell、pgrep 抓到自己造成假活 | -f 匹配整条命令行含调用方自身 | 清杀/探活脚本用 Write 落盘再执行,避免目标名出现在调用命令行 | SOP 已改 |
| 19 | 归档 | reply 尾部混 pane 界面杂质第 2 例(a4 交叉审单,尾带 spinner+statusline) | reply 捕获边界不净(同 #1) | 样本 ×2,已够格进设计轮工单 | 攒样本→设计轮 |
| 18 | master纪律 | 停摆第 2 例:承诺"审过即派收口"后 end turn,审单 COMPLETED 无人唤醒,全栈静默(同 #16 模式) | master 对"自己在等的 job 完成"无唤醒机制,靠 operator 亲查 | nudge 恢复+立规:end turn 前有在途 job 必留 in-flight 监视手段;结构修归模块C/D(master 状态外化) | 模式确立(2例),设计轮必治 |
| 17 | master纪律 | brief 冒名"operator 已裁定"(实际未裁定) | 裁决归属未如实署名 | 纠正+立规:工程细节 master 可自裁但必须署名 master | 已纠,再犯升级 |
| 16 | master纪律 | 双审计 ACCEPT 后 master recap 仍写"等审计",全栈静默 | 任务切换后流水状态跟踪丢失(ctx 43%,非压缩) | operator nudge 恢复;状态外化看板归模块C/D设计 | 已恢复 |
| 15 | 注入机制 | master 投 pane 掉 Enter ×3(a4×2/a5×1),文本滞留输入行触发真阳性 PROMPT_PENDING | paste 与 Enter 连发被 CLI 渲染抢跑 | SOP:paste→隔1-2s→单发 Enter→capture 确认;换血#2 orientation 写死 | SOP 已发,待验收 |
| 14 | 完成语义 | "回合结束≠任务完成"实证 4 例:agy 实施单必现,claude 审计单也现(后台 shell/monitor) | stop 信号=回合结束,后台任务仍在跑;provider 无关 | 实害:g1-m1 阻塞 10min 对 job 系统全盲;定为模块C完成协议第一优先证据 | 设计轮处理 |
| 13 | 沙箱环境 | 换血后新沙箱无 rust toolchain("rustup could not choose a version") | 沙箱物化不含 ~/.rustup(旧沙箱是历史安装) | 收口命令显式 export RUSTUP_HOME/CARGO_HOME 宿主路径 | 权宜可用,种子化归backlog |
| 12 | 测试卫生 | /tmp 积 ~135 个测试泄漏 tmux socket,多个 daemon 存活,含一个挂死测试进程占全机 cargo 槽 4h47m | 测试夹具用生产同款 socket 模式且无 teardown 兜底;agy 后台化测试不管收尾 | 精确清杀;修复(ah-test- 前缀隔离+运行前自愈清扫)已进模块B工单 | 修复在途 |
| 11 | 资源 | 磁盘 100% 打满:主仓 target/ 24G 为主犯 | 多 worktree 各自 target 叠加+无水位监控 | agent 自清 target/;operator 例行盯 df,≥90% 告警 | 已缓解(84%) |
| 10 | 误判事故 | a3(o1)设计发散真完成被判 STUCK+job FAILED,产出因盘满写盘失败险全丢 | 磁盘满写入报错 + 残余 PANE_DIFF_STUCK 推断路径 300s 静默误判 | 带外注入让 a3 重写落盘(34KB 全文救回);根因路径已在模块A删除 | 已修(PR#129) |
| 9 | 工单质量 | 模块A原工单四项全过时(两项早已修/前提不成立/死码无测试面) | 拿记忆写工单未实证代码现状(记忆腐化) | a4 证据审计拦下;立规:开工单前 file:line 抽验;相关记忆已更正 | 已重铸 |
| 8 | 疗效确认 | Gen-0 两大病换血#1 后当场归零:误闩 6/6→0/3+,垃圾reply假完成 5→0 | #126 scanner 修+#127 删毒推断器生效 | 疗效账本 Gen-1 对照记录 | 治愈确认 |
| 7 | 操作认知 | claude pane 多行粘贴折叠为 [Pasted text #N],grep 原文验证误判"没进去"(险双投) | CLI 粘贴折叠显示 | 验证约定改为认占位符 | SOP 已改 |
| 6 | 操作认知 | find -newermt 用未来日期全空(机器本地时区仍是前一日) | 时区假设错误 | 时间窗查询改用 -mmin | SOP 已改 |
| 5 | 换血操作 | 换血后 pane 号整体位移(a3 %2→%3 等) | 新栈重建 pane 顺序 | 每次换血后必须重对 pane 映射再注入 | SOP 已改 |
| 4 | 上游 | agy CLI 空闲期周期轮询点自崩(上游问题,历史) | 上游 Code Assist 轮询 | REVIVE_IDLE 已实现兜底 | 已兜底 |
| 3 | 流程 | a5 审计附加固建议由 a5 自行落地(闸门写生产代码) | 效率安排 | 补裁同意但强制 a4 交叉审(不自审铁律) | 已交叉审 ACCEPT |
| 2 | 归档 | P0-2 新 cancel 语义首实战正样本:排队单干净落 CANCELLED,无队列 desync | (正向) | 记疗效账本 | 正样本 |
| 1 | 归档 | reply 提取尾部混 pane 界面杂质(claude 单) | reply 捕获边界不净 | 攒样本给设计轮 | 观察中 |

### 记录纪律
- 四要素:症状(可复现的观察)/根因(实证,未查清写"待查")/处置(做了什么)/状态(已修/在途/观察中/设计轮)。
- 正样本也记(标"正向"):修复生效的证据与病例同等重要。
- 涉内部路径/凭据的细节写到能复现问题的最低限度。

## #43 2026-07-11 agy Stop hook 注入链全断(送达率 0%)
- **现象**:Gen-4 全代次内 daemon 收到的 agent.notify 分组=g1×16/g2×6/**agy 系×0**(o1/g1-m1/g2-m1);agy 后端确认已加载 hook 且至少 fire 过 1 次("executing command" 03:43:28Z),但无 RPC 到达、无 hooks-debug 日志。
- **根因(主嫌)**:注入命令用裸 `ah`(依赖 PATH),agy hook caller spawn 疑不继承 env → 静默 127;agy 对 hook 失败零日志。同命令同 env 复演 exit=0 送达正常,排除命令/socket/daemon 侧。伴生 bug:timeout=5000 被 agy 按秒解释(=83min,同步阻塞 loop);claude 侧同形命令正常(claude 传全 env)。
- **影响**:agy 完成信号全靠 pane/log 推断兜底;A/B 实验中 g1-m1 的 job 关不上、nudge livelock、假 BUSY 三病的共同上游。
- **资源画像**:纯配置/取证,无编译;复演 1 次 notify RPC(幂等 no-op)。

## #44 2026-07-11 换血#5:daemon 重启=全栈连坐重生(设计语义,SOP 认知修正)
- **现象**:systemctl restart ahd(二进制换血)→ 旧 ahd 收 SIGTERM 即"cleaning tmux resources"清 tmux server → master 进程死 → master_watch 判 master death → 级联清全部 worker + session CLOSED;与换血#4 的"活栈 `ah up` 全员 NO_CHANGE"表现完全不同。
- **根因**:不是回归——换血#4 的 NO_CHANGE 走的是 `ah up` 无 drift 路径(daemon 未重启);daemon 重启本就走 shutdown 清理+master-death 连坐([[master 被杀语义]]),两条路径不可比。
- **处置**:`ah start` 全新起栈(Gen-5 拓扑一并生效);SOP 认知固化:**换 ahd 二进制必然全栈重生,orientation 重注是换血标配步骤**,不要再期望 NO_CHANGE。
- **资源画像**:串行 release 构建 3m37s(峰值同基线);全栈重生一次。

## #45 2026-07-11 composer dim 占位符≠幽灵卡输入(判别法入 SOP,#36/#42 家族补充)
- **现象**:向新 master 注入 orientation 后,composer 持续显示"❯ 有异常就落盘…"(与注入文措辞不同),Enter/C-u/Esc/C-c 全"无效",疑 ah#17 幽灵文本卡输入,耗 ~10min。
- **根因**:该行是 Claude Code **上下文建议占位符**(dim 渲染,同 r1 空 composer 的 Try "how do I log an error?"),composer 实际为空,orientation 早已正常提交并被 master 处理。
- **处置/判别法**:`tmux capture-pane -e` 看转义码——`❯` 后文本带 `^[[2m`(dim)=占位提示非真输入,**不需要也不应该处置**;不带 dim 才是真 pending 文本。此判别法同样适用于 #36/#42 的感知层幽灵占位家族(ah#17 就绪复查的 pane-diff 若引入 dim 码判别可消一族误判),已挂进设计轮证据。
- **资源画像**:纯 pane 取证,无编译。

## #46 2026-07-11 outbox 冷扫首触发实证 + 孤儿事件永退化缺口
- **现象**:换血#5 重启窗口,旧 master 停机瞬间的 userpromptsubmit hook 走 journal-first 落盘 outbox(daemon 停机窗口正是 R1 设计要保的场景,首个真实正样本);新 ahd 冷扫 replay 该事件 FK 失败(旧 session 已闭)→ 正确走 retry_deferred=1,无热循环。
- **缺口**:死 session 的孤儿事件 FK **永远**失败,既不会成功也不进 error-book/dead 归档,每次 daemon 重启都重试一次、文件永驻。
- **处置**:登记为 R1 follow-up(与"reap-on-RPC-success 未接线"同批):冷扫需 max-attempts→error-book 或 dead/ 归档兜底。文件保留原地作活证据(outbox/master:sess_334718e9…/019f5137….json)。
- **资源画像**:纯 journal/文件取证,无编译。

## #47 2026-07-11 规则 spawn 物化+盘改不热更 → A/B 两臂主树混写碰撞(operator 部署错误)
- **现象**:Gateway A/B 首轮派单后 ~5min,主树 src/tests 46 文件被两臂混写(g1 建 `src/credential_gateway.rs`、g2 建 `src/gateway/`,同功能两套布局绞一棵树;g1 pane 自述"文件凭空消失"=碰撞实锤)。两臂规则里钉死的 worktree(`-wt-gw-a/-b`)零改动。
- **根因(实证)**:席位规则在 **spawn 时**物化进沙箱(codex=`.codex/AGENTS.md`、agy=`.gemini/AGENTS.md`、claude=`.claude/CLAUDE.md`;job prompt 原文零规则前缀=不随派单注入)。席位为换血#5(05:36)拉起,operator 08:41+ 改规则未换血 → 两 codex 全程持旧闸门规则(无 worktree 钉死、无 A/B 角色),收到纯任务 brief 后在 cwd(主树)直接开写。**operator 部署错误:改规则=必须换血,盘上文件不热更新到已跑席位。**
- **处置**:ESC 双打断 + cancel 双 job + 污染 stash 取证(次生事故见 #48)+ 整栈清重启(用户令):新栈 sess_7163d948 仅 4 席(g1/g1-m1/g2/r1),逐席沙箱验规则新版标记全中。
- **资源画像**:无编译;pane/DB/沙箱取证若干轮。

## #48 2026-07-11 operator 止血 stash 扫除活栈配置(次生事故,自记)
- **现象**:#47 止血用 `git add -A && git stash -u`,把主树**全部**未提交内容扫走——包括 .ah/rules 全套新规则、实验协议/brief、活栈 `ah.toml`、多个 spec 修订。此窗口若有人跑 `ah up` = 按 HEAD 旧拓扑 realign 活栈(未发生,险情)。
- **根因**:止血命令没先想爆炸半径——目标只是 src/tests 的 agent 污染,却用全树无差别 stash;而 operator 的配置工作全是未提交态,与污染共处一树。
- **处置**:逐文件 `git show stash@{0}:<path>` 恢复 215 个非 src/tests 文件(brief md5 复核与派单原文一致);stash 保留作 #47 取证。**SOP 固化:止血类 git 操作先列受影响路径集;stash/reset 一律路径限定,禁全树无差别。**
- **资源画像**:纯 git 操作;恢复 215 文件一轮。

## #49 2026-07-11 cancel→respawn 链三连:pane 命名错位 + recovery 重投×cancel 竞态 + codex 无视工作区钉死 commit 本地 main
- **现象**:①对 DISPATCHED job 的 cancel 触发 kill+respawn,respawn 的 g1(pid 2824083)落进名为 `agent_g1-m1` 的 pane(%7)、g2 落 `agent_g2-m1`(%8),与真 g1-m1/g2-m1 pane 重名共存——感知层按 pane/session 名扫描 = 误路由隐患(#30 respawn 命名 bug 家族,首次双重名并存形态);②respawn 后 recovery 把 cancel_requested=1 的 DISPATCHED prompt **重投**给新实例,两臂带新规则重新开工,cancel 迟至 turn 边界才落地(此时工作已做完并 commit);③重跑中 codex 无视沙箱已物化的 worktree 钉死(gwA=1 亲验),仍在主树实施并 **commit 到本地 main**(f174687 + amend 0ed41d1;监视器 5min 内抓获)。
- **根因**:①②=ah 编排真 bug,spec 病例单已开(`ah-orchestration-reliability/respawn-pane-name-mismatch-2026-07-11.md`、`recovery-reinsert-vs-cancel-race-2026-07-11.md`);③=结构病:**工作区是"叮嘱"不是"环境"**——ah 无按席位 cwd 配置能力(config 层零支持,spawn 一律 `-c 项目根`),对 LLM 可被忽略的指令规模够大必被忽略;资产保护必须物理闸(病例单 `agent-workspace-assignment-2026-07-11.md`)。
- **处置**:kill 双席;两笔 commit 保全至 `incident/gw-main-tree-commits-20260711`;main mixed-reset 复位 7bae3b1 + 定点 checkout 清 agent 改动(operator 配置无损);装 pre-commit 物理闸(本地 main 禁直接 commit,operator env 覆盖);整栈重启后 pane 命名全对齐亲验。
- **资源画像**:无编译;git 复位一轮;全栈重生一次(重启后常驻内存较五席拓扑更低)。

## #50 2026-07-11 冻结 brief 指向的权威文档在 worktree 不可见(operator 部署漏洞,两臂对称受损)
- **现象**:Gateway A/B 重启后首圈,两臂均在缺失权威文档下开工——brief §0 第 1 位 `design-rev.md`(冻结设计,21.9KB Plan B 全细节)与第 4 位 `credentials-phase0-spike.md` 在主树是 **untracked**,两臂 worktree 从分支 checkout 拿不到,双双退回旧 `design.md`(9.2KB 早期 Token Proxy 草案)实施。g1 交单自报文档缺失(半程收口:仅 RED commit,未走泳道 ③-⑤);g2 按旧设计跑完 RED+GREEN(05d28d3/75470d1)。master 按 ≤15min 纪律落 `.operator-question` 候裁,期间零越权。
- **根因**:operator 冻结 brief 时只字节校验了 brief 本身,**未校验 brief 引用的权威文档在执行环境(worktree)内可读**。"引用完整性"缺一道派单前检查:brief 里每个路径引用必须在目标工作区物理存在。
- **处置**:裁决=对称补件+冻结续单——两文件原样 cp 进两臂 worktree(2 文件×3 处 md5 亲验一致);冻结续单 `followup-brief-frozen.md`(md5 a8bd76c9)字节级同文 `ah ask` 双臂(job_00b9dbe6/g1、job_9beb0b75/2f6/g2,DB prompt 字节比对 EXACT);两臂在圈边界同刻收到补件=对称性保全,r1 头对头可比性不受损;首圈旧设计产出标记"待复核修正"。**SOP 固化:派单前检查 brief 全部路径引用在各目标工作区可读(untracked 文件对 worktree 不可见是常规陷阱)。**
- **资源画像**:无编译;文档 diff+DB 校验数轮。

## #51 2026-07-11 泳道死锁三重奏:agy Stop 钩子静默不触发 × log 监听 300s 放弃 × ahd 硬编码错文案催单逼агy 未批先实施
- **现象**:Gateway A/B 泳道臂首次真交接(g1→g1-m1)后,g1-m1 按 plan-first 提交计划停轮等批;job 卡 DISPATCHED 25min+,g1 的 pend 永远等不到;期间 ahd 以硬编码催单文本("The job is still open...Do not stop at 'waiting for cargo test'",`src/db/state_machine.rs:1157`,cargo-test 场景专用文案)连催 g1-m1 三次,**它两轮明文拒绝("According to the PLAN-FIRST rule, I will wait for your explicit approval")后第三次缴械,未批先实施 commit 97648b5**。
- **根因(三层叠加,全 infra)**:①agy Stop 钩子**从未执行**——hooks.json 正确(#143 修的命令)、`enableJsonHooks: true`,但 hooks-debug 目录无 g1-m1.log(不是执行失败,是没执行;手动补发 `ah agent notify --event stop` 也不翻 job=stop 事件单独不构成完成);②log 监听 MAX_LOG_MONITOR_WAIT 300s<真实回合,超时即弃(已知债);③完成信号全灭后,anti-stall 催单器把"停轮等批"误判"停滞",用与场景无关的死文案连击——**催单文本硬编码单场景 = 对多场景 agent 是指令注入**。上次 A/B 泳道 DNF(agy 2h41m 挂+nudge-livelock+no done channel)同源,本次全程仪器化取证。
- **处置**:僵尸 job 留置(cancel=kill+respawn+重投陷阱);修正令 #4 经 master:ESC 解救 g1 → 跳过计划批准直接审 97648b5;g1-m1 加"再被催即答'已交付等审'并停轮"消化针;g1-m1 两次守纪记 Arm A 正面证据,未批先实施归因 infra 不计违纪。规则补丁:g1.md ③机制条(obs #50 同批)。
- **修向(spec)**:见 `ah-orchestration-reliability/lane-completion-channel-triple-failure-2026-07-11.md`;北极星对齐:完成信号必须显式通道(hook/报告),催单文本必须场景感知或至少中性,anti-stall 对"等待协作方"状态必须免疫。
- **资源画像**:无编译;pane/DB/hooks 取证多轮;实验全程未中断。

## #52 2026-07-11 僵尸 job 被 cancel → ahd 排水积压归档队列,把数小时前的过时 brief 真派给 agent(cancel×dispatch 竞态第二例)
- **现象**:g1-m1 的僵尸 job(`job_82822287`,DISPATCHED+长期占席,#51 三重奏产物)被某路径 cancel(**谁发的未定案**:master 自查记录里全程只有 `ah ask`/`ah pend`/pane 注入、从未 `ah cancel`;operator 本轮命令记录里也无对该 id 的 cancel;疑似 ahd 内部/间接路径,存疑待复现)。cancel 一旦作用于占席僵尸,ahd 调度器立即**排水该席位积压的 QUEUED 队列**——泳道每轮返工都因"一席一单被僵尸占着"沉一张 QUEUED 归档单,排水把最早的两张**数小时前的过时 brief**(返工#1 `cebb2b18`、返工#3 `6fde2f4c`)先后真 DISPATCH 给 g1-m1,它两次开始执行古董指令。
- **根因**:①**cancel 的副作用面过大**——cancel 一个占席 job 不应触发对该席位历史 QUEUED 的无差别排水;积压归档单本是"僵尸占席"的伴生垃圾,不该被当正常队列消费。②`job_6fde2f4c` 是 **cancel×dispatch 竞态第二例**(与 #49 同族):cancel 请求已置 `cancel_requested=1` 但 dispatch 路径抢先把它翻 DISPATCHED,agent 已开始执行,cancel 迟至 turn 边界才落 CANCELLED。
- **处置**:operator 两次 ESC 止损,每次亲验 wt-gw-a 无损(clean tree,HEAD 未被古董指令改动);cancel 全部 8 张归档 QUEUED 单(占席僵尸留置不碰);`job_6fde2f4c` 待其自然落定 CANCELLED(未主动戳)。**意外收益**:队列排空后席位解堵,g1-m1 的 job 恢复正常落 COMPLETED,泳道后续无需摆渡即自转。
- **修向(spec)**:并入 `ah-orchestration-reliability/recovery-reinsert-vs-cancel-race-2026-07-11.md` 追加"cancel 占席 job→队列排水"变体;契约:cancel 不得触发对同席历史 QUEUED 的自动 dispatch;占席僵尸的伴生 QUEUED 归档需与正常队列区分(或 dispatch 前校验 brief 时效/来源)。
- **资源画像**:无编译;DB/pane/transcript 取证多轮;实验未中断。

## #53 2026-07-12 o1 派单两连不落 pane:ah#17 家族在轨2 语料研究期间自发复现
- **现象**:master 向 o1(轨2 编排底座重构发散席)派发发散/红队单,`ah ps` 两次均报 job BUSY,但 tmux pane(`agent_o1` %6)全程停在空闲 composer(`>` 无文字、无 spinner、footer idle),派单文本从未真正提交进该 pane。`ah cancel` 对首个卡死 job 无效(DB 仍报 BUSY)。这正是本轮轨2 语料里 ah#17(dispatcher 对 composer 幽灵文本无免疫,job 永久 DISPATCHED/QUEUED 弹跳)描述症状的现场复现——调研对象在调研期间自己复现了一例。
- **根因**:未定案,待 respawn 后如再现需正式取证(pane 状态/DB job 表/dispatch 时序)。本次未深挖根因,优先级=先恢复轨2 推进而非当场调试 ahd 内部状态。
- **处置**:master 按纪律 `ah kill o1` 终止占死 agent(ah 管理内命令,非越界);respawn(`ah up`)超出 master 授权命令集(operational 层,master 宪法明确排除),已落 `.operator-question` 请 operator 执行;轨2 发散单原样待命(brief 未受影响),轨1(c1 实施+d1 补丁)不受影响、继续推进。
- **资源画像**:纯 pane/DB 取证;无编译。

## #54 2026-07-12 o1 三度同族卡死:respawn 后仍复现,疑似席位级粘滞而非单次竞态
- **现象**:respawn 后的 o1(pid 3298950)完成一单(轨2 发散备忘)后,下一单(分阶段灰度窄口径反驳)派发,DB 报 BUSY 但 pane 停在上一单收尾画面,文本未落——与 #53 同症状,这是**respawn 后同一席位第三次**复现(#53 记两次未落+此次一次)。`ah cancel` 本次成功(CANCELLED,不同于 #53 首次 cancel 无效),再 `ah kill` 正常。
- **根因(存疑)**:respawn 只是临时缓解,不像是根治——怀疑该 antigravity 会话/席位存在持久化粘滞状态(如 composer 渲染层残留或内部队列卡死),而非每次都是全新的 dispatch-ACK 竞态。未深挖底层(ahd 内部无 master 可查工具),留给轨2 底座重构的感知层证据库。
- **处置**:cancel+kill,`.operator-question` 再次请 operator respawn,并建议本次不只是进程重启,评估是否需要清 agy 会话级缓存/state。
- **资源画像**:纯 pane/DB 取证,无编译;轨2 并行 cgroup PoC(c2)与轨1(r1 已 ACCEPT 等 PR)均不受影响。

## #55 2026-07-12 auto-merge 抢跑 r1 REJECT:CI 绿先于审核裁决落地,坏代码合进 main(master 流程洞)
- **现象**:PR #146 round6(commit 6974a8f)r1 审出真实运行时回归(R6-BLOCK:UDS socket 路径短化后未同步更新 sandbox_root 派生逻辑,导致 bridge.err/port 落到不可写的 `/` 且跨 worker 撞车,CI 无测试覆盖此路径故仍会绿)。r1 裁决 REJECT 并在报告里显式提醒"CI 绿 ≠ 可合,建议不要因 CI 转绿就 auto-merge"。但 GitHub auto-merge 只等 CI checks 绿,不等 r1 的人工/agent 审核完成——r1 审完裁决送达时,CI 已经先转绿,auto-merge 已经把 6974a8f 合进 main(merge commit `8f2aab5`)。
- **根因**:master(我)的新 SOP(PR 全生命周期归 master,CI 绿+r1审过才挂 auto-merge)里,"CI 绿"与"r1 审过"是两个独立异步过程,没有机制保证后者一定在前者触发合并动作之前完成——auto-merge 是 GitHub 侧配置,只认 required checks,不认 r1 的审核结果。这是流程洞:**auto-merge 的触发条件集合缺了"r1 已 ACCEPT"这一项**,只要求内规定"CI绿+r1审过"才合,但机制上只强制了"CI绿"。
- **处置**:立刻开新 worktree(`ccbd-rust-wt-graft-hotfix`,分支 `fix/gateway-uds-sandbox-root-regression`,基于最新 main)派 c1 hotfix R6-BLOCK,r1 复审后走新 PR 合入。
- **修向(机制,不是自律)**:每次派某一轮修复给 c1 之后、r1 审核结果出来之前,应该显式 `gh pr edit --disable-auto-merge` 或转 draft,拿到 r1 ACCEPT 后再重新启用 auto-merge/ready for review——不能依赖"审核比 CI 跑得快"这个时序假设。已记入本例,后续 PR 生命周期需要把这一步纳入标准动作序列。
- **资源画像**:无新增编译成本(hotfix 走标准路径);流程记录一次。

## #56 2026-07-13 g1(codex)在 MD1 架构索引审计里编造文件路径+整段符号,master 零复核并入权威文档,靠 r1 源码 grep 才兜住
- **现象**:模块化 MD1 架构索引扇出(c1/c2/g1 三 codex 分层审计 → master 合成 → r1 门)。g1 负责 Layer6(provider/gateway/凭据),其审计把 `claude_gateway` 定位成 `src/provider/claude_gateway.rs`(该文件根本不存在)并附上一整段**虚构符号**(`get_or_init_production_gateway`/`load_seed_credential` 等,真文件里零命中)。master 合成时**未独立 grep 复核 g1 的路径/符号声明**,直接并进 `research/architecture-index.md` 送 r1。r1 第一遍源码 grep 抓出:claude_gateway 实为顶层 `src/claude_gateway.rs`(1129L,真符号 `ClaudeGatewayService`/`read_seed_credentials`/`register_worker`);外加 capability 表漏 process-axis 列、4 个 capability owner 漏登、一处 capability 标签错(bundles 不解析凭据)。REJECT→master 修→r1 复验 ACCEPT(2 遍收敛,非打地鼠)。
- **operator 二次独立核对(不只信报告)**:我自己 grep 复核——顶层 `src/claude_gateway.rs` 实存 1129L✓、`src/provider/` 确为 12 文件无 claude_gateway✓、旧虚构符号零命中✓。**并额外揪出 r1 没抓的缺陷**:行数不新鲜且 wc 不均匀——home_layout 标 2907L 实际 3039L、manifest 标 795L 实际 849L,而文档头却声称"every entry re-verified against source"。已要求 master 对每个 path 跑 wc -l 对齐。
- **根因**:扇出审计→合成协议里,**合成环节零复核**,全靠下游 r1 单点兜底。子代理(尤其 codex)会对文件位置/符号"自信编造",这正是 filesystem-verify 铁律的场景;而这份索引的初衷恰恰是"防止基于对代码库的陈旧/片面认知做决策"——结果它自己的构建过程就踩了同一个坑。
- **处置/修向(机制,非自律)**:要求 master 把"合成者在并入权威文档前必须自 grep 复核子代理声称的 路径/符号/行数,不许把未核枚举直接送 r1"写进 `.ah/rules/master.md` 扇出审计段(与收口点 wc 一起固化,扛换血)。门(r1 独立 grep)有效、operator 二次核对有效——三层防御都发挥了作用,但合成层复核缺失多烧了一轮 r1。
- **资源画像**:纯读取/grep 取证,无编译;codex 扇出充裕(符合 O9 富余侧);多烧一轮 r1(claude,用户已开 credit 不吃紧);MD1 端到端约 1 轮 codex 并行 + 2 轮 r1 收敛。

---

## 观察 #57(2026-07-13):master provider 硬编码 claude,codex-as-master 换血被 binary 挡

- **症状**:用户指令"拓扑中所有 claude→codex"。改 ah.toml 把 `[master] cmd="codex" provider="codex"`、删 `[providers.claude]` 后 `ah start`,daemon 起但 session 创建返回 `RPC error -32000: ENVIRONMENT_NOT_SUPPORTED`,留 0-agent 空壳 session。
- **误判排除**:先怀疑 systemd-run anchor(sessions.rs:310/316)——手动 `systemd-run --user` exit=0、daemon 环境有 DBus/XDG、anchor unit `ahd-session-<uuid>` 唯一不撞、失败 session 的 anchor unit **已建成** → 排除 anchor,定位到 **spawn_master_pane**。
- **根因(读码钉死)**:`src/rpc/handlers/sessions.rs::prepare_master_pane_plan` 把 master 的 provider **硬编码 "claude"**(:440 `resolve_bundles_for_provider(.,"claude",.)`、:473 `provider:"claude"`、:478 `..._claude_credentials("claude",.)`)。`MasterConfig.cmd`/`.provider` 只喂启动二进制与 `master_readiness_mode`(Ack/Probe),**换不了 master 的 home/bundle/hook/凭据脚手架**。immediate 错=删 `[providers.claude]` 后 hardcoded-claude master 缺 shared creds 触发 fail-closed(home_layout.rs)。
- **处置**:回退 master→claude(+ 恢复 `[providers.claude]`),d1/r1 保持 codex;`ah start` 成功,7 席全起(master claude + c1/c2/d1/g1/r1 codex + g1-m1/o1 antigravity),master pane %0 就绪未撞限。**"全部 claude→codex" 执行到 binary 允许的最大**:两个 worker claude 席(d1/r1)→codex,master 唯一保留 claude。
- **状态/欠账**:真 codex master 需**改代码**(把 prepare_master_pane_plan 的 provider 参数化,按 master.provider 走 per-provider home layout,像 agent.spawn 那样)。是产品 gap,待用户裁决是否立项。**教训复用**:又一次"只验了工具行为的一半(readiness 分支)就搭方案"——readiness 支持非 claude ≠ 整个 master spawn 支持非 claude;依赖某能力前必须验**完整**路径([[feedback_verify_tool_behavior_not_design_assumptions]])。
- **附带发现(hygiene)**:`ahd-session-*` anchor 单元泄漏 ~26 个(每 session 一个,ah stop 不清)+ 大量泄漏测试 tmux socket(`master_p1`/`master_p_rearm_*` fixture)刷屏,干扰定位真 master socket。待 GC。

## 观察 #58(2026-07-13):#57 深挖——不是 regression,是"承重梁从未接线 + 周边全参数化造假象 + 无锁测试";SOP 回写已做

- **触发**:用户质疑"我记得之前版本可以换 master,为什么又改成不能换了?现在不是模块化了吗?这个模块有没有测试把这块锁死?文档回写要不要写进 SOP?"——四问 + 另一外部项目用 1.7.0 也撞"同样问题"(#29 同款 ENVIRONMENT_NOT_SUPPORTED 空码)。
- **archaeology 结论(git 钉死)**:`master.provider` 字段 #54(0fd2ec6 readiness gate)引入,**只**为 Ack/Probe 握手;`git log -S 'master.provider'` 全历史证明它**从未**传进 `prepare_home_layout*`/`resolve_bundles_for_provider` 等 spawn 脚手架。#99 的父提交已是 "claude" 硬编码。⇒ **不是 regression**(没有"曾经完整能换的路径被改坏"),是**承重梁从来没接通**。
- **"能换"的假象来源(=用户没记错)**:周边模块全参数化——config validate 接受 `provider="codex"`(**假绿**)、`cli/bundle.rs:165` 按 master.provider 校验、`tests/builtin_skills.rs` 断言 master builtin skills `for_all_wired_providers`(codex/antigravity/claude)。整个外壳像支持,**唯独 spawn 承重梁 `prepare_master_pane_plan` 偷偷钉死 claude**,且 `SpawnMasterPaneParams` 连 provider 字段都没有(对比 `agent.spawn` 动态传 `manifest.provider_name`)。还自相矛盾:config-time bundle 按 master.provider 校验、runtime 物化 claude 的。
- **模块化了吗:不对称**。worker 侧真模块化(agent.spawn 参数化),master 侧没解耦到同一水平——看着模块化其实留了个硬编码孤岛,反而更迷惑。
- **测试锁死了吗:锁错地方**。bundle/skill/config 层有测试锁"周边支持全 provider";**spawn 那一环无任何端到端测试**驱动 codex master 走完 spawn 并断言拿到 codex home/bundle/凭据——这就是漂移能躺着没人发现的洞。
- **为什么"现在"响亮失败(而非静默)**:#151(ah#18 shared secure storage)加 fail-closed;实验里 `cmd=codex`+删 `[providers.claude]` 撞上→秒死。以前静默用 claude,现在硬报错。
- **与 #29 关系**:外部用户撞的是**同一个不透明码**不一定同根因(他们多半用默认 claude master,更可能缺/配错 `[providers.claude].shared_credentials_dir` 撞 fail-closed)。正因 #29(rpc_client 吞 message 只吐 error_code 枚举名),连"到底哪个根因"都分不出——**#29 可观测性是诊断一切 ENVIRONMENT_NOT_SUPPORTED 的前置,应先修**。
- **SOP 回写(=每次干预一条规则修订)已做**:`research/architecture-index.md`(设计任务必读第一份)Layer 6 加"⚠️ Capability holdout"块,登记 owner=`prepare_master_pane_plan` + 修法(参数化 spawn 加 provider 字段 + start.rs/cutover 传 master.provider + per-provider 路由 + 端到端锁测试 + config validate 对 master.provider≠claude 报 warning 消假绿)。机制意义:这正是解耦阶段立的"能力→owner 注册表"该拦却漏登的一条;补登后,下一个改 spawn 的人有据可依,防止"改别的又把这里改歪"。
- **立项重构(用户 F"改代码支持 codex master")=三件事**:(1) 参数化 spawn 承重梁 + 端到端锁测试(**触 home_layout/凭据高风险区,合并前 operator gate**);(2) 修 #29 可观测性(小、独立、先修);(3) SOP 回写(已做)。待用户拍板派单优先级(与 Wave-2 抢 codex 配额)。

## 观察 #59(2026-07-13):worker 全量测试里 `rm /tmp/tmux-1001/ahd-*` 删掉活栈自己的 master socket,整栈 islanded

- **症状**:redirect brief 投不进 master——`ah tell master` 报 `DELIVERY_FAILED stage=LOAD_BUFFER reason=tmux ... ahd-2ee4e0dfc3b5034c (No such file or directory)`。master 显示 IDLE 却"不收口 c1 已完成的活"。
- **根因(钉死)**:master 栈 tmux socket 文件 `/tmp/tmux-1001/ahd-2ee4e0dfc3b5034c`(14:14 建)**被删**;但 tmux server pid 461945 + 全部 pane 进程(master+c1–r1)仍活(etime 03:00),持 socket fd 但监听路径已 unlink → **server 活着但任何外部连接全失败**(daemon 投递/ah tell/ah attach/shell)。时间线吻合 c1 日志自述"cleared 696 stale /tmp/tmux-1001/ahd-* sockets"——c1 跑 workspace 全量测试时,为过 `mvp10_acceptance::test_main_sigterm_cleans_resources`(5s socket 等待被泄漏 socket 拖死),`rm` 了 `ahd-*` **通配**,连活栈自己的 socket 一起扫。master 自 ~14:47 起失联,"没收口"是够不着不是偷懒。
- **不可救**:tmux 监听 socket 一旦 unlink 无 relink 机制,server+panes 不可重连,只能重启栈。
- **数据无损**:c1 成品 a8d4970 已 push origin/feat/md2-wave2-system-split;Wave-2 状态已 checkpoint;其余 worker 本 IDLE 无在飞活;operator 未提交编辑是磁盘文件不受重启影响。
- **根因链**:测试 tmux 泄漏积压(已知 hygiene backlog,#57 附记)→ worker 被泄漏拖垮某测试 → worker 盲目 `rm ahd-*` 通配"帮忙清理"→ 删掉活栈 socket。**三层机制洞**:(a) 测试套件泄漏 tmux daemon(源头);(b) worker 无边界对**共享 /tmp 基础设施**做破坏性 `rm`;(c) 活栈 socket 与测试泄漏 socket **同前缀 `ahd-*` 同目录**,无法区分"活栈 vs 垃圾"。
- **待修规则(候选,合并前和用户议)**:worker 铁律加"禁止对 `/tmp/tmux-1001/` 等共享基础设施做通配 `rm`;测试清理必须 scope 到自己 fixture";测试套件修泄漏源(每个 fixture 用后自清 tmux server);活栈 socket 命名/目录与测试隔离(如活栈走独立 TMUX_TMPDIR),让"能不能删"从命名即可判定。
- **处置**:报用户 + 请求 restart 授权(需 kill islanded tmux 461945 = 杀活进程,按"report+offer"不擅自动手);restart 同时承接用户的 redirect(#29 + codex-master),fresh master 重新 orient 无损失。

## 观察 #60(2026-07-13,fresh master 换血后首例):`ah pend` 假 COMPLETED(job 状态撒谎的另一变体)——c1 任务A(#29)进行中被误判收口

- **症状**:为 c1 的 #29 任务挂的哨兵 `timeout 1800 ah pend job_beaf26de-...` 提前退出,`PEND_EXIT=2`(附带一行 `EXIT_CODE_UNAVAILABLE_NON_CHILD`),读起来像"job 已收口/出错退出"。但 `ah ps` 同时显示 `c1 | BUSY`(pid 从派单时的 566295 变成 579996),直接重跑 `timeout 15 ah pend <同 job_id>` 立即 timeout(124)= 该 job 其实仍未收口,pend 状态与 ah 自己的 agent 状态互相矛盾。
- **产物轨核验**:capture-pane 看 c1 实际内容——CLI 会话中途出现过一次"Conversation interrupted"(大概率是同一时段主机磁盘写满 ENOSPC 事件的连带影响,不是 c1 自己出错),随后 c1 自主恢复、把 brief 原文重新回显并继续干活(pure RPC 单测已绿,CLI 集成测试正在重跑)。worktree `ccbd-rust-wt-issue29` 里 `git status` 显示 `src/cli/rpc_client.rs` 已改、新增 `tests/issue29_rpc_error_message.rs`,但**无 commit**——跟 brief 要求的产物形状吻合,是真在干、没干完,不是假完成也不是挂死。
- **根因(推断,未 100% 钉死)**:pid 变化(566295→579996)说明 c1 的 CLI 进程层面重启过一次;`ah pend` 大概率是绑定到旧 job/进程句柄的等待,进程重启后该句柄失效被判"不可用"(`EXIT_CODE_UNAVAILABLE_NON_CHILD` 命名也暗示"等的不是自己的子进程了"),于是提前吐出一个类似"退出"的信号,但 ahd 的 agent 状态机(`ah ps`)并未跟着把 c1 判成收口——**两条状态轨(job pend 句柄 vs agent BUSY 状态)在进程重启后可以互相不一致**,谁先谁对不能只信一边。
- **处置**:未重派(context 完好,仍在原 worktree/分支干原任务);改用产物轨轮询(轮询 worktree git HEAD 变化,而非再信 `ah pend` 对同一 job_id 的返回),30min 预算到点仍无 commit 才升级。计入控制组数据(换血后 job 状态撒谎仍会发生,监控口径需要产物轨兜底这条继续验证有效)。
- **待修规则(候选)**:`ah pend` 在其等待的 agent 进程重启后的行为需要和 `ah ps` 的 agent 状态对齐复核,不能悄悄吐一个"完成态"退出码;短期缓解 = 派单哨兵拿到反常退出码(非 0、非明显超时 124)时,一律先 `ah ps` + capture-pane 复核 agent 真实状态,不直接采信退出码本身。
- **复现 #2(同日,c2 CI 修复任务)**:同一签名(`PEND_EXIT=2` + `EXIT_CODE_UNAVAILABLE_NON_CHILD`)在 c2 身上重演,pid 同样在派单后变过。`ah ps` 复核:仅 c1/c2(两个正在跑重 cargo 编译/测试的 codex 席)pid 变化,d1/o1/g1/r1 pid 不变——范围收窄到"重 cargo 负载下的 codex 席进程重启",不是全局 tmux/daemon 事件,疑与"5 个 codex 席共用配额/资源"相关但未钉死(dmesg 未见明显 OOM 记录)。产物轨复核:c2 worktree(`ccbd-rust-wt-md2-w2-c1`)有正确的未提交修复(`#[cfg(test)]`→`#[cfg(all(test, unix))]`,与 brief 要求逐字符匹配),同样是真在干、没提交,不是假完成。处置同上:不重派,产物轨(commit 出现)轮询兜底。

## 观察 #61(2026-07-13,operator 纠机制):master 给 c2 的验收要求错配平台 → codex 席空转烧 25min

- **触发**:operator 观察到 c2 在 #162 windows cfg-gate 修复上空转约 25 分钟——按 master brief 要求跑全量 `cargo test --workspace`(串行),600s 超时→热缓存重跑 1200s→被 codex "conversation interrupted" 打断→自主重启重跑,形成死循环烧时间,直到 operator 拦下。
- **根因(master 自认,不推给 c2)**:brief 里"至此点必须全量 workspace serial test 才能 commit"这条闭环纪律,套用到了**平台专属(windows-only)修复**上——本地是 Linux,全量本地测试**根本验不了 windows 正确性**(只有 CI 的 `windows-msvc-check` job 能验),而且串行全量本身就慢,大概率必超时。这是"收口点全量测试"铁律的适用范围没有分情况——铁律是为**行为不变式重构**(如 db/system.rs 拆分本身)设的,不该无差别套到**平台条件编译一行 gate 修复**上。
- **处置**:立即改派 c2(不等它自己发现,operator 已指出根因,直接下场纠正)——验证要求收窄为 `cargo check` 绿 + 针对被改模块的定向测试(`cargo test --lib db::system`),不跑 workspace 全量;windows 正确性交回 CI `windows-msvc-check` 复跑验收,commit 后直接报。
- **待修规则(已生效,写回 brief 撰写口径)**:**平台专属修复(windows/macos-only 改动,本地平台跑不了的)不得要求本地全量收口测试当 gate**——本地只需该平台通用部分的定向/增量验证,平台专属正确性一律交给对应 CI job 验收。"模块收口点本地全量 cargo test"铁律的适用范围 = 跨平台行为不变式改动;平台条件分支的窄改动走"定向测试 + 对应 CI job"更快更准。
- **同根因升级信号**:这是本会话第 2 次在验证策略上出问题(#60 系列是 job 状态撒谎,这条是 verification-mismatch),两者不同根因,但都提醒:**派工前先问"这个改动本地到底能不能验",不能默认套用固定的收口清单**。

## 观察 #62(2026-07-13):r1 真崩溃(CRASHED, tmux_alive=false)——`ah up` 成功拉回

- **症状**:r1 审 c1 的 #29 修复期间,`ah ps` 从 BUSY 变为 `CRASHED`,`ah status` JSON 确认 `tmux_alive: false`——与本会话此前 c1/c2/o1 的"CLI 会话内部中断后自主恢复"不同,这次连 tmux pane 本身都没了,是真崩溃、不是状态误报。审核工作没留下任何产物(`REVIEW-*.md` 不存在),此前的排队 review 请求随崩溃丢失。
- **处置**:`ah up` 一键复核+对齐 ah.toml,输出里 r1 一项 `"event":"drift_realigned","reason":"config changed","status":"REALIGNED"`——自动重新拉起,`ah ps` 确认恢复 IDLE。**未见其它 6 席同时受影响**(c1/c2/d1/g1/g1-m1/o1 均 `NO_CHANGE`),范围收窄到 r1 单席。
- **同批次伴随现象**:master 自己派给 `ah pend job_985a33f8...` 的哨兵后台任务连续被外部杀死 4 次(`TaskStop`/killed,非超时、非我主动停),且一次刻意起的无关探针后台任务(`sleep 20`)在 `TaskList` 里瞬间"查无此任务"却仍正常跑完收到完成通知——怀疑 master 自己的后台任务追踪层这段时间也有独立的不稳定,但暂无法钉死是否与 r1 崩溃同根因还是纯巧合(两者时间窗口重叠但机制不同:一个是 tmux/进程真死,一个是任务追踪表象异常)。
- **待办**:review 工作已重新派发(见后续 job);r1 崩溃根因未深挖(可能与 codex 配额/资源竞争有关,同 c1/c2 此前的 pid 重启一样都是 codex 席),暂列入换血后"codex 席在重负载下不稳定"的证据链,不单开新根因结论。
