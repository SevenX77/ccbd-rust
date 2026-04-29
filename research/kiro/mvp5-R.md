# Kiro Requirements: MVP 5 (内核硬化 / The Hardening)

> **文档定位**：本文件是 ccbd-rust MVP 5 阶段的官方 R (Requirements) 规格。本阶段是**纯重构 MVP**——不增加任何业务能力、不改任何 RPC 接口契约、不动任何状态机语义。唯一目标：在 ccbd-rust 接 MVP 6 (M7) 真实负载之前，把 Gemini 在 MVP4 完结 review 中点出的两个生产隐患修掉——**消灭 SQLite 同步调用阻塞 Tokio 工作线程的风险**，**砸碎 `db/queries.rs` 1303 行 + `rpc/handlers.rs` 1029 行的巨石趋势**。

---

## 0. 立项背景与边界共识

### 0.1 为什么要做这个 MVP（不是 over-engineering）

MVP 1-4 实现期间，所有对 SQLite 的 `rusqlite` 同步调用（`Connection::execute / transaction`）都直接写在 `tokio::async fn` 函数体内。当前测试场景下不出问题，原因是：a) 单进程低并发；b) 单测里 SQLite 操作几乎全部毫秒级返回，Tokio 工作线程没真正被噎住。

但 MVP 6 之后 ccbd-rust 接管旧 Python CCB 的真实负载，会出现：a) 多个 agent 高频写 `output_chunk` 事件；b) `agent.read since=N` polling 大查询；c) WAL checkpoint 触发时几十毫秒级毛刺。**这些场景里，SQLite 同步调用会真实阻塞 Tokio 工作线程**，导致整个 daemon 响应抖动甚至卡顿。等部署上线再发现再修，代价比现在重构高一个数量级。

**两个基础事实由 Claude grep 验证（不是猜测）**：
- `src/db/mod.rs:10` 的 `Db = Arc<Mutex<Connection>>` 已经天然支持跨线程 `Clone + Send`，**不需要改 DB handle 类型**就能 `spawn_blocking`。
- `src/rpc/handlers.rs:30 / 378 / 457` 三处直接 `transaction_with_behavior(TransactionBehavior::Immediate)` 裸写事务——SQL 逻辑下沉到 `db` 模块的工作量是必须先做的，否则 spawn_blocking 包不干净。

### 0.2 本 MVP **不做**的事

- 不增加任何 RPC 方法，不改任何 RPC 入参 / 出参 schema
- 不改任何状态机转移规则，不动 CAS 协议
- 不引入连接池（r2d2 / deadpool-sqlite）——`Arc<Mutex<Connection>>` + `spawn_blocking` 已足够，连接池是 future hardening
- 不加 bench 测试——AI vibecoding 节奏下属于过度工程，依靠 code review 守 spawn_blocking 100% 覆盖
- 不动 `src/db/schema.rs`（schema 已固化）

### 0.3 与上下游 MVP 的关系

- **依赖 MVP 1-4 已完成的全部能力**：本 MVP 是对它们的内核硬化，不是新增。
- **为 MVP 6 (M7 部署 + L3 骨架) 铺路**：MVP 6 接真实负载前，daemon 响应性必须先达标。
- **R-* 矩阵无变化**：本次不改任何 R-* 状态值。所有 R-RECONCILE-1 / R-DISPATCH-1 等 Partial / Deferred 项都按原状保持，留给 MVP 6 之后的真实流量驱动决定。

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 5 验收必须全部通过：

1. **测试零回归**：完成全部改动后 `cargo test --quiet` 输出与改动前完全一致——91 个 lib 单测全绿、`mvp2_acceptance.rs` / `mvp3_acceptance.rs` / `mvp4_acceptance.rs` 三套 acceptance 全绿，ignored 数量保持不变。**禁止**为了规避新引入的 async 边界问题而把任何测试改 `#[ignore]`。

2. **`db/queries.rs` 物理消失**：完成阶段一拆分后 `src/db/queries.rs` 文件不再存在（或仅保留 `pub use` 兼容外壳，不超过 30 行）。`src/db/` 下任何**单个 .rs 文件**（除 `schema.rs` 已固化和 `mod.rs` 仅 re-export 外）**实际代码行数（不含空行 / 单行注释）≤ 300 行**。这条是物理硬约束——目的是杜绝"假装拆分但单个文件还是 800 行"。

3. **handlers.rs 内零裸 SQL**：完成阶段一后 `src/rpc/handlers.rs` 内 `grep -E 'rusqlite::|TransactionBehavior::|conn\.execute|conn\.transaction'` 必须返回 0 行。所有 SQL 操作（包括目前 handlers.rs 内 `transaction_with_behavior` 三处）必须下沉到 `db/<domain>.rs` 模块的同步函数中，handlers.rs 只调这些函数，不直接碰 rusqlite API。

4. **spawn_blocking 100% 覆盖 SQL 调用**：阶段二完成后，`src/rpc/handlers.rs` 和 `src/monitor/*.rs` 内**所有**进入 `db::*` 模块的同步函数调用必须经由 `tokio::task::spawn_blocking` 包裹（封装位置见 D 文档）。判定方式：在 `src/rpc/handlers.rs / src/monitor/agent_watch.rs / src/monitor/master_watch.rs / src/pty/tasks.rs` 内 `grep -n 'db\.conn()\|\.lock()\.unwrap()'` 必须返回 0 行。所有锁获取都被关进 `spawn_blocking` 闭包。

