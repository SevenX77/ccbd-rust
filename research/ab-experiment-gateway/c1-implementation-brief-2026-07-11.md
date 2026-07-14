# c1 实施单 · 模块 D 网关嫁接(轨1,2026-07-11)

## 你的角色与边界
你是 c1(codex,独立实施者)。本单是**全链路自写自测**(RED+实现+自验),这是自证风险最高的一环——**你交付后必派 r1 审,不自审**。你的代码与测试真实性由 r1 回滚自检把关,这是唯一铁律:不许同实例自审,不代表你可以降低自身测试标准。

## 工作目录(已建好隔离 worktree,不要在别处改)
`/home/sevenx/coding/ccbd-rust-wt-graft-c1`,分支 `feat/gateway-graft-modD`(从 main `7bae3b1` 切出,干净,当前仓库主线尚无任何网关代码)。所有改动在此 worktree 内进行;只 `git commit`,**不要 push**(push 由 master 亲自做)。

## 权威输入(唯一执笔权限持有者 d1 已冻结,零开放问题;新会话必读,按顺序)
1. `.kiro/specs/ah-per-worker-credentials/design-graft-frozen-2026-07-11.md` —— **本任务唯一架构权威**。含:
   - 一、嫁接后架构总纲(三层拆分)
   - 二、七个分歧点裁决(UDS 服务器/桥接/JWT 签名/master 剥离/重试风暴/测试策略/端口分配,全部锁死)
   - 三、安全机制冻结(威胁模型基线、双防线、凭据面 incident 铁律)
   - 四、单飞刷新+失败缓存状态机
   - 五、第一性实施边界(Kill List——明确哪些 A/B 遗留代码模式禁止带入)
   - **六、tasks.md 结构大纲(Phase G0-G4)—— 你的实施顺序按此走**
   - **七、验收契约(AC 表)—— 外部锚定,逐条必须绿,不得自行削弱断言**
2. 参考(仅供理解上下文,不可依赖其内部代码细节,细节以冻结设计为准):
   - `research/ab-experiment-gateway/REVIEW-gateway-ab-verdict.md`(A/B 终审裁决全文,含 file:line 级证据)
   - `.kiro/specs/ah-per-worker-credentials/design-rev.md`(原始 Plan B 设计)
   - `.kiro/specs/ah-per-worker-credentials/incident-2026-07-11-wsl2-symlink-logout.md`(WSL2 验收铁律:任何路径不得写穿宿主 `/mnt/c` 凭据)
   - 两臂源码(仅供 diff 参考,**不得直接搬运 A 的 `register_worker`/python heredoc 桥/全局 HMAC 签名——冻结设计已明确 Kill 这些**):`research/ab-experiment-gateway/ARM-A-diff.patch`、`ARM-B-diff.patch`

## 实施顺序(冻结设计 §六)
按 Phase G0 → G1 → G2 → G3 → G4 推进:
- **G0**:干净核落地(继承 B @ `d55c26b` 的 `GatewayCore`/凭据剥离逻辑,验证性搬运,非盲目 cherry-pick——按冻结设计的架构总纲重新落地到本 worktree)。
- **G1**:运行时 UDS 服务器层(B 核 + 薄 per-worker UDS 翻译层;**禁止**搬 A 的 `register_worker`;Header 8KB/Body 10MB/15s timeout 硬限)。
- **G2**:沙箱桥 + 端口 + 挂载(内置 `ah internal-bridge` 子命令,Rust `copy_bidirectional`;桥自身 `bind 127.0.0.1:0`;彻底删除 python3 heredoc 路径)。
- **G3**:master 席位接入(master 无特例,同接 gateway;删 `link_credentials`/`PROVIDER_AUTH_WHITELIST` 项/host env 透传)。
- **G4**:非代码——前置条件文档(hidepid 等)与失效模式记录写入 spec。

## 验收契约(冻结设计 §7.1,外部锚定,逐条必须绿——不得自行削弱或跳过任意一条)
| AC | 断言 |
|---|---|
| AC-单飞 | 并发 expired worker 请求仅触发 1 次上游刷新:mock `refresh_calls()==1 && message_calls()==N` |
| AC-零凭据 | worker home 无 `.credentials.json` 且全树无真 token 字节:真 `prepare_home_layout("claude",Worker)` 递归扫描 |
| AC-重写 | 上游见真 token;任何 header 不含假 JWT |
| AC-通道隔离 | 通道 worker_id≠JWT worker_id → 403,0 上游调用(**真 UDS listener 端到端**,不是打副本) |
| AC-失败缓存 | 持续 invalid_grant:TTL(30s)内上游刷新不再增长;TTL 后自愈 |
| AC-桥不挂死 | 桥启动失败→健康探测失败→spawn fail-fast 报错(非挂起);spawn 返回错误 + `bridge.err` 有内容 |
| AC-端口不冲突 | 多 worker 并发各得独立动态端口,无 AddrInUse |
| AC-master 无级联 | master revive 后经 gateway 取 token,无 `.credentials.json` symlink |

**测试策略铁律(冻结设计分歧点 6)**:100% 生产路径,**禁止**造任何"测试专用副本"函数(如 A 的 `worker_gateway_for_test`)。核心逻辑用 `MockUpstream`,新增运行时层(UDS listener/桥)必须真起真测,`serial_test` 隔离并发改 env 的测试,动态端口用 `:0`,资源用 Drop Guard 保证测试退出时 100% 回收。

## cargo 政策(铁律,不可违)
- 本地**只跑** `cargo check` + 单个定点测试复现(`cargo test <test_name> -- --test-threads=1`,`CARGO_BUILD_JOBS=1`)。
- **不要**在本机跑全量 `cargo test` 或全量构建——那是 CI 的职责,本机重编译会 OOM。
- 不要逐 task 跑一轮 cargo;测试轮只在 Phase 收口点跑一次定向验证(比如 G1 收口跑 UDS 相关测试子集,而非每个小改动都跑)。

## 最高开发原则(CEO 定,不可违)
第一性原理 · **不打补丁**(发现同族问题≥2 处直接结构修,不要各自打补丁)· 模块化低耦合高内聚 · **不要后兼容**(这是全新代码路径,不背 A/B 实验分支的历史包袱,可以推倒重来)。

## 阻塞与收口
- 遇到阻塞(设计有歧义、断言无法达成、发现冻结设计与代码库现状冲突):**不要自己降级断言**,在 worktree 根目录写 `.operator-question` 说明阻塞点,等待处理。
- 完成 G0-G4 全部 AC 绿后:`git commit`(不 push),在会话里回报:改动文件清单、AC 逐条通过证据(测试名+输出摘要)、Kill List 是否全部清除(确认无 A 的 `register_worker`/python heredoc/全局 HMAC 残留)。
