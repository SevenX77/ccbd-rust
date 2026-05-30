# ah 真 dogfooding 验收 — 真实行为记录 (findings)

环境: ccbd-rust, AH_STATE_DIR=target/dogfood_play, 真 ahd binary (13.5MB, 源码 build), bash provider (真 OS 进程真 tmux pane真 reader 路径). systemd --user 可用.

> 标注: 证据度 × 影响度 × 方案置信度 (宪法3 三轴)

---

## ✅ 已验证工作正常 (真实观察)

### 守护进程基础 (C1 基础 / E5 锚点)
- `ahd` 起来: PID 活, `ah ping` 返 `ok=true socket=.../ahd.sock`, `ah ps` 空表正常.
- DB 初始化 WAL 模式: state dir 有 `ahd.sqlite` + `ahd.sqlite-wal` + `ahd.sqlite-shm` (E5 WAL 锚点真实).
- daemon.log 干净: db initialized → startup reconcile complete (reconciled=0) → UDS RPC listening.
- state dir 文件: `ahd.sock` (srw, UDS), sqlite 三件套, daemon.log.

### Agent 生命周期推进 SPAWNING→IDLE (D 维度)
- `ah --config mech.toml start --wait`: session_id 返回, a1/a2 bash pid 返回, EXIT=0.
- `ah ps`: a1/a2 state=IDLE sub_state=Matched, pid 真实. 生命周期推进正确.

### 进程隔离架构 (A / E4)
- 每个 agent 跑在**独立 systemd --user scope**: `run-<hash>.scope` = `ccbd-agent-a1@ahd-<sockethash>`, a2 同理.
- tmux server 也在独立 scope: `ahd-tmux-<hash>.scope`.
- **socket 隔离 (E4)**: ah 用自己的 tmux socket `ahd-71f248638a334b87`, 跟 ccb 的 `default`/`ccbd-*` socket 完全隔离. 多实例可并存.
- spawn cmd (daemon.log 实证): `systemd-run --user --scope --description=ccbd-agent-a1@... --slice=ccb-ccbd-rust-ccbd-agents.slice -- env ... bash --noprofile --norc -i`. provider HOME/env 重定向真实 (非 bwrap).

### 孙子进程 cgroup 归属 (B3 前半)
- a1 `ah ask a1 "sleep 1000 & disown"` → sleep 进入 a1 的 scope cgroup.procs (与 a1 bash 同 cgroup). 证明 cgroup 层面**能**收割孙子.

### 修复假设验证 (针对下面 BUG)
- `systemctl --user stop <agent-scope>` → 孤儿 sleep **立即死**. 证明 scope-stop / cgroup.kill 能收割 disown 孙子. 修复路径确定.

---

## 🔴 BUG-1: `ah kill --session` 泄漏 disown 孙子进程为孤儿 (证据High × 影响High × 方案A)

### 真实行为
1. session sess_b7e7eab7 (master bash + a1/a2 bash), a1 内 `sleep 1000 & disown` 生成孙子 (PID 1682508, 在 a1 scope cgroup 内).
2. `ah kill --session sess_b7e7eab7` → DB state=KILLED (a1/a2), **a1/a2 主 bash 进程被杀** (1678576/1678605 ESRCH ✓), tmux server 拆除 ✓.
3. **但孙子 sleep 1682508 存活, PPID=1 (reparent 到 init, 成孤儿), 持续运行**. ❌
4. **a1 的 systemd scope `run-r179eeef2` 仍 active/running**, cgroup.procs 里只剩这个孤儿 sleep.

### 根因 (设计缺陷, 非实现缺陷)
- ah 为每个 agent 创建 systemd scope (本意 = cgroup reaping 兜底), 但 **kill 路径只对 agent 主进程 pidfd SIGKILL + tmux kill-session** (daemon.log: `registry: killed agent session during cleanup`), **没有 `systemctl --user stop <scope>` / 没写 cgroup.kill**.
- 主进程死后, 它 disown 的后台子进程 reparent 到 PID 1, **scope 因仍有进程而不退出**, 孤儿 + scope 双泄漏.
- 印证 a1 audit (system.rs:128-174 `cascade_kill_session_agents_sync` 只对主进程 pidfd SIGKILL).

