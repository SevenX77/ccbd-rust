# Telemetry Sources Audit (read-only, for a future usage-collection spec)

**Author:** a4 (claude), read-only audit. No code/cargo/git touched. Only file written: this one.
**Scope:** Where would per-job usage/context/quota telemetry come from, given ah's *existing* provider-log tail? Every claim is grounded in `file:line` and/or an exact JSON key observed in code, tests, or fixtures. Provider-log facts not visible in this repo are marked **unverified from repo** with the nearest ah code that parses/consumes that log.

**One-sentence headline:** ah's completion-detection tail (`src/completion/`) reads each provider's log line-by-line but currently extracts **only** turn boundaries (`turn_id` + reply text) — **no token usage, model id, effort level, or rate-limit signal is parsed or persisted anywhere in the repo** (verified: `grep -rniE "input_tokens|output_tokens|total_tokens|token_count|usage|context_window" src/ tests/` returns zero hits in parsing code). So all four requested telemetry classes are today **greenfield**; this audit maps where each datum lives relative to code ah already runs.

---

## 0. Where the logs live (anchor, verified)

`resolve_agent_log_root` (`src/completion/log_layout.rs:50-55`) maps provider → log root under the agent's sandbox home:

| provider | log root (`src/completion/log_layout.rs`) | file match rule (`src/completion/reader.rs:217-244`) |
|---|---|---|
| codex | `<home>/.codex/sessions` (`:51`) | filename `starts_with("rollout-")` && `ends_with(".jsonl")` (`:219-222`) |
| claude | `<home>/.claude/projects` (`:52`) | any `*.jsonl` (`:223`) |
| antigravity | `<home>/.gemini/antigravity-cli` (`:53`) | path tail `brain/<conv>/.system_generated/logs/transcript.jsonl` (`:233-244`) |

`<home>` is `sandbox_home_for_sandbox_dir(state_dir/sandboxes/<session_id>/<agent_id>)` (`log_layout.rs:45-49`). With `--unsafe-no-sandbox`, the log signal is disabled entirely (`log_layout.rs:41-43`, reason `"unsafe_no_sandbox_shared_home"`) → a usage collector riding this tail would also be dark in that mode. Files are discovered by recursive re-glob each tick (`reader.rs:194-215`), so newly created rollout files are picked up mid-job (`reader.rs` test `dynamic_reglob_reads_file_created_after_dispatch:326`).

---

## 1. Per-provider usage fields

**What the tail extracts today (all providers):** only `LogParseResult::TurnComplete { turn_id, reply }` (`src/completion/parser.rs:88-100`). `turn_id` and reply text are the *sole* fields lifted off any log line; every other key on the line is discarded. There is no usage/model/effort extraction path.

### 1a. Claude (`.claude/projects/**/*.jsonl`, transcript)

