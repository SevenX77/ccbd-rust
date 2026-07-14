# T1 brief — RuntimeState 补 `Starting` 相位 (a1 codex)

派单人:Master PM。你是 a1(codex),本条你主力实施,TDD 红绿。
背景根因见 `docs/reports/studio-open-in-handoff-2026-07-06.md` §0 + T1。**先把这份 brief 读完再动手**,
里面纠正了 handoff 的一处错误前提,照 handoff 直译会踩雷。

## 目标(一句话)

`ah events` 冷启动窗口的快照 `runtime_state` 现在被误归为 `Degraded`,导致订阅方(Studio)把正在启动
的 daemon 当残骸 `ah stop` 掉。要新增一个 **`Starting`** 相位,把「正在启动、尚在预期内」和「真残骸」区分开。

## ⚠️ 纠正 handoff 的一处错误前提(必读,否则你会去做多余的 schema 迁移)

handoff T1 说「用 master_state 仍在 spawn 流程」来判 Starting。**这条不成立**:

- `sessions.master_state` 是 STRICT 表上带 `CHECK(master_state IN ('IDLE','BUSY'))` 的列
  (`src/db/schema.rs:20`),整个 spawn 生命周期里它**恒为 `'IDLE'`**(`session.create` 落 DEFAULT 'IDLE';
  `spawn_master_pane` → `record_spawned_master_runtime` 也写 'IDLE',`src/master_revival.rs:460,490`)。
  它区分不出「spawn 进行中」和「idle 已就绪」。
- 因此**不要**去加 `master_state='STARTING'`,那要改 STRICT 表的 CHECK,风险大且没必要。
- **正确做法:`Starting` 在快照构建时(`build_runtime_snapshot`)派生出来,不落库、不改 schema、不改任何写入方。**
  只读 DB 已有信息 + 当前时间即可判定。这也是 handoff 「判定用 DB 已有的 state/generation/时间戳,不许猜」的正解。

## 落点