### 影响 (正中 user #1 关切)
- user 原话: "master 被 kill...所有进程全部被杀干净, 没有任何孤儿进程". **当前不满足**.
- 任何 agent 后台跑进程 (dev server / cargo watch / `&` disown / nohup) 在 `ah kill` / 级联 kill 后**泄漏为孤儿**, 长期占内存/端口/fd.

### 修复方向 (置信度 A)
- kill 路径改用 **`systemctl --user stop <agent-scope>`** 或写 **`cgroup.kill`** (cgroup v2 一次性杀全树), 替代/补充当前 pidfd-主进程-only.
- 已验证 scope-stop 能收割孤儿.
- 性质 = `feat/refactor` (重画 kill 接口契约用上本就创建的 scope), 非 `fix` 补丁.

---

### ✅ BUG-1 已修复 + 物理实证确认 (a1 实施, 主控用新 binary 复验)
- a1 修法: 新增 `stop_agent_scopes_with_runner` (SystemctlRunner trait 抽象) → `systemctl --user stop <agent-scope>` 收割整棵 cgroup; session.kill + master 死级联**共用同一 reaper** (system.rs:136 / handlers.rs:117 / master_watch.rs:38). 失败 WARN 降级.
- a1 测试: 单元测 `test_cascade_kill_session_agents_stops_matching_agent_scopes` 绿 + 真复现测 `tests/orphan_reap.rs::session_kill_reaps_disowned_grandchild_in_agent_scope` (真 bash agent+scope+sleep disown) 绿 + ah_dogfooding 回归 13 passed.
- **主控物理实证 (新 binary 16:01)**: session sess_62ae117f, a1 spawn `sleep 7777 & disown` (PID 1865973 在 a1 cgroup), `ah kill --session` → **孙子 1865973 ESRCH (收割) ✅**, a1 scope 不再 active.
- 🔸 小尾巴 (待精修): scope 结束在 `failed` 状态 (内含进程被 SIGKILL), failed 单元残留需 `reset-failed` 才消失, 会累积. 建议 reaper 在 stop 后 reset-failed 或用 cgroup.kill 让 scope 干净 inactive. (BUG-2 同类, 不阻塞 — 孤儿已收割.)

### BUG-1 扩展确认: master kill 级联路径同样漏孤儿 (B2, 修复前)
- fresh session sess_869b3ad4, a1 内 `sleep 4321 & disown` (PID 1737708 在 a1 scope cgroup).
- `kill -9 <master pane pid>` → 级联触发, a1/a2 被杀, ahd auto-shutdown (master 死→守护退出, 符合预期).
- **但孙子 1737708 存活, PPID=1 (孤儿), a1 scope 仍 active**. 与 `ah kill --session` 完全一致.
- **结论**: orphan 泄漏是**通用**的, 不限显式 `ah kill` —— `cascade_kill_session_agents` 所有路径 (显式 kill / master 死级联) 统一用 pidfd-主进程 + tmux-kill, 从不 cgroup-kill. 一个根因, 一处修.
- 正向: ahd 在 master 死后 auto-shutdown 正常; 优雅退出时 socket 被清理 (无 stale socket).

## 🟠 BUG-2: 泄漏的 systemd scope + tmux server 残留 (证据High × 影响Med × 方案A)

### 真实行为
- 启动 dogfood 前, 发现 4 个 stale scope: `ahd-tmux-{0459872f,4bd86c92,5928f35c,dd7838c6}.scope`, 全是 `agent_dogfood_a1` tmux session, 14:06-14:10 创建 (= 我早先 `cargo test ah_dogfooding` in-process 测试残留), **测试结束从未清理**.
- 同根于 BUG-1: scope/tmux server 不被 stop, 测试 harness teardown 杀进程但没 stop scope.

