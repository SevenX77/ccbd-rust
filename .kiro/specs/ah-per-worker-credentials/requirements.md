# ah Per-Worker Credentials — Requirements

## 目的(北极星 · 2026-07-12 用户厘清 · 高于下面一切机制讨论)

**用户只想登录一次——用他平时自己用的那个 CLI 登录一次,就够了。** 之后 ah 栈里 N 个 claude worker 全部**骑在这一次登录上**干活,**既不逼用户为每个 worker 重复登录,也不能因为多 worker 并发把用户(或彼此)登出**。

- 这不是抽象的"per-worker 凭据隔离"目标——隔离只是手段;**目标是"单次登录、零重复登录、零互相登出"的用户体验**。
- "平时自己用的那个 CLI"= 用户在自己机器(Windows)上正常登录的 claude;凭据从那里来。
- 反面就是现状 ah#18:多 worker 共享一份 → 谁刷新谁把别人(乃至用户 Windows 本机)踢下线。

> **设计层状态(2026-07-12 重开)**:下面的"根因/结论"曾把方案收敛到 **Plan B Gateway**。但 Gateway 历经 #146/#147/#149 三个 PR **从未端到端激活过任何 claude**(current_exe + claude 秒死两层缺陷,见 `incident-2026-07-12-gateway-bridge-ahd-current-exe.md`)。据此**设计层重新打开**,回到第一性:由 o1 基于 claude 二进制机制**发散多套方案**、辩论收敛,不再默认 Gateway。**下面的不变量(P1/P2)保留;实现机制 TBD**——P1 原验收里"必须走 host 代理"的措辞是 Gateway 时代的具体化,已泛化回不变量(见 P1)。

