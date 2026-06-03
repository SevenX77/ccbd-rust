# ah 通用抓取机制设计

## 0. 目标与非目标

本设计把 ah 对 CLI 屏幕的三类脆弱硬编码表面纳入同一套“稳定未知屏幕 -> master 学习 -> Rust 校验 -> 回填消费”的闭环：

- `StartupReadiness`: SPAWNING 期就绪门。用于修复 CLI 更新后 init probe 失配，例如 Claude v2.1.158 已到 idle 但 `ClaudeInitProbe` 未识别的 gap #5。
- `RuntimeMarker`: IDLE/BUSY 稳态 marker。用于修复运行中完成符、忙碌反模式和光标锚点漂移。
- `ReplyExtraction`: reply 纯净提取规则。用于修复 antigravity first-ask banner / scroll-state 噪音，不继续堆硬编码 chrome filter。

非目标：

- v1 不做治理引擎；规则纠偏、淘汰、隔离、长期排序延后。
- v1 不引入 `confidence`、`fail_count`、quarantine、LRU 淘汰字段；这些只在 Phase 4 重新设计。
- v1 不做多步登录状态机；冷启动交互仍优先由 provider seed 保底，复杂登录会话延后。
- v1 不引入 API-key LLM 兜底；master 通过现有 TUI 订阅事件介入。

## 0.5 继承字段表