### 影响
- 反复跑测试/反复 start-kill → systemd scope 单元 + tmux server 无限堆积 (F1/F4 风险真实存在).

### 修复方向
- 同 BUG-1: 统一在 cleanup 路径 stop scope. 测试 harness 也需 teardown stop scope.
- (已手动清理本次 4 个 stale scope.)

---

## ✅ 守护进程侧机制 (真 bash provider, 续)

### C4 不存在 agent (✓)
- `ah ask ghost` → 立即 `AGENT_NOT_FOUND` RPC error, 不空等, 守护进程不崩.

### E5 SQLite 并发写争用 (✓)
- 20 并发 `ah ask a1` → **无 `database is locked`, 无 panic**, ahd 存活. WAL + busy_timeout 生效.
- 仅有 WARN `failed to mark agent BUSY ... current_state=BUSY` = 并发 dispatch 竞态的良性日志, 非崩溃.

### B8 master hang 检测 (GAP, 低severity)
- `kill -STOP <master>` 冻结 master (进程活但不响应) → **ah 不检测** (ah ps 仍 session alive / a1 IDLE, 无 STUCK).
- `master_watch` 只有 pidfd 死亡检测, 无 master 健康检测 (印证 a3 audit). SIGCONT 后正常恢复.
- 评级: GAP 非 BUG (hung master 罕见, 死亡路径 pidfd 正确). 可选补 master health.

## 🟡 BUG-3: cancel 不清 in-flight 子进程 + agent 卡 WAITING_FOR_ACK (证据High × 影响Med × 方案B)
- `ah ask a1 "sleep 20"` 运行中 `ah cancel <job>` → job 标 cancelled, 但:
  1. a1 状态停在 `WAITING_FOR_ACK` (没干净回 IDLE).
  2. `sleep 20` 子进程仍在 a1 cgroup (没杀).
- 同 BUG-1 根因延伸 (无人 cgroup-kill in-flight 子进程). 影响: cancel 后残留子进程 + 状态不干净.
- (bash provider cancel 语义可能与真 LLM 不同, 真 provider 需复验; 方案置信度 B.)

## 待真 provider 验 (bash provider 不触发)
- **配置隔离 A1/A2/A6 必须真 claude/codex/gemini**: bash provider `requires_home_materialization:false`, spawn cmd 是 `HOME=/home/sevenx` 真 home, 不物化隔离 home. 真 provider 才有 CLAUDE_CONFIG_DIR/CODEX_HOME/GEMINI_CLI_HOME 隔离.

## ✅/🔴 真 provider 配置隔离 (A 维度, 真 codex/gemini/claude)

真拓扑: master=bash, a1=codex, a2=gemini, a3=claude. `ah start --wait`.

### A1 配置隔离 ✓ (真 provider 实证)
- 各 agent 独立 sandbox HOME + provider 专属 config env (daemon.log spawn cmd 实证):
  - a1 codex: `HOME=~/.cache/ah/sandboxes/d1d4e606fb34`, `CODEX_HOME=.../.codex`
  - a2 gemini: `HOME=~/.cache/ah/sandboxes/98c0d0717e59`, `GEMINI_CLI_HOME=.../.gemini`
  - a3 claude: `HOME=~/.cache/ah/sandboxes/39f17e89e8ff`, `CLAUDE_CONFIG_DIR=.../.claude`
- 三 HOME 互不相同, 完全隔离. session sandbox 也在 `$STATE_DIR/sandboxes/sess_xxx/{a1,a2,a3,master}`.

