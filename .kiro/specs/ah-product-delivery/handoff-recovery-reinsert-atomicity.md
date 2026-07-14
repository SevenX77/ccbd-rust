# Handoff: recovery 的 delete→reinsert 原子化 (master-death 工作线遗留)

> 写给下一个 session 的 Master PM。这是 2026-06-23 lifecycle-detection-audit 收尾时**主动标记并刻意推迟**的一个 hardening,不是 bug 回归。前置三件事(G0 修复 / master_watch 重启重装 / 那条 flake 修复)已全部合入 main(PR #56, merge commit `c84c4da`)。

## 一句话任务
让 worker 复活时"级联删旧 job → 用同一 job id 重新插回"在**单个数据库事务内原子完成**,消除任何并发观察者(orchestrator tick / 状态查询 / 测试)看到该 job **短暂不存在**的窗口。

## 为什么有这件事(背景)
- 现象来源:`src/monitor/master_watch.rs::master_revive_stale_inflight_dispatch_failure_does_not_overwrite_requeued_job` 这条测试在负载中的 VPS 上 flake。
- root-cause(已查实,a1 + 主控复验):
  - `jobs.agent_id REFERENCES agents(id) ON DELETE CASCADE` —— `src/db/schema.rs:136`。
  - 复活重装 worker 会 `DELETE FROM agents`(KILLED agent)—— 触发点 `src/monitor/master_watch.rs:824`,底层 `src/db/agents.rs:162` —— 这会**级联删掉该 agent 名下的 job**。
  - 现有保护(**已在、且有效**):删前捕获 recovery intent(`src/monitor/master_watch.rs:757`),reprovision 后用**同一 job id 重插**(`src/db/recovery.rs:329`, `src/db/jobs.rs:72`)。a1 的聚焦单测 `rr_worker_requeue_uses_captured_intent_after_agent_delete_cascades_intent` 证明保护有效,job 最终不丢。
  - **但**:cascade-delete 与 reinsert 之间 job 行**短暂不存在**。负载下测试在该窗口查 `query_job_sync(job_id)` 拿到 `None` 就 panic。
- 已做的(收尾时):**只修了测试**(`3682370` de-flake:redispatch 循环改 poll end-state + 容忍 run_once 返 false + 容忍 job 瞬时 None;0/3→5/5 稳过)。**没碰产品代码的事务**——那正是本 handoff 要做的。

## 为什么推迟、为什么不 drive-by
这片是 master 死亡/级联杀/复活的硬骨头(见关联记忆),改 recovery 的事务边界容易引入真 bug。收尾时刻不该顺手改,应该**先设计再动手**。生产里这个窗口**无害**(同 id 重插,job 最终在),所以不是 P0,是 robustness hardening。

## 怎么做(建议,但你自己定)
1. **先确认范围**:grep 出 reprovision/复活路径里"删 agent"和"重插 job"分别在哪个事务里(`src/db/recovery.rs` + `src/db/agents.rs` + `src/monitor/master_watch.rs:757/824`)。确认它们目前是**两个独立事务**(这是窗口的来源)。
2. **设计(派 a2)**:把"删 KILLED agent(级联删 job)+ 按 captured intent 用同一 id 重插 job"包进**一个事务**;或者改顺序/改成"先 detach job 再删 agent 再 reattach",任一能消除中间空窗的方案。要评估:级联删是否还有别的副作用行(events? evidence?)需要一起原子处理;state_version / CAS 是否受影响。
3. **tests-first**:写一条测试,在 delete→reinsert 期间并发查 job,断言**任何时刻都能查到该 job**(要么旧行、要么新行,绝不 None)。这条测试现在应该是红的(因为有窗口),修完转绿。注意别再写成对负载敏感的脆测试。
4. **scope 纪律**:**只**收窄这个原子窗口。**不要**借机改"master 死后连坐清 worker vs 反孤儿级联杀 vs 复活"那个更大的未决语义(那是独立工作线)。
5. 之后可以把 `master_watch.rs` 那条 de-flake 测试里"容忍 job 瞬时 None"的那段注释/容忍逻辑**收回**(原子化后不再需要容忍 None),但这是可选清理,别为它返工。

## 硬约束(本仓库铁律,务必带给 worker)
- dogfood:验证用 `ah ask` 派 a1-a4,**不要用 ccb 测 ah**。
- gemini 已弃用,**严禁**对 gemini 投 dogfood/修复;目标 provider 是 antigravity。
- VPS cargo **必须串行**:`CARGO_BUILD_JOBS=1 cargo test ... -- --test-threads=1`,**绝不并行多个 cargo**(会 OOM 杀主控 + 崩 ahd)。
- OAuth-only,禁 API key。
- 真 e2e 测试(起真 tmux)跑前 `env -u AH_STATE_DIR -u CCBD_STATE_DIR` 隔离,别污染运行中的 ahd。
- feature 分支自动 commit/push 不用问;**合 main 须 PM ack**(CI 三连绿 + 你点头)。
- commit footer:
  `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
  `Claude-Session: <本 session 链接>`

## 角色与 SOP
- 你是 Master PM(无 `CCB_CALLER_ACTOR`):规划/分派/审阅/收敛,自驱跑 research→design→tests-first→impl→audit→dogfood goal-verify 闭环,只在目标层 escalate 给人。
- 严谨操作(grep/file:line/全覆盖)派 a1 codex;独立判断/设计派 a2 gemini……**注意 a2 是 gemini 已弃用**,设计类如果要 dogfood 就改用其它途径或主控自审;纯设计文档产出可让 a1 出草稿主控审。e2e/审计可派 a3 antigravity。

## 关联记忆(读)
- `project_ah_master_revive_stale_inflight_test_red` —— 本 handoff 的直接来源,含全部 file:line 证据。
- `project_master_death_corrected_semantics` / `project_ah_session_watch_cascade_defeats_revive` / `project_ah_master_watch_not_rearmed_on_restart` —— 同一片 master-death 工作线的其它缺陷,注意区分,别混改。
- `project_ah_health_check_redispatch_false_stuck` —— G0,已修(本轮)。

## 起手第一步
读 `project_ah_master_revive_stale_inflight_test_red` 记忆 + 上面那几个 file:line,自己先把"两个独立事务造成空窗"这个前提在代码里证实/证伪,再决定派谁设计。别一上来就改事务。
