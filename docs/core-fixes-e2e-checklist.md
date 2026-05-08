# core-fixes R1-R4 e2e 验证 checklist

## 适用范围

`core-fixes` 系列 (R1 1-Session-per-CLI / R2 ack chain / R3 absolute_path + sandbox / R4 master cmd default + attach + doctor) 的端到端验证。覆盖 NO_SANDBOX 模式 + sandbox argv 验证 (R3 部分项)。

## 跑法

```bash
cd /home/sevenx/coding/ccbd-rust

# 单 R 范围
bash scripts/r1_e2e.sh > /tmp/r1-e2e-out.log 2>&1
bash scripts/r3_e2e.sh > /tmp/r3-e2e-out.log 2>&1
bash scripts/r4_e2e.sh > /tmp/r4-e2e-out.log 2>&1

# R1+R2+R3+R4 联动 (4 真 LLM agent + ack chain)
bash scripts/core_fixes_full_e2e.sh > /tmp/full-e2e-out.log 2>&1
```

**绝对不 `tee`**: 历史 tee 触发卡死 30 分钟; 一律 `> file 2>&1`。

## 当前状态 (2026-05-08 跑出)

| 脚本 | 通过 | 失败 | 状态 |
|---|---|---|---|
| r1_e2e.sh | 10 | 3 | 部分通过 (R1 主线全过, T1.3.3 + R2 chain 子项 fail) |
| r3_e2e.sh | 7 | 0 | 全过 |
| r4_e2e.sh | 4 | 0 | 全过 (本会话刚修 T4.2.1 strace 检测) |
| core_fixes_full_e2e.sh | 3 | 3 | 部分通过 (state machine 子项 fail) |

总: 24 PASS / 6 FAIL。**必须先修干净 6 项 fail 才算 core-fixes ship 完成** (按 must-fix = 不合格)。

---

## R1 covered: 1-Session-per-CLI lifecycle

引用脚本 `scripts/r1_e2e.sh`。

| 任务 ID | 期望 | 检查方法 | 当前状态 | 证据 |
|---|---|---|---|---|
| T1.1.1 | 旧 shared `ccbd-agents` session 已不存在 (R1 反向彻底) | `tmux ls` 无 `ccbd-agents` 行 | **PASS** | `r1_e2e.sh:188-191`; log 行 |
| T1.1.2 | daemon SIGTERM 后 tmux `agent_*` / `master_*` 全清 | SIGTERM 5s 后 `tmux ls` 无 `agent_*/master_*` | **PASS** | `r1_e2e.sh` cleanup phase |
| T1.2.1 | ensure_session 锁定 PTY 尺寸 150 列 (后台 pane 不被 attach 改宽) | `tmux display -p -t agent_a1 #{pane_width}` == 150 | **PASS** (a1 + a2 + window-size manual) | `r1_e2e.sh` 三 PASS 行 |
| T1.3.1 | systemd-run scope property 含 `BindsTo=ccbd.service` | `systemctl --user list-units` 找 ccbd-tmux scope | **PASS** | `r1_e2e.sh:194-218` |
| T1.3.2 | 杀 master PID 后 5s 内 daemon 退出 (master.enabled=true) | killdog master, sleep 5, `pgrep ccbd` 空 | (覆盖在 [7/9] 段) | r1_e2e 跑过 (T1.3.3 fail 同 stage 暴露 master cascade 行为异常) |
| T1.3.3 | `master.enabled=false / auto_shutdown=false` 时 master 退出**不**杀 daemon | 配 false + kill 假 master + daemon 仍在 | **FAIL** "daemon 不应自杀但已退出" | `r1_e2e.sh:87-88`; `/tmp/r1-e2e-out.log:88` |
| T1.4.1 | agent.spawn 后 `tmux ls` 出现 `agent_<id>` (a1 + a2) | `tmux -S $SOCK ls` grep 两行 | **PASS** | `r1_e2e.sh:174-177` |
| T1.4.2 | `ccb-rust start` 不发 `layout_*` 字段 (内部协议) | grep `cli/start.rs` + `rpc/handlers.rs` 非 test 代码 | **PASS** | `r1_e2e.sh` PASS 行 |
| T1.4.3 | 旧 `layout=grid` config 给迁移错误 | 写 toml 含 `layout = "grid"`, 期望 spawn 报迁移错 | **PASS** | `r1_e2e.sh` PASS 行 |

