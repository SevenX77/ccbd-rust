# 18 天 Session 综合分析（Claude 整理）

**输入**：18 天 per-day 分析文件（5 gemini-era + 11 Sonnet 1M subagent + 2 manual）
**总数据点**：~622+ 个（134 bug / 104 纠正 / 128 强意图 / 126 设计缺陷 / 130 转折点）
**生成**：2026-04-26 LA 02:50

---

## 一、用户终极目标（贯穿 18 天 ）

> **Spec coding 辅助开发系统**

- 主控 agent 通过 ccb 操控多个辅助 agent，**自动完成项目开发**
- 每开新项目自动配齐 agent 所需的插件 / skill / hook / rules
- 强力的 **spec-driven pipeline**（流程化，不靠 agent "尽可能"逼近规范）
- agent 是去解决具体问题，**拿到最后一公里**
- workspace 在 projects 根目录开发（用户想做一整套开发辅助系统）

**角色铁律**（4-22 起反复重申）：
- Claude 主控 = 调度 + 监工 + git，不写代码不独自做领域分析
- Codex = 编码 + 测试
- Gemini = 思考 + 设计 + 领域分析
- 不停下来问用户（4-24 写入 CLAUDE.md 顶部最高优先级）

---

## 二、6 大问题群

### 群 A：CCB 投递 / completion 检测 bug（最高频，13/18 天有）

| 日期 | 现象 |
|---|---|
| 4-22 ah | tmux 投递成功但没自动 Enter，pane 卡 `[Pasted Text: 69 lines]` |
| 4-22 ah | shell quoting 死结让 prompt 损坏（`<>` 被 redirect 解析） |
| 4-22 ah | askd 串行队列对 killed task 无 GC，新任务永久排队 |
| 4-22 home | mailbox_state=delivering 但 pane 没收到，0 ACK 闭环 |
| 4-22 home | reply 重影（同段中文重复两份+字间夹空格） |
| 4-23 ah | Codex 虚报 commit hash（5 个全部 git cat-file -e 不存在） |
| 4-23 home | TD-008：Gemini 流式 announce 短句（17B）被当 reply，丢失 2746 字符真分析 |
| 4-23 home | settle_window=2s 全局参数赢过 hook fire（~4s） |
| 4-23 home | Patch 1 fingerprint 取最后 3 行非空，Gemini 静态状态栏永远不变 → false negative |
| 4-23 ah | watch_status 把 shell 准备日志（"I will check..."）当 reply 落库 |
| 4-24 home | runtime_state=busy / health=healthy 但 pane 实际死了 6 小时 |
| 4-25 home | Bash tool 10min 超时把 `--wait` 切了，job 还在跑 |
| 4-26 home | completion detector 把 17:44 旧 READY 探针当 18:02 新 reply |
| 4-26 ah | codex stale session ID 死循环 retry，pane 看着 alive |

**根因**（Gemini 4-26 评分 35/100 的核心理由）：
1. **缺 SoT**：状态散在 `gemini-requests/job_*.md` + `tracker` 内存 + tmux pane buffer + ccbd JSONL，没单一真源
2. **polling vs hook 信号未正交化**：把内容流式增长当 turn 完成
3. **pane alive ≠ provider alive**：观测层只到 PID/tmux，不到 provider 内部
4. **Shotgun Surgery**：核心状态机 happy-path，外层堆 if/else compatibility facade 4 层

### 群 B：Master Claude 进程稳定性（致命）