> **设计层再更新(2026-07-12 晚 · operator 逆向复核 + 用户方向裁定 · 非静默变更)**:
> 1. **前提推翻**:上文"根因/补充二进制事实"里的 **F4「claude 刷新前不重读文件」(原标强推断)经逆向 `2.1.207` 二进制证伪**——claude 刷新关键段先做 mtime 检查 + 强制重读磁盘 + 竞态保护(磁盘 token 变了就用磁盘的、不再拿自己那张死 RT 去 POST)+ 跨进程文件锁(`.storage-write`)+ 原子写(tmp+rename)。故"共享一份文件无效(内存那份不共享)"的论断**作废**:共享**一份真文件**(非 symlink、非拷贝)现在是可行解,claude 自带多进程协调。
> 2. **apiKeyHelper 排除**:逆向实证其返回值只进 `x-api-key` 头、配置即关闭 OAuth 模式 = 从订阅掉成 API 按量计费,**不能用于集中下发订阅 OAuth token**,排除出方案空间。
> 3. **用户方向裁定(高于机制)**:唯一真相源(SSOT)**必须是用户 Windows 侧那个真实凭据文件本身**(用户宿主 claude 平时用的那份)。所有 ah worker + 用户宿主 claude **共享它、原地轮换、单次登录人人骑**。**明确拒绝任何"独立副本"方案(含 ahd 在 Linux 侧持有保鲜副本)**:独立副本 = 又一个"两文件共用同一 OAuth 身份",副本一刷新就轮换作废 Windows 那份 → 用户宿主被登出 → 被迫反复重登,与北极星"零重复登录"直接冲突。用户原话:"直接写 Windows 侧真实文件……我多登录几遍没关系(指实施期),关键是把功能做完善"。
> 4. **对 P1 的影响**:P1「没有任何 worker 能独立发起会轮换令牌的刷新」的**动机(缺陷 A 级联)在"共享一份真文件"下由 claude 自带的重读 + 竞态 + 跨进程锁化解**,故"给 worker 发 dummy RT 阉割 / 单飞刷新器"的三层 neuter 方向**可能过度设计,降级**;保留的真不变量是 P2 结果层("零互相登出、单次登录")。设计据此**第三次重开**,收敛到"让各席位凭据*目录*直挂 Windows 真文件目录 + 靠 claude 自协调"。
> 5. **新的 load-bearing 假设(替代旧的'代理/保鲜'假设),必须真机 spike 先证**:worker 凭据**目录**直挂 `/mnt/c` 真目录(**不能用 symlink**——rename 会把 symlink 替换成普通文件、写不穿真文件)后,drvfs 上:①rename 是否原子;②chmod 0600 是否生效(WSL 无 metadata 挂载可能忽略/报错致写失败);③WSL worker 与 Windows 宿主 claude 跨 OS 同写一文件的锁是否互认(退一步有 claude 重读 + 竞态兜底)。**只能在用户 Win11/WSL2 真机验**,spike runbook 已交用户。
> 6. **验收三条(tier-3,真机)**:① worker 骑用户单次登录起到 IDLE;② 刷新时新 RT **原地写进 Windows 真文件**;③ 第二个 worker 或用户宿主 claude **不被登出**。
>
> 详见 memory `[[project_ah_claude_cred_atomic_write_clobbers_symlink]]` 与本会话逆向三 agent 结论。
>
> **drvfs 真机 spike 结果(2026-07-12 晚,用户 Win11/WSL2 亲跑;SSOT=`/root/.claude/.credentials.json` → `/mnt/c/Users/test/.claude/.credentials.json`)**:上条 §5 的三条 load-bearing 假设**实测**。挂载为 **9p**(`aname=drvfs;path=C:\`,`cache=0x5`)、**无 `metadata` 选项**。①**rename 原子:通过**(tmp→target 落成 regular file、内容正确);②**chmod 0600:被 9p 静默忽略(mode 停在 777)但*不报错***——故 claude 写盘那步 chmod 不会致写失败,凭据停在 777(WSL /mnt/c 既有情况、非本方案引入,NTFS ACL 另算);③**mkdir 跨进程锁:WSL 侧有效**(首锁成、二锁拒)——防惊群并发刷新的关键闸;④**实锤 rename 替换 symlink 成普通文件、不写穿 SSOT**——故设计**必须"目录直挂" /mnt/c 真目录、禁用 symlink**;也解释近期现场 host 文件为何未坏。**仍留 tier-3 真机验**:WSL worker 与 Windows 宿主 claude **跨 OS 同写同锁是否互认**;SDK 层内存持旧 RT 的窄窗口。**结论:direct-dir 方案在 drvfs 层绿灯,可进设计。**
>
> **作用域隔离验证(2026-07-12 晚,逆向 2.1.207 穷举)**:确认 `CLAUDE_SECURESTORAGE_CONFIG_DIR` **只**喂 `jpe()`(凭据存储目录),`jpe()` 全部 8 个调用点均为凭据/OAuth;settings.json / `.claude.json`(trust+projects)/ session·todos / CLAUDE.md·角色规则 / MCP 配置 全走**独立**的 `mn()`(=`CLAUDE_CONFIG_DIR ?? HOME/.claude`),二者正交。→ **设计 = 保持每沙箱 `CLAUDE_CONFIG_DIR=<sandbox>/.claude` 不变(其余配置各自隔离)+ 设 `CLAUDE_SECURESTORAGE_CONFIG_DIR=<共享真目录>`(仅凭据共享)**;并使现有 claude `.credentials.json` symlink/revival-relink 逻辑冗余可删(消 ah#18 病根)。**实施须知**:①共享目录还会落 4 类刷新锁 + 父目录一个 `<dir>.lock`(多沙箱刷新经此串行=防惊群所需,但 WSL2↔Windows 跨 OS 锁语义留 tier-3 验);②`CLAUDE_SECURESTORAGE_CONFIG_DIR=""` 空串强制 `HOME/.claude` 并忽略 CLAUDE_CONFIG_DIR,注入必须给非空真实路径;③designOauth/MCP OAuth token 同存 `.credentials.json` 内,一并共享一并轮换。**待真机确认**:Windows 侧 claude 用 plaintext 文件后端而非 Cred Manager 密钥库(近期事故中 /mnt/c 那份 `.credentials.json` 活着且随登录更新,已强烈提示为 plaintext,真机可 10 秒复核)。

---

Status: 设计层 2026-07-12 重开(见上);历史 — converged after a3 adversarial review (2026-07-10) — the review found the original copy-on-create mechanism does not survive OAuth refresh-token-rotation in production; P1 was substantially redesigned to a host-side token-proxy architecture as a result (see design.md and `research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §五/§七.4). Scope grew from a one-line fix to a small service — flagged explicitly, not downplayed. **Not yet cleared for implementation** — awaiting operator/user sign-off. Independent line — no dependency on `ah-perception-arbiter` or `ah-control-plane-refactor`; can schedule separately.