`src/runtime_events.rs`(PR #99 引入)。三处:
1. `enum RuntimeState`(L23,现 `Active/Inactive/Degraded`)→ 加 `Starting`,serde 序列化为 `"starting"`
   (enum 已带 `#[serde(rename_all = "snake_case")]`,加一行 variant 即可)。
2. 派生逻辑(L214-220,现 `if active {Active} else if ahd_has_inventory {Degraded} else {Inactive}`)。
3. 数据管道:`InventorySession` / `InventoryAgent` 结构体 + `query_runtime_inventory_sync` 的 SELECT
   需要补时间戳/字段(见下)。

`inactive_runtime_snapshot`(客户端本地降级快照,L116)**保持 `Inactive` 不动**——那是 daemon 不在时用的。

## 判定语义(这是本条的核心,照它写)

只在 `ahd_has_inventory == true`(存在 ACTIVE session)这一分支里,把原来一律 `Degraded` 的结果细分为
`Starting` vs `Degraded`。`active==true` 仍 `Active`,无 inventory 仍 `Inactive`,都不变。

对**每个 ACTIVE session 的 master** 和**每个非终结 agent**,分三类:

- **OK(活着)**:master_tmux_alive / worker tmux_alive == true。
- **Corpse(真残骸 → 逼 Degraded)**,满足任一:
  - 「曾经活过又消失」:master 的运行期已被记录(`master_pane_id.is_some()`,即 spawn_master_pane 已落 pane;
    等价信号还有 `master_generation > 0` / `master_pid > 0`)但现在 tmux 不活;
    worker 侧对应:agent 已越过 `SPAWNING`(即状态是 IDLE/BUSY/WAITING_FOR_ACK 等「进程起来过」的状态)但 tmux 不活。
  - 「起不来超时」:尚未记录运行期(master pane 未落 / agent 仍 `SPAWNING`)**且**已超出启动预期窗口。
- **Starting(启动中,尚在预期内 → 不算残骸)**:尚未记录运行期(master pane 未落 / agent 仍 `SPAWNING`)
  **且**未超启动预期窗口。

聚合规则:
```
if active                      -> Active
else if !ahd_has_inventory     -> Inactive
else if 存在任一 Corpse         -> Degraded
else (剩下的非 OK 全是 Starting) -> Starting
```

「启动预期窗口」= `now - session.created_at < STARTING_WINDOW`。用 session 的 `created_at`(epoch 秒,
`src/db/schema.rs:22` DEFAULT `unixepoch()`)作为该 session 冷启动统一时钟;master 与该 session 下的 worker 都用它。
（worker 也可用自身 `agents.updated_at` 作更精确的「进入当前状态时刻」——你可自行取舍,但要在测试里能确定性构造。）

`now` 用 `std::time::SystemTime::now()`(Rust 侧无限制)。测试通过 UPDATE `created_at` 来把 session 推到窗口内/外,
不依赖真实时钟。

### STARTING_WINDOW 取值

新加一个模块常量,默认 **120 秒**(对齐既有 master readiness 默认 `default_master_readiness_timeout_s()=120`,
`src/cli/config.rs:285`)。建议再留一个 env 覆盖(仓里惯例,如 `AH_RUNTIME_STARTING_WINDOW_SECS`,带 clamp),
但这是 nice-to-have,常量足够过验收。

## 数据管道要补的东西

现结构体没带派生所需字段(SELECT 里 `sessions.created_at` 在列 8、`agents.created_at` 在列 6,但没被读进结构体):

- `InventorySession`(L94)+ SELECT:补 `created_at: i64`(读 `row.get(8)`)。若你用 generation 作「起来过」信号,
  再补 `master_generation`(SELECT 里现在没选它,需要加列)。用 `master_pane_id`(已在结构体里)判「起来过」则不必加 generation——**优先用 pane_id,改动最小**。
- `InventoryAgent`(L106)+ SELECT:若 worker 窗口用 agent 自身时间戳,补 `updated_at`(现未选)或复用已选的 `created_at`(列 6,读 `row.get(6)`)。
  若 worker 统一用 session.created_at 作时钟,则 agent 侧只需知道 `state=="SPAWNING"`(已有 `state` 字段),无需加时间戳——**这条更省**。

选「改动最小」的组合:master 用 `master_pane_id.is_some()` 判起来过、session `created_at` 判窗口;
worker 用 `state=="SPAWNING"` 判未起来、同一 session `created_at` 判窗口。这样只需给 `InventorySession` 加一个 `created_at` 字段。

`SPAWNING` 常量:`crate::db::state_machine::STATE_SPAWNING == "SPAWNING"`(`src/db/state_machine.rs:14`)。
终结态仍是 `is_terminal_agent_state` 的 `CRASHED|KILLED`(`src/runtime_events.rs:359`),终结 agent 不参与判定(现逻辑已过滤)。

## 测试(TDD:先写红,再实现转绿)

仿照 `src/runtime_events.rs` 现有 tests(`test_ctx()` + `insert_session_sync` / `insert_agent_sync` + UPDATE)。至少:

1. `starting_snapshot_is_not_degraded`:ACTIVE session,master_pane_id NULL、master_pid 0(默认 insert),
   created_at ≈ now,agent 处 `SPAWNING`(tmux 不活)→ `runtime_state == Starting`,断言 `!= Degraded`。
2. `master_died_after_alive_is_degraded`:ACTIVE session,master_pane_id 置 `'%404'`、master_pid 123
   (运行期已记录),tmux 不活 → `Degraded`。**注意现有测试 `missing_tmux_for_active_inventory_is_degraded_not_active`
   正是这个形状,它必须仍然是 Degraded(不许回归)**——你可以让它继续过,或把它并入本用例。
3. `starting_past_timeout_is_degraded`:ACTIVE session,master_pane_id NULL、`UPDATE ... created_at = <now-10000>`
   → 超窗 → `Degraded`。
4. `empty_inventory_snapshot_is_inactive`(现有)必须仍 `Inactive`。

补充:「冷启动=starting → master 活=active → kill master=degraded」的整条序列里的 **active 那一档在单测里做不出来**
(单测无真实 tmux,`session_exists` 恒 false,master_tmux_alive 永远 false),现有测试也从不断言 Active。
所以 active 档归 e2e/手测,单测只需把 **Starting↔Degraded 的判别**钉死即可。别为了单测 Active 去 mock 一整套 tmux。

## 文档 + CHANGELOG(本条一并做,别漏)

- 在仓内 runtime events 文档一节写清四相位语义(`Inactive/Starting/Active/Degraded` 各自含义 + Starting 的判定窗口)。
  先 `grep -rn "runtime_state\|runtime events\|ah events" docs/ README.md` 找到 #99 落的那节接着写;没有就在最贴切的 docs 文件里补。
- `CHANGELOG` 加一条:新增 `Starting` 相位,说明消费方(Studio)应「仅 `Degraded` 可清理,`Starting` 不许动」。

## 消费方契约(实现完口头回我即可,不用你去改 Studio)

Starting 落地后我(PM)会通知 Studio 把 stale 判定升级为「Degraded 才清理、Starting 不动」。你只要在回执里
确认 `"starting"` 这个 JSON 字面值稳定(序列化名)即可。

## 纪律(钉死)

- **从 `origin/main`(0a48e81,v1.3.3)开一个新分支干**,别在 main 直接改。分支名建议 `t1-runtime-starting-phase`。
- **串行 cargo**:`CARGO_BUILD_JOBS=1`,测试 `--test-threads=1`,**跑完整 `cargo test`(别只 `--test` 过滤子集)**,
  贴全量结果。cargo dist 之类绝对别本地跑。
- TDD 红绿:先给出失败测试的输出,再给转绿后的全量 `cargo test` 输出。
- 只改 `src/runtime_events.rs` + 必要的 docs/CHANGELOG。**不要动** `master_state`/schema/任何写入方/未跟踪的 `.kiro`、
  其它 WIP。不 reset/discard 任何东西。
- 完成后**别 push**,把:分支名、改动摘要、失败→通过的测试输出、`git diff --stat` 回给我。我会让 a2/a4 审、再决定 push+开 PR。
  如果你判断某个「红灯」和本改动无关,必须用 baseline 对照证明(`git stash` 后在 main 单跑同一测试),别口头说「无关」。

有任何判定语义上的疑问,先问我再写,别自行扩大范围(比如别顺手去碰 T2 的 IS_SANDBOX、别去改 provider 层)。
