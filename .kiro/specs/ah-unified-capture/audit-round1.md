# audit round 1: a1 (工程可行性) + a3 (PM 替身) — 判定不合格, 回 1c 重出思路

> SOP-08 §1.1 sub-step 1b+1d. 主控已 fact-check a1/a3 引用的 file:line, 准确.
> 结论: 方向 (推 master + 沉淀) 对, 但有重大设计空洞 + 工程硬伤, 不能直接进实施. 回 1c 让 a2 重出思路.

## a1 工程硬伤 (6 条, 全 file:line 实证)

1. **[命门] event.subscribe 不是通用 event bus — UNKNOWN_PATTERN_STABLE 推不到 master**
   - 证据: `event.subscribe` 只处理 stuck + job terminal frame, 且默认强制要 job_id (src/rpc/handlers.rs:1013-1022, :1569-1635, :1655-1660)。现有 `UNKNOWN_PROMPT_DETECTED` 入表后**没有 notify_event 调用**, master 订阅不到 (src/prompt_handler/events.rs:10, :68)。
   - 三轴: 证据 High × 影响 High × 置信 High。
   - **这是 a2 最大误判**: "推给 master" 的前提 (event bus 能推任意新事件) 现在不成立, 必须先把订阅链路泛化。

2. **删 try_llm_slow_path 不是一刀删函数**
   - 证据: 调用 runner.rs:327, 定义+依赖 :414-529; RunnerContext 有 llm_classifier 字段+builder (:47-57, :95-98); integration 硬接 RealHaikuClassifier (integration.rs:287-290); LLM 行为测试集中 runner.rs:1120-1258。
   - 状态机: 新的 `mark_prompt_pending_and_emit_unknown_sync` 可从 IDLE/BUSY/SPAWNING/WAITING_FOR_ACK 转 PROMPT_PENDING (integration.rs:329-351); 但旧 `mark_agent_prompt_pending_sync` 只允许 IDLE/BUSY (state_machine.rs:194-218), 测试明确拒绝 transient (:1168-1225)。
   - 三轴: 证据 High × 影响 Medium-High × 置信 High。

3. **PromptCase 加 Category 枚举有兼容坑**
   - 证据: category 已是必填 String, 现有值 "auto-skip"/"auto-accept"/"manual-resolve" (schema.rs:70-87, seeds.rs:24/51, resolve.rs:248)。
   - 坑: 直接改枚举 → 旧 JSON KB 反序列化失败, 需 serde(rename) / Other(String) / 迁移。DB prompt_experience.category 是 TEXT (schema.rs:85-98), 不需表结构迁移但语义迁移要处理。
   - 三轴: 证据 High × 影响 High × 置信 High。

4. **save-to-kb 不够通用**
   - 证据: 现有 `ah prompt resolve --save-to-kb` 绑定"解决 prompt 并顺手保存" (CLI prompt.rs:14-27, RPC router.rs:25/86, 实现 resolve.rs:52-88 要求 agent 是 PROMPT_PENDING + 发 action + 转回 BUSY/IDLE)。保存逻辑写 escaped 整屏 prompt case (resolve.rs:221-269)。
   - 坑: 不能存 Marker/Cancel 类规则, 需新增明确 RPC (保存 rule + 带 category + 校验样例 + 不强制发键)。
   - 三轴: 证据 High × 影响 High × 置信 High。

5. **cancel: Manifest 加 cancel_sequence 可行, 但需 normalize**
   - 证据: Manifest 无 cancel 字段 (manifest.rs:5-21); cancel 硬编码 send_ctrl_c (handlers.rs:1025-1047); tmux 有 send_keys_keysym async API (session.rs:603-612), send_ctrl_c 只是 C-c 包装 (:615-618)。
   - 坑: 配置字符串要用 tmux keysym 规范, "Ctrl-C" ≠ tmux "C-c"; 建议复用 PromptAction key whitelist 或显式 normalize。
   - 三轴: 证据 High × 影响 Medium × 置信 High。

