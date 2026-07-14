# 事故/缺陷:Module D 网关激活即打死所有 claude worker(ah_bin=current_exe 解析错误)

**日期**:2026-07-12
**发现场景**:operator 执行「登录鉴权换血」推进 ah#18(共享 OAuth 轮换登出),把含 Module D 的新 `ahd` 装上 → `ah start` → **spawn 第一个 claude worker(d1)即 `AGENT_UNEXPECTED_EXIT`,整个 session 回滚**。
**严重度**:P0(网关一旦激活,所有 claude provider 的 worker + master 全部无法 spawn;等于 Module D 不可上线)。
**当前处置**:已回滚到 pre-ModD 的 `ahd`(gateway 符号=0),活栈 `sess_64f60d67` 恢复正常(旧 ahd 无 gateway 路径,claude worker 正常起)。新 ahd 存于 `~/.local/bin/ahd.modD-2966de4` 待修复后重装。ah#18 回到已知债务态(worker 仍共享 `~/.claude/.credentials.json` symlink)。

---

## 根因(高置信度,filesystem + 源码双实证)

网关为每个 claude worker 包一层 bridge:sandbox 内起一个本地 TCP↔host-UDS 网桥,把 `ANTHROPIC_BASE_URL` 指向该 TCP 端口。启动命令(`src/claude_gateway.rs::bridge_wrapper_shell`):

```
{ah_bin} internal-bridge --uds {uds} --port-file {port_file} & 
for i in 1..5; do test -s {port_file} && break; sleep 0.1; done   # 只等 0.5s
if ! test -s {port_file}; then ... exit 126; fi                    # 没写 port 就 126
```

**`{ah_bin}` 解析错误** —— `src/platform/linux/scope.rs:448-450`:
```rust
let ah_bin = ah_bin
    .map(...)
    .unwrap_or_else(|| std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ah")));
```
- 这段在 **daemon(ahd)进程内**执行 → `std::env::current_exe()` = `/home/sevenx/.local/bin/ahd`。
- 但 `internal-bridge` 子命令与 `run_internal_bridge` 函数**只编进 `ah` CLI 二进制,不在 `ahd`**。实证符号计数:
  - `run_internal_bridge`:`ah`=13,`ahd`=**0**
  - `register_worker`/`claude_gateway`:`ahd`=55/265(网关服务端在 ahd,客户端 bridge 在 ah)
- 于是实际执行的是 `ahd internal-bridge …`。`ahd` 不认这个子命令 → 回落到「启动 daemon」路径并**挂起**(实测 `ahd internal-bridge --help` 挂死 >2min 不返回;手动跑给足 5s 仍不写 port-file)。
- `bridge.port` 永不出现 → 0.5s 等待失败 → `exit 126` → worker `sh -lc` 整体退出 → ahd 判 `AGENT_UNEXPECTED_EXIT` → 回滚 session。

**实锤 spawn 命令**(journalctl,d1):`… --property=BindPaths=/tmp/ah-gw-e77962c064d4f6bf.sock:/var/run/ah-gateway.sock -- sh -lc "… '/home/sevenx/.local/bin/ahd' internal-bridge --uds '/var/run/ah-gateway.sock' --port-file '…/d1/bridge.port' …"` —— ah_bin 就是 `ahd`,实锤。

## 为什么 CI 绿却运行时全崩(疗效债,呼应 operator O2/O5)