| 现状字段/接口 | 现状证据 | 本设计处理 |
|---|---|---|
| `PromptKb.version`, `PromptKb.cases` | `PromptKb` 是文件 KB，字段在 [src/prompt_handler/schema.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/schema.rs:37)，文件读写在 [src/prompt_handler/kb.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/kb.rs:12)。 | 保留。Prompt KB 继续只承载 legacy prompt action case。 |
| `PromptCase.id/provider/fingerprint/action/category/...` | 字段在 [src/prompt_handler/schema.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/schema.rs:70)，`category` 现为 `String`，现有 seed 使用 `"auto-skip"`/`"auto-accept"`：[src/prompt_handler/seeds.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/seeds.rs:24)。 | 不破坏 JSON。新增 `LearnedRuleCategory` 只用于新表和新 RPC；legacy `PromptCase.category` 继续自由字符串。 |
| `prompt_experience` 表 | 表字段为 `id/provider/fingerprint_type/fingerprint_value/action_json/category/confidence/source/used_count/created_at/last_used_at/trigger_state`：[src/db/schema.rs](/home/sevenx/coding/ccbd-rust/src/db/schema.rs:85)。 | [BREAKING] 不再作为新学习规则主表；保留只读兼容，Phase 3 后可停止写入。迁移见 §6。 |
| `agents.state` | 当前常量有 `SPAWNING/IDLE/WAITING_FOR_ACK/BUSY/PROMPT_PENDING/STUCK/CRASHED/KILLED/UNKNOWN`：[src/db/state_machine.rs](/home/sevenx/coding/ccbd-rust/src/db/state_machine.rs:13)。 | [NEW] 增加 `SPAWNING_INTERVENTION` 与 `FINALIZING`。旧数据不迁移；新状态仅新流程写入。 |
| `agents.exit_code` / `schema::Agent.exit_code` | DDL 有 `exit_code INTEGER`：[src/db/schema.rs](/home/sevenx/coding/ccbd-rust/src/db/schema.rs:18)，Rust 字段在 [src/db/schema.rs](/home/sevenx/coding/ccbd-rust/src/db/schema.rs:120)。 | [BREAKING] 从完成真值设计中删除。DB 可先保留 nullable 列供 crash 诊断，禁止用于 job 完成/idle 判定；后续 schema v2 可重建表移除。 |
| `events` 表 | `seq_id/agent_id/request_id/event_type/payload/created_at`：[src/db/schema.rs](/home/sevenx/coding/ccbd-rust/src/db/schema.rs:35)，插入函数在 [src/db/events.rs](/home/sevenx/coding/ccbd-rust/src/db/events.rs:30)。 | 保留表结构。[NEW] 事件类型 `UNKNOWN_PATTERN_STABLE`，插入后同步 publish `EventFrame`。 |
| `event.subscribe` | 非 stream 路径强制 `job_id`：[src/rpc/handlers.rs](/home/sevenx/coding/ccbd-rust/src/rpc/handlers.rs:1013)；stream 路径也强制 `job_id`：[src/rpc/handlers.rs](/home/sevenx/coding/ccbd-rust/src/rpc/handlers.rs:1622)。 | [BREAKING] `job_id` 改可选；无 `job_id` 表示订阅全局事件。兼容：带 `job_id` 的旧客户端语义不变。 |
| `EventFrame` | 已有 pubsub frame: `event_id/kind/agent_id/job_id/state/ts_unix_micro/payload`：[src/orchestrator/pubsub.rs](/home/sevenx/coding/ccbd-rust/src/orchestrator/pubsub.rs:4)。 | 扩展 payload schema，不改字段名。 |
| Init probe | `InitGateProbe.detect` 是 provider 硬编码：[src/provider/init_probe.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe.rs:8)；Claude 依赖 `Sonnet/Haiku/Opus` + `❯`：[src/provider/init_probe.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe.rs:24)。 | 保留为 seed 层；新增 learned `StartupReadiness` 在 `init_probe_task` 中优先/并行消费。 |
| init timeout | 现在超时直接 `mark_unknown_after_timeout`：[src/provider/init_probe_task.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe_task.rs:75)，写 `UNKNOWN`：[src/provider/init_probe_task.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe_task.rs:296)。 | [BREAKING] 稳定未知屏幕不再直接 dead-end UNKNOWN，先进入 `SPAWNING_INTERVENTION` 并发事件。 |
| Runtime marker | `MarkerMatcher` 由 manifest regex 构建：[src/marker/matcher.rs](/home/sevenx/coding/ccbd-rust/src/marker/matcher.rs:43)，Claude runtime regex 仍是 `(?m)^\s*❯\s*$`：[src/marker/matcher.rs](/home/sevenx/coding/ccbd-rust/src/marker/matcher.rs:98)。 | 保留为 seed 层；新增 learned `RuntimeMarker` 匹配器。 |
| Reply distill | 目前从 `output_chunk` 重建 vt100，再 `distill_reply`：[src/db/jobs.rs](/home/sevenx/coding/ccbd-rust/src/db/jobs.rs:451)，硬编码过滤在 [src/db/jobs.rs](/home/sevenx/coding/ccbd-rust/src/db/jobs.rs:517)。 | [NEW] learned `ReplyExtraction` 优先；硬编码 distill 作为 fallback。 |
| RPC method list | 当前无 `agent.learn_rule`：[src/rpc/router.rs](/home/sevenx/coding/ccbd-rust/src/rpc/router.rs:13)。 | [NEW] 注册 `agent.learn_rule`。 |

## 1. Learned rule 数据模型

采用一张新表 `learned_rules`，用 `category` 区分三类规则。原因：三类规则共享 provider、fingerprint、正例、创建来源、启停语义；分表会重复校验和事件回填路径。

### 1.1 数据库 schema

```sql
CREATE TABLE IF NOT EXISTS learned_rules (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    category TEXT NOT NULL CHECK(category IN (
        'StartupReadiness',
        'RuntimeMarker',
        'ReplyExtraction'
    )),
    fingerprint_type TEXT NOT NULL CHECK(fingerprint_type IN ('regex')),
    fingerprint_value TEXT NOT NULL,
    regex_flags TEXT NOT NULL DEFAULT '[]',
    positive_examples_json TEXT NOT NULL,
    action_json TEXT,
    extraction_json TEXT,
    cursor_anchor_json TEXT,
    source_event_seq_id INTEGER REFERENCES events(seq_id),
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(provider, category, fingerprint_type, fingerprint_value)
) STRICT;
```

