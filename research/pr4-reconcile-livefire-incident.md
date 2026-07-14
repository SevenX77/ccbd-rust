# PR4 e2e 测试三轮屠活栈事故(2026-07-09 02:11-02:16)
状态:风暴已止(operator 掐死 a1 的 cargo test);根因诊断已派 a1-codex(经 master)。

## 时间线(journal 铁证)
- 02:10:54-02:11:15 e2e 测试会话起:agent_ag_kill_*、t1-1-2-kill-session-sync、mvp10_scope_probe、agent_ag_mvp11_explicit_kill(各自独立 tmux -L 随机 socket)
- 02:11:30 活栈 a1(pid 2631737, BUSY on PR4 job_3f650092)pidfd confirmed dead → REVIVE
- 02:12:54-02:13:15 又一批 kill/scope 测试会话
- 02:13:30-02:13:34 活栈 a1(新代次)/a2/a3/a4 四 worker 同拍死亡 → REVIVE + REVIVE_IDLE×3;systemd 记录 4 个长命 scope(1-3.5h CPU)同拍回收
- 02:13:35 a4 respawn(claude --continue)后 init 探针超时 → UNKNOWN INIT_PROBE_TIMEOUT(claude 进程本身活着,探针的"1"被已恢复会话当垃圾输入回怼)
- 02:14:59 t1-1-2-kill-session-sync 再起 → 02:15:26 a1 又死(三连)
- 02:16 a1 新代次 resume 后自动重跑全量 suite → operator pkill 掐断,风暴止

## 关联规律
每次 kill/scope 类 e2e 测试启动 ~30s 后活栈 worker 死一批,3/3 全中;内核无 OOM 记录,排除内存风暴。

## 两个候选根因(都必须查死,PR4 本体问题)
1. **orphan-scope reconcile 越权杀外人**:PR4 正在接线的 reconcile 把不属于本 daemon DB 的活栈 agent scope 判为孤儿清理——正是 spec 评审钉的最高风险(ccb Bug Y 前车之鉴)的活火实证。修向=归属锚定(scope 必须可证属于本 daemon 的 state-dir/DB 才准杀)。
2. **测试 env 泄漏**:a1 沙箱 env 带 CCB_SOCKET=活栈 sock,若测试子进程继承并被任何代码路径回退使用,kill-session 测试直接打到活 daemon。修向=测试 harness 清洗继承 env(env -u CCB_SOCKET -u AH_STATE_DIR)。

## 要求的修复验收
- 实锤定位:哪个测试+哪条代码路径杀了外人(file:line)
- 修复带回归测试:同机起两个隔离 daemon,验证 reconcile/kill 互不越界
- 修复落地前禁跑全量 suite(单测/无 kill 类集成测试可跑)

## 遗留待办
- a4 UNKNOWN INIT_PROBE_TIMEOUT 处置(claude 活着,等诊断后 realign)
- ~419 个残留测试 tmux server 清扫(测试卫生 backlog 又+1)
- 完成检测对"探针注入被老会话吞掉"场景的鲁棒性(与 revival-zombie 族相关)

## 更新(02:25):机制实锤 + 自续燃循环
- **第四波 02:22:29 journal 直接抓到凶器**:systemd 记录 `Stopping ccbd-agent-a{1,2,3,4}@ahd-9819d8d7587886a9`——测试代码用 systemctl 按 unit 名(疑 glob `ccbd-agent-*`)stop 活栈 scope,零归属锚定。此前 02:22:22 测试会话 agent_ag_mvp11_anchor_stop 刚起(7 秒前因果)。exit_code=None+scope 干净回收与 systemctl stop 语义吻合。
- **自续燃循环(第二个结构病)**:worker 被杀→REVIVE 语义把中断的 PR4 job 原样重派→codex resume 继续执行 brief(内含跑全量测试)→测试再屠栈→循环。master 排队的 STOP 指令永远插不进 BUSY 的 a1。破环=cancel 中断 job 本体(operator 已令 master 执行)。
- 嫌疑代码域:e2e teardown guard(86678f6)/ PR4 orphan-scope reconcile 新代码 / mvp11 anchor_stop 测试的清扫路径。修复铁律=一切 scope kill/stop 必须锚定本 daemon 的 socket-hash,禁止无归属 glob。
- 新增北极星差距:**revive 重派缺"重派前安全性评估"**——若中断 job 本身就是杀手,自愈机制成为放大器(自愈→重派→再屠)。修向=同一 job 触发 N 次 revive 后熔断转人工(类似 crash-loop backoff)。