Module D(#146)+ UDS hotfix(#147)CI 全绿,但**从未有测试用真实 `ahd` daemon 端到端 spawn 一个真实 claude worker 走完 bridge 路径**。测试环境里 `current_exe()` = 测试二进制(恰好含 internal-bridge 子命令,因测试链接整个 lib),掩蔽了「daemon 的 current_exe 是 ahd、ahd 无此子命令」这一生产事实。→ 典型「本地/CI 绿 ≠ 真实激活」(O5),且「代码闭环 ≠ 实证闭环」(O2)。

## 设计根因(比代码 bug 更深一层:为什么设计没想到)

**不是"缺乏全局架构认知",而是"没调研、没复用已有解法"——正确解法就在隔壁 30 行,gateway 却在筒仓里另造了个错的轮子。**

项目里对"ah/ahd 是两个兄弟二进制、current_exe 在 daemon 里=ahd"这一事实**早有认知且已解决**:
- `src/provider/home_layout.rs:684 resolve_ah_binary()` —— 专门的帮手,docstring 明写这个坑(hook 在无 PATH 环境里裸 `ah` 静默 127 → 解析 `current_exe.parent()/"ah"`)。hook 注入 `build_ah_hook_command` 正确复用它。
- `src/bin/ah.rs:716` —— CLI 找 daemon 用 `current_exe.parent()/"ahd"` + fail-closed。
- 但 `scope.rs:450` gateway bridge 用裸 `current_exe().unwrap_or("ah")`,**没复用 `resolve_ah_binary`**,把 hook 那边已根治的 bug 又引回来。

失误性质:**P2 违反**(「解析 ah 二进制」应有唯一 owner=`resolve_ah_binary`,不该两套实现漂移)+ **P10 违反**(设计前未 survey 既有原语、未追「这段跑在 ahd 里」的全局影响)。**评审/CI 未拦下**:测试拓扑是单体(current_exe=测试二进制,含 internal-bridge 路由),生产是双二进制,测试里对、生产里错(O5 + 实证闭环洞)。

## 修复方向(交给执行模块,operator 不亲手改;消病根不打补丁=P1)

1. **首选:gateway 复用既有 `resolve_ah_binary()`**(现为 `home_layout.rs` 私有,提为 pub 或下沉到共享位置),让 `scope.rs:450` 的 ah_bin 走这个唯一 owner,而非裸 `current_exe()`。解析不到 `ah` 兄弟时 **fail-closed 报错(P7)**,不静默回落 ahd/裸 "ah"。
2. bridge 冷启动的 **0.5s 端口等待过紧**是次生风险(即便二进制修对,首次冷启动可能 >0.5s):放宽为有界重试 + 明确超时上抛(P7)。
3. **顺带审计**:全项目 `current_exe()` 共 5 处,确认只有 `scope.rs:450` 是裸用(ah.rs:716 与 home_layout 均已正确),防同类残留。
4. (可选、别扩 scope)让测试用**真 ahd 端到端 spawn 一个真 claude worker** 走完 bridge 路径,补上「测试拓扑=生产双二进制拓扑」的实证缺口——这条若做大就另开 spec。

## 验收判据(实证闭环,不是 CI 绿就算)

新 ahd 重装后 `ah start`:
- 7 agent(含 d1/r1 两个 claude worker + master)全部起到 IDLE,无回滚;
- worker environ 出现 `CLAUDE_CODE_USE_GATEWAY=1` + fake `ANTHROPIC_AUTH_TOKEN` + `ANTHROPIC_BASE_URL=http://localhost:<port>`;
- `bridge.port` 落盘、bridge 进程存活、host UDS 有 listener;
- worker 沙箱 `.credentials.json` **不再是**指向 `~/.claude/.credentials.json` 的共享 symlink;
- 真跑一轮任务,claude worker 能正常调用模型(网关换 token 生效),且 host token 轮换后 worker 不掉线(ah#18 正题)。

## 关联
- spec:`ah-per-worker-credentials`(Module D 本体)
- 原理:项目架构原则 A1(job-id/信号)、通用 P7(fail-closed)、P10(调研+全局影响)
- operator 纪律:O2 疗效闭环、O3 干预即补机制、O5 串并行验收
- 前序:#146 Module D、#147 UDS sandbox_root hotfix

---

## 追加(2026-07-12 换血实证):current_exe 是**第一层**拦路虎,还有第二层

从 `b1d67a8`(含 #149 `resolve_ah_binary_strict` 修复)重建 ahd 换血,实证:

- **修复生效**:spawn cmd 现在用 `'/home/sevenx/.local/bin/ah' internal-bridge`(正确兄弟 ah,不再 current_exe=ahd)。resolver 修复无误。
- **但仍崩**:`ah start` 三次全在 spawn d1(claude)时 `AGENT_UNEXPECTED_EXIT` 回滚整个 session。
- **规律**:**每个走网关 bridge 的 claude 进程(master + d1 + r1)都在 <200ms 秒死**(journal:master pid spawn 后 `failed to arm master revive watcher ... pid not alive`);codex(c1/c2)/antigravity 不走 claude 网关,是被 session 回滚**连坐** cascade 杀,非自身故障。
- **抓不到 bridge.err/bridge.port**:竞速 copier(无 sleep 紧循环)一个文件都没抓到 → cascade `rm -rf` 状态沙箱比抓取快;bridge 是否真写过 port 未知。

**根本结论**:**Module D 网关从未端到端激活过任何 claude 进程**。#146/#147/#149 三个 PR 全靠 CI 绿(单测)+ 代码审过,**第三层端到端 smoke(A2 tier-3)从来没做** → 根本性激活缺陷带着三个 PR 进了 main。这正是 A2/O2 说的病:tier-3 缺失。

**止血**:回滚 preModD ahd(网关符号=0,不做激活,claude worker 拿共享凭据但能跑),活栈恢复(session sess_6ca42bc7,master IDLE,7 worker IDLE),带已知 ah#18 共享凭据 bug(现状,可容忍)。

**发版阻断**:补丁版本不能发——不能发一个崩掉所有 claude worker 的网关。

## 第二层缺陷·下一步诊断(未做)
1. **隔离复现**:单个 claude worker 走网关,禁用 cascade(或用独立 harness),捕获 bridge.err + claude stderr,定位 claude 秒死真因(候选:bridge 连不上 host UDS / claude 拒 fake token 快退 / gateway host 侧 register_worker 未真正 serving / sandbox HOME 缺 onboarding 种子致 claude 首启退出)。
2. 定位后走 master+worker SOP 派修(operator 不代修),带精确 brief + RED 复现 + tier-3 端到端验收判据。
3. **发版 hold 到网关真激活**(过 tier-3)。