Indexes:

```sql
CREATE INDEX IF NOT EXISTS idx_learned_rules_lookup
ON learned_rules(provider, category, enabled, created_at);
```

迁移：

1. 在 `SCHEMA_DDL` 增加表和 index。
2. 在 `db::init` 的 migration 链后追加 `migrate_learned_rules`，使用 `CREATE TABLE IF NOT EXISTS`，不改旧表。
3. 不从 `prompt_experience` 自动迁移；旧经验多为 prompt action，不可安全升级为 readiness/marker/extraction。

### 1.2 Rust 类型

[NEW]

```rust
enum LearnedRuleCategory {
    StartupReadiness,
    RuntimeMarker,
    ReplyExtraction,
}

struct LearnedRule {
    id: String,
    provider: String,
    category: LearnedRuleCategory,
    fingerprint: RuleFingerprint,
    regex_flags: Vec<String>,
    positive_examples: Vec<String>,
    action: Option<Vec<PromptAction>>,
    extraction: Option<ReplyExtractionSpec>,
    cursor_anchor: Option<CursorAnchor>,
    source_event_seq_id: Option<i64>,
    enabled: bool,
}

enum RuleFingerprint {
    Regex { pattern: String },
}

struct CursorAnchor {
    cursor_row_delta_from_match: i16,
    cursor_col_delta_from_match_end: i16,
}

struct ReplyExtractionSpec {
    start: ExtractionAnchor,
    end: ExtractionAnchor,
    drop_lines: Vec<String>,
}

enum ExtractionAnchor {
    LastPromptEcho { prompt_markers: Vec<String> },
    NextRegex { pattern: String },
    StatusLine { pattern: String },
}
```

`PromptCase` 不改成 enum；其 `category: String` 保持 legacy 兼容。新规则的 category 在 `learned_rules.category` 层校验。

### 1.3 分类语义

`StartupReadiness`:

- 触发学习：SPAWNING 期 capture 连续稳定但 seed init probe + seed/learned marker 均未命中。
- 消费点：`init_probe_task` 每次 capture 后，先查 `learned_rules(provider, StartupReadiness)`；命中且 cursor anchor 通过则允许 `SPAWNING_INTERVENTION|SPAWNING -> IDLE`。
- 必须带 `positive_examples`，至少 1 条真实整屏或状态行样本。

`RuntimeMarker`:

- 触发学习：IDLE/BUSY/WAITING_FOR_ACK 期间，稳定屏幕既不是 prompt KB，也不是 seed/learned marker。
- 消费点：`MarkerMatcher` 新增 learned matcher，seed regex 后或前执行；命中进入 `FINALIZING` 而非立刻完成。
- `action_json` 必须为空；该类只判断完成/idle，不发键。

`ReplyExtraction`:

- 触发学习：job 完成后 reply distill 结果含 chrome 噪音，master 根据截图提交答案区域锚点。
- 消费点：`collect_reply_for_dispatched_job_sync` 在 `distill_reply` 前尝试 provider learned extraction；失败才 fallback 到现有 `distill_reply`。
- `extraction_json` 必填，`action_json` 必须为空。

## 2. 就绪门可学习化

### 2.1 现状问题

当前 `init_probe_task` 的主循环在 deadline 到期时直接调用 `mark_unknown_after_timeout`：[src/provider/init_probe_task.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe_task.rs:75)。该函数把 agent 标为 UNKNOWN：[src/provider/init_probe_task.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe_task.rs:296)。这使 gap #5 这种“真实已 idle，但 probe 不认识”的画面变成死终局，master 无法学习。

Claude seed probe 的硬编码条件在 [src/provider/init_probe.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe.rs:27)，runtime marker 对 Claude 仍要求裸 `❯` 行：[src/marker/matcher.rs](/home/sevenx/coding/ccbd-rust/src/marker/matcher.rs:98)。真实 `❯ Try "fix lint errors"` 会压测这条路径。