## R2 covered: WAITING_FOR_ACK ack chain

引用脚本 `scripts/r1_e2e.sh` (bash agent ack chain) + `scripts/core_fixes_full_e2e.sh` Stage [5/8] (codex + gemini real LLM ack chain)。

| 任务 ID | 期望 | 检查方法 | 当前状态 | 证据 |
|---|---|---|---|---|
| T2.2.2 | ACK→BUSY 转换可观察 (state_change events 链) | 真 ask 后 events 链含 `IDLE→WAITING_FOR_ACK→BUSY→IDLE` | **FAIL** | `r1_e2e.sh:83` (bash agent: cmd=45 busy_idle=空); `core_fixes_full_e2e.sh:213` ("T2.2.2 未观察到 BUSY 转换") |
| T2.4.5 | send reply 含 ACK 状态; daemon log 含 `collect_reply` | tail daemon log grep `collect_reply` | **FAIL** | `r1_e2e.sh:85` "reply 未收集" |
| T2.5.1 | 并发 ask 互斥 (复用 IDLE guard, 串行处理) | fire 2 concurrent ask, events 链含 2 个 `command_received` | **PASS** | `core_fixes_full_e2e.sh:189` |
| T2.x WAITING_FOR_ACK 链 (codex) | 真 codex ask 后 events 链含 WAITING_FOR_ACK | sqlite `select * from events where new_state='WAITING_FOR_ACK'` | **PASS** | `core_fixes_full_e2e.sh:209` |
| T2.x WAITING_FOR_ACK 链 (gemini) | 真 gemini ask 后 events 链含 WAITING_FOR_ACK | 同上 (a3=gemini) | **FAIL** "WAITING_FOR_ACK 缺失" — gemini 直接 SPAWNING→IDLE | `core_fixes_full_e2e.sh:228`; full e2e log |

## R3 covered: absolute_path / sandbox bwrap

引用脚本 `scripts/r3_e2e.sh`。

| 任务 ID | 期望 | 检查方法 | 当前状态 | 证据 |
|---|---|---|---|---|
| T3.1.1 / T3.1.4 | sessions.absolute_path 存为 project_root_abs | sqlite `select absolute_path from sessions` | **PASS** | `r3_e2e.sh:163` |
| T3.1.2 | NO_SANDBOX agent pwd 含 project_root_abs (bash agent + 真 codex pane) | bash agent: 进 pane 跑 pwd; codex: `tmux display -p '#{pane_current_path}'` | **PASS** (bash + codex 两路) | `r3_e2e.sh:148, 176` |
| T3.1.3 | master pane cwd 一致 | (master.enabled=false 时跳过) | **SKIP** | `r3_e2e.sh:153` |
| T3.2.1 | sandbox agent 可读 project root 文件 (via bwrap argv `--bind <abs> /workspace`) | grep daemon log spawn cmd | **PASS** (argv 验证, 不真 spawn sandbox) | `r3_e2e.sh:253` |
| T3.2.2 | sandbox pwd 为 `/workspace` | grep daemon log argv `--chdir /workspace` | **PASS** | `r3_e2e.sh` PASS |
| T3.2.3 | sandbox 内 `.git` 默认 ro-bind | grep argv `--ro-bind .*\.git .*\.git` | **PASS** | `r3_e2e.sh` PASS |
| T3.2.4 | additional_ro_binds 注入 argv (host=sandbox 路径一致) | 写临时 ro_binds, grep argv | **PASS** | `r3_e2e.sh` PASS |

**注**: T3.2.1-T3.2.4 当前是 argv 静态验证 (不真 spawn bwrap). 真 sandbox spawn e2e 列入 mvp13 sandbox-e2e (待 NO_SANDBOX 通过后跑)。

## R4 covered: master cmd default + sh -lc + attach + doctor warn

引用脚本 `scripts/r4_e2e.sh`。

