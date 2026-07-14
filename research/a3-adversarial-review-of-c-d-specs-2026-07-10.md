# a3 对抗审报告：模块 C/D + 独立线 spec 忠实度与漏洞审查

**文件状态**：评审完成稿
**评审人**：a3-antigravity (架构设计与忠实度审计席)
**收件人**：master / 实施团队

---

## 一、 架构评级与置信度评估 (Executive Summary)

根据“反讨好”与“第一性原理”原则，本席对收敛定稿后的三份 spec 进行深度静态走读，给出的整体审计结论如下：

*   **感知仲裁器模块 (Module C - ah-perception-arbiter)**：
    *   **置信度评级**：**中 (70%)**（从收敛前的 90% 发生回退）。
    *   **核心风险**：主干逻辑成立，但 **Q4 归属机制的接口协议被严重悬空**；OS 层的 Sibling-cgroup 方案（Q3）存在**沙箱隔离与 cgroup 写入权限的致命物理冲突**；2s 延迟窗口与 D1 状态机的强硬转移存在**逻辑死锁冲突**。
*   **控制面重构模块 (Module D - ah-control-plane-refactor)**：
    *   **置信度评级**：**低 (50%)**。
    *   **核心风险**：**F3/F2 解耦设计引入了严重的“物理空闲后工作区读写竞态 (Read-After-Write Hazard)”**。在异步二阶段验证完成前，Agent 被标记为 `IDLE` 派发新任务，会直接覆盖破坏前序任务的物理证据。同时，7月9日收敛确立的**“双安全护栏”（is_mutating 静态标注 + 连续拦截上限）被 master 执笔时单方面稀释/排除**。
*   **独立凭据模块 (ah-per-worker-credentials)**：
    *   **置信度评级**：**极低 (30%)**。
    *   **核心风险**：**物理文件复制（Copy-on-Create）在 OAuth 刷新令牌轮转（Refresh Token Rotation, RTR）机制下是死路一条**。该设计将 bug 从“本地 inode 共享”往后挪到了“服务端 Token 轮转击穿”，在标准 OAuth 场景下必然发生级联下线，所声称的“隔离”是镜花水月。

---

## 二、 必答四题忠实度与悬空审查 (Review of the 4 Must-Answer Questions)