Source material: `research/architecture-assessment-converged-2026-07-09.md` §一.13.

**2026-07-11 第三例现场(优先级升级)**:用户 Win11/WSL2 真机上,refresh 竞态残根(`expiresAt: 0`)写穿 symlink 链 `/root/.claude/.credentials.json → /mnt/c/Users/test/.claude/.credentials.json`,ah 栈全部 claude 席(master/d1/g1/g2)登出,且**首次殃及用户宿主机 Windows 的 claude 登录态**。完整证据链与 spec 影响见 `incident-2026-07-11-wsl2-symlink-logout.md`——design 需补 WSL2 验收项:任何路径不得写穿宿主 `/mnt/c` credentials。同族第 3 例,排期应提前。

## Scope

In scope:
- Replace the current shared-credential symlink model with per-worker independent credential materialization for the Claude provider (the only provider this mechanism currently exists for — see Existing Grounding).
- Failure/rotation isolation: one worker's credential rotation or logout must not invalidate another worker's live session.

Out of scope:
- Non-Claude provider credential handling (codex/antigravity have their own auth models, not touched by this spec unless investigation finds an analogous shared-credential pattern — if so, file as a follow-up, don't silently expand scope here).
- Credential storage security hardening beyond what's needed to fix the sharing bug (e.g. this spec does not take on secrets-at-rest encryption as a goal unless it falls out naturally from the fix).

## Existing Grounding

- `link_credentials` (`src/provider/home_layout.rs`, function present on current HEAD, ~line 658-664 per architecture assessment though exact line has likely drifted) symlinks `{source_home}/.claude/.credentials.json` into each materialized worker's Claude home directory (`layout.claude_dir.join(".credentials.json")`) via `symlink_auth_file`. All workers sharing one `source_home` therefore share **one physical credentials file** — not a copy, a symlink to the same inode.
- This is named in the architecture assessment as directly corresponding to a known incident class: an OAuth token rotation or logout on one worker's session invalidates the shared file, and every other worker sharing that symlink loses its session simultaneously, mid-task, with no independent recovery path.
- `materialize_trust` (same file, adjacent function) has a related but distinct pattern for `.claude.json` trust state — copy-if-missing rather than symlink. This spec should determine whether trust-state sharing has the same failure class or is already safe by virtue of being a copy (design.md to confirm by reading the actual copy-vs-symlink semantics, not assumed from this note).

## 根因(二进制实证,2026-07-11 operator 逆向 CLI `2.1.207`)

对生产二进制的凭据存储层逆向,坐实了平台差异与两个独立缺陷(证据函数名可复核):

- **凭据存储是可插拔多后端,`.credentials.json` 只是兜底**。二进制含:macOS Keychain(`execFile("security",["find-generic-password",...])`)、Windows Credential Manager(`isWindowsCredManagerAvailable`)、libsecret;`name:"plaintext"` 这个 provider(明文文件)**仅在无系统密钥库时启用**——正是 headless Linux 与 WSL2 的处境。→ macOS/Windows 原生用单一 OS 仲裁的密钥库(一份令牌、系统串行),天生免疫本类 bug;只有落到明文文件后端才中招。
- **写盘是原子改名**:`写 <path>.tmp.<rand> → rename(tmp, target)`;`rename` 失败(如跨设备 `EXDEV`)**退化成 `copyFile`**(函数 `Jm`)。
- **缺陷 A(所有文件后端平台):共享刷新令牌 + RTR = 级联登出。** 原子改名让*文件*各自独立,但*服务器端令牌血缘*没独立:N worker 同 seed 起步,谁先刷新就轮换作废旧 RT,其余持旧令牌者下次刷新 `invalid_grant` → 写登出残根 → 被踢。原生桌面碰不到(单份令牌);原生 headless 少碰(人手错峰);我们同步拉起 N worker 同到期 → 惊群精准命中。**只有把刷新搬出 CLI 做单飞才能解**(CLI 内部刷新无法注入锁)——即 Plan B Gateway 的 single-flight。
- **缺陷 B(WSL2 特有,第三例的越栈severity):跨设备改名 → copyFile 写穿宿主。** WSL2 的 seed 是指向 `/mnt/c` 的 symlink,`rename(tmp, target-symlink)` 跨 9p/drvfs 设备边界报 `EXDEV` → 退化 `copyFile` 跟随 symlink **写穿到 Windows 真文件**,登出残根盖掉用户宿主机登录。纯 Linux 单文件系统上同样的 rename 会*替换本地 symlink*(不写穿),故本缺陷是 `/mnt/c` 跨设备符号链接专属。→ Plan B 下 worker 无 `.credentials.json`,自然消灭;WSL2 验收硬项(任何路径不得写穿 `/mnt/c`)天然满足。

**结论(2026-07-12 修正)**:copy-per-worker 的**天真版**(每 worker 一份含真 refresh token 的独立拷贝、各自刷新)只堵缺陷 B、堵不了缺陷 A(RTR 级联),已弃。但"解 A"的正道**不止 Gateway 一条**——核心不变量是 **"没有任何 worker 能独立发起会轮换令牌的刷新"**;实现它的机制是开放设计问题(单飞刷新器 + 保鲜 access token 让 worker 永不触发刷新 / claude 自带 keystore 后端仲裁 / 代理 …),交 o1 发散收敛。2026-07-12 补充实证(见 `incident-2026-07-12-...`):Gateway 那条具体实现三 PR 从未激活,故设计层重开、不再钦定 Gateway。

**2026-07-12 补充二进制事实(operator 调研,佐证发散空间)**:①claude 只在**收到 401 时被动刷新**(非主动定时),故"access token 始终保鲜 → worker 永不撞 401 → 永不刷新 → 永不触发轮换"是一条不需要代理的解 A 路径;②claude 把 refresh token **缓存在内存、刷新前不重读文件**(强推断),故"共享一份文件"无效(内存里那份不共享),但"单一刷新器 + per-worker 保鲜拷贝"可行;③凭据后端可插拔,`plaintext` 只是无系统密钥库时的兜底——**原生 macOS/Windows 靠单一 OS 密钥库仲裁天生免疫**,提示"给 WSL claude 一个仲裁式 keystore 后端"可能是比刷新器更底层的解(o1 要挖)。

## Requirement P1: No Worker Independently Holds a Refreshable Credential

**Revised 2026-07-10 after a3 adversarial review** (`research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §五) found the original "give each worker its own copy of the file" mechanism does not achieve isolation in practice: OAuth refresh token rotation (RTR), standard on Auth0-family flows, means N independent copies of the *same starting token* will cascade-invalidate each other the first time any two workers' refresh windows overlap — copying the file does not copy independent *server-side* session lifetime. See design.md for the full mechanism and why a token-proxy architecture (a3's proposed fix, adopted) is required instead of a file-level change.

Acceptance criteria(2026-07-12 泛化为不变量,不再钦定"代理"实现):
- **核心不变量**:改动后,**没有任何 worker 能独立发起一次"会轮换服务器端令牌血缘"的刷新**。达成方式开放(单一刷新器持有唯一可刷新凭据 / 让 worker 永不触发刷新 / keystore 后端单点仲裁 / 代理 …),由设计收敛。凡"worker 自己手里有可独立使用的真 refresh token 且能自刷新"的方案都不满足本条。
- **隔离验收**:worker A 活动触发的凭据刷新,不影响 worker B 继续发认证请求——用测试驱动:一个模拟 worker 上下文触发刷新,断言并发的第二个 worker 上下文请求不受影响(替代原 file-mutation 测试——文件独立性从来不是关键属性,**请求层/会话层独立性**才是)。
- **单飞**:多 worker 并发本会各自触发刷新时,实际只发生**恰好一次**真实上游刷新,不是 N 次竞态(无论刷新由刷新器还是任何单点承担)。
- **初始种子**:唯一那份可刷新凭据从哪来,是设计决策(design 写清)——大概率=用户平时那个 CLI 的**单次登录**物化一次,**绝不下发到 worker 沙箱**。呼应北极星:单次登录。
- **开放技术假设(按方案分支)**:若方案依赖"把 CLI 指向本地端点/替代端点"(代理类),则该能力是 load-bearing 假设,spike 必先证实;若方案走"保鲜文件让 CLI 永不刷新"或"keystore 后端",则依赖点不同(CLI 是否每请求重读文件 / 是否接受自定义 keystore 后端)——**每套候选方案在 o1 发散时各自标注自己的 load-bearing 假设与验证 spike**。

Testability: `--lib` for the single-flight/isolation logic against a fake upstream token endpoint; the real CLI-proxy integration is CI-integration or manual verification, gated on the open question above being resolved first.

## 需求变更记录(2026-07-12,operator 签字,非静默削减)

设计收敛(`design-rev.md` 三层架构)冻结并经真二进制 spike 验证后,operator 对两处原始不变量做了显式松动,记录如下(依据:2026-07-12 会话中 operator 原话"你升级的 c1 spike 阻塞 + D-1 范围,两条都定了"):

1. **F5 残余赌注(上游对未签发 dummy token 的 invalid_grant 处理)不花真凭据验证**——operator 裁定:c1 spike#1/#2 已用 dummy 凭据把两条生死 spike(Layer2 pinning 可行性、Layer1 dummy 行为安全性)打穿,均 PASS;唯一未测的"语法真实但已失效的 RT 打到上游是否触发盗用检测级联"这一残余赌注,**接受为显式验证债,不发放真实可废弃测试账号去验证**。前提条件:**仅当 Layer 2(定点代理)被停用/失效时才需要回验**——Layer 2 在场时 dummy 永不出网,此赌注归零。不得在文档中把这条标为"已验证",须始终标"已接受残余赌注 + 验证债"。
2. **D-1(ahd↔用户原生 CLI 的 defect-A)从"零本机登出"松动为"接受残余窄竞态"**——user 亲自确认:接受"ahd 宕机 + AT 过期 + 用户正好在用原生 CLI"三条件同时发生才会登出一次的物理天花板(RTR 机制本身的极限,非实现疏漏);不要求实现完全独立于 ahd 的原生刷新器保护。此为 user 签字的需求松动,非 master/d1 自行削减。

上述两条不改变 P1/P2 的核心不变量表述,只框定其验证范围与残余风险边界;`design-rev.md` §十一(spike 验证结果)与 §四 C-4 已据此更新。

## Requirement P2: Rotation/Logout Failure Isolation

A credential failure (expired token, revoked session, explicit logout) on one worker is observable and recoverable independently of other workers' sessions.

Acceptance criteria:
- When worker A's credential becomes invalid, workers B-E's sessions continue unaffected (direct consequence of P1, but stated separately because it's the actual operational property being bought — P1 is the mechanism, P2 is the outcome).
- Worker A's credential failure is surfaced as an observable event (not a silent hang or an opaque provider-CLI error the operator has to dig for) — design.md should specify what "observable" means concretely (log line, `ah ps` sub_state, event stream entry) given whatever the codebase's existing failure-surfacing conventions are.

Testability: `--lib` for the "other workers unaffected" property; the observability surface may need a small integration check depending on where in the stack the failure is currently caught (or not caught) today — implementer should trace the current failure path before designing the fix, since "how does the CLI currently behave when credentials are invalid" hasn't been audited for this spec pass.
