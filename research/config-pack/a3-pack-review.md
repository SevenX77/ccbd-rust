# ah 编程场景配置包独立 Review 报告 (a3-pack-review.md)

本报告由 a3 worker (explore/调研角色) 独立完成。对照事实基准 [dev-journey-research.md](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md)，对 `research/config-pack/pack/` 下的 10 个成品配置文件进行了三维度的 Review，挑出以下毛病：

---

## 一、失真/事实错误 (4 条)

### 1. 设计审阅者 (review) 在示例和配置中错误地关联了 Codex Provider
* **File & Line**: 
  1. [ah.toml.example:L17-18](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/ah.toml.example#L17-18)
     ```toml
     # 设计审阅者:强推理
     [agents.review]
     provider = "codex"
     ```
  2. [ROLES.md:L25](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/ROLES.md#L25)
     ```toml
     [agents.review]    provider = "codex"        # 设计审阅者
     ```
  3. [.ah/rules/review.md:L1](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/.ah/rules/review.md#L1)
     ```markdown
     # 设计审阅者 · 设计与分析  (示例 id:review-codex)
     ```
* **该改成什么**: 将 `provider = "codex"` 改为 `provider = "claude"` 或 `provider = "antigravity"` (或其它强推理型 Provider)；将示例 ID `review-codex` 改为 `review-claude`。
* **原因与出处**: 
  这属于严重的角色与能力错配。根据 [ROLES.md:L9](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/ROLES.md#L9)，设计审阅者 (review) 承担方案设计与长日志推理，需要「强推理 / 跨概念关联」能力。而根据 [dev-journey-research.md:L39/L71](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L39)，Codex 属于「强机械精确、不发散」的实现者角色（极弱于抽象推理，绝不让其处理过度发散的设计大方向问题）。强行将 review 角色配给 codex，违反了「严格角色分工 (Strict Role Split)」的红线，会导致 Codex 发散越界或出现推理幻觉。

### 2. operator 代理对 CI/PR 合并权力的描述失真
* **File & Line**: [GUIDE.md:L20](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/GUIDE.md#L20)
  ```markdown
  - operator(人的代理):把用户目标翻译成 brief 转给 master;持续盯到真出可信结果;独占 CI/PR/发布权。
  ```
* **该改成什么**: 改为 `独占最终发布与 Milestone 合并/Squash 权（开发期 mid-stream 合并仍由 master 自驱进行）`。
* **原因与出处**: 
  说 operator "独占" PR 权力与自驱闭环的设计事实不符。根据 [dev-journey-research.md:L38](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L38)（Goal Closure Loop 目标闭合自驱环）以及 [dev-journey-research.md:L68](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L68)（Master 职责），Master 协调者需要「执行 mid-stream PR 的合并决策」，自主驱动跑通闭环，不把中间的工程选择推给用户。只有最终 Milestone 级别的 squash merge 才是 operator/PM 把关。

---

## 二、缺漏 (6 条)

### 1. 遗漏了沙箱环境变量路径必须为绝对路径的硬性约束
* **事实基准**: [dev-journey-research.md:L11](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L11) (zombie agent 事故)
* **表现**: 10 个成品文件中，没有任何一处提到沙箱环境配置（如 `CLAUDE_CONFIG_DIR` 等）在复活或启动时必须绝对路径化的约束。
* **后果**: 隐式暴露了「由于 cwd 变化导致环境隔离失效、读取到项目仓库下空白的 `.claude` 目录而卡在 onboarding 向导变成僵尸进程」的巨大隐患。需要在 [GUIDE.md](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/GUIDE.md) 中明确指出。

### 2. 遗漏了 realign (up 拓扑重整) 后的双重物理验证纪律
* **事实基准**: [dev-journey-research.md:L23](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L23) (realign 删旧建新非原子 bug)
* **表现**: [GUIDE.md:L58](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/GUIDE.md#L58)（直连 live 栈）或 [GUIDE.md:L72](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/GUIDE.md#L72) 纪律表中，没有任何关于 realign 失败导致「拓扑悄缩」的防御指引。
* **后果**: operator 容易在运行 `ah up` 重置拓扑后，单凭控制台的成功输出而轻信状态。必须在纪律中补齐「每次 `ah up` 重置拓扑后，必须执行 agents 表与 `tmux list-sessions` 双重物理验证，确保没有 slot 丢失」。

### 3. 遗漏了测试环境 Flock 锁 FD 继承死锁的故障排查 SOP
* **事实基准**: [dev-journey-research.md:L15](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L15) (flock 锁 fd 继承死锁)
* **表现**: [GUIDE.md:L71](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/GUIDE.md#L71) 虽然提及了 `CARGO_BUILD_JOBS=1 + --test-threads=1` 串行构建，但漏掉了常驻后台进程（如 e2e 测试中的 tmux server）继承 lock fd 扣死编译的严重事故。
* **后果**: 测试人员如果遇到 cargo flock 锁永久挂起死等（如 80+ 分钟），会缺乏直接诊断手段。应当把「`lsof <lock>` 查找持有锁的 tmux server 并使用 `tmux -L <sock> kill-server` 精确清理」的 SOP 写入配置包中。

### 4. 遗漏了测试沙箱 GC 机制与磁盘空间清理的指引
* **事实基准**: [dev-journey-research.md:L16](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L16) (测试沙箱泄漏打满磁盘)
* **表现**: 成品文件中没有任何关于长期跑 e2e/集成测试导致 `~/.cache/ah/sandboxes` 空间积累爆满的警告。
* **后果**: 用户长时间运行测试会导致磁盘 100% 打满崩溃。应当将「利用 `/proc/*/environ` 过滤在用集合并按 mtime 清理无主测试沙箱」的 GC 清理法写入 [GUIDE.md](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/GUIDE.md) 中。

### 5. 遗漏了多 Watchdog 级联删除竞态的 Status-gate 规则
* **事实基准**: [dev-journey-research.md:L14](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L14) (session_watch 级联杀抢跑 wipe 目录)
* **表现**: 无论是 [master.md](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/.ah/rules/master.md) 还是 [GUIDE.md](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/GUIDE.md) 都没有说明多监测器之间的协调限制。
* **后果**: 开发自建多级 watchdog 监控时，容易重蹈「两个 watchdog 在 master 死亡后竞态冲突，导致 session_watch 强行删除已被 master_watch 保护重拉的 worker home」的覆辙。应写入「破坏性级联操作前必须检验全局 DB 状态（Status-gated）」的设计纪律。

### 6. `ah.toml.example` 缺少 `[env]` 变量声明的模板示例
* **事实基准**: [dev-journey-research.md:L19](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L19) (`CLAUDE_CODE_OAUTH_TOKEN` 白名单透传) 以及 v1.3.2 变更日志
* **表现**: [ah.toml.example](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/ah.toml.example) 仅给出了 agents 实例和 hook 推送的配置模板，缺少了 `[env]` 配置块。
* **后果**: 用户无法直接参考如何在 `ah.toml` 中通过环境变量（例如 `IS_SANDBOX = "1"` 或 `CLAUDE_CODE_OAUTH_TOKEN`）来实现项目级别的安全认证与沙箱注入。

---

## 三、可优化 (3 条)

### 1. 错字修正 (Double-word Typo)
* **File & Line**: [.ah/rules/audit.md:L22](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/.ah/rules/audit.md#L22)
  ```markdown
  - e2e 别 early-exit 在终态那刻,延长观察确认系统系统真收敛/真清理。
  ```
* **具体建议**: 将 "系统系统" 修正为 "系统"。

### 2. 语句表述优化
* **File & Line**: [ROLES.md:L17](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/ROLES.md#L17)
  ```markdown
  同一个人的项目里，你可以让「实现者」用任何强机械精确的 provider；换 provider 不换角色语义。
  ```
* **具体建议**: "同一个人的项目里" 语义略显突兀和别扭，建议修正为 `在同一个项目里` 或 `在任意项目里`。

### 3. 补充长任务 Stuck 时优先从 Pane 捞结论的运维策略
* **File & Line**: [GUIDE.md:L69](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/GUIDE.md#L69) (不信二次信号) 或 [.ah/rules/master.md:L21](file:///home/sevenx/coding/ccbd-rust/research/config-pack/pack/.ah/rules/master.md#L21)
* **具体建议**: 建议在「不信二次信号/亲自 check pane」的纪律下方补上一行实用提示：`当任务因超时等兜底限制被 health-check 判为 STUCK 时，重派前务必先登录目标 pane 检查，很可能 agent 已输出完整产出；直接在 pane 中捞回数据可以避免浪费整轮重复工作开销。`
* **原因与出处**: 根据 [dev-journey-research.md:L26](file:///home/sevenx/coding/ccbd-rust/research/config-pack/dev-journey-research.md#L26)（health_check redispatch 假 stuck 现场），许多时候任务其实已经在 pane 中完美产出结论，仅因为超时时间硬编码（5分钟限制）或 staleness 计数 bug 被系统误标为 STUCK。先物理捞取能挽回极高的计算资源开销。
