# Plan Review Rubrics

These rubrics are used for MVP plan review. Score each dimension from 1 to 10, cite concrete evidence, and apply the verdict rule at the end.

## Verdict Rule

PASS requires all of:

- Overall average score >= 7.0.
- Every dimension score > 3.
- `real_provider_parity` > 5.

FAIL otherwise. A `real_provider_parity` score <= 5 is an automatic FAIL regardless of the other dimensions.

## Dimensions

### 1. spec_fidelity

Measures whether the design and tasks accurately implement the stated requirements.

- 1-2: Contradicts major requirements or omits the primary outcome.
- 3-4: Covers fragments, but misses important acceptance criteria or changes their meaning.
- 5-6: Mostly matches requirements, with some ambiguous or weakly supported areas.
- 7-8: Requirements are implemented with clear mappings and only minor gaps.
- 9-10: All requirements and acceptance criteria are represented precisely, including edge cases and verification.

### 2. carve_out_clarity

Measures whether in-scope and out-of-scope boundaries are clear and consistent.

- 1-2: Scope is unclear or internally contradictory.
- 3-4: Some boundaries exist, but key exclusions are missing or conflict across R/D/T.
- 5-6: Main scope is understandable, with several gray areas.
- 7-8: Scope boundaries are explicit and mostly consistent.
- 9-10: Scope is crisp across requirements, design, tasks, and tests, including deferred work.

### 3. architecture_consistency

Measures alignment with existing architecture and prior MVP decisions.

- 1-2: Introduces incompatible architecture or breaks established contracts.
- 3-4: Fits only superficially and risks bypassing core mechanisms.
- 5-6: Mostly compatible, but has unresolved integration concerns.
- 7-8: Uses established modules and patterns with defensible changes.
- 9-10: Strengthens the architecture while preserving prior invariants and lifecycle semantics.

### 4. pseudocode_rigor

Measures whether design pseudocode and module-change descriptions are implementable without guesswork.

- 1-2: Hand-wavy prose with no executable shape.
- 3-4: Names components but leaves major control flow or data contracts undefined.
- 5-6: Enough to start implementation, but several cases require interpretation.
- 7-8: Clear step-by-step behavior, key APIs, and error paths.
- 9-10: Implementation-ready pseudocode with state transitions, fallbacks, and test hooks.

### 5. task_atomicity

Measures whether tasks are small enough for a Codex executor to implement and verify safely.

- 1-2: Large bundled tasks spanning unrelated behavior.
- 3-4: Coarse tasks that mix multiple files, features, and tests.
- 5-6: Mostly workable but still combines separable changes.
- 7-8: Tasks are file/function/test scoped with clear dependencies.
- 9-10: Each task is independently reviewable, testable, and sequenced.

### 6. ac_traceability

Measures whether every acceptance criterion maps to concrete tasks and tests.

- 1-2: No reliable AC-to-task mapping.
- 3-4: Some ACs map to tasks, but tests are missing or vague.
- 5-6: Most ACs map to tasks and test areas, with gaps.
- 7-8: Each AC maps to named tasks and specific test files.
- 9-10: Complete AC -> task -> test traceability, including negative and regression tests.

### 7. risk_coverage

Measures whether failure modes, races, fallbacks, and operational risks are covered.

- 1-2: Ignores obvious failure modes.
- 3-4: Mentions risks but lacks implementation or test coverage.
- 5-6: Covers common failures, misses important races or degraded modes.
- 7-8: Covers major edge cases with practical fallbacks and tests.
- 9-10: Includes lifecycle races, degraded environments, retries, idempotency, observability, and rollback paths.

### 8. real_provider_parity

Measures whether the plan proves real `codex`, `gemini`, and `claude` CLIs work end-to-end in fresh sandboxes.

This is the red-line dimension. Score <= 5 means automatic FAIL.

- 1-2: Mock-only coverage; no real provider intent.
- 3-4: Mock tests plus nominal provider references, but no real CLI execution.
- 5: Bash-equivalent coverage only, or real-provider tests are soft-skipped by default.
- 6-7: Real-provider tests exist with hard gates, but coverage is incomplete or not fresh-sandbox end-to-end.
- 8-9: Real `codex`/`gemini`/`claude` are exercised end-to-end with spawn, ask, env, and lifecycle coverage, but TUI interference is not strongly proven.
- 10: Real providers run end-to-end in fresh sandboxes and demonstrate resilience to TUI interference such as update, trust, or second-enter prompts.

## Review Output Requirements

For each score, cite file and line evidence. For traceability, include an AC -> task -> test mapping. If the verdict is FAIL, list blocking issues first and tie each one to a violated rubric or pass criterion.