6. **健康检查内存: 无 scope→cgroup 正式映射**
   - 证据: health tick 只查 active agents + pane capture + last output/marker (health_check.rs:27-53, :131-145); agent scope 创建只写 description (sandbox/systemd.rs:23-31); db/system.rs 可 systemctl list-units --type=scope 按 description 找 scope (:282-315, :418-443), 但无 ControlGroup 字段。测试里临时做法: `systemctl --user show <scope> -P ControlGroup` 拼 /sys/fs/cgroup/.../cgroup.procs (tests/orphan_reap.rs:230-244)。master 只有 pidfd 退出监控 (master_watch.rs:12-36), master scope 无 description (sandbox/systemd.rs:51-68)。
   - 坑: agent 内存可补 (找 scope→show ControlGroup→memory.current); master cgroup 需从 pane pid /proc/<pid>/cgroup 反查或给 master scope 加 description/unit 追踪。
   - 三轴: 证据 High × 影响 Medium-High × 置信 High。

## a3 PM 替身 must-fix (7 条)

1. **[命门 §1.1] Marker 泛化最核心歧义"屏幕稳定 = 任务完成 还是 还在算?"未解** — 完成检测的真值来源问题, 设计空白。
2. **[黄金原则硬伤 §2.1] 删 api-key 兜底后 master 不在线 → agent 发 UNKNOWN 没人接 → 永久挂起且无日志无告警 = 新静默失败** (a1 硬伤1 给出工程根因: 现在本来就推不到 master)。
3. **[接口契约缺失 §3.1] 沉淀只增不减、无纠偏、无淘汰 → KB 必脏**; master 判断错怎么纠正/回滚?
4. **[§2.3] Marker 假绿无物理实证交叉校验** (master 写正则过宽 → 没真完成就判完成)。
5. **[§1.2] antigravity 多步登录未覆盖 — 首用例就跑不通**。
6. **[§3.2] 三类规则 (prompt/marker/cancel) 冲突仲裁未定义**。
7. **[§4.1] 零验收点** — 没有 UNKNOWN 触发精确条件 / 正则二次验证拒绝标准 / PROMPT_PENDING 超时升级值 / Marker 假绿负向测试。a3: 一个没验收点的机制设计不能进实施。

a3 最关键定性: **设计把"已存在的 Prompt 闭环"和"真正困难的 Marker 泛化 + 错误沉淀治理"混为"复用同一机制"一笔带过, 导致最该设计的两块 (完成检测真值来源、沉淀纠偏淘汰) 是空白。回 1c 重出思路时把 Marker 泛化和 KB 生命周期治理当独立第一性问题单独论证。**

## 主控 synth → 1c 重出思路的解题清单 (喂回 a2)

a1+a3 两份 audit 不冲突, 高度互补。回 1c 必须解决:

A. **先泛化 event.subscribe 成通用 event bus** (a1硬伤1 = a3§2.1 工程根因): UNKNOWN 类事件要 notify_event 实时推 master, 不强制 job_id。
B. **master 不在线 fallback** (a3§2.1): PROMPT_PENDING 不能永久挂起, 要超时升级 + 可观测告警 (不是静默失败)。
C. **Marker 真值来源** (a3§1.1 命门): "屏幕稳定 = 完成" 的歧义怎么解 — 独立第一性问题单独论证。
D. **KB 生命周期治理** (a3§3.1 + a1硬伤3/4): 纠偏 / 淘汰 / 冲突仲裁 + Category 兼容 (serde rename/Other) + 通用沉淀 RPC (不复用绑定发键的 resolve, 带 category + 校验样例)。
E. **antigravity 多步登录覆盖** (a3§1.2): 首用例的真实交互链路 (登录框可能多步 + quota 弹窗 + auth 方式选择)。
F. **验收点** (a3§4.1): 每项给可测断言 (UNKNOWN 触发精确条件 / 正则二次验证拒绝标准 / 超时升级值 / Marker 假绿负向测试 / 已知 provider codex 回归对照)。
G. **cancel normalize + 健康检查 cgroup 映射** (a1硬伤5/6): 工程细节, 实施时解。
