# Per-Worker Credentials · 冻结设计(design-rev.md)

> **状态**:`FROZEN · spike-validated`(2026-07-12,master 已裁 D-1/D-3 并对 C-1~C-5 无异议;三条 spike 已跑通、无一层需推翻或降级,见第十一节;翻新自 2026-07-10 已废弃的 Fake Gateway 稿)。执笔 = d1-claude(本项目唯一设计执笔席)。
> **输入链(权威基线,按序)**:`requirements.md`(北极星 + 二进制实证事实 + P1/P2 不变量) → `design-divergence-2026-07-12.md`(o1 四套方案 A/B/C/D + 红队) → `convergence-2026-07-12.md`(master 二轮红队收敛,三层骨架)。
> **本稿取代**:2026-07-10 的 "Plan B: Fake Gateway" 稿——该全局网关方向历经 #146/#147/#149 三 PR 从未端到端激活任何 claude(`incident-2026-07-12-gateway-bridge-ahd-current-exe.md`),设计层已据此重开。
> **执笔立场**:我认可收敛的**三层防御方向**(不推翻),但对收敛稿有 **5 处补强 / 1 处逻辑改判**,已在本稿显式收口。其中 **D-1(ahd↔用户宿主 CLI 的 defect-A 一层)是收敛骨架未覆盖的真缺口,需 master 裁北极星范围**——本稿给推荐解并标为开放点,**不单方冻结**。分歧全量列于第十节。
> **边界**:本稿只产 markdown。不改 src、不跑构建/测试、不碰活栈(preModD 止血态在跑)。所有代码引用带 grep 实证 `file:line`。

---

## 一、北极星与不变量回锚(高于一切机制)

**用户只想用他平时那个 CLI 登录一次,ah 栈里 N 个 claude worker 全部骑在这一次登录上干活——既不逼用户为每个 worker 重复登录,也不能因多 worker 并发把用户(乃至用户 Windows 本机)登出。** 隔离是手段,"单次登录 / 零重复登录 / 零互相登出(含不登出用户本机)"才是目标(`requirements.md` §目的)。

两条不变量(实现机制已从"必须走代理"泛化,`requirements.md` P1/P2):

- **P1(核心不变量)**:改动后,**没有任何 worker 能独立发起一次"会轮换服务器端令牌血缘"的刷新**。凡"worker 手里有可独立使用的真 refresh token 且能自刷新"的方案都不满足。附带:隔离(A 刷新不影响 B)、单飞(并发只发生恰好一次真实上游刷新)、初始种子来自用户单次登录且**绝不下发到 worker 沙箱**。
- **P2(失败隔离)**:任一 worker 的凭据失效(过期/撤销/登出)是**可观测**且**可独立恢复**的,不波及其他 worker、不静默 hang。

**已坐实的二进制事实(operator 逆向 CLI `2.1.207`,`requirements.md` §根因;本稿据此推理,未重新逆向)**:

| 编号 | 事实 | 对本设计的约束 |
|---|---|---|
| F1 | 凭据后端可插拔;`plaintext` 文件后端**仅在无系统密钥库时启用**(headless Linux / WSL2 的处境)。原生 macOS/Windows 走单一 OS 密钥库,天生免疫本类 bug。 | 本设计只针对 plaintext 后端处境;不假装解决原生桌面(它本就不中招)。 |
| F2 | 写盘是原子改名:`写 tmp → rename(tmp,target)`;`rename` 跨设备失败(`EXDEV`)**退化 `copyFile`(函数 `Jm`)**。 | defect B 的机制根;Layer 1 让 worker 文件为 Linux 本地真文件即可结构性消除。 |
| F3 | claude **只在收到 401 时被动刷新**(非主动定时)。 | "worker 永不撞 401 ⇒ 永不刷新"是可能的解 A 路径,但受 access token TTL 与任务时长约束(见 C-3)。 |
| F4 | claude 把 refresh token **缓存在内存、刷新前不重读文件**(强推断)。 | "共享一份文件"无效(内存那份不共享);但"启动时注入 dummy RT ⇒ 内存里也是 dummy"成立(见 C-2)。**外部改写磁盘救不了运行中的进程**(见 C-3)。 |
| F5 | 缺陷 A(所有文件后端):共享 RT + RTR = 谁先刷新作废旧 RT,余者 `invalid_grant` 级联登出。 | 本设计要消除的主敌;注意它**不止 worker↔worker,也存在于 ahd↔用户 CLI**(见 C-4 / D-1)。 |
| F6 | 缺陷 B(WSL2 专属):种子是指向 `/mnt/c` 的 symlink,`rename` 跨 9p/drvfs `EXDEV` → `copyFile` **跟随 symlink 写穿 Windows 真文件**,登出残根盖掉用户宿主登录。 | 第 5 节论证其结构性消除。 |

**当前缺陷代码锚点(grep 实证)**:`link_credentials`(`src/provider/home_layout.rs:658-664`)对 `{source_home}/.claude/.credentials.json` 直接 `symlink_auth_file(&source, &target)`,所有共享 `source_home` 的 worker 因此**共用同一 inode**;`materialize_auth_file_with_ladder`(`:54-89`)是 symlink 失败退化 copy 的阶梯,`.claude/.credentials.json` 在白名单内(`:20-26`)。**Layer 1 的落点就是替换 `link_credentials` 这条 symlink 通道。**

---