### 2.2 新状态

[NEW] `STATE_SPAWNING_INTERVENTION = "SPAWNING_INTERVENTION"`

语义：agent 仍处于启动期，屏幕稳定但 readiness 未识别，正在等待 master 学习规则。它不是可调度状态，也不是终态。

状态转换：

```text
SPAWNING
  ├─ seed InitGateProbe / learned StartupReadiness 命中 → IDLE
  ├─ 已知 prompt 自动处理 → SPAWNING
  ├─ 稳定未知屏幕 → SPAWNING_INTERVENTION + UNKNOWN_PATTERN_STABLE
  └─ capture/tmux fatal → UNKNOWN

SPAWNING_INTERVENTION
  ├─ agent.learn_rule(StartupReadiness) 入库后下一轮命中 → IDLE
  ├─ master timeout → FAILED
  └─ kill/session.kill → KILLED
```

[NEW] `STATE_FAILED = "FAILED"` 用于 master 离线/干预超时。现有 job 已有 `FAILED` status；agent 缺少 `FAILED` 常量，需新增。若不想扩 agent state，也可用 `UNKNOWN` + `error_code = MASTER_OFFLINE_INTERVENTION_TIMEOUT`，但 formal design 推荐新增 `FAILED`，避免 UNKNOWN 混淆“检测未知”和“干预超时”。

迁移：

- `state_machine.rs` 增加常量与 `test_state_constants_match_strings`。
- `is_active_state` 不包含 `SPAWNING_INTERVENTION` 和 `FAILED`；调度器不得向这两态派 job。
- `query_agents_by_state` 无 schema 迁移，只是字符串值新增。

### 2.3 稳定未知触发条件

在 `init_probe_task` 内增加 `StableUnknownDetector`：

- `capture.trim()` 非空。
- sanitize 后 hash 连续 3 次相同。
- 采样间隔 500ms。
- seed `InitGateProbe.detect` false。
- learned `StartupReadiness` false。
- `scan_startup_prompt` 未处理 prompt，且不是 `Pending`。

触发后：

1. 插入 `UNKNOWN_PATTERN_STABLE` event。
2. 转 `SPAWNING -> SPAWNING_INTERVENTION`。
3. `wake_up` 并 `notify_event`，master 订阅可见。

伪代码：

```rust
if stable_unknown.count >= 3 && state == STATE_SPAWNING {
    let payload = UnknownPatternPayload::startup_readiness(...);
    mark_spawning_intervention_and_emit_unknown_pattern(db, agent_id, payload)?;
    return Ok(());
}
```

### 2.4 learned rule 消费

`run_init_probe_task` 每次 capture 后新增顺序：

1. `learned_startup_matcher.matches(provider, capture, cursor)`。
2. seed `probe.detect(&capture)`。
3. startup prompt scan。

命中 learned rule 时仍需 `STEADY_COUNT` 连续确认，复用当前 seed 连续确认语义：[src/provider/init_probe_task.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe_task.rs:107)。

## 3. `agent.learn_rule` RPC

### 3.1 路由

[NEW] 在 [src/rpc/router.rs](/home/sevenx/coding/ccbd-rust/src/rpc/router.rs:13) 注册：

- `agent.learn_rule`

并在 dispatch match 中调用 `handle_agent_learn_rule`。

### 3.2 参数 schema

```json
{
  "agent_id": "ag1",
  "category": "StartupReadiness",
  "provider": "claude",
  "fingerprint": {
    "type": "regex",
    "pattern": "(?m)^\\s*❯\\s+Try \"fix lint errors\""
  },
  "regex_flags": ["Multiline"],
  "positive_examples": [
    "❯ Try \"fix lint errors\"\\nOpus 4.8 (1M context)\\nbypass permissions"
  ],
  "action": null,
  "cursor_anchor": {
    "cursor_row_delta_from_match": 0,
    "cursor_col_delta_from_match_end": 1
  },
  "extraction": null,
  "source_event_seq_id": 123
}
```