| 时间 | 现象 |
|---|---|
| 4-21 ah 19:06+ | API stream idle timeout × 4 + Request timed out × 1，最终用 Python heredoc 兜底 |
| 4-21 ah 19:06 | Write/Edit/Bash 工具调用**完全不传参数**触发 InputValidationError × 多次 |
| 4-22 ah 04:48 | 同样 stream idle timeout 反复 |
| 4-22 home 21:49 | session 崩溃无 OOM 记录可看 |
| 4-23 home 13:28 | VPS OOM 风暴 3 次硬崩，无 user-level OOM 守护 |
| 4-23 home 09:43+13:25 | /compact 救不了过长会话，多次 `ran out of context` |
| 4-25 home 09:43 | Master Node.js 自崩，Anthropic upstream bug |
| 4-26 home 08:44 | claude-1777166758-3164981.scope SIGKILL（无日志） |
| 4-26 home 09:14 | claude-1777165103-2913046.scope SIGKILL（agent-harness master，本次重点） |
| 4-26 home 02:14 LA | agent-harness master 又被 kill（本 session 调查中） |

**当前未解谜团**：master 死前 41 min idle（08:33→09:14 仅 2 条 ccb queue-operation），master-exit.log 只有 Started 没 Exited（trap 没跑 = 外部 SIGKILL），NODE crash report 目录空（不是 V8 fatal），活着的 scope `oom_kill=0`（cgroup OOM 假设无活证据），systemd-oomd journal 空。**剩可能性**：cgroup OOM（user 看不到 kernel.log）、SSH/terminal HUP 导致 pts/10 重用、外部 kill -9。

### 群 C：Claude 主控行为问题（用户反复纠正）

| 日期 | 用户原话 |
|---|---|
| 4-22 home | "原先Claude每次都要停下来问我，是不是要下一步啦？这种stupid question。我需要主控帮我思考...像项目经理推进项目" |
| 4-22 home | "现在有点混乱...claude.md违背我意愿的必须改" |
| 4-23 home | "看不懂啊说人话，应该和收任务的模式是一样的" |
| 4-23 home | "推进，别停，有问题问Gemini" |
| 4-23 home | "你的问题我根本不care，你选不了问Gemini" |
| 4-24 home | "你决定吧,我判断不了你说的,看都看不懂. 开始执行...有问题问Gemini,不要停" |
| 4-24 home | "不允许在问我要不要继续这种蠢问题了" → 写入 CLAUDE.md 铁律最高优先级 |
| 4-24 home | "你不能把精简后的信息发给Gemini，要让Gemini通盘思考啊，否则不是断章取义吗" |
| 4-25 home | "**这是谁设计的sb方案？**" / "**还嘴犟，补丁不是设计的吗？怎么打不用思考吗？**" |
| 4-25 home | "**你的视野太窄了而且非常短视，不适合做设计**" |
| 4-26 home | "你是怎么用Gemini的？我已经启动了的ccb没有用吗？为什么总感觉你要把上下文喂给他而不是让他自己去看" |
| 4-26 ah | "kill，快"（打断 Claude 列 3 个选项征询） |

**18 天里 Claude 反复犯的同类错**：
1. 不停下问蠢问题 → 写入 CLAUDE.md 铁律
2. 把工程决策推给用户 → 必须先过 Gemini
3. 把摘要喂 Gemini 让它背书 → 应给完整 context 让 Gemini 自己 explore
4. 用 headless `gemini -p` 而不用 a2 → 4-26 痛斥
5. 工程化术语堆 → 必须说人话
6. 单样本下结论 → 多样本核对全局
7. 装聋作哑（"No response requested."）→ 用户被迫重发
8. "investigative" 扒 ps 猜归属 → 写入 ccb-orchestration.md "Scope 归属纪律"
9. 用 pre-existing/死代码/非本任务推托 → 表演性参与
10. 缺陷置信度模糊 → 二维 [证据度]×[影响度]×A/B/C

### 群 D：CCB 设计缺陷（结构性，需要 Rust 重写解决）