5. **事务原子性保留**：所有 mvp1-4 的 CAS 事务（特别是 `agent.send` 的幂等检查 + state 校验、`agent.assert_state` 的 UNKNOWN→IDLE_Asserted CAS、`mark_agent_unknown` 的 evidence + state_change 单事务）必须保留单 `transaction_with_behavior(Immediate)` 边界。**禁止**为了走 async 而把单事务拆成多次 `db::*().await` 调用——这会破坏 MVP4 千辛万苦建立的 CAS 韧性。判定方式：手工 review + 单测中保留并通过 `agent.send` 幂等回放、`agent.assert_state` CAS 失败、`mark_agent_unknown` SEALED 三个关键事务路径的测试。

6. **错误处理新增 `DatabaseRuntimePanic`**：`src/error.rs` 新增 `CcbdError::DatabaseRuntimePanic { details: String }`，对应 spawn_blocking 闭包内 panic 时的 `JoinError`。`to_rpc_error()` 映射 `error_code="DB_RUNTIME_PANIC"`。round-trip 单测覆盖。本错误码**不预期**在正常路径出现——它是 spawn_blocking 闭包 panic 的 last-resort 捕获，确保 daemon 不被一个 SQL 路径的 panic 拖垮。

7. **公共 RPC 接口零变化**：JSON-RPC 请求 / 响应 schema、错误码表（除新增 `DB_RUNTIME_PANIC` 外）、状态机转移、所有 mvp1-4 的语义对外完全等价。判定方式：mvp2/3/4 acceptance 测试不改一行业务断言即全绿。**这条是 MVP 6 部署兼容性的根基**，任何破坏 = 整个 MVP 5 失败。

---

## 2. 状态机激活范围 (Delta)

**无变化**。本 MVP 不动任何状态值、子状态、转移规则。状态机示意图等同 mvp4-R §2.1。

---

## 3. R-* 需求切割矩阵 (Scope Definitions)

**本 MVP 不动 R-* 矩阵任何条目的状态值**。新增一条横向需求，仅对本 MVP 内有效：

### R-RUNTIME-1: 异步运行时不阻塞
*   **状态**：🟢 **In-scope**（MVP 5 新增需求）
*   **定义**：所有进入 `db::*` 模块的同步 SQL 调用必须从 Tokio worker 线程剥离，包裹进 `spawn_blocking` 在阻塞线程池执行。Tokio worker 不持有 SQLite 互斥锁，不直接执行 SQL。
*   **验收**：AC4 + AC5 联合判定。

### R-MODULARITY-1: 数据层模块化
*   **状态**：🟢 **In-scope**（MVP 5 新增需求）
*   **定义**：`db/queries.rs` 单文件巨石拆分为按领域聚合的多个文件，每文件 ≤ 300 行。所有 SQL 逻辑（包括 handlers.rs 中裸写的 transaction）下沉到 db 模块。
*   **验收**：AC2 + AC3 联合判定。

### R-API-COMPAT-1: 协议破坏性变更约束
*   **状态**：🟢 **In-scope** — 维持 MVP4。本 MVP 仅新增 `DB_RUNTIME_PANIC` 错误码（向后兼容的扩展）。
*   **验收**：AC7。

---

## 4. 范围分阶段（实施视角，不影响验收）

为降低单 PR 风险，实施按两阶段推进，**任一阶段失败可独立回滚**：

| 阶段 | 内容 | 安全检查点 |
|---|---|---|
| **阶段一：模块化** | 拆 `queries.rs` → 多领域文件 + `handlers.rs` 内裸 SQL 下沉 → `db::*` 同步函数。**全程同步签名不变**。 | `cargo test` 全绿（所有现有测试不需改） |
| **阶段二：异步硬化** | 在 `db::*` 模块对外暴露 async wrapper（内部 `spawn_blocking`）。`handlers.rs` 切 `.await`，私有同步层保留给 unit test 用 `pub(crate)`。 | `cargo test` 全绿 + grep AC4 判定通过 |

阶段一验收即同步通过 AC1 / AC2 / AC3 / AC7（功能等价、模块化、handlers 净化、接口兼容）；阶段二验收触发 AC4 / AC5 / AC6（spawn_blocking 覆盖、事务原子性保留、错误码新增）。

---

## 5. 非验收点（延后到后续 MVP 或永不做）

- **连接池**（r2d2 / deadpool-sqlite）：不在本 MVP 范围。当 `Arc<Mutex<Connection>>` + `spawn_blocking` 在生产暴露真问题时再考虑。
- **bench 验证 WAL busy 不阻塞**：不做。code review 守 grep 100% 覆盖即可。
- **针对 `db::*` 同步函数的 ORM-style trait 抽象**：永不做。纯函数 + `&Connection` 入参是 Rust 的最佳路径，引入 trait 增加生命周期复杂度。
- **schema 演进 / 表新增 / 字段新增**：不在本 MVP 范围。

---

## 6. 验收脚本（H 类辅助）

D 文档提供具体的 grep 命令模板和实测 cargo test 输出格式。R 文档只规定**判定标准**，不规定**判定脚本**。