字段规则：

- `agent_id`: 必填，用于权限和当前 provider 校验。
- `provider`: 可选；缺省取 agent.provider。若传入必须等于 agent.provider。
- `category`: 必填，三值之一。
- `fingerprint.type`: v1 只支持 `"regex"`。
- `positive_examples`: 必填非空；每条长度限制 16 KiB，总数 1-10。
- `action`: v1 仅 legacy prompt 可用；三类 learned rule 中必须为 null。
- `cursor_anchor`: `StartupReadiness` 和 `RuntimeMarker` 推荐；`ReplyExtraction` 可为空。
- `extraction`: 仅 `ReplyExtraction` 必填。
- `source_event_seq_id`: 推荐必填；若提供，必须属于同一 agent 的 `UNKNOWN_PATTERN_STABLE`。

### 3.3 防假绿校验

Rust 端入库前必须校验“过宽”和“过窄”两边：

```rust
let regex = build_regex(pattern, regex_flags)?;
for example in positive_examples {
    ensure!(regex.is_match(example), "regex must match positive example");
}
for negative in ["", "\n", "hello", "Working", "Thinking..."] {
    ensure!(!regex.is_match(negative), "regex matches trivial negative");
}
if category != ReplyExtraction {
    ensure!(extraction.is_none());
}
if category == ReplyExtraction {
    validate_extraction_spec(extraction, positive_examples)?;
}
```

这条校验直接针对 dogfood gap #1/#3/#5 的失败模式：不能只用干净 fixture，必须用真实带 model 后缀、banner 或 prompt 后缀的正例。

### 3.4 与 `resolve_prompt` 的关系

不复用 `resolve_prompt_with_io`。现有 resolve 路径要求 agent 是 PROMPT_PENDING 并会发键：[src/prompt_handler/resolve.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/resolve.rs:52)。现有保存逻辑把整屏截图 escape 成 prompt regex 并写 `manual-resolve`：[src/prompt_handler/resolve.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/resolve.rs:221)。这不适合 Marker/Readiness/Extraction。

`agent.learn_rule` 只做：

1. 校验请求。
2. 校验 regex 和正例。
3. upsert `learned_rules`。
4. publish `rule_learned` event。
5. wake orchestrator/init probe 重新扫描。

## 4. Event bus 泛化

### 4.1 当前限制

`event.subscribe` 非 stream 入口强制 `job_id`：[src/rpc/handlers.rs](/home/sevenx/coding/ccbd-rust/src/rpc/handlers.rs:1013)。stream 入口也强制 `job_id`：[src/rpc/handlers.rs](/home/sevenx/coding/ccbd-rust/src/rpc/handlers.rs:1622)。但 `UNKNOWN_PROMPT_DETECTED` 当前插入 events 后没有 publish `EventFrame`：[src/prompt_handler/events.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/events.rs:68)。

### 4.2 新订阅语义

[BREAKING-compatible]

- 带 `job_id`: 旧语义不变，订阅该 job terminal/state event。
- 不带 `job_id`: 订阅全局事件。
- 可选 `agent_id`: 缩小到单 agent。
- 可选 `kind`: 字符串或数组。推荐新字段 `kind`；兼容旧字段 `event_kind`。
- 可选 `since_seq_id`: 先 backfill DB 中 `seq_id > since_seq_id`，再挂 broadcast。

请求：

```json
{"kind": "unknown_pattern", "agent_id": "ag1", "since_seq_id": 0}
```

frame：

```json
{
  "event_id": 123,
  "kind": "unknown_pattern",
  "agent_id": "ag1",
  "job_id": null,
  "state": "SPAWNING_INTERVENTION",
  "ts_unix_micro": 1760000000000000,
  "payload": {
    "category_hint": "StartupReadiness",
    "provider": "claude",
    "pane_screenshot": "...",
    "capture_hash": "...",
    "stable_scans": 3,
    "source_state": "SPAWNING"
  }
}
```