### A2 OAuth 共享 = symlink, host 未破坏 ✓
- sandbox 里 OAuth 文件是 **symlink 到 host 原始**: a3 `.credentials.json -> ~/.claude/.credentials.json`; a2 `oauth_creds.json -> ~/.gemini/oauth_creds.json` (+ google_accounts/installation_id symlink).
- 跑完整轮 (含 gemini 失败重试) 后 host 三个 OAuth 文件 sha256 **全部未变** (A2 PASS). agent 读共享 OAuth 不污染 host.
- 风险点 (待观察): provider 若写 OAuth (gemini token refresh) 会穿 symlink 写 host. 本轮未触发.

### A6 codex/claude 隔离启动成功 ✓ (claude onboarding gap 已修复)
- a1 codex **IDLE/Matched**, a3 claude **IDLE/Matched** — 真 provider 在隔离 sandbox 里认证成功到 IDLE.
- a3 claude sandbox 已**种子化** (`.claude.json` + CLAUDE.md + projects/ 预置), 所以**没卡 first-run 向导** (旧 memory `project_ah_pr2_smoke_onboarding_gap` 的 claude onboarding gap 此环境已修复).

## 🔴 BUG-4: gemini agent 在隔离 sandbox 认证失败, 永卡 SPAWNING (证据High × 影响High × 方案B)
- a2 gemini 90s 没到 IDLE, 卡 SPAWNING. pane 实证: gemini-cli 掉进**交互式 OAuth 流程**:
  ```
  Please visit the following URL to authorize... Enter the authorization code:
  Failed to authenticate with authorization code: invalid_grant
  Failed to authenticate with user code. Retrying...
  ```
- gemini **没认到** symlink 的 `oauth_creds.json` (GEMINI_CLI_HOME=sandbox/.gemini + symlink creds 没让 gemini 跳过 auth), 掉进 re-auth.
- **叠加 ah 缺陷**: ah prompt matcher 把 "Enter the authorization code:" 判为 UNKNOWN prompt (没匹配 trust_path/codex_update case), 且日志明示 "prompt integration deferred unknown prompt while agent is transient (SPAWNING)" → SPAWNING 期不处理 unknown prompt → gemini 无限重试卡死.
- **双重根因**: (1) gemini OAuth 隔离方式 (symlink + GEMINI_CLI_HOME) 不生效; (2) ah 对 SPAWNING 期的 unknown/auth prompt 推迟处理, 造成 readiness 死锁.
- **影响**: 当前 gemini agent 无法在 ah 隔离 sandbox 干净启动 (codex/claude 可以). 直接影响 user "配置隔离 + 生命周期推进" 目标.
- 对比: ccb 的 a2 gemini worker 正常 (ccb 用不同 mount 方式), 说明是 ah 隔离实现的 gap.

### BUG-4 修复设计 (a2 调查, file:line 实证)
- **根因 A (OAuth symlink)**: `home_layout.rs:17` PROVIDER_AUTH_WHITELIST 含 .gemini/oauth_creds.json; `:231-256` link_auth_file_into_sandbox 用 **symlink** 挂 host 凭证. gemini-cli 需读写 oauth token (刷新 access token), 遇 symlink/不可写 → 不认 → re-auth. **修复**: OAuth 动态凭证 (whitelist) 改 **Copy** (可写) 进 sandbox, 各 agent 独立维护 token 刷新.
- **根因 B (SPAWNING defer 死锁)**: `integration.rs:146-149` is_prompt_demote_deferred_state 把 SPAWNING+WAITING_FOR_ACK 定为 defer 状态; `runner.rs:515` is_llm_steady_state 要 IDLE/BUSY 才走慢路径; `integration.rs:93-100` SPAWNING 期 unknown prompt → Deferred. gemini auth prompt 在 SPAWNING 被 defer → agent 等输入 → 永不 IDLE → 死锁. **修复**: SPAWNING 静默宽限期, unknown prompt 持续滞留 (snapshot 稳定超阈值) → 晋升 PROMPT_PENDING + emit UNKNOWN_PROMPT_DETECTED.
- **关键**: host gemini 凭证有效 (整场 ccb a2 gemini worker 正常) → invalid_grant 是 ah symlink 隔离问题, 非 stale creds → 纯代码修复.
- 双修才全自动鲁棒 (只修 A → gemini 认证成功不弹 prompt; 只修 B → 死锁解开但需人工填码).
- ⚠️ SOP-04 注意: copy OAuth 凭证进各 sandbox = 凭证多副本落盘, 需 0600 权限; memory 提过 gemini-cli migration 会删 plaintext oauth_creds.json, copy 方案需验证不被删.

