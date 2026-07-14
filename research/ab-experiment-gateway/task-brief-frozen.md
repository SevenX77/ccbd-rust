# 任务 Brief:实现 Plan B Fake Gateway(per-worker credentials 根治)

冻结版。本 brief 只描述任务与验收契约,不含任何角色/流程指令——任务怎么组织实施由接单席位自己的规则决定。工作区/分支以接单席位规则钉死的为准;所有 commit 只落该分支。

## 0. 权威文档(实现以此为准,冲突时按此优先级)

1. `.kiro/specs/ah-per-worker-credentials/design-rev.md` — 冻结设计(Plan B Fake Gateway 架构、假 JWT 构造、UDS 多租户隔离、单飞刷新锁伪码)。
2. `.kiro/specs/ah-per-worker-credentials/requirements.md` — P1/P2 验收要求 + 「根因(二进制实证)」节(缺陷 A/B 机制)。
3. `.kiro/specs/ah-per-worker-credentials/tasks.md` — Phase 1–3 任务框。
4. `research/credentials-phase0-spike.md` — CLI 承重机制实证(`CLAUDE_CODE_USE_GATEWAY` 语义、gateway 模式下 CLI 不做 OAuth 刷新、到期直接报错退出)。

认为设计有误/有缺口时:停下写明问题与出处,按席位规则的阻塞出口上报;不得按自己的理解偏离设计实现。

## 1. 任务范围(= tasks.md Phase 1–3,不多不少)

**Phase 1 · Gateway 核心(宿主侧)**
- HTTP Gateway 服务(`ahd` 内异步任务或独立 loopback/UDS daemon,按 design-rev 定):持有唯一真 seed 凭据。
- API 转发(`/v1/messages` 等)+ Authorization header 重写:剥离 worker 的假 JWT,替换为当前有效的真 access token,转发上游 `https://api.anthropic.com`。
- 单飞刷新:到期/401 触发刷新时,并发请求只产生**一次**真实上游刷新调用(`RwLock`+`Mutex`+`watch` 协调,见 design-rev 伪码)。

**Phase 2 · Worker 侧接入**
- 沙箱 bootstrap:Claude provider **不再 materialize 任何 `.credentials.json`**(废除 symlink/copy 两条路径)。
- per-worker UDS + 沙箱内 TCP→UDS 桥(design-rev 拓扑);注入 `CLAUDE_CODE_USE_GATEWAY=1`、`ANTHROPIC_BASE_URL=http://localhost:<port>`、`ANTHROPIC_AUTH_TOKEN=<假长寿命 JWT>`。

**Phase 3 · 失败可观测(P2)**
- Gateway 刷新失败(seed 被上游吊销等)以可辨识错误码/错误体透传,CLI 侧可见失败而非静默挂起。
- daemon 侧凭据状态可观测;seed 过期需人工重登时有明确的用户可见通知路径(按 requirements P2 "observable" 的既有约定落点)。

**范围外(碰=越界)**:非 Claude provider 的凭据处理;secrets-at-rest 加密加固;与本任务无关的重构。

## 2. 验收契约(外部锚定;测试必须断言这些可观测行为,不是实现内部状态)

- **AC-1 单飞刷新**:N 个并发模拟 worker 请求在令牌到期窗内同时打进 gateway → mock 上游收到**恰好 1 次**刷新调用;所有 N 个请求最终都拿到有效响应。
- **AC-2 隔离(P1/P2 核心)**:worker A 的活动触发一次经 gateway 的刷新,并发中的 worker B 请求不受影响(不失败、不被登出)。
- **AC-3 worker 零凭据**:接入后的 worker 沙箱 home 中**不存在** `.credentials.json`(既非 symlink 也非 copy);grep 全沙箱无真 access/refresh token 落盘。
- **AC-4 header 重写**:worker 侧发出带假 JWT 的请求,mock 上游收到的是真 access token,且假 JWT 不出现在上游请求任何 header。
- **AC-5 WSL2 硬验收**:任何被写入的凭据类路径 resolve 后不得落在 `/mnt/c` 下(路径级断言即可,不需要真 WSL2 环境)。
- **AC-6 失败可观测**:mock 上游对刷新返回 `invalid_grant` → gateway 向 worker 请求回以可辨识错误(区别于普通 5xx),且 daemon 侧留下可查的凭据失败事件。
- 测试层次:单飞/隔离/重写逻辑对 mock 上游做 `--lib` 级验证;真 CLI 端到端联通(真 claude 二进制 + 本地 gateway 跑通一次对话)为集成级,留 CI/活栈验证,不在本机跑全量。

## 3. 工程约束

- **TDD 红绿**:验收测试(对应 AC-1~6)红灯先行,留红灯输出;实现变绿,留绿灯输出;红绿轨迹入 commit 史。
- **本机 cargo 限 `cargo check`**(`CARGO_BUILD_JOBS=1`);`cargo test`/全量构建一律不在本机跑——commit 后报告,由 CI 跑测试并回灌结果。
- 任何命令 `timeout <预算秒>` 包裹;挂死即报告,不静候。
- 只 commit 到当前分支并报 commit 号;不 push、不碰 main、不动另一实验分支。
- 与周边代码风格/错误处理惯例一致;不引入无关依赖。

## 4. 完成定义

1. Phase 1–3 全部实现,AC-1~6 验收测试存在且在 CI 上绿;
2. 工作树干净,红绿轨迹在 commit 史可查;
3. 工作区根落 `COMPLETION-REPORT.md`:改动清单、各 AC 的测试名与 CI 证据、已知限制(如集成级验证留待活栈)。