### 4.3 publish 点

`db/events.rs::insert_event_sync` 是纯 DB helper，当前签名无 async/pubsub 依赖：[src/db/events.rs](/home/sevenx/coding/ccbd-rust/src/db/events.rs:30)。为避免 DB 层依赖 orchestrator，新增包装：

```rust
pub async fn insert_event_and_notify(...)
pub(crate) fn insert_event_sync(...) // 保持纯 DB
```

所有会被 master 订阅的事件必须走 wrapper：

- `UNKNOWN_PATTERN_STABLE`
- `UNKNOWN_PROMPT_DETECTED`
- `rule_learned`
- `stuck`

Ordering:

- DB insert 先发生，拿到 `seq_id`。
- 使用同一个 `seq_id` 构造 `EventFrame.event_id`。
- broadcast lag 时订阅者用 `since_seq_id` backfill。

## 5. ReplyExtraction

### 5.1 当前问题

当前 reply 收集把 `output_chunk` 喂给 vt100，再从 screen contents distill：[src/db/jobs.rs](/home/sevenx/coding/ccbd-rust/src/db/jobs.rs:451)。`distill_reply` 已有一批硬编码过滤：[src/db/jobs.rs](/home/sevenx/coding/ccbd-rust/src/db/jobs.rs:517)，并有 antigravity PONG/Thought 单测：[src/db/jobs.rs](/home/sevenx/coding/ccbd-rust/src/db/jobs.rs:936)。这能修稳态，但 first-ask banner / scroll state 仍不应继续靠 filter。

### 5.2 Extraction 规则

`ReplyExtractionSpec` 使用区域提取：

```json
{
  "start": {
    "kind": "LastPromptEcho",
    "prompt_markers": ["> ", "❯ ", "✦ ", "* "]
  },
  "end": {
    "kind": "NextRegex",
    "pattern": "(?m)^\\s*(?:[─━]{8,}|\\? for shortcuts\\b|esc to cancel\\b)"
  },
  "drop_lines": [
    "(?m)^\\s*▸\\s+(?:Thought|Thinking)\\b.*$"
  ]
}
```

算法：

1. 在 `screen_text` 和 raw fallback text 中找最后一个 prompt echo 行。
2. 从该行下一行开始。
3. 到下一个 separator/status/end regex 前结束。
4. 对区域内行应用 `drop_lines`。
5. 对结果整体 trim；若空，fallback 到 `distill_reply`。

### 5.3 消费点

`collect_reply_for_dispatched_job_sync` 在 [src/db/jobs.rs](/home/sevenx/coding/ccbd-rust/src/db/jobs.rs:491) 得到 `screen_text` 后：

```rust
let reply = learned_extractor
    .extract(provider, &screen_text, &raw_concat, prompt_text)
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| distill_reply(&screen_text, prompt_text));
```

需要把 provider 传入收集函数。当前签名只有 `agent_id/dispatched_at_seq_id/prompt_text`：[src/db/jobs.rs](/home/sevenx/coding/ccbd-rust/src/db/jobs.rs:451)。可在函数内通过 agent_id 查询 provider，避免改外层签名。

## 6. `try_llm_slow_path` 退场

### 6.1 当前调用树

`RunnerContext` 仍持有 `llm_classifier`：[src/prompt_handler/runner.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/runner.rs:47)。unknown prompt 分支先调用 `try_llm_slow_path`：[src/prompt_handler/runner.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/runner.rs:323)。`try_llm_slow_path` 会调用 classifier、写 `prompt_experience`：[src/prompt_handler/runner.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/runner.rs:420)。integration 硬接 `RealHaikuClassifier`：[src/prompt_handler/integration.rs](/home/sevenx/coding/ccbd-rust/src/prompt_handler/integration.rs:278)。