### B5 ah stop / auto-shutdown 优雅关 ✓
- `ah kill --session` (master=bash 死) → master_watch → 级联 → ahd **auto-shutdown** → `ahd.sock` **被删** → 0 残留 scope. socket 清理干净, 无 stale socket.

## ✅ BUG-4 已修复 + 物理实证 (新 binary 真 gemini)
- 根因 A (copy): gemini sandbox oauth_creds.json 现为真文件 (copy, 0600, 非 symlink) ✓; host OAuth sha256 未变 (copy 方案 SOP-04 安全).
- 根因 B (晋升): gemini auth prompt 不再死锁 SPAWNING → 晋升 **PROMPT_PENDING** ✓ (daemon.log: deferred while transient stabilizes → promote).
- 完整恢复路径: `ah prompt resolve a2 --keys 1` → PROMPT_PENDING → **IDLE** ✓; gemini 真答题 `✦ PONG-GEMINI-OK` ✓.
- commit: fix(ah-gemini) f45329c.

## 🟡 新增 follow-on 发现 (BUG-4 修复后暴露的下一层, 未修)
- **BUG-5 (SOP-04 潜在违规, 证据High×影响High×方案A)**: `src/provider/manifest.rs:52-89` ENV_PASSTHROUGH 白名单含 `ANTHROPIC_API_KEY/GEMINI_API_KEY/GOOGLE_API_KEY/OPENAI_API_KEY`. 若 host 设了这些 env, 会透传给 agent → provider 可能走 API key 认证 → **违反 SOP-04 OAuth-only 铁律**. 本次 host 未设故未触发, 但白名单本身是潜在违规. 修复: 从 ENV_PASSTHROUGH 移除 API key 变量 (或显式 strip). ⚠️ 触及 OAuth 策略, 宜 user/a2 确认.
- **BUG-6 (gemini trust + auth 对话框未自动匹配, 证据High×影响Med×方案B)**: gemini 首启弹 "Do you trust the files in this folder?" + auth 方式选择, ah 的 trust_path_01 matcher 没匹配 gemini 这些对话框文本 → 需 1-2 次 `ah prompt resolve` 手动解才到 IDLE. 全自动启动需补 gemini trust/auth 对话框的 matcher case. (修复 B 已让它不死锁而是可解的 PROMPT_PENDING.)
- **BUG-7 (gemini completion 检测延迟, 证据Med×影响Med×方案B)**: gemini 已答题 (`✦ PONG-GEMINI-OK` 在 pane) 但 `ah ask --wait` 60s 超时没返回. gemini idle/completion 检测慢或漏. 需查 gemini idle_detection (LineEndRegex?) 真实性.
- **BUG-3 (cancel 不清 in-flight 子进程 + 卡 WAITING_FOR_ACK)**: 见上 (真 provider 需复验).
- **master hang 无检测 (GAP, 低severity)**: 见上 B8.

## ⏳ 待测 (campaign 继续)
- A1/A2/A6 配置隔离 (真 provider env 验证 + 凭证破坏) — 需真 claude/codex/gemini
- B1 /exit 级联 / B2 kill -9 master / B6 SIGHUP / B7 detach / B8 master hang
- C1/C6 真 SOP (需真 provider) / C2 证据拦截 / C3 排队 / C4 不存在 agent / C5 cancel
- D1 onboarding / D2 crash / D3 STUCK 真触发 / D4 缺 binary / D5 CAS 并发
- E1 编译 false STUCK / E2 tmux killed / D6 真 completion
- F1/F2/F3/F4 资源压力
