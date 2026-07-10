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