### 6.2 迁移步骤

[BREAKING]

1. `PromptRunOutcome::Unknown` 改名或包装为 `InterventionRequired`，携带 snapshot、depth、category_hint。
2. 删除 `RunnerContext.llm_classifier` 字段和 builder。
3. 删除 `try_llm_slow_path`、`LlmSlowPathDecision`、LLM 行为单测。
4. `scan_prompt_and_apply_outcome` 遇到 `InterventionRequired` 时调用通用 `mark_intervention_and_emit_unknown_pattern`。
5. `RealHaikuClassifier` 相关代码移出主路径；可保留为独立实验模块但不接 runner。
6. `prompt_experience` 停止写入新 LLM 经验；legacy lookup 可读到 Phase 3 结束。

测试退场：

- 删除/改写 runner 中 `llm_high_confidence_executes_action...`、`llm_low_confidence...`、`llm_missing_key...`、`llm_timeout...` 等慢路径测试。
- 新增 `unknown_prompt_emits_unknown_pattern_without_llm`。
- 新增 `master_learned_prompt_rule_resolves_next_occurrence`。

## 7. 防假绿与 master 离线 fallback

### 7.1 FINALIZING

[NEW] `STATE_FINALIZING = "FINALIZING"`

RuntimeMarker 命中不直接完成 job：

```text
BUSY + marker matched
  -> FINALIZING(entered_at, pending_job_id, matched_rule_id)
  -> 500ms 无新增 output_chunk 且 cursor anchor 仍满足 -> IDLE + job COMPLETED
  -> 有新增 output_chunk 或 anchor 失效 -> BUSY
```

实现点：

- marker watcher 命中后调用 `mark_agent_finalizing_sync`。
- `events_progress` 或 output path 看到新 output 时撤回 FINALIZING。
- finalizing watcher 每 100ms 检查超时。

完成真值来源：

- 屏幕稳定。
- cursor anchor 满足。
- FINALIZING 观察期无新增输出。
- 不使用 exit code；`agents.exit_code` 只保留 crash diagnostic，不参与完成判定。

### 7.2 cursor anchor

`CursorAnchor` 以 regex match span 和 vt100 cursor position 之间的相对关系表示。Rust 校验：

- 学习时可选但推荐；RuntimeMarker 规则若无 anchor，必须要求 regex 更窄且 positive_examples 覆盖整屏。
- 消费时若 anchor 存在，regex 命中但 cursor 不在 anchor 允许范围内则 NoMatch。

这直接覆盖“scrollback 里有旧 prompt/marker，但当前任务还没完成”的假绿风险。

### 7.3 master 离线 fallback

[NEW] intervention states 记录 entered time。

DB 选择：

- v1 不给 agents 加 `entered_at` 列，避免迁移复杂；用 state_change event payload 记录 `entered_at`，watcher 用最近 state_change 推算。
- 若性能不足，Phase 3 可加 `agents.state_entered_at INTEGER`。

超时：

- `MAX_MASTER_WAIT_S = 60`。
- `SPAWNING_INTERVENTION` 超时：agent -> `FAILED`，error_code `MASTER_OFFLINE_INTERVENTION_TIMEOUT`。
- `PROMPT_PENDING` 超时：agent -> `FAILED`；若有 dispatched job，job -> `FAILED`，error_reason 同上。

必须 publish event，不能静默挂起。

## 8. 实施分期

### Phase 1: seed 保底 (已完成)

现状：

- antigravity manifest 已有 `InitProbeKind::Antigravity`、`LineEndRegex`、`esc to cancel` anti-pattern：[src/provider/manifest.rs](/home/sevenx/coding/ccbd-rust/src/provider/manifest.rs:222)。
- antigravity init probe 用 `? for shortcuts` 前缀：[src/provider/init_probe.rs](/home/sevenx/coding/ccbd-rust/src/provider/init_probe.rs:117)。
- dogfood 已验证 spawn/cancel/kill/stop 生命周期。