### Q1：单写入口硬约束形态 —— 内部防线“虚设”与 Trigger 被拒的深度质询
*   **定稿方案**：采用 `db::perception::gate` 私有模块进行 Rust 编译期可见性隔离，加 CI 静态扫描；拒绝数据库 Trigger。
*   **审计结论**：**存在严重边界泄漏漏洞，设计被部分软化。**
*   **失效模式与论据**：
    *   编译期可见性隔离（`MutConnection` 封装）仅能限制 `db::` 模块**外部**的代码。然而，目前系统的大量状态写入本就散落在 `db/` 子目录下的各个子模块中（如 [db::jobs](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs)、[db::recovery](file:///home/sevenx/coding/ccbd-rust/src/db/recovery.rs)、[db::state_machine](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs)）。这些子模块为了读写各自的表，**必须**持有并操作原生数据库连接。
    *   在没有数据库 Trigger 的情况下，[db::jobs](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs) 或 [db::recovery](file:///home/sevenx/coding/ccbd-rust/src/db/recovery.rs) 的开发者在后续迭代中，可以直接编写带有 `UPDATE agents SET state = ...` 的 SQL 语句，Rust 编译器对此没有任何感知。
    *   Master 拒绝 Trigger 的理由是“同进程单连接下，Token 会发生复用/泄露”。这忽略了 **RAII 事务锁屏障**：在 Rust 中，事务独占锁生命周期与数据库连接锁定是强绑定的。在单连接串行架构下，如果我们在事务启动时写入临时 Token、事务提交/回滚时（利用 Drop 卫士）强行清除 Token，那么在这个独占周期内，其他并发任务根本无法获取连接并执行 SQL。
    *   因此，拒绝 Trigger 使得 `db::` 内部的防线形同虚设，CI 静态扫描（grep）成为唯一的后盾，设计可靠性显著下降。

### Q2：各信号类 Unknown 预算 —— 2s 超时“断头台”与 late-completion 的逻辑冲突
*   **定稿方案**：OS 存活（30s）、Log 静默（900s）、Hook 延迟（2s）。
*   **审计结论**：**存在逻辑闭环冲突。**
*   **失效模式与论据**：
    *   Hook 的 2s 超时窗口被定位为“等待 Host Daemon 消费落盘 outbox 的时间”。但是在单 SQLite 连接架构下，频繁的写事务会导致锁竞争，`ahd` 自身的主循环/事件消费延迟极易突破 2s（特别是在容器高负载或重构期间）。
    *   一旦消费延迟突破 2s，Arbiter 会强行判定 Hook 丢失并判定 Agent 进入 `FAILED` / `Stalled`。
    *   根据 Module D 的 D1 状态机转移矩阵，Job 的状态会被同时写入 `FAILED`（终态）。由于该状态机去除了 `FAILED -> COMPLETED` 的转移路径，当 `ahd` 随后完成锁解锁并读取到 outbox 中原本合法的 Success Hook 时，**系统将无法再把 Job 改写为 COMPLETED**。
    *   这直接废除了原系统中“允许 late-completion 挽救 STUCK 状态”的自愈机制（[late_health_completion_stuck_allows_terminal](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L1176)）。超时的 2s 变成了不可逆转的“断头台”，系统容错度严重倒退。

### Q3：父/子 cgroup 委托布局 PoC —— 沙箱隔离与 cgroup 写入权限的致命物理冲突
*   **定稿方案**：平级 Sibling 布局（`cli.scope` 与 `workload.scope`），子 scope 开启 `Delegate=yes`。
*   **审计结论**：**机制无法闭环，核心安全细节被悬空至 PoC。**
*   **失效模式与论据**：
    *   定稿承认了 cgroup v2 的叶子节点限制，改用兄弟布局。但在沙箱（Sandbox）物理隔离环境下，**运行于 `cli.scope` 内的 Sandboxed Agent CLI，如何在不破坏沙箱安全的前提下，将新 Spawn 的子进程 PID 移动到 `workload.scope/cgroup.procs` 中？**
    *   在安全沙箱模型（如 Docker, Podman, gVisor, 或严格的 Mount Namespaces）下，`/sys/fs/cgroup` 默认是只读或被屏蔽的（Masked Paths）。若要让 Agent CLI 有权向 `workload.scope/cgroup.procs` 写入 PID，必须将主机的 cgroup 文件系统写权限挂载引入沙箱内部。
    *   **这是极其危险的沙箱逃逸漏洞**。一旦沙箱内进程拥有了 cgroup 子树 of 写权限，它就可以通过操纵 cgroup 控制器或移动进程，对宿主机资源实施拒绝服务攻击（OOM 注入）甚至逃逸。
    *   如果不给写权限，就必须依赖宿主机 Daemon（`ahd`）去执行 PID 移动。但 `ahd` 运行在宿主空间，无法感知沙箱内 Shell 的 fork 时序。这一核心安全冲突在设计中被一句“由 PoC 验证，未决则阻止毕业”直接带过，实质上处于未决状态。

### Q4：hook 上报归属竞态机制 —— 接口协议的彻底悬空与 epoch-safety 丧失
*   **设计方案**：Outbox 模式落盘；Attribution Key “复用现有分发标识符，具体由实施者查证”。
*   **审计结论**：**这是全 spec 最严重的悬空，属于文字游戏规避。**
*   **失效模式与论据**：
    *   Master 拒绝了 a3 提出的 `CCB_JOB_COOKIE` 环境变量方案，声称“复用现有分发标识符”。但事实上，目前系统在分发时，注入到 Agent 容器环境中的**仅有静态的 `jobs.id`**，并不存在任何能够区分“同 Job 多次分发/重试（Redispatch）”的唯一 epoch 标识符。
    *   如果不引入全新的环境参数，在快速重试（Fast Redispatch）场景下，前一次运行残留的、延迟到达的 Hook（因网络或磁盘 full 积压后在 cold-scan 被拉起），由于其 payload 中只有 `jobs.id`，会被 Host 错误地归属于**当前正在运行的全新 Dispatch 实例**。
    *   这完全击穿了“物理证据隔离性”，重置了“假完成/假失败”的竞态。
    *   此外，如果不定义该环境变量的具体键名 and 格式，沙箱内的 Hook CLI 开发者与 Host 侧的消费端开发者就无法基于 Spec 独立开发，两边必须各自进行假设，集成时必然发生接口不一致导致的“空中碰撞”。

---

## 三、 对 Master 裁决合理性的深度质询 (Critique of Master's Rejections)

### 1. 关于 Q1 拒绝数据库 Trigger 的辩驳：忽略了 `db/` 内部的边界泄漏
*   **Master 的辩解**：在单进程 `Arc<Mutex<Connection>>` 下，Trigger 只能通过临时表/变量授权，该 Token 容易被其他持有连接的代码复用，因而 Trigger buy 不到额外保证。
*   **本席的驳斥**：
    1.  **忽略了 `db/` 内部子模块的安全边界**：编译期模块隐私隔离（`MutConnection`）只能防止 `db` **包外部**的代码执行 SQL。但 `db/` 子目录下的所有模块（如 [jobs.rs](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs)、[recovery.rs](file:///home/sevenx/coding/ccbd-rust/src/db/recovery.rs)、[state_machine.rs](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs)）在开发中是天然有权获取原生连接的。当一个开发人员在 [recovery.rs](file:///home/sevenx/coding/ccbd-rust/src/db/recovery.rs) 模块里为了处理复杂的崩溃自愈逻辑，手写 SQL 更新 `agents.state` 时，编译期隔离对此**毫无防备**。
    2.  **忽略了 RAII 锁对 Token 泄漏的隔离保障**：在 Rust 配合 `Mutex` 的模式下，对连接的获取和 SQL 执行是由 `MutexGuard` 串行的。如果我们设计一个 RAII 辅助结构，在事务/执行开始时写入 Token，并在其 `Drop` 时自动清除 Token。由于整个过程在 `Mutex` 锁内执行，**在持有锁的期间，没有其他线程的代码能够插进连接中读取或使用该 Token**。当锁释放时，Token 已经被擦除。因此，“其他代码复用 Token”的风险在机制上是可以通过合理的 Rust RAII 设计完全规避的。
    3.  **结论**：Trigger 不仅能提供纵深防御，还是防御 `db/` 包内部“自己人”写错 SQL 的唯一手段。Master 的拒绝理由是不符合 Rust 并发与生命周期机制的虚假命题。

### 2. 关于 Q4 拒绝 `CCB_JOB_COOKIE` 的辩驳：以“合理简化”为名掩盖“协议缺失”
*   **Master 的辩解**：复用现有 dispatch 标识符，不造 parallel 标识符。
*   **本席的驳斥**：
    1.  **事实上无“现有标识符”可用**：当前注入给 sandbox 的只有 `jobs.id`。如果复用它，直接导致同 Job 两次运行的 Hook 碰撞，导致 redispatch 时旧 Hook 篡改新运行状态（Epoch 击穿）。
    2.  **如果修改现有标识符，等同于 minting 新 Cookie**：如果为了支持 Epoch 判定，Master 不得不将 `dispatched_at_seq_id` 或 `state_version` 与 `jobs.id` 拼接后再注入，这在实质上就是 mint 一个新的 Unique Cookie。
    3.  **接口悬空的集成灾难**：在定稿设计中，Master 拒绝确定该拼接后的环境变量名称，写道：“留给实施者查证”。这导致 Hook CLI 脚本的编写者（运行在 sandbox 内）与 Host Daemon 消费端的编写者失去了契约标准。在没有契约的情况下，双方代码无法独立编写 and 测试，极易在集成阶段引发接口不匹配灾难。

---

## 四、 模块 D (Control Plane Refactor) 稀释与竞态漏洞审查

### 1. F3/F2 解耦的致命副作用 —— “物理空闲工作区读写竞态 (Read-After-Write Hazard)”
*   **定稿方案**：Phase 1 物理释放 Agent 至 `IDLE` 并发出 `JobExecutionFinished` 事件，Phase 2 异步读取事件并跑 evidence check。
*   **物理现实与漏洞分析**：
    1.  **物理介质的独占性**：每个 Agent 绑定一个物理沙箱/工作区（Workspace）以及一个 TMUX Pane。
    2.  **读写冲突时序**：
        *   当 Phase 1 完成，Agent 状态立即写为 `IDLE`。
        *   Orchestrator 调度器检测到 Agent `IDLE`，可以立即将队列中的下一个 Job B 分发（Dispatch）给该 Agent。
        *   Job B 在该 Agent 相同的物理沙箱中启动，开始执行并产生新的文件改动、覆盖 TMUX 缓存、更新 `git status`。
        *   此时，控制面的异步协程刚刚开始执行 Job A 的 Phase 2 验证（进行 Git diff 扫描、文件修改时间校验等物理检查）。
        *   由于 Job B 已经污染了物理工作区，Job A 的验证逻辑读取到的将是 **Job B 写入的物理数据**！
    3.  **后果**：这会导致严重的证据误判（Job A 因为读取到 Job B 的 diff 而通过，或者因为冲突而失败）。如果为了解决此问题而阻止 Agent 在验证完成前接单，那么 Agent 就必须停留在 `VERIFYING` 状态，这与“物理 turn-end 立即释放”的初衷完全矛盾。定稿 spec 对此物理级竞态完全视而不见。

### 2. 调试现场损毁与 In-Situ Triage 丧失
*   **漏洞表现**：在旧版设计中，Job 验证失败或卡死时，Agent 处于 BUSY / STUCK 状态，操作员可以通过 `tmux attach` 或是直接进入沙箱对现场进行 Debug / 恢复（In-Situ Triage）。
*   **审计结论**：新 spec 中，Agent 只要跑完就立马归还为 `IDLE` 并被新任务抢占，前序任务的现场（Pane 状态、临时文件）瞬间被抹去。这意味着，一旦出现 Evidence 缺失导致的 FAILED 判定，操作员根本没有任何手段去定位“为什么 LLM 吐出了不符合格式的代码”，大幅削弱了系统的开发调试友好度。

### 3. converged 决策中“双安全护栏”的被动稀释
*   **历史收敛结论**（`perception-final-convergence-2026-07-09.md` §2.4）：
    *   “必须带 a3 自查出的两护栏：任务派发时静态标注 `is_mutating`；连续拦截上限（2 次 nudge 后第三次放行+上抛人工），防只读任务误标或权限不足时死循环”。
*   **定稿 spec 对抗审查**：
    *   `ah-control-plane-refactor/requirements.md` 的 "Out of scope" 章节明确写道：“Any behavior change to what counts as 'done' for a job ... this spec only relocates where and how ...”。
    *   Master 在执笔时，以“只做结构搬迁，不改判定逻辑”为借口，将好不容易通过 107 实例对抗验证得出的“双护栏”**剔除出了设计范围**。这导致二阶段验证上线后，仍然面临只读任务误报 `EvidenceDenied` 进而触发死循环的无解困境。

### 4. `master_watch.rs` 拆分的实质性稀释
*   **原定目标**：解决 2245 行生产代码 + 3300 行测试代码、承载 13 项职责的上帝文件问题。
*   **定稿方案**：Step 1 移走测试；Step 2 移走 Claude 知识；Step 3（复活/切除管线）“仅命名为 Step 3，本轮不予实现”。
*   **审计结论**：核心重构任务被严重注水。复活与切除管线是 [master_watch.rs](file:///home/sevenx/coding/ccbd-rust/src/monitor/master_watch.rs) 中最臃肿、竞态最多的逻辑，Spec 仅要求将其移入“概念目标”，实际上将 90% 的生产逻辑铺设在了上帝文件中，未能达到实质性解耦。

---

## 五、 独立线 (Per-Worker Credentials) 漏洞审查

### 1. OAuth 刷新令牌轮转 (Refresh Token Rotation, RTR) 机制下的“致命击穿”
*   **定稿方案**：在 Worker 创生时，通过 `fs::copy` 物理复制种子凭证文件 `.credentials.json`，替代原本的 `symlink` 软链接；承认“Provider 侧会话耦合”为已知残余风险。
*   **物理现实与漏洞分析**：
    1.  **什么是 RTR（Refresh Token Rotation）**：在现代主流 OAuth 2.0 提供商（包括 Anthropic/Claude CLI 依赖的 Auth0 机制）中，为了防止 Token 泄露重放，广泛采用了“刷新令牌轮转”安全策略。
    2.  **崩溃过程演示**：
        *   种子凭证包含 `AccessToken A`（有效期 1 小时）和 `RefreshToken R1`。
        *   Worker 1 与 Worker 2 通过 `fs::copy` 获取了相同的内容。
        *   1 小时后，`AccessToken A` 到期。Worker 1 率先发起请求，Claude CLI 自动调用 `RefreshToken R1` 向 Auth 服务器刷新。
        *   服务器向 Worker 1 返回新的 `AccessToken B` 和 **全新的 `RefreshToken R2`**，同时在服务端**注销并失效 `R1`**。Worker 1 将新凭据写入本地副本。
        *   片刻后，Worker 2 的 Token 也过期。它试图使用自己本地的 `RefreshToken R1` 发起刷新。
        *   **服务器端检测到已被失效的 `R1` 再次被使用**。根据 RFC 6819 安全规范，服务器会立即判定发生“重放攻击/凭据泄露”，启动入侵防御机制：**强行作废由 `R1` 派生出的所有会话，包括 Worker 1 刚刚获取的 `AccessToken B` 和 `R2`**。
    3.  **后果**：所有并发运行的 Worker 会在一瞬间**全部发生凭据失效（401 Unauthorized）并掉线**。
    4.  **审计结论**：定稿 spec 声称的物理复制能实现“隔离”，完全是不懂 OAuth 安全规范的想当然。RTR 会直接击穿这种本地物理隔离，且其触发概率是 100%（只要运行时间超过 Token 寿命）。将此定性为“残余风险”是极不负责的悬空，此方案在生产环境下根本不可用。

---

## 六、 跨 Spec 一致性与接口悬空 (Cross-Spec Review)

### 1. D7 协作接口事件形状的悬空 —— 降级为“各自假设”的泥潭
*   **Spec 现状**：`ah-perception-arbiter` 与 `ah-control-plane-refactor` 的 D7 章节均声称“事件形状留给两边实施者对表，不在这轮定死”。
*   **审计结论**：这是典型的**架构设计缺位**。
    *   感知事件（Verdicts）是两套庞大系统（感知仲裁与控制状态机）之间的**唯一 API 契约**。
    *   在不确定 API 形状（如：Stalled 事件是否携带超时时长、是否包含具体的判定子层、Epoch 的比对格式等）的情况下，任何针对这两个模块的单元测试、集成测试、Mockito 桩（Fakes）都无法定义。
    *   如果两边实施者各自假设，拼接集成时一定会因为 Rust 的强类型系统发生“空中碰撞”，导致大量的临时重构和返工。

### 2. 谁来写入 `agents.state` 的逻辑割裂
*   **冲突表现**：
    *   `ah-perception-arbiter` C1 宣称：“所有信号生产者必须停止调用改写 `agents.state` 的函数，只有 Arbiter 消费循环可以写”。
    *   `ah-control-plane-refactor` D2 要求：“物理 turn-end 逻辑（`mark_agent_idle_*` 等）在**自己的事务**中写 `agents.state = 'IDLE'` 并发出事件”。
*   **审计结论**：这两处描述存在直接冲突。如果 `mark_agent_idle_*` 可以在自己的事务中写 `agents.state`，那它就是 Arbiter 之外的第二个 Write Entry Point，直接击穿了 Module C 宣称的“单写仲裁纪律”。这表明 Master 在拼凑两份 Spec 时，未能理清状态流动的确切归属。

---

## 七、 审计建议与替代机制 (Recommendations & Alternative Mechanisms)

为了纠正上述架构设计缺陷，本席建议推翻现有定稿中的部分方案，强制引入以下改进机制：

### 1. Q1 硬约束：引入 RAII 事务 Token 兜底 Trigger
*   **替代机制**：不要彻底放弃 Trigger。在 `src/db/` 内部，通过包装 `Connection`，在 `MutConnection::begin_transaction` 时向 SQLite `temp.db_config` 注入随机授权 Token；利用 Rust 的 `Drop` 机制，在事务提交、回滚或 Panic 发生时，无条件擦除 Token。
*   **价值**：在 `MutexGuard` 锁定的生命周期内，此 Token 绝无可能被其他并发任务复用。配合 DB Trigger，可在物理上杜绝 `db/` 内部开发人员手写 SQL 造成的状态污染。

### 2. Q3 cgroup：将 `workload.scope` 的创建与移动彻底上提至 Host 侧
*   **替代机制**：禁止在沙箱内部挂载 writeable cgroup 文件系统或暴露 DBus。
*   **实现机制**：由 Host 侧的 `ahd`（宿主空间拥有完全 systemd 权限）在 Spawn Agent 时，预先通过 DBus 创建好平级的 `ah-agent-xxx-workload.scope`。
*   在拉起 Agent CLI 时，Host 侧的启动器（如 `systemd-run` 命令包装器）使用 `--slice` 和 `--scope` 属性，直接把 spawned shell 放入对应的 `workload.scope` 中，而 Agent CLI 本身运行在 `cli.scope`。
*   **价值**：避免了在沙箱内挂载物理 cgroup 文件系统，消除了沙箱逃逸的致命安全隐患。

### 3. D2 解耦竞态：引入 `VERIFYING` 物理中间态保护工作区
*   **替代机制**：取消 Phase 1 将 Agent 直接标记为 `IDLE` 的设计。
*   **实现机制**：
    1.  当物理运行结束时，Agent 状态转移至 `VERIFYING`（而不是 `IDLE`）。
    2.  处于 `VERIFYING` 状态的 Agent **无法**被调度器接纳分发新任务。
    3.  Phase 2 异步验证器在 `VERIFYING` 状态保护的工作区内，安全、无竞争地读取物理证据（tmux pane、文件 diff）。
    4.  验证结束后，由控制面触发 `transit_agent_state(VERIFYING -> IDLE)`，此时 Agent 才真正被释放。
    5.  如果验证失败，Agent 状态进入 `FAILED_VERIFICATION`，保持物理工作区现场不被破坏，留待操作员 debug。
*   **价值**：彻底消除 F3/F2 解耦后物理工作区的 Read-After-Write 竞态冲突，且保留了生产现场的 Debug 价值。

### 4. 独立凭据：在 Host 引入 Token Proxy 隔离 RTR 击穿
*   **替代机制**：放弃在沙箱内进行 Copy-on-Create 的做法。
*   **实现机制**：
    *   在 Host 侧（`ahd` 进程）启动一个轻量级的 Token Proxy 服务，持有主 credentials 并统一负责 OAuth 令牌的刷新与轮转。
    *   Worker 沙箱内部不放置任何真实的 `.credentials.json` 文件。
    *   Worker 内的 Claude CLI 或 API 调用，全部配置代理指向宿主机的 Token Proxy Socket，由宿主机 Proxy 附加最新有效的 AccessToken 后转发给 Anthropic。
*   **价值**：从物理和网络协议上彻底隔离了凭据。不论服务器端如何轮转（RTR），所有 Worker 共享同一个经过代理代理的单点 Refresh 过程，彻底消除了服务端会话失效导致的级联崩盘。

---