- **Parser:** `parse_claude_log_value` (`parser.rs:159-205`); reply text via `claude_text_reply` (`parser.rs:243-261`).
- **Token usage:** **not read by ah.** Real Claude transcripts carry a per-assistant-line `usage` object (`input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`) — **unverified from repo** (no fixture or code references these keys; `grep` for `input_tokens`/`usage` in `src/` = 0 hits). Nearest ah code that already receives the exact line these keys sit on: `parse_claude_log_value` matches `value["type"]=="assistant"` → `value["message"]` (`parser.rs:169-179`); the `usage` object lives on that same `message` in real transcripts, so it is reachable in the same pass but is currently ignored.
- **Model id:** present in the line ah already parses but **not read**. Proof it's on the line: the parser's own unit test feeds `{"type":"assistant","message":{"model":"claude-opus-4-8",...}}` (`parser.rs:326`) — i.e. `message.model` — yet `parse_claude_log_value` never reads `.get("model")`. So model id is a *free* add (same object, same pass).
- **Effort / thinking level:** **not present as a level.** Claude transcripts encode thinking as a *content block* `{"type":"thinking","thinking":"..."}` (see `claude_log_value_has_assistant_progress`, `parser.rs:230-241`, which enumerates `"text"|"tool_use"|"thinking"`), which is the thinking *text*, not an effort/budget setting. A numeric thinking-budget/effort level is **not emitted in the transcript** — unverified, and no ah code looks for one.
- **Per-line timestamp for windowing:** real Claude transcript lines carry a `timestamp` field — **unverified from repo** (ah's claude fixtures in `reader.rs`/`parser.rs` tests omit it). ah does not read it; it windows by byte cursor, not time (see §1-aggregation).

### 1b. Codex (`.codex/sessions/**/rollout-*.jsonl`)

- **Parser:** `parse_codex_log_value` (`parser.rs:127-157`). Terminal only on `{"type":"event_msg","payload":{"type":"task_complete",...}}`; lifts `payload.turn_id` and `payload.last_agent_message` (`parser.rs:147-156`).
- **Token usage:** **not read by ah.** Codex rollout streams a separate usage event — real codex emits `{"type":"event_msg","payload":{"type":"token_count","info":{...}}}` (field names `info.input_tokens` / `info.output_tokens` / `info.total_token_usage` are **unverified from repo** — no fixture, no code reads them). Nearest ah code: `parse_codex_log_value` explicitly filters on `payload.type` and returns `NotTerminal` for anything that isn't `task_complete` (`parser.rs:134-145`) — it even has a debug branch for `agent_message`/`final_answer` (`parser.rs:135-143`), demonstrating ah sees these non-terminal payload variants stream past and drops them. A `token_count` payload would arrive on the same `event_msg` stream and be dropped identically.
- **Model id:** **not read.** Codex records model/config in a session-header line (real codex writes a `session_meta`/config line at rollout start) — **unverified from repo**. No ah code reads it.
- **Effort / thinking level (codex `-c model_reasoning_effort` / `reasoning.effort`):** **not read**, and its presence in the rollout jsonl is **unverified from repo**. If present it would be in the session-header/config line, same as model id.
- **Per-line timestamp:** codex lines carry `"timestamp"` (ISO-8601), proven by the parser test fixture `{"timestamp":"2026-06-02T02:03:43.476Z","type":"event_msg",...}` (`parser.rs:302`). ah does **not** read it.

### 1c. Antigravity (`.gemini/antigravity-cli/brain/<conv>/.system_generated/logs/transcript.jsonl`)

- **Parser:** `parse_antigravity_log_value` (`parser.rs:263-292`). Terminal only on `source=="MODEL"` && `type=="PLANNER_RESPONSE"` && `status=="DONE"` && no non-empty `tool_calls`; reply = `content` (`parser.rs:270-291`).
- **Token usage:** **not present in the transcript fixtures at all.** The six fixtures under `tests/fixtures/antigravity_log/` carry only `step_index`, `source`, `type`, `status`, `created_at`, `content`, `thinking`, `tool_calls` — **no usage/token key** (verified by inspecting all six `.jsonl` files). Whether the real CLI writes token usage elsewhere is **unverified from repo**; nearest ah code is `parse_antigravity_log_value` which reads only `source/type/status/tool_calls/content`.
- **Model id:** **not present** in the transcript fixtures (no `model` key on any line). Unverified whether the real CLI records it; ah does not read it.
- **Effort / thinking level:** transcript carries a `thinking` *text* field on `PLANNER_RESPONSE` lines (fixture `final_reply.jsonl:2` → `"thinking":"**Summarizing**..."`), which is reasoning *text*, not an effort level. No numeric effort/budget in the fixtures. ah reads neither.
- **Per-line timestamp:** every antigravity line carries `"created_at"` (ISO-8601), verified across all fixtures (e.g. `final_reply.jsonl:2` `"created_at":"2026-06-23T07:12:28Z"`). ah does **not** read it.

### 1-aggregation: can usage be aggregated per job by time-window?

**Mechanism gap — ah windows by byte offset, not time.** The tail's cursor is `LogCursorMap = BTreeMap<PathBuf, u64>` (byte offset per file, `reader.rs:10`), advanced to `bytes.len()` each tick (`reader.rs:172`); progress is measured as "cursor advanced," not "newer timestamp" (`monitor.rs:75-80`). The DB job window *is* time-based: `jobs.dispatched_at` and `jobs.completed_at` are unix-epoch integers (`src/db/schema.rs:164,166`; `Job` struct fields `dispatched_at`/`completed_at`, `schema.rs:306,308`).

So the two clocks don't currently meet:
- **Per-job attribution by cursor (already available):** the monitor already runs *per agent from a baseline cursor captured at dispatch* — `prepare_log_monitor_before_send` snapshots `collect_provider_log_cursors` right before sending the prompt (`src/orchestrator/mod.rs:1214-1230`) and tails forward from there until `TurnComplete`. **Everything the tail reads between that baseline cursor and the completing line is, by construction, this job's turn.** That byte-range *is* a clean per-job boundary without needing timestamps — a usage extractor riding the same pass could sum usage events in exactly that range and attribute them to the dispatched job (the monitor already resolves `affected_job` on completion — `monitor.rs:49`).
- **Per-job attribution by timestamp (would need new plumbing):** aligning raw `timestamp`/`created_at` log times against `dispatched_at`~`completed_at` is *possible* (all three providers timestamp lines — codex `timestamp` `parser.rs:302`; antigravity `created_at` fixtures; claude `timestamp` unverified) but ah parses **none** of these times today, and `dispatched_at`/`completed_at` are epoch-seconds while logs are ISO-8601 strings — a parse+compare layer that does not exist yet.

**Bottom line:** per-job aggregation is *feasible today via the cursor window* (strongest path, needs no timestamp parsing); the timestamp-window path is a fallback that needs new time-parsing plumbing.

---

## 2. Context-usage derivation

- **Ready-made "context left" signal in logs?** **None found in-repo.** No provider fixture or parser reads a context-percentage/`context_left` field (grep for `context_window`/`context left` in `src/` = 0 hits). Antigravity/codex/claude transcripts in this repo carry no such key.
- **Ready-made signal in the UI?** For Claude specifically, a context-usage percentage exists in the *statusline* surface, **not** the transcript: prior research notes the correct field is `.context_window.used_percentage` (`research/findings/per-day/home-sevenx-2026-04-22.md:30,87`) and that `.context_window.current_usage.total_input_tokens` does **not** exist (same file). **Caveat:** that is Claude Code's statusline JSON (a different surface than the `.claude/projects` transcript ah tails), and it is **unverified from ah code** — ah does not consume the statusline. It's cited only as evidence that a per-turn context % is *emitted by the claude CLI somewhere*, just not on the path ah reads.
- **Therefore context usage must be computed:** `cumulative_input_tokens ÷ model_context_window`. The numerator would come from the per-turn usage objects in §1 (all currently unparsed). The denominator (window size, e.g. 200k / 1M) is **not in any provider log ah reads** — it would have to be a static per-model table keyed by the model id (itself unparsed today; for claude it's `message.model`, on the line ah already touches — `parser.rs:326`). No such table exists in the repo.

**Net:** context % is fully derived, not observed, on ah's current log path; it needs (a) usage extraction + (b) a model→window lookup table + (c) model-id extraction — none of which exist yet.

---

## 3. Subscription quota / rate-limit signals

**Realistic ceiling today: nothing structured, and no rate-limit scraper even exists.** ah's prompt handler recognizes exactly **two** pane patterns, neither quota-related:
- `codex_update_01` — "Update available" (`src/prompt_handler/kb.rs:327`, matcher test pane `matcher.rs:310`), category `auto-skip` (`matcher.rs:265`).
- `trust_path_01` — "Do you trust this directory?" (`kb.rs:328`, `matcher.rs:352`).

These are the only built-in default cases (`matcher.rs:192`: `is_default_case(...) && matches!(case.id, "codex_update_01" | "trust_path_01")`). A repo-wide grep for `rate.?limit|usage limit|quota|credit|approaching|reset at|too many requests|weekly limit|5-?hour` across `src/**.rs` returns **zero** matches in handler/matcher/seed code — there is **no** rate-limit/quota fingerprint anywhere.

- **Capture mechanism that exists:** the prompt handler works purely by **pane-text scraping** — `tmux capture_pane` (`src/tmux/mod.rs:81`, `src/rpc/handlers/ack.rs:62`, `src/orchestrator/mod.rs:184`) → `sanitize_pane_text` → `match_prompt_for_scan` against regex fingerprints (`PromptScanPurpose`, `matcher.rs:12`; gating in `src/prompt_handler/gating.rs`). So *if* a rate-limit banner rendered in the TUI (e.g. Claude's "approaching usage limit" / "resets at …", codex "usage limit reached"), it would be **detectable only via a new pane-regex fingerprint added to the KB/seeds** — the plumbing (capture → regex → action) is already there; the pattern is simply not authored.
- **Log events:** no provider log ah reads carries a quota/rate-limit event (none in the antigravity fixtures; codex/claude quota events **unverified from repo**). No structured signal.
- **Exit conditions:** on a stuck/never-completing turn the monitor gives up at `MAX_LOG_MONITOR_WAIT = 15 min` (`monitor.rs:10`, `:99-120`) and falls back to UI recapture/health-STUCK — this is a *timeout*, not a quota classifier; it cannot distinguish "rate-limited" from "hung."

**Ceiling statement:** today, quota/rate-limit is capturable **only** by pane-text scraping, and even that requires authoring new fingerprints — there is currently **no** quota detection of any kind (structured or scraped). A structured signal would depend on a provider emitting it in a log ah tails, which is unverified for all three.

---

## 4. Log-tail infra extension point

The clean attach point is `read_provider_log_tail_with_state` (`src/completion/reader.rs:107-180`): it already opens each provider file, walks it line-by-line, and calls `parse_provider_log_line` per line (`reader.rs:147`) inside the same byte-cursor pass that drives completion — a usage extractor could ride this exact loop (add a `parse_provider_usage_line` alongside the existing parse, accumulating per-file over the same `line_start..next_line_start` range already computed at `reader.rs:129-170`), and `run_log_monitor_tick` (`src/completion/monitor.rs:20-73`) already has `agent_id` + `provider` + resolves the `affected_job` on completion (`monitor.rs:37-49`), so per-job attribution context is in hand at emission time with no new lookup. What needs **new plumbing**: (a) a return-channel — `LogTailReadResult`/`LogParseResult` (`reader.rs:34-39`, `parser.rs:88-100`) carry only completions, so a `usage: Vec<UsageSample>` field and a `LogParseResult::Usage{...}` variant are new surface; (b) a persistence store — there is **no** usage table in `SCHEMA_DDL` (`src/db/schema.rs:1-236`), so a `job_usage`/`agent_usage` table + writer (paralleling `mark_agent_idle_log_event`, `src/db/state_machine.rs:1872`) is net-new; (c) aggregation — the monitor returns on the *first* completion (`monitor.rs:53-58`), so summing usage across a turn's many intermediate lines needs an accumulator that survives across the 250 ms ticks (`monitor.rs:9`), most naturally hung off `LogReadState` (`reader.rs:12-25`) next to the existing cursors; and (d) emission — a `notify_job_update`-style pubsub for usage (`monitor.rs:50`) if the UI is to show it live. In short: the *read pass* extends cleanly (one extra parse per line, same loop, per-job window already established by the dispatch-time cursor baseline at `orchestrator/mod.rs:1214-1230`); the *store + aggregate + emit* half is entirely new.

---

## Verification notes / unverified list

- **Verified from repo:** log root paths & file-match rules (§0); that only `turn_id`+reply are parsed (§1); codex line has `timestamp` (`parser.rs:302`); claude line has `message.model` but it's unread (`parser.rs:326`); antigravity lines have `created_at`+`thinking` and no usage/model key (all 6 fixtures); jobs time columns (`schema.rs:164,166`); only two prompt fingerprints exist and no rate-limit fingerprint (`matcher.rs:192`, `kb.rs:327-328`, grep=0); no usage table in schema; monitor timeout 15 min (`monitor.rs:10`); dispatch-time cursor baseline (`orchestrator/mod.rs:1214-1230`).
- **Unverified from repo (provider-log-format specific — do not treat as field names):** Claude transcript `usage.{input_tokens,output_tokens,cache_*}` and per-line `timestamp`; codex `token_count` payload and its `info.*` keys, codex session-header model/effort; antigravity token usage / model id anywhere; any provider's structured rate-limit/quota log event; Claude statusline `.context_window.used_percentage` is real per prior research but is a **different surface** than the transcript ah tails.
- A future spec must confirm the unverified field names against live provider logs (a captured `rollout-*.jsonl` with a `token_count` line, a real `.claude/projects` transcript with a `usage` block, and a real antigravity `transcript.jsonl`) before wiring extractors.