验收：

- antigravity fresh sandbox 能到 IDLE。
- `? for shortcuts ... Gemini 3.5 Flash (High)` 和 `esc to cancel ... Gemini 3.5 Flash (High)` 覆盖。

### Phase 2: event bus + learn_rule + readiness 可学习

改动：

- 新建 `learned_rules` 表和 DAO。
- `event.subscribe` 支持无 job_id 全局事件。
- 新增 `UNKNOWN_PATTERN_STABLE` frame。
- 新增 `agent.learn_rule`。
- `init_probe_task` 支持 `StartupReadiness` learned matcher 和 `SPAWNING_INTERVENTION`。

验收：

- 人造 Claude gap #5 fixture：seed ClaudeInitProbe 不命中，稳定 3 次后进入 `SPAWNING_INTERVENTION`，master 学规则后 agent 自动进 IDLE。
- regex 正例校验拒绝不匹配真实带噪正例的规则。
- 旧 `event.subscribe {job_id}` 行为不变。

### Phase 3: runtime marker + reply extraction + LLM 退场

改动：

- RuntimeMarker learned matcher + FINALIZING。
- ReplyExtraction 消费点接入 collect_reply。
- 删除 runner LLM slow path。
- `prompt_experience` 停止新写入。

验收：

- 运行中 marker 命中先进入 FINALIZING，500ms 无输出才完成。
- 注入旧 scrollback marker 但 cursor anchor 不满足时不完成。
- antigravity first-ask banner 样本通过 ReplyExtraction 提取干净答案。
- 无 API key 时 unknown prompt 仍发 master 事件，不走 Haiku。

### Phase 4: 延后项

延后治理和复杂多步会话。进入条件：Phase 2/3 dogfood 至少一周，learned_rules 有真实积累后再设计。

## 9. 验收矩阵

| 场景 | 期望 |
|---|---|
| Claude v2.1.158 idle 画面 `❯ Try ...` | seed 失配后触发 `UNKNOWN_PATTERN_STABLE`；learn StartupReadiness 后 SPAWNING -> IDLE。 |
| Antigravity idle/busy 带 model 后缀 | regex 正例必须使用带后缀样本；过窄规则入库失败。 |
| Master 离线 | intervention 超 60s 后 agent/job FAILED，payload 可观测。 |
| Runtime false positive | marker 命中但 cursor anchor 不满足，不进入 FINALIZING。 |
| FINALIZING 撤回 | 500ms 内有 output_chunk，状态回 BUSY。 |
| Reply first-ask banner | learned extraction 输出纯答案，不含 banner/status/separator/prompt echo。 |
| Legacy prompt KB | `auto-skip/auto-accept/manual-resolve` JSON 仍可读，resolve_prompt 行为不变。 |

## 10. 文件级改动清单

- [NEW] `src/db/learned_rules.rs`: DAO、校验、lookup。
- [NEW] `learned_rules` DDL + migration: [src/db/schema.rs](/home/sevenx/coding/ccbd-rust/src/db/schema.rs:1), [src/db/mod.rs](/home/sevenx/coding/ccbd-rust/src/db/mod.rs:49)。
- [NEW] `src/provider/learned_matcher.rs`: StartupReadiness/RuntimeMarker matching + cursor anchor。
- [NEW] `src/db/state_machine.rs`: `SPAWNING_INTERVENTION`, `FINALIZING`, `FAILED` constants and transitions.
- [BREAKING-compatible] `src/rpc/handlers.rs`: event.subscribe optional job_id; `handle_agent_learn_rule`.
- [NEW] `src/rpc/router.rs`: register `agent.learn_rule`.
- [BREAKING] `src/prompt_handler/runner.rs`: remove LLM slow path.
- [NEW] `src/db/jobs.rs`: learned ReplyExtraction before hardcoded distill.
