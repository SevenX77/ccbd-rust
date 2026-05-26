# PR4b Tasks: DB Learning Layer + LLM Slow Path

## Async Boundary Decision

**Decision: choose Option B for the current PR4b implementation. Keep `runner.rs` synchronous and keep LLM work behind the existing blocking prompt-scan boundary.**

Reasoning:
- Current prompt scanning already runs inside `tokio::task::spawn_blocking(move || run_prompt_scan(request))` in `integration.rs`, and `handle_prompt_chain` plus `PromptIo` are synchronous.
- Full runner async conversion would touch `integration`, `runner`, tmux IO, timer/probe callers, and existing lifecycle tests. That is a broad refactor unrelated to the first DB learning-layer slice.
- The minimal-correct boundary is to preserve the synchronous runner contract for PR4b. The later LLM slice should either use a synchronous HTTP client from inside the blocking worker or use a tightly scoped runtime bridge from that worker.

**Flag for a2/design convergence:** this conflicts with `pr4b-design.md` Decision Log item 2 (`reqwest` async + long-term prompt_handler async migration) and the `.await` pseudocode in `handle_prompt_chain`. This task file records the engineering decision, but does not rewrite a2's design doc.

## Tasks

- [x] PR4b task plan
  - Add this task list under `.kiro/specs/prompt-handler/`.
  - Record async boundary decision and the a2 design conflict.

- [x] DB learning layer: `prompt_experience`
  - Add `prompt_experience` SQLite STRICT schema and migration-safe initialization.
  - Add sync insert/upsert API with `UNIQUE(provider, fingerprint_type, fingerprint_value)` conflict handling and `used_count` increment.
  - Add sync lookup API by provider plus sanitized capture fingerprint: regex matches before hash matches.
  - Validate/deserialise `action_json` into existing `PromptAction` values.
  - Tests: table creation, insert/upsert, regex priority, hash lookup, provider matching.

- [x] Runner/gating L2 integration
  - Keep L1 JSON regex as the first classifier.
  - On L1 miss, query DB learning layer.
  - On L2 hit, return `KnownAction` and execute existing action path.
  - On L2 miss, keep current pending behavior; do not call LLM in this slice.
  - Tests: L2 hit executes `KnownAction`; L2 miss remains pending.

- [ ] LLM client slow path
  - Add Anthropic Haiku client behind the chosen synchronous boundary.
  - Resolve API key from `ANTHROPIC_API_KEY`, then config.
  - Return structured outcomes for success, low confidence, unsafe, timeout, missing key, and invalid JSON.
  - Do not trigger from `SPAWNING` or `WAITING_FOR_ACK`.

- [ ] Runner slow-path integration
  - L1 JSON miss -> L2 DB miss -> L3 LLM only for steady states.
  - Confidence >= 0.8 and safe=true executes action and writes DB experience.
  - Low confidence, unsafe, missing key, timeout, and errors become `PROMPT_PENDING`.
  - Preserve max-depth behavior and current unknown prompt event contract.

- [ ] Mock tests
  - Mock HTTP success stores experience and executes action.
  - Mock low confidence/unsafe returns pending.
  - Mock timeout/network error returns pending with reason.
  - Mock invalid JSON returns pending.

- [ ] VPS-gated real Haiku verification
  - Run with real `ANTHROPIC_API_KEY` on VPS only.
  - Verify an unknown EULA/update prompt is classified correctly.
  - Verify learned prompt is served from DB on the next scan without another LLM call.
