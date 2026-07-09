# 工单 Fix D — hook 起止双时间节点为主信号(北极星 T1/T3 分级在 scanner 的第一片落地)

**收件:a1(antigravity,实施)。worktree `/home/sevenx/coding/ccbd-rust-wt-scanner`,分支 `feat/scanner-delete-park-inference`,叠在 Fix C 之上(先 Fix C 收口再起本单)。TDD 框线由 master(claude)钉死。审计 a4。**

> 方向(用户裁决):hook 事件 = 生命周期**主信号**(turn 起止);pane = 对话框驱动 + 人类调试面,**不再是状态机输入**。本单在 Fix C(已删 unknown→park)之上,把 hook 时间线正式升为主信号。

## 环境铁律(同前)

cargo 串行 `CARGO_BUILD_JOBS=1` + `--test-threads=1`;只 `cargo test --lib` + `cargo check --all-targets`;禁全量/集成/e2e;**严禁后台跑测试**;前台 commit 不 push。遇阻写 `.operator-question` 停下。

## 现状实锚

- `[completion]` hook_push 目前只推 **stop** 事件(turn 结束);claude/codex/antigravity 都推 stop。stop 落库经 `mark_agent_idle_hook_event`(state_machine.rs:798),events 表 `state_change` / payload `"source":"hook"`。
- **任务开始侧缺失**:派发确认目前靠 pane-ACK 推断(`mark_agent_waiting_for_ack` + `command_received` 事件),不是 hook 实证。
- **claude 已有 UserPromptSubmit hook 物化能力**:`src/provider/home_layout.rs:230`(条件见 :229)已 push `materialized_ah_hook(ctx, "UserPromptSubmit")`,:232 push `"Stop"`。server 侧 hook 入口 `src/rpc/handlers/agent.rs:804/880`(`normalize_hook_event` 只小写化,接受任意 event 名)。即 claude 的 start-side hook 基建大体在,缺 server 侧对 UserPromptSubmit 的语义处理(记"提示词已进 agent")。

## 要做什么(四部分)

### 1. 任务开始侧 hook 事件(派发确认从 pane-ACK 升级为 hook 实证)
- **claude**:启用/接通 UserPromptSubmit hook → server 侧 hook 处理器在收到该事件时 insert 一条"提示词已进 agent / 任务开始"事件(events 表,语义清晰,payload 带 job/request 关联)。派发确认优先采信该 hook 事件,pane-ACK 降为兜底(hook 缺失时才用)。
- **codex / antigravity**:先**摸一个 per-provider 能力矩阵**——各 provider 有无等价的 start-side hook(prompt-submit / turn-start)?**有就接,没有就保留现有 ACK 机制,别硬造**。矩阵结论(哪个 provider 接了 hook-start、哪个留 ACK)写进 commit message。
- fail-closed:hook-start 缺失或没到 → 退回现有 ACK 路径,不 panic、不卡派发。

### 2. scanner 分类前咨询 hook 时间线
Fix C 之后 scanner 只剩"已知对话框白名单"判定。在它做判定前,先查该 agent 的 hook 时间线(`db/events.rs::query_last_event_of_type` / `query_last_event_of_type_matching_payload`,hook stop = state_change/`source:hook`):
- 若**最新 Stop 事件晚于/接近本次 capture 时间**,且**其后无新派发**(用 job dispatched_at / start-side hook 时间判定)→ pane 上的残影**不得触发任何动作**(白名单对话框判定也应让位——除非该对话框形状明确要求人类介入且在 hook 之后出现)。
- 原则:**只有白名单对话框可以覆盖 hook 抑制**,且仅当有证据它出现在 hook Stop 之后。
- fail-closed:无 hook 时间线(如某 provider 没接 start/stop hook)→ 退回 Fix C 后的纯白名单行为,不误抑制、不 panic。

### 3. 原则落进代码注释 + 事件语义
在 scanner / hook 处理处写清:hook 事件 = 生命周期主信号(turn 起止),pane 文本 = 对话框驱动 + 人类调试面,不再作状态机输入。让后来者不会把 pane 推断加回来。

### 4. 边界(明确不做,写进 commit)
hook 投递可靠化(outbox / 投递 ACK / 重放 / 配置自检)是**设计轮**的活(显式完成协议课题,北极星 R1/G4),**本单不做**。本单只做"start-side hook 接通 + scanner 咨询 hook 时间线"。理由:即便 hook 偶发丢失,删除后的过渡态(Fix C:scanner 只发事件不造状态 + 本单:hook 优先)已**严格优于**现状(pane 裸判),失效方向安全(停滞+A/B 告警,而非伪造状态),故不被设计轮 block。

## TDD 框线 · 验收(先 RED 后 GREEN)

- **D1(hook 抑制残影,RED)**:最新 Stop 事件晚于 capture、其后无新派发,pane = 任意文本(含白名单外)→ 断言 scanner **不触发任何动作/状态**。现码(Fix C 后但无 hook 咨询)对白名单外已不 park,但要断言 hook 抑制路径确实走到(用观测事件或 spy 证明是 hook 咨询导致的短路,而非 Fix C 的默认)。
- **D2(白名单对话框在 hook 之后仍可覆盖)**:hook Stop 之后出现白名单对话框形状 → 断言仍正常处理/park(hook 抑制没误杀真对话框)。
- **D3(start-side hook 升级派发确认,claude)**:模拟 claude UserPromptSubmit hook 事件到达 → 断言 insert 了"任务开始"事件,且派发确认采信它(而非仅 pane-ACK)。先 RED(现无该处理)后 GREEN。
- **D4(fail-closed 退回)**:无 hook 时间线的 provider / hook 缺失 → 断言退回 ACK + Fix C 白名单行为,不 panic、不误抑制、不卡派发。
- **D5(per-provider 矩阵)**:矩阵结论有测试或明确文档佐证(哪个 provider 接 hook-start、哪个留 ACK)。

先把 D1/D3 写成会 RED 的测试;实现后转 GREEN。保留 Fix C 及既有 hook/ACK 测试全绿。

## 本地验证 & 收口

- `cargo check --all-targets` + `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`(前台)。不跑全量、不放后台。
- 回滚自检:改动面 = hook 处理器(agent.rs / state_machine.rs)+ scanner hook 咨询(gating/resolve)+ provider hook 物化(home_layout.rs,若需启用 UserPromptSubmit)+ 测试。不动 A/B 看门狗、不动 KnownAction 自动处理、不碰投递可靠化。
- 完成回报 master:commit 号 + per-provider hook-start 矩阵结论 + hook 咨询/抑制的实现要点 + D1-D5 RED→GREEN 实证 + 测试名 + `--lib`/`check` 结果。master 亲验后派 a4 审。

拿不准/越界(尤其 codex/antigravity 有无 start-side hook 拿不准、或"白名单对话框该不该覆盖 hook"边界):STOP,写 `.operator-question`,回报。