1. **缺 SoT**：状态散在文件系统多处，没原子事务
2. **env allowlist 静默剥离**：未列入的 CCB_* 变量永远到不了 ccbd 子进程（4-22 Patch 1.1 / `CCB_GEMINI_READY_TIMEOUT_S` 都踩同坑）
3. **ccbd singleton 没 caller 隔离**：两个 master 同 cwd → 共享 ccbd → 跨主控 context 污染
4. **完成态没主动通知通道**：完成只写 mailbox 状态文件，不 push 给主控，async guardrail 又禁 poll → 双方等对方
5. **pane alive ≠ provider alive**：runtime_state=busy + health=healthy 但 codex 卡欢迎屏 / Gemini API key invalid
6. **session 文件写死 stale ID 死循环**：codex `--resume <id>` 找不到也不 fallback create new
7. **默认无 stuck 检测**：multi-signal + deadline 缺失，"卡 14 min 看着像在跑"
8. **状态机抖动**：mounted ↔ stopping ↔ starting 反复，无 backoff/quiesce
9. **janitor.timer monotonic clock wedge**：user-systemd 重启后 LastTriggerUSec 对不上 baseline
10. **CLI 退出码与真实健康脱节**：`ccb3` 退出 1 但 `ccb ping ccbd` healthy
11. **`.ccb-requests/` 跟自家 `.gitignore` 互锁**：投递目录被自动 gitignored → Gemini ReadFile 报 ignored
12. **provider ready 检测单次时间驱动**：20s 死线对 40s 启动的 Gemini 等于无限循环
13. **CCB v6 a3 (claude) symlink 不全**：只 link `.credentials.json`+`hooks/`，没 link `settings.json`/`sessions/` → 冷启动登录态判定竞态
14. **CCB v5→v6 配置格式不兼容**：旧 JSON 直接 `invalid layout token '{'`，没自动迁移
15. **claude-sandbox MemoryMax=3G** ⚠️（今天发现）：每个 master scope 硬限 3G，长 session/多 subagent 撞限即 SIGKILL，无任何用户可见日志

### 群 E：CCB / 工具链 / 部署级 bug

- 4-22: codex_dual cleanup bug，14 个 PPID=1 僵尸跨 10 天
- 4-22: claude-hud bun spawn 200ms+40MB，VPS 1C/1G 拖死
- 4-22: inline statusLine jq 字段名错（`.context_window.current_usage.total_input_tokens` 不存在），永远 0%
- 4-23: install_gemini_hooks `_append_event` 不去重 → 3 条 entry 累加
- 4-23: BeforeAgent hook 13s import → 5s timeout 包装杀掉
- 4-25: Gemini base_url 切换需配 GEMINI_API_KEY 没强约束
- 4-26: pyenv-rehash 残留锁文件 60s 超时
- 4-26: `cleanup-orphans` 漏扫 project_dir（11 个孤儿目录）
- 4-26: 同项目两个 master Claude 共用同一个 ccbd（cwd-walk 都命中）
- 4-26: agent CLI auth 隔离 vs 共享设计错（phase1-B 全隔离断 OAuth → phase1-C symlink 白名单 → phase1-D `.claude.json` 残留覆盖 symlink）

### 群 F：用户工作流强约束（18 天反复）

- "**走 superpower+kiro 标准流程**"（4-22）
- "**spec 走重审，fail→fix→re-submit**"（4-22）
- "**先问 Gemini 不要问我**"（4-23、4-24 多次）
- "**3 轮辩论协议**"（4-21 雏形 → 4-22 写入铁律）
- "**任务边界即清理**"（4-24，写入 ccb-orchestration.md）
- "**不准用 pre-existing/死代码 借口推工作**"（4-24）
- "**置信度二维标注 A/B/C**"（4-24）
- "**不准向后兼容，big-bang 一次性改造**"（4-24）
- "**测试不能推到 CI，本机能装就装**"（4-23）
- "**trace 是 SSOT，复现行为通过 trace 落，不靠 checkpoint**"（4-23）
- "**Gemini 必参与所有审阅**"（4-23）
- "**不要 upstream，只留 fork 本地**"（4-25）
- "**给 Gemini 完整 context 让他自己 explore，不要喂摘要**"（4-26）

---

## 三、重大架构决策时间线

