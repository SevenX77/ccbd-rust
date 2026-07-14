# d1 冻结设计补丁单 · daemon 级所有权点缺失(轨1,2026-07-12)

## 背景
你上一单执笔的 `.kiro/specs/ah-per-worker-credentials/design-graft-frozen-2026-07-11.md` 已交 c1 实施。c1 在 worktree `ccbd-rust-wt-graft-c1`(分支 `feat/gateway-graft-modD`)按 Phase G0-G1 落地了可编译的 `GatewayCore`/UDS listener/internal bridge/home-env 剥离,定点 AC 测试(单飞/零凭据/通道隔离/失败缓存/桥动态端口)全部通过。但推进 G1-G3 生产接线时发现:**冻结设计定义了网关的核心逻辑,但没有定义它在 `ahd` 守护进程生命周期里的所有权点**——这是设计遗漏,不是实施者理解错误,不升级 operator(非产品方向选择,是实施链路内的设计完备性缺口)。

c1 完整阻塞记录见 `/home/sevenx/coding/ccbd-rust-wt-graft-c1/.operator-question`(worktree 内,未清除,供你核对原文)。已确认的三个具体缺口:

1. `src/rpc/mod.rs::Ctx` 当前只有 `db/state_dir/env_state/daemon_unit/tmux_server`,**没有** daemon 生命周期内的 `GatewayCore` 单例、listener registry、credential event sink 或 seed credential owner 的字段/初始化路径。
2. `src/bin/ahd.rs` 初始化 `rpc::Ctx` 时,**没有**读取/持有 Claude seed credential(真实 refresh token)的结构;代码库目前只有沙箱 home materialization 的旧 `.claude/.credentials.json` symlink 路径(已被你的冻结设计裁决删除),没有可复用的 host-side credential manager。
3. worker spawn path 可以追加 UDS bind + wrapper env,但 **master revive/cutover path** 的 `systemd::master_command_with_env` 当前不接受 sandbox bind overrides——冻结设计分歧点 4 裁决"master 无特例,同接 gateway",但 master 的 UDS 生命周期"绑定 daemon session、不随 revive 销毁"这条约束目前没有落点:该由 `Ctx` 持有 session-scoped listener registry,还是由 DB recovery spec 持久化后在 startup reconcile 里重建?这需要你裁决,不是 c1 该自行决定的实施细节。

## 我已经替你核实的一件事(不需要你再猜):真实 OAuth refresh endpoint 契约
c1 的阻塞里还问了"生产 `ClaudeUpstream.refresh()` 的真实 OAuth refresh endpoint/schema 在当前代码库没有锚点"。**这不需要猜测或升级 operator——A 臂实验已经实证并实现过**,证据在 `research/ab-experiment-gateway/ARM-A-diff.patch`(约 1110-1160 行,函数 `perform_real_refresh`):
- **Endpoint**:`https://platform.claude.com/v1/oauth/token`(A 臂从 CLI 自身逆向得到,见同 patch 586/636/656 行的 `TOKEN_URL` 常量)。
- **请求**:`POST`,form-encoded body:`grant_type=refresh_token&refresh_token=<refresh_token>`。
- **响应**:JSON,字段 `access_token`(string)、`refresh_token`(string,可能轮转)、`expires_in`(u64 秒)。
- **失败态**:HTTP 400 + body `{"error":"invalid_grant"}` → 判定为不可恢复的 `invalid_grant`(与你冻结设计的 30s TTL 失败缓存状态机对接,即该错误码触发失败缓存,不是网络错误)。

请把这份契约正式写入你的补丁设计里(标注来源=A 臂实证,而非你新猜的),供 c1 直接实现 `ClaudeUpstream::refresh`(生产实现,替换掉 mock)。

## 你需要裁决并补写进设计文档的内容
在 `design-graft-frozen-2026-07-11.md` 追加一节(或新开 `design-graft-addendum-2026-07-12.md`,你决定,但必须明确引用关系,不要留两份互相矛盾的权威),裁决:

1. **daemon 级所有权结构**:`GatewayCore`/seed credential/listener registry 应该以什么形态挂进 `rpc::Ctx` 和 `ahd.rs` 启动路径?(字段?独立 service 结构体?初始化时机——ahd 启动时 eager 初始化,还是 lazy 等第一个 claude worker spawn 才建?)
2. **seed credential 的来源**:host 侧唯一真实凭据从哪里读——是复用当前 `.claude/.credentials.json`(host 侧,非沙箱侧,你冻结设计裁决删除的是沙箱内 symlink,不是 host 本体文件)?若是,给出具体读取路径与刷新后回写策略(回写是否要保持文件, 还是只在内存持有、定期 flush)。
3. **`ClaudeUpstream::refresh` 生产实现**:按上面给你的 endpoint/schema 契约,写明错误映射(网络错误 vs 400 invalid_grant vs 其它非 200)与你已有的失败缓存状态机的对接点。
4. **master UDS 生命周期所有权边界**:给出明确裁决(c1 阻塞里的选项 C)——是 `Ctx` 持有 session-scoped registry,还是 DB recovery spec 持久化 + startup reconcile 重建,或第三选项。必须给出理由,不留开放问题。
5. **`systemd::master_command_with_env` 的必要改动范围**:是否需要新增 sandbox bind overrides 参数,给出函数签名级别的方向(不需要你写代码,但要写清楚"这个函数需要能做到 X"这一级别的约束,c1 才能照此实施而不用再猜)。

## 边界(不要做的事)
- 不要重新裁决已冻结的 7 个分歧点(UDS/桥/JWT/master 剥离方向/重试策略/测试策略/端口)——那些已锁死,本单只补"daemon 所有权点"这一层遗漏,不重新开放已决问题。
- 这仍然是设计执笔,不要写实现代码;c1 会照你的补丁裁决继续 G1-G3 实施。
- 范围克制:只回答上面 5 点所需的架构裁决,不要因为这次要顺手扩大到无关的 daemon 重构(轨2 已经在处理编排底座重构,不要在这里越界)。

## 完成后
写清楚补丁文档路径,并在会话里给出 5 点裁决的一句话摘要,我会据此重新给 c1 派单续跑 G1-G3。