| 任务 ID | 期望 | 检查方法 | 当前状态 | 证据 |
|---|---|---|---|---|
| T4.1.1 | master cmd default (config 留空) 真启动 claude CLI 含完整 argv | 启 daemon w/ master.enabled=true cmd=null; tmux ls 含 `master_<project>`; pane 内有 `claude --dangerously-skip-permissions --continue /remote-control` | **PASS** | `r4_e2e.sh:103-127`; `/tmp/r4-e2e-out.log` "T4.1.1 真 claude CLI 已带完整 argv 启动" |
| T4.1.2 | sh -lc 透传 (cmd 含 shell 元字符不裂) | (覆盖在 T4.1.1 内 — master_cmd_default 通路用 sh -lc 包装) | **PASS** (隐含) | T4.1.1 间接验证 |
| T4.2.1 | `ccb-rust attach a1` exec `tmux ... attach -t agent_a1` | `strace -f -e execve` 捕获 ccb-rust → tmux execve, grep `"agent_a1"` | **PASS** (本会话刚修 strace 检测) | `r4_e2e.sh:151-194`; `/tmp/r4-e2e-out.log` "T4.2.1 attach a1 execve 含 \"agent_a1\"" |
| T4.2.2 | (paste-buffer deferred) | — | **DEFER** | commit `2460b0b` body 注 |
| T4.3.1 | doctor 健康检查无 false positive | (覆盖在 T4.3.2 同 daemon 启动内, doctor 无误报) | **PASS** (隐含) | T4.3.2 PASS 即 doctor 链路活 |
| T4.3.2 | 旧 `ccbd-agents` shared session 残留时, doctor 输出清理建议 | spawn fake `ccbd-agents` tmux session, 跑 `ccb-rust doctor`, grep "ccbd-agents" warning | **PASS** | `r4_e2e.sh:213-256`; `/tmp/r4-e2e-out.log` "T4.3.2 doctor 警告 legacy ccbd-agents 出现" |

## 已知 race / limit

1. **真 LLM 4 agent 并行 spawn 慢**: codex/codex/gemini/claude 同时 cold start 60-120s, full e2e Stage [4/8] 设 `timeout 180`; 系统 busy 时可能不足
2. **gemini WAITING_FOR_ACK 路径异常**: gemini 在 ack 前 SPAWNING→IDLE direct, 跳过 WAITING_FOR_ACK marker — 待 R2 dispatcher state machine fix
3. **bash agent ack chain T2.2.2 / T2.4.5 fail**: bash provider 没有 reply collector, daemon log 无 `collect_reply` — bash 用作 placeholder, 这两项行为偏差需要确认是 must-fix 还是 SKIP-bash
4. **T1.3.3 daemon 不应自杀但已退出**: master.enabled=false / auto_shutdown=false 配置下 daemon 仍跟 master 联动退出, 跟 R1 设计 T1.3.3 反 — must-fix

## must-fix 清单 (ship 前必解, 按 §3.3.5)

- [ ] T2.2.2 ACK→BUSY 转换缺失 (bash agent + codex/gemini 都暴露)
- [ ] T2.4.5 reply 收集 daemon log 无 `collect_reply` (bash 路径)
- [ ] T1.3.3 daemon 不应自杀但已退出 (master.enabled=false 路径)
- [ ] R1+R3+R4 联动: 4/4 进 IDLE 现 0/4 (full e2e 关键 PASS 卡这一项)
- [ ] R2 ack chain on gemini: WAITING_FOR_ACK 缺失

## 跑出问题反馈给 master 的 minimal info

1. 完整 stdout (`> /tmp/<name>-e2e-out.log 2>&1`, **不 tee**)
2. daemon log (`/tmp/r4-ccbd-*.log` / `/tmp/full-e2e-ccbd-*.log`)
3. `sqlite3 target/dev_state/ccbd.sqlite "select seq_id, agent_name, kind, payload from events order by seq_id desc limit 50"`
4. `pgrep -af "codex|gemini|claude" | head -10` (zombie agents)
5. `systemctl --user list-units --all | grep -E "ccb-|ccbd-"` (systemd state)
6. `git status --short` (untracked / dirty 列表)

## 后续 stage

1. 解 5 项 must-fix 后再 commit
2. **R2 dispatcher state machine review**: 跨 bash + codex + gemini 三路一致让 ACK→BUSY 链路对齐 (mvp12 r2 dispatcher refactor)
3. **mvp6 reader marker**: gemini WAITING_FOR_ACK 缺失推测跟 marker 路径差异有关
4. 真 sandbox e2e (R3.2.x 静态 argv → 真 bwrap)