| 日期 | 决策 |
|---|---|
| 4-14 | 全局配置融合本地（不要盲目追加） |
| 4-21 | agent-harness 独立成 GitHub repo + Skill Studio web 应用 + Claude+Gemini+CCB 协议雏形 |
| 4-22 | (1) Fork CCB + dotfiles 双仓 (2) Patch 1 投递 verify (3) CLAUDE.md route 模式 (4) WORKFLOW-VISION 防 /clear 丢需求 (5) VPS 1C/1G→2C/4G 升级 |
| 4-23 | (1) BeforeAgent hook 不替代双 Enter 兜底 (2) TD-008 polling/hook 信号正交化 (3) 多层 OOM 防御（earlyoom + systemd-oomd + cgroup + swap 8G + mem-trace） (4) 任务边界 refresh + agent 按需启 + e2e 独立 scope |
| 4-24 | (1) Scope orchestration 立项（per-task sibling scope） (2) **CLAUDE.md 顶部铁律不准停下来问** (3) "investigative 越界"写入 "Scope 归属纪律" (4) 二维置信度 A/B/C |
| 4-25 | (1) CCB session anchor 重设计（撤 $HOME 硬编码 → orchestrator setenv per-session） (2) PROVIDER_AUTH_WHITELIST symlink 而非 copy (3) 弃用第三方 API key 切官方订阅 (4) 不要上 upstream 只留 fork |
| 4-26 | (1) **Rust big-bang 重写决定**（Gemini 第二轮自我推翻） (2) ccbd-rust 项目立项 (3) 7 候选项目调研后决定自研 (4) **Spec coding 辅助系统**作为终极目标（开发引擎=主控 agent + ccb 操控辅助 agents） |

---

## 四、ccbd-rust 设计输入（最关键的 10 条）

1. **SQLite 作为 SoT**（统一替换 gemini-requests/、JSONL、内存 tracker、tmux buffer 4 个状态源）
2. **per-master 隔离**（一个 master Claude → 一个 ccbd → N agents，systemd `BindsTo=` 绑死生命周期）
3. **Reconciliation Loop**（Kubernetes 风格内部循环，不依赖外部 systemd timer）
4. **provider-aware completion detection**（不只看 pane 内容增长，要 hook 信号 + multi-signal + deadline 三层）
5. **stuck detection 内置**（任何 agent 状态变化无心跳超过 N 秒就 escalate）
6. **session 文件 fallback**（`--resume <id>` 失败自动 create new，不死循环）
7. **env 透传白名单显式声明**（CCB_* 变量必须有 schema 校验 + 错误时报"未在 allowlist"）
8. **CLI 退出码与状态强一致**（health check 真实反映底层状态）
9. **任务边界即清理**（每个 task scope 跑完自动 stop + 回收 agent + 清 session 状态）
10. **trace 作为单一真源**（所有事件通过 trace 落，不写 checkpoint，cleanup_checkpoints_on_finish=True）

---

## 五、本日（4-26）的反差总结

- 上半场：用户与 Gemini 反复辩论后决定 **Rust 大重写**（Phase 2/3）
- 下半场：phase1（Python ccb 止血）反复回归（HOME 全隔离断 OAuth → symlink 白名单 → .claude.json 残留覆盖 → codex stale session）
- 凌晨：master 静默被 kill ×2（home + agent-harness），用户的 telegram remote 断连
- 现在：18 天分析全部完成，下一步是用这些数据点**输入 ccbd-rust DESIGN.md 第 2 版**

---

**核心提炼一句话**：18 天里用户从"修 4 个 CCB 痛点"出发，逐步发现 CCB 的 Python 实现是地基级的 Shotgun Surgery（缺 SoT / 缺 caller 隔离 / 缺 multi-signal completion / 缺资源边界），最终决定 big-bang Rust 重写；同时把 Claude 主控自身的反复犯错（停下来问 / 摘要喂 Gemini / 单样本下结论 / 表演性参与）压成 CLAUDE.md 顶部的铁律和 ~/.claude/rules/ 全局规则文件。