## 二、三层防御架构骨架(总览)

**不是单选一套方案,而是"主动保鲜 + 协议兜底 + 失败恢复"三层,各买不同的东西:**

```text
                          +----------------------------------------------+
                          |  ahd(宿主守护)= 唯一真 RT 持有者 / 唯一刷新器  |
                          |  · 私有 master 凭据(真 RT,Linux 本地真文件)  |
                          |  · 单一后台刷新 loop(F3 被动 → 改为主动保鲜)  |
                          +---+-----------------+-----------------+--------+
                              | Layer 1         | Layer 2         | Layer 3
                 原子覆盖写入   | 阉割拷贝        | 定点代理         | 空闲滚动重启
                 (仅 AT+exp)  v                 v(HTTPS_PROXY)   v
             +------------------------+  拦 platform.claude.com   +--------------+
             | worker 沙箱 A .cred.json|  /v1/oauth/token 这一条   | IDLE 时 kill  |
             |  accessToken+expiresAt |  ← single-flight + 中和    | & respawn     |
             |  refreshToken = dummy  |  ────────────────────►     | 重载新鲜映像   |
             +------------------------+   模型 API 100% 直连(不代理)+--------------+
```

- **Layer 1(主动,日常路径 · 采纳 o1 方案 B)**:ahd 持有**唯一**真实、含真 RT 的凭据(来自用户单次登录,**绝不下发 worker**),跑单一后台刷新 loop,在 access token 剩余 TTL ~20% 时**主动**刷新;每次刷新后**原子覆盖写入**各 worker 沙箱内的**阉割版**凭据文件(仅 `accessToken`+`expiresAt`,`refreshToken` 去除或填 dummy)。**买到**:①worker 永不掌握真 RT ⇒ 结构性满足 P1 主干;②每个新起/重启的 worker 拿到新鲜 AT;③defect B 消除(worker 文件是 Linux 本地真文件,非 `/mnt/c` symlink)。
- **Layer 2(安全网,协议级兜底 · 采纳 o1 方案 C,作用域收窄)**:本地 HTTPS 定点代理**只**拦 `platform.claude.com` 的 `/v1/oauth/token`(模型 API 100% 直连,规避 #146/#147/#149 全局网关秒死教训);任一 worker 真的发出刷新请求时,在此**改写请求体 + single-flight + 中和响应**(见 C-1),收拢为一次真实上游调用回填。**买到**:①长任务运行中撞 401 的**活体续期**(唯一不重启就能续命的路);②消除"dummy RT 打到真上游"这个上游行为赌注(见 C-2 steelman);③即便 Layer 1 中和有 bug 泄漏了真 RT,协议边界仍 single-flight 兜住。
- **Layer 3(失败隔离与恢复,对应 P2 · 采纳 o1 方案 D 的恢复语义,不作主防线)**:worker 凭据判定失败时作为**可观测事件**上抛;ahd 在该 worker 处于 IDLE 时对其**平滑重启**以重载新鲜凭据映像,给出独立于其他 worker 的恢复路径。**买到**:P2 的"可观测 + 可独立恢复";兜住 Layer 2 缺席/上游主动撤销时的长任务失效。

**三层的依赖关系(关键,防误解)**:P1 主干由 **Layer 1 单独结构性承载**(worker 无真 RT);Layer 2 是**纵深防御 + liveness 优化 + 消除一个上游赌注**,**不是** P1 成立的前提;Layer 3 是 P2 的落地。这决定了 **spike 失败时的降级形态**(第 6、10 节),也纠正了收敛稿"Layer 2 是不变量唯一硬保证"的表述(见 D-3/C-2)。

---

## 三、P1 / P2 如何被每层满足(逐条对账)

### P1 核心不变量:没有 worker 能独立发起会轮换血缘的刷新

| 达成路径 | 机制 | 依赖的假设 | 承载层 |
|---|---|---|---|
| worker 内存里的 RT 就是 dummy | F4:claude 启动时从文件读 RT 入内存、之后不重读。Layer 1 写的是 dummy ⇒ 内存里也是 dummy。worker 任何刷新尝试用的都是 dummy。 | F4 成立 + spike#2 验证 claude 拿 dummy 的行为 | **Layer 1(结构性)** |
| dummy 打不穿到上游 | Layer 2 拦 `/v1/oauth/token`,worker 的 dummy 请求根本不出网;由 ahd 用真 RT 单飞刷新 | Layer 2 spike#1 通过(pinning/HTTPS_PROXY) | **Layer 2(纵深)** |
| 即便 dummy 出网也不轮换真血缘 | dummy 从未被上游签发,不属于真令牌血缘,`invalid_grant` 不轮换 ahd 手里的真 RT(见 C-2) | 上游对"未知 token 的 invalid_grant"不惩罚真会话(残余赌注,Layer 2 在场时归零) | **Layer 1 兜底论证** |
| 单飞 | 并发 401 时,真实上游刷新只发生恰好一次:Layer 1 里由 ahd 单一 loop 承担;Layer 2 里由代理 single-flight 承担 | — | **Layer 1 / Layer 2** |
| 初始种子 | ahd 从用户单次登录位置读种子一次 → 私有真文件,**绝不写入 worker 沙箱** | 种子位置可读(C-4/C-5) | **Layer 1** |

> **隔离验收(P1)**:worker A 触发的刷新不影响 worker B——在本架构下,A/B 都没有真 RT,A 的"刷新"要么被 Layer 2 单飞成一次共享刷新(B 拿同一新 AT),要么 dummy 失败只影响 A 自己;B 的会话与 A 的凭据事件解耦。可用 `--lib` 对假上游端点驱动:一个 worker 上下文触发刷新,断言并发的第二个 worker 上下文请求不受影响(替代原 file-mutation 测试)。

### P2 失败隔离:失效可观测、可独立恢复

- **不影响他人**:worker A 凭据失效是 A 的沙箱文件/进程局部事件,B-E 的 AT 各自独立(Layer 1 各写各的文件)⇒ B-E 继续。
- **可观测**:失败上抛为事件。**具体接口待 spike#3 审计后钉死**,候选(grep 实证已有设施):
  - agent 状态列已有 `IDLE/BUSY/UNKNOWN/CRASHED`(`src/db/state_machine.rs:16-18` 定义 `STATE_IDLE/STATE_BUSY`,`STATE_CRASHED` 在 `src/db/state_machine.rs:26`;**原稿误引的 `events_progress.rs:103` 是测试断言、非定义处,spike#3 已纠**)——`sub_state` 列已存在(`src/db/schema.rs:82`,`src/runtime_events.rs:95-101` 暴露),可新增 `sub_state='CRED_INVALID'`。
  - 事件流已有 `state_change` 事件与 `mark_agent_idle_matched`(`src/db/events.rs:265`),凭据失效可作为一条 `cred_failure` 事件入 events 表,`ah ps` 可见。
- **可独立恢复**:Layer 3 在 A 处于 IDLE 时平滑重启 A,重载 ahd 已保鲜的新 AT;B-E 无感。恢复动作复用"idle-gated action"既有范式:`mark_dispatched_job_cancelled_if_agent_idle_sync`(`src/db/jobs.rs:618`,门控条件 `st == "IDLE" || st == "UNKNOWN"`,`:643`)证明"只在 agent IDLE 时才动它"的锁定模式在本仓已有先例,Layer 3 的 restart 应挂同一门控。

---

## 四、关键设计契约(钉死;漏一条即静默破不变量)

> 这一节是实施线与 r1 审核线的**硬契约边界**。每条给机制、前置、失效模式,不给断言。

### 契约 C-1(D-2):Layer 2 是"改写请求 + 合成响应"代理,响应必须中和 RT

**问题**:worker 手里是 dummy RT(Layer 1 所致)。Layer 2 拦到 worker 的刷新 POST 后:

1. **不得透传** worker 请求体(dummy RT 会被上游 `invalid_grant` 拒);
2. **必须忽略 worker 请求体,用 ahd 私有的真 RT 自己向上游刷新**(single-flight);
3. **必须中和回填响应**:上游 RTR 响应里带一个**新的真 RT**;Layer 2 **绝不能原样回给 worker**——否则 worker 当场重新武装真 RT,**当场破 P1**。回给 worker 的响应必须像 Layer 1 一样只含 `accessToken`+`expiresAt`(RT 字段去除或塞回 dummy);新的真 RT 只更新 ahd 私有状态(并按 C-4 回写用户种子)。

**失效模式**:若实现者把 Layer 2 写成透明转发代理 → 要么 dummy 被拒(worker 续期失败,退化 Layer 3),要么真 RT 泄漏回 worker(破 P1)。二者都必须在 spike#1 之后的实现中显式测试(断言 worker 拿到的响应体不含真 RT)。

### 契约 C-2(D-5):dummy RT 的"双态安全"是骨架成立的隐藏支点

给 worker 的 dummy RT 必须**语法看起来合法、语义确定非法**(如 `"ahd-neutered-<uuid>"`)。它在两种情况下都安全,这正是 Layer 2 能从"P1 的 load-bearing"降为"纵深防御"的原因:

- **Layer 2 在场**:worker 撞 401 → POST dummy → Layer 2 拦下,用真 RT 换成活 AT 回给 worker(中和 RT)→ worker 续命。
- **Layer 2 缺席/失败**:worker 撞 401 → POST dummy → 打到真上游 → `invalid_grant`。**dummy 从未被上游签发,不属于真令牌血缘,拒绝它不轮换 ahd 手里的真 RT** ⇒ 真血缘与用户会话不受影响,worker 干净失败 → Layer 3 重启。

**残余赌注(诚实标注)**:Layer 2 缺席时,dummy 会真的打到上游。我们赌 Auth0 对"未知 token 的 invalid_grant"只回错、不对真会话做惩罚/盗用检测(F5 的盗用检测是针对**已签发后被轮换出的旧 RT 的重放**,dummy 从未签发,不该命中该检测器)。**Layer 2 在场时此赌注归零**(dummy 永不出网)。这条赌注是"为什么即使 Layer 1 已结构性满足 P1,我们仍要 Layer 2"的**真正理由**——不是 single-flight,而是消除这个上游行为赌注(纠正收敛稿把 Layer 2 之必要性归因于 single-flight 的表述,见 D-3)。

### 契约 C-3(D-4):谁保鲜什么——Layer 1 的主动刷新**不**救运行中的长任务

由 F4(claude 不重读文件),必须写清三层各自的保鲜边界,否则实施会误以为 Layer 1 能救长任务:

- Layer 1 的"20% TTL 主动刷新 + 覆盖写 worker 文件"保鲜的是:**①ahd 自己的 master token**(始终有活 AT 可发给新起/重启的 worker);**②磁盘上的沙箱文件**(下一次 (重)启动时被读入)。
- Layer 1 **保鲜不了运行中 worker 的内存 AT**——磁盘覆盖对已在跑的进程不可见(F4)。
- 因此**长任务运行中 AT 过期只有两条出路**:**Layer 2 活体续期**(唯一不重启就续命的路)或 **Layer 3 空闲重启**。若长任务非 IDLE 且 Layer 2 缺席 → 该任务中途撞 401 会失败(o1 方案 B/D 共同死穴),由 Layer 3 在其回到 IDLE 后恢复。这是本架构对"超长不可重启任务"的诚实天花板,写入开放点 6。

### 契约 C-4(D-1 · **需 master 裁范围,不单方冻结**):ahd↔用户宿主 CLI 的 defect-A 一层

**收敛骨架只解决 worker↔worker 隔离,但北极星明确把"不能把用户 Windows 本机登出"纳入范围**(`requirements.md` §目的、§9;2026-07-11 incident "首次殃及宿主 Windows 登录态")。这里有一条骨架未自动覆盖的 defect-A:

- ahd 成为唯一刷新器后,用**种子 RT** 刷新 → RTR 轮换 → **种子 RT 作废** → 用户自己那台原生 CLI(若仍持种子 RT 并独立刷新)下次刷新吃 `invalid_grant` → **用户被登出**。这是 F5 缺陷 A 的**结构性重演**,主角从 worker 换成 ahd 自己。三层骨架(只管 worker)不自动堵这条。

**我的推荐解(折进 Layer 1,但范围问题需你裁)**:

1. **破 symlink**:ahd 的 master 拷贝与所有 worker 拷贝都是 **Linux 本地真文件**,种子只读取一次、不再被 symlink 链穿透(直接消除 F6 写穿的同时,也让"用户文件"与"ahd 私有状态"物理分离)。
2. **回写保鲜种子**:ahd 每次刷新后,把新 TokenSet(**含真 RT**)**原子回写到用户登录文件**。用户文件在用户自己可信家目录、**不是 worker 沙箱**,写真 RT 不违反 P1(P1 只约束 worker)。这样用户原生 CLI 无论何时读文件都拿到活 AT + 活 RT,**几乎永不需要自刷新**(它只在 0% TTL 撞 401 才刷,而 ahd 在 20% TTL 已保鲜)。
3. **残余窄竞态(诚实天花板)**:ahd 与用户原生 CLI 在同一刷新窗口都刷 → 仍可能互相轮换。因用户原生 Windows CLI 不会设 `HTTPS_PROXY` 指向 ahd,**Layer 2 覆盖不到它**,故这条对"完全独立的原生刷新器"维度是 Layer-1-回写的概率性收窄,不是协议级根除。

**需你裁的北极星范围问题**:用户是否被期望"在 ah 栈内使用交互 claude(骑 ahd 保鲜)",还是要保住一条**完全独立的原生刷新器**?
- 若前者(推荐):ahd 统筹该登录血缘,用户交互会话也读 ahd 保鲜的种子文件,窄竞态可接受。
- 若后者:三层骨架无法根除 ahd↔原生 CLI 的 defect-A,需要额外机制(如让用户原生 CLI 也走 Layer 2,或种子文件也中和 RT——但那会让用户 CLI 在 ahd 宕机时无法刷新)。**这是范围决策,不是技术细节,我不替你定。**

### 契约 C-5:种子获取与 ahd 私有状态的落盘位置

- 种子来源:用户单次登录产生的 plaintext `.credentials.json`(F1;WSL2 处境下即 `/mnt/c/.../.claude/.credentials.json` 或其 WSL 侧登录文件)。ahd 在凭据 provisioning 时**读一次**,拷入 ahd **私有状态目录**(Linux 本地、非 `/mnt/c`、非任何 worker 沙箱、权限 `0600`)。
- ahd 私有状态是**唯一**长期持有真 RT 的进程内 + 磁盘位置(加上 C-4 回写的用户种子文件)。worker 沙箱、Layer 2 回给 worker 的响应,一律无真 RT。
- ahd 回写用户种子文件时(C-4)须保证原子性:tmp 文件建在**与目标同设备**(若目标在 `/mnt/c`,tmp 也建在 `/mnt/c`)避免 `EXDEV` 退化 copy 造成半写;这是 ahd **有意**写用户文件,与 defect B 的"worker 无意写穿 symlink"是两回事,但原子性要求相同。

---

## 五、WSL2 defect B 消除论证(结构性,非防御代码)

**defect B 机制(F2+F6)**:worker 的 `.credentials.json` 是指向 `/mnt/c` 的 symlink;claude 写凭据 `rename(tmp, target-symlink)` 跨 9p/drvfs 设备边界 `EXDEV` → 退化 `copyFile` **跟随 symlink 写穿** Windows 真文件,把登出残根(`expiresAt:0`)盖到用户宿主登录。

**本架构下为何结构性消除(不需要额外防御代码)**:

1. Layer 1 用 ahd **主动物化的独立真文件**替换 `link_credentials`(`src/provider/home_layout.rs:658-664`)的 symlink 通道。worker 沙箱内 `.claude/.credentials.json` 是 **Linux 本地文件系统上的真文件**,不再是指向 `/mnt/c` 的 symlink。
2. 于是 worker 内 claude 写凭据时,tmp 与 target 同在 Linux 本地 fs ⇒ `rename` 成功、无 `EXDEV`、无 copyFile 退化、无 symlink 可跟随 ⇒ **没有任何路径能写穿到 `/mnt/c`**。
3. worker 文件里即便被 claude 写了登出残根,那也只是 worker 自己那份 dummy 文件被写坏,**不波及用户宿主文件**,且 Layer 3 会重启该 worker 重载新鲜映像。

**验收硬项(WSL2)**:任何 worker 路径不得写穿 `/mnt/c` credentials——在本架构下是**结构性满足**(worker 文件非 symlink、非跨设备),而非靠运行时检查。但仍须在 A2 端到端验收里**显式断言**:真 WSL2 机上 spawn 真 worker、触发一次凭据写(含模拟登出残根),校验 `/mnt/c/.../.claude/.credentials.json` 的 inode/mtime **未被 worker 触碰**。ahd 自身对用户种子的**有意**回写(C-5)不在此禁令内,但须原子。

---

## 六、四条 spike 的验收判据(通过/失败分别怎么办)

> **我对收敛稿 spike 优先级的改判见第十节 D-3**:两条生死 spike 都便宜(mitmproxy / 改本机文件),**应并行先跑**;但按"失败后重设计代价"排序,**keystone 是 spike#2(Layer 1 行为)而非 spike#1(Layer 2 pinning)**,因为 Layer 1 是主路径且是 Layer 2 缺席时 P1 的唯一承载者,spike#1 失败只是"降级"(收敛稿自己给了降级路径),spike#2 失败才"推翻主路径"。下面按此重排,保留 master 原编号以便对账。

### Spike#2(keystone,主路径生死):claude 拿 dummy / 缺失 RT 撞 401 的行为

- **验证方式**:手动改本机 `~/.claude/.credentials.json` 的 `refreshToken` 为 dummy(并把 `expiresAt` 设为过去),`timeout` 包裹跑一次真实 `claude` 命令触发 401,观察进程行为。
- **通过判据**:claude **干净地进入"需要重新登录/凭据失效"的可观测失败态**(明确退出码或明确错误日志),**不 crash、不无限重试死循环、不做破坏性动作(如递归删凭据目录)**;且能观测它是否 POST 了 `/v1/oauth/token`(决定 Layer 2 有无请求可拦,见 spike 耦合)。
- **失败(推翻主路径)怎么办**:若 claude 拿 dummy RT 会 crash/死循环/破坏 → **Layer 1(阉割拷贝主路径)不可行**,须把架构翻过来:worker 保留**真 RT** 但 **Layer 2 变 load-bearing**(每次刷新都强制走代理 single-flight),此时 spike#1 反升级为唯一生死门槛。**这是唯一能推翻"三层以 Layer 1 为主路径"骨架的结果**,故必须先出结论。

### Spike#1(Layer 2 生死门槛,但失败是降级非推翻):`/v1/oauth/token` 的 pinning 与 HTTPS_PROXY 遵从

- **验证方式**:本地起 `mitmproxy`,设全局 `HTTPS_PROXY`,`timeout` 包裹跑真实 claude 触发一次刷新,观察能否解密并替换 `platform.claude.com/v1/oauth/token` 的响应。
- **通过判据**:claude 的 OAuth 客户端**遵从 `HTTPS_PROXY`** 且**不对该端点做客户端证书 pinning**(自签 CA 可被信任、握手成功、响应可被拦改)。
- **失败(降级,非推翻)怎么办**:若 pinning 挡死 / 不遵从代理 → **Layer 2 整层拿掉**,降级为**纯 Layer 1 + Layer 3**:
  - P1 仍由 Layer 1 **结构性**满足(worker 无真 RT,dummy 双态安全 C-2);
  - **代价 1**:失去长任务活体续期,长任务撞 401 只能靠 Layer 3 空闲重启恢复(C-3 天花板显性化);
  - **代价 2**:失去"消除 dummy 打上游赌注"(C-2 残余赌注不再归零)——须在文档显性登记这条残余赌注,并可加一条可选运行时防御:ahd 探测到 worker 反复 POST dummy 时,主动触发 Layer 3 重启而非放任其反复打上游。
  - **明确判词**:pinning 失败**不**推翻收敛骨架,只把 P1 从"结构性 + 协议兜底"降为"结构性 + 一条上游行为赌注",不变量本身不塌。**(这正是我与收敛稿 §四.1 表述的分歧点,见 D-3。)**

### Spike#3(Layer 3 前置,非生死):现有凭据失效失败路径审计

- **验证方式**:审计 claude 当前遇凭据失效时,现有代码里失败落点(日志/退出码/hang);对照 `src/rpc/handlers/agent.rs` 的 spawn/销毁流程与 agent 状态机(`src/db/state_machine.rs`、`src/db/jobs.rs:618` idle 门控)。
- **通过判据**:定位到一个可挂钩的失败信号(退出码 / stderr 模式 / 缺 AT 的探测),据此钉死 P2 "可观测"的具体接口(sub_state `CRED_INVALID` 或 events 表 `cred_failure` 事件),并确认 Layer 3 的 IDLE 门控可复用既有 idle-gated 范式。
- **失败怎么办**:若现有代码把凭据失效吞成不可观测的 opaque error/hang → P2 需要一小段集成改动把该失败显性化(requirements P2 testability 已预告),不阻塞 Layer 1/2,但作为 Layer 3 的实施前置。

### Spike#4(方案 A keystore,可选 · 非 blocking):claude 是否 per-refresh 重新 lookup keystore

- **仅在 spike#1-3 都通过且时间允许时做**。验证方式:极简 C 程序编译为 `libsecret-1.so.0` 的 lookup 劫持存根,`LD_PRELOAD` 跑 claude,观察 API 请求时是否频繁触发 `secret_password_lookup_sync`。
- **判据与处置**:即便证实 per-refresh lookup,收益也仅是"把 Layer 1 的信道从文件换成 keystore",**不预期改变架构方向**(见 D-3:方案 A 与方案 B 同层,只是更脆的信道)。证实才评估是否值得,**不证实不影响冻结**。

**spike 之间的隐藏耦合(收敛稿未点,我补)**:spike#2 里"claude 拿 dummy RT 是否**真的 POST** `/v1/oauth/token`"直接决定 spike#1 里 Layer 2 **有没有请求可拦**。若 claude 见 dummy 就短路(不 POST、直接失败),则 dummy-RT 设计下 Layer 2 对 worker **永不触发**(Layer 2 只在真 RT 泄漏的纵深场景才有用),长任务续期只能靠 Layer 3。**故 spike#2 必须同时记录"是否 POST",作为 spike#1 结论的前置输入**——两条 spike 不是完全独立的。

---

## 七、验收判据(钉死 A2 三层验收第三层)

**本不变量的最终放行判据 = A2 三层验收的第三层:真二进制端到端。**

- `--lib`(第一层)对**假上游 token 端点**驱动 single-flight/隔离逻辑(P1 隔离验收、单飞计数),是**必要非充分**证据。
- CI 集成(第二层)可跑 Layer 2 代理对假上游的拦改、Layer 3 idle 门控重启。
- **真二进制端到端(第三层,唯一放行条件)**:用**真 ahd 二进制** spawn **真 claude worker**,在**真实(或受控可复现的)401/RTR 场景**下端到端跑通:
  1. 单次种子登录物化到 ahd 私有状态;
  2. 并发 N(≥3)个真 worker,制造同到期惊群,断言**恰好一次**真实上游刷新、无 `invalid_grant` 级联、无 worker 掉线;
  3. **用户宿主登录态不被登出**(WSL2 上校验 `/mnt/c` 种子文件 inode 未被 worker 触碰,ahd 回写后用户 CLI 仍可用);
  4. 长任务撞 401:Layer 2 在场则活体续期成功;Layer 2 缺席则 Layer 3 在 IDLE 后重启恢复,失败作为可观测事件出现在 `ah ps`/events。
- **禁止**:仅 `--lib`/CI 单测绿即放行本条不变量。单测证逻辑,端到端证"真 claude 二进制在真处境下真的没被登出"——这才是北极星。

---

## 八、开放待定项(design 需写清但不改变架构方向)

**收敛稿 §五 的四项(保留)**:

1. **Layer 1 保鲜阈值**(剩余 TTL 20% 触发)可配置 vs 写死 —— 建议:先写死 20%,留 `ah.toml` 键位但不暴露,待端到端观测再定;阈值不是不变量,不阻塞冻结。
2. **Layer 2 实现载体**(ahd 内嵌 tiny TLS 拦截 vs 独立子进程)+ CA 证书注入 worker 沙箱的机制 —— 依赖 spike#1 结论;若走独立子进程,须复用现有 sandbox home 物化通道把 CA 注入 worker 信任库。
3. **Layer 3 "空闲"判定信号源** —— 建议锚定 agent 状态列 `IDLE`(`src/db/state_machine.rs:16`),复用 `mark_dispatched_job_cancelled_if_agent_idle_sync`(`src/db/jobs.rs:618`)的门控范式;是否要更细的"无 in-flight RPC"信号待 spike#3。
4. **A2 三层验收第三层为最终放行**(已升为第 7 节硬判据,不再是开放点)。

**我(d1)新增的开放点**:

5. **【master 已裁:ahd 统筹用户交互会话】C-4 的北极星范围问题**:裁决为**前者**——ahd 统筹用户交互 claude 的凭据血缘(破 `/mnt/c` symlink、种子只读一次、每次刷新回写真 RT 到用户原生登录文件),**残余窄竞态可接受**;**不**保留完全独立的原生刷新器。C-4 据此转入冻结,ahd↔原生 CLI 的 defect-A 按"Layer-1 回写 + 概率性收窄"处置,不再引入额外机制。
6. **C-3 的"超长不可重启任务"天花板**:长任务非 IDLE + Layer 2 缺席时撞 401 会中途失败——是否需要一个"任务前预刷新 + 声明最长任务窗 ≤ AT TTL"的编排约束来收窄?留待端到端观测长任务 401 频率后定。
7. **dummy RT 的具体形态与 claude 兼容性**:dummy 字符串格式是否会被 claude 的凭据解析器拒绝在**加载 AT** 之前(spike#2 的子问题)——若 claude 见 dummy RT 就连 AT 都不加载,则 dummy 要改为"合法格式的废值"而非空/明显脏值。

---

## 九、方案取舍溯源(采纳/驳回记录,忠实度锚点)

对 o1 四套方案与 master 收敛裁决,我采纳/驳回如下(供 o1 忠实度审 + r1 审核对账):

| 来源观点 | 我的处置 | 理由 |
|---|---|---|
| o1 方案 A(keystore/so 劫持)"是更底层的终极仲裁点" | **驳回其"更底层"定性**,降为可选 spike#4 | 采纳 master §一:keytar 是纯存取抽象、不是 HTTP 客户端,与 F4 内存缓存正交;方案 A 与方案 B **同层**(进程启动注入点),只是更脆信道(LD_PRELOAD 擦除 / 静态链接击穿)。 |
| o1 方案 B(去 RT 阉割拷贝) | **采纳为 Layer 1 主路径** | 复杂度最低;死穴"长任务内存不重载"由 Layer 2/3 兜(C-3),不否决方案本身。 |
| o1 方案 C(oauth 定点代理) | **采纳为 Layer 2**,但**收窄职责**为"改写请求+中和响应"(C-1),非透传 | o1 原文只说"single-flight 回填",未点透 dummy 请求不能透传、响应真 RT 必须中和——我补为硬契约。 |
| o1 方案 D(RT 剔除+到期欺骗+空闲重启) | **采纳其空闲重启为 Layer 3**;**驳回"到期欺骗(expiresAt 设 2099)"** | 到期欺骗与 F3(claude 只被动 401 刷新)叠加会让 worker 内存 AT 真过期后**既不刷新也不知情**,反而更难恢复;Layer 1 保鲜 + Layer 3 重启已足,不需要伪造 expiresAt 误导 claude。 |
| master §一"方案 C 是唯一协议级硬保证,A/B/D 全是行为假设" | **部分驳回**(见 D-3/C-2) | Layer 1 对 P1 是**结构性**保证(worker 无真 RT),不是概率;Layer 2 的真正增量是"消除 dummy 打上游的赌注",不是"唯一硬保证"。收敛稿此表述会误导 spike 优先级。 |
| master §四"spike#1 是唯一能推翻骨架的假设,必须第一个" | **改判**(D-3) | 收敛稿自己给了 pinning 失败的降级路径(退 L1+L3)⇒ spike#1 失败是降级非推翻;真 keystone 是 spike#2。两条并行先跑,按重设计代价 spike#2 优先。 |
| master §三 三层骨架 | **整体采纳**,补 C-1~C-5 五条契约 + D-1 范围缺口 | 方向正确;补的是收敛稿未钉死的实现契约与一条未覆盖的 defect-A(ahd↔用户 CLI)。 |

---

## 十、给 master 的分歧摘要(辩到冻结用)

**骨架方向(三层防御)我认可,不推翻。** 以下 5+1 点是我要跟你钉死/辩论的:

- **D-1(真缺口,需你裁范围)**:三层只解 worker↔worker,但北极星含"不登出用户本机"。ahd 成唯一刷新器后会与用户原生 CLI 构成 defect-A(RTR 轮换掉种子 RT → 用户被登出),是 2026-07-11 incident 的结构性重演。我的解=破 symlink + 回写保鲜种子(C-4),但"用户是否保留完全独立原生刷新器"是**北极星范围决策**,我不替你定,标为开放点 5。
- **D-2(安全洞,已钉死为 C-1)**:Layer 2 必须是"用 ahd 真 RT 刷新 + 中和响应里的真 RT",不是透传;漏了当场破 P1。
- **D-3(逻辑改判,要吵)**:你把 spike#1(Layer 2 pinning)定为"唯一能推翻骨架";但你自己给了它的降级路径。真 keystone 是 spike#2(Layer 1 dummy 行为)。两条并行先跑,按重设计代价 spike#2 优先。
- **D-4(语义澄清,已钉死为 C-3)**:Layer 1 主动刷新救不了运行中长任务(F4 不重读文件),只救 (重)启动。长任务续期靠 Layer 2 或 Layer 3。
- **D-5(隐藏支点,已钉死为 C-2)**:dummy RT 双态安全,是 Layer 2 从"P1 load-bearing"降为"纵深防御"的原因;Layer 2 对 P1 的真正增量是消除"dummy 打上游"的上游赌注,不是 single-flight。
- **(附)驳回 o1 方案 D 的"expiresAt 到期欺骗"**:与 F3 叠加反而更难恢复,只取其空闲重启做 Layer 3。

**master 裁决(2026-07-12,本稿据此冻结)**:
- **D-1(范围)**:采纳推荐——**ahd 统筹用户交互 claude 的凭据血缘**(破 `/mnt/c` symlink、种子只读一次、每次刷新回写真 RT 到用户原生登录文件),残余窄竞态可接受;**不**保留完全独立原生刷新器。(落到 C-4 与开放点 5)
- **D-3(spike 优先级)**:采纳改判——**spike#2(Layer 1 dummy 行为)是真正 keystone**,spike#1(Layer 2 pinning)失败是降级不是推翻;两条并行跑,**spike#2 结果优先看**。
- **C-1~C-5 及驳回方案 D 的 expiresAt 欺骗**:无异议,冻结。

---

## 十一、spike 验证结果(2026-07-12,spike-validated)

> 来源:c1 `spike-1-2-report-2026-07-12.md`(spike#1+#2,以"Spike#2 修正重跑(消费级 claude.ai 登录路径)"节为权威结论)、c2 `spike-3-report-2026-07-12.md`(spike#3)。本节代码引用均经我 grep 复核。**两条生死 spike 全过,无一层需推翻或降级——三层骨架维持不变。**

### Spike#2(keystone · Layer 1 主路径生死)= **PASS**

- **方法论修正(诚实记录)**:首轮误设 `ANTHROPIC_PROFILE`+config-dir 变量,落到企业/profile 鉴权路径,表现为 `attempt 6/11` 的本地重试直到 `timeout` 杀掉(exit 124)——那是**方法论伪影,非 claude 真行为**。修正为**仅隔离 HOME**、写消费级 `claudeAiOauth` 凭据(`~/.claude/.credentials.json`)后重跑,才命中我们关心的路径。
- **结论(修正重跑)**:dummy `accessToken`+dummy `refreshToken` 触发**真实 `POST platform.claude.com /v1/oauth/token`**,上游回 **400**;claude **不 crash、不死循环**(~2s 结束,exit 1),返回可辨识错误 `"OAuth refresh token is no longer valid; run /login to re-authenticate"` / 结果 JSON `"Failed to authenticate: OAuth session expired and could not be refreshed"`。
- **正中 C-2 契约预测**:dummy 即便打到真上游也安全——它从未被真实签发,上游只回 400、**不轮换 ahd 手里的真 RT**。C-2 的"双态安全"经真二进制坐实。
- **新增实证 · 强化 §5 与 C-5**:失败后 claude 把**自己那份隔离凭据文件改写成登出残根**(`accessToken:"" / refreshToken:"" / expiresAt:0`)。报告明确警示(§"Design implication"):**这只有在 worker 凭据是物理隔离的 Linux 本地真文件时才安全;若它是指向 live/shared 凭据的 symlink,这次残根写就是 defect B 的写穿**。⇒ 第 5 节"worker 文件必须是非 symlink 本地真文件"从设计推断升级为 spike 实证的**硬前置**;且 Layer 3 重启时必须用 Layer 1 的新鲜映像**覆盖**该残根文件,不得读残根。

### Spike#1(Layer 2 · pinning/HTTPS_PROXY 门槛)= **PASS**

- 修正重跑里,`HTTPS_PROXY` + `NODE_EXTRA_CA_CERTS` 被遵从,mitmproxy 成功解密并拦到 `platform.claude.com:443` 上的 **`POST /v1/oauth/token` → 400**(报告§"Evidence: `/v1/oauth/token` POST"),**无 SSL pinning 阻挡**。Layer 2(定点代理 + single-flight + 中和响应)**技术可行**。
- 呼应 D-3:spike#1 通过意味着**无需触发降级路径**(纯 L1+L3);但即便它当初失败,也只是降级非推翻——本次通过再次印证 spike#2 才是 keystone 的判断成立。

### Spike#3(Layer 3 前置 · 失败可观测性审计)= 完成,给出插桩契约

- **现状**:凭据失效**基本不可观测**——只有通用信号(claude 退出→`CRASHED`+可选 `exit_code`;或原始 `output_chunk` 文本;或 `UNKNOWN`/`SPAWNING_INTERVENTION`/`STUCK`)。无任何 `cred_failure`/`CRED_INVALID`/401/invalid_grant 专用信号(报告 Answer 1)。
- **P2 落地插桩点(报告 Answer 2,钉死 §3 的"待 spike#3 审计后定"）**:①在 `src/db/state_machine.rs` 加状态/事件 helper(仿 `mark_agent_unknown_sync` `:1297-1367`),置 `sub_state='CRED_INVALID'`、插 `events(event_type='cred_failure')`;②活体检测挂 `src/agent_io/reader.rs:140-163`,匹配 spike#2 坐实的真实串(如 `"run /login"`);③启动屏分类挂 `src/provider/init_probe_task.rs:143-228`。
- **Layer 3 重启门控的修正(报告 Answer 3,补强 §3/§8 建议)**:**不得直接复用** `mark_dispatched_job_cancelled_if_agent_idle_sync`——它是 job-specific,且其 idle 门控含 `UNKNOWN`(`src/db/jobs.rs:643`),对"凭据重启"不安全(`UNKNOWN` 可能仍有在途工作)。应另写 `restart_agent_if_idle`,门控收紧为 `state='IDLE' AND state_version=?`(**去掉 `UNKNOWN`**),记 `cred_failure` 后用存好的 `AgentSpawnSpec` 走受控 kill/respawn。**额外警示**:`agent.kill`(`src/rpc/handlers/agent.rs:796-835`)会**删除沙箱目录**,会丢掉 Layer 1 刚保鲜的 home——Layer 3 需要一条"重启专用、保留/重铺新鲜凭据映像"的清理策略,不能裸用 `agent.kill`。

### 对冻结稿的净影响

- **架构方向零变更**:三层骨架、C-1~C-5、A2 第三层端到端验收判据全部维持。
- **升级为实证的项**:C-2 双态安全(spike#2 坐实);第 5 节 worker 文件非 symlink(从推断升为硬前置);Layer 2 可行性(spike#1 坐实)。
- **实施线新增待办(不改设计方向)**:①P2 观测按 Answer 2 三处插桩;②Layer 3 用 `restart_agent_if_idle`(去 UNKNOWN 门控)+ 重启专用凭据保留策略,不裸用 `agent.kill`;③Layer 3 重启必须以 Layer 1 新鲜映像覆盖失败 worker 写下的登出残根文件。
</content>