## 更新(02:35):第五次点火 + 第三个结构 bug
- cancel_requested=1 落库后,a1 的 revive 代次仍把同一 job 原样重派并继续执行(第五次点燃全量 suite)——**revive 重派路径不检查 cancel 标记**,记为本事故第三个结构 bug(前两个:scope kill 无归属锚定、revive 重派无安全熔断)。
- operator 防火墙:后台循环(30min,每 10s)杀光 cargo/target-debug 进程,阻断点火;治本走 STUCK SOP(ah kill agent a1 → job 见终态 → ah up 重建 → 派只读诊断)。

## 定案(02:50,a1 只读诊断,全 file:line 实锤)
- **真凶**:PR4 orphan-scope reconcile。链条=scope.rs:31 全局枚举所有 user scope → scope.rs:96 精确 marker 匹配之后 **scope.rs:99 known_refs 兜底:agent-id 撞名即认亲**(任何 ccbd-agent-a1@别人家marker 只要本 DB 认识 a1 就当自家)→ system.rs:606 stop → scope.rs:54 systemctl --no-block stop。PR4 在 system.rs:1215 首次把这条休眠路径接进 startup——每个测试 daemon(都有 a1-aN)起动即扫杀活栈。
- **无辜排除**:teardown guard(86678f6)只停自己 socket 的 tmux scope;mvp11 anchor_stop 只停自己 session 的 anchor;既有 cascade 路径(system.rs:461/469)本来就是 marker 精确锚定的正确范本。
- **假设修正**:CCB_SOCKET env 泄漏假设排除,机制是 known_refs 认亲,与 env 无关。
- **修复验收**:marker-only 归属+废除跨 marker 认亲;回归=双 daemon 同名 agent 互不杀+startup 接线集成测试;TDD 先红后绿。测试解禁分级:先 reconcile 相关,绿后全量。
- 本事故四 bug 清单:①reconcile 认亲无 marker 锚定(本体,PR4 内修);②revive 重派无熔断(北极星 R 系列);③revive 重派无视 cancel_requested(orchestration-reliability 待办);④session matcher 把终态当 running 卡 ah up(状态契约系列)。

## 更新(03:05):全量 suite 首跑=部分成功+残留单点路径
- marker 修复实证有效:本轮只停了 live a1 一个 scope(此前是全家),a2/a3/a4 完好;a1 遵守 no-relaunch 纪律干净停车,无循环。
- 残留路径签名:agent_ag_mvp11_anchor_stop 起后 7 秒 live a1 被 systemctl stop,两次事故复现同签名(02:22:22→:29、02:56:25→:32);只死 cargo test 宿主 scope。
- 热线索:scope.rs:137 detect_scope_policy → detect_current_service_unit()(/proc/self/cgroup 身份嗅探)。in-process 测试 daemon 跑在 live a1 scope 内,会把宿主 scope 嗅探成"自己的 daemon unit"(BindsTo/anchor 语义)。第二轮只读诊断已派 a1(要求 file:line 实锤 + 修向=身份禁止环境嗅探,来自显式 daemon state;test harness env 清洗双保险)。
- 又一个 ah 结构 bug 复现:revive 把中断的 suite job 自动重排(no-relaunch 靠 agent 纪律兜住而非机制)——熔断需求二次实证。

## 定案2(03:15,a1 第二轮只读诊断)
- 无任何 Rust 代码直接 stop live a1;机制=**环境身份收养**:identity.rs:3-18 从 /proc/self/cgroup 嗅探"我的 daemon unit"(认 ahd.service/ah-*.service),scope.rs:136/151/120 把它接成 BindsTo/PartOf。
- 野嗅探点五处:TmuxServer::new(session.rs:25)/from_socket_name(:44)、ahd_test_helper.rs:39(算了显式 socket 又用环境 policy)、ahd.rs:75(启动)/:246(关机 anchor 检查)、sessions.rs:305(anchor 开关判定)。mvp11 harness 本身反而是显式 policy(我的热线索方向被纠正)。
- 时点:suite 从 mvp11 进入 mvp2(mvp2 harness 走环境嗅探路径),live a1 被停。
- 统一机制解释(operator 综合):身份污染(嗅探 or CCB_SOCKET/AH_STATE_DIR env 泄漏)→ 测试 daemon 拿活栈 marker 做"合规"的 marker 锚定操作 → 对自己的 agent a1 级联时按 description 精确命中活栈 a1 → 只死 a1(测试恰好只有同名 a1)。marker 锚定无辜,"我是谁"输入被污染。
- 修复2(已放行):嗅探全灭改显式身份(state_dir 推导)+ harness env 清洗;实证门=修复后单独跑 mvp2+mvp11 盯活栈,7 秒签名消失才放全量。
- 北极星补一条:**身份必须来自显式配置/状态,永远不从运行环境(cgroup/继承 env)嗅探**——与 kill 锚定同族,都是"输入的出处"问题。
