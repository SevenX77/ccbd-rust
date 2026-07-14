# ah v1 Public Release — Design Spec

> Status: APPROVED by user (2026-06-30). Implementation: master sequences → a1 (codex) writes src/tests. PM-proxy authored this spec; does not write src.

## Goal

Make ah installable and usable by an **external integrator** with **per-agent rule docs the integrator can edit**, so one ah binary hosts different scenarios (first target: graph agent studio's copilot). Posture: keep the config surface **loose** for fast iteration.

## Scope

**v1 (this spec):**
1. **Rules split** — separate the compiled-in agent rules into a FIXED *ah-coordination kernel* (stays in the binary) + an EXTRACTABLE *scenario layer* (user-editable docs).
2. **Per-provider auto-injection** — integrator writes normalized `.ah/rules/<slot-id>.md`; at sandbox prep ah composes `[kernel] + [user doc]` and writes it to the **provider-appropriate** rules file.
3. **Provider typo → hard error** — no silent fallback to a bash shell.
4. **Real README + one-line install.**
5. **GitHub auto-release** (cargo-dist).

**Deferred to v2 (explicitly NOT in v1):**
- Master running as a **non-claude** provider. v1 master stays claude (sandbox claude-locked at `src/rpc/handlers/sessions.rs:292`), but its *rules doc* becomes user-editable (point 1/2). Only "make the master sandbox provider-parameterized like workers" is v2.

---

## Design 1 — Rules kernel / scenario-layer split

Current compiled-in rules live at `assets/builtin/master_rules.md` (→ `src/provider/builtin.rs:3` `include_str!`) and `assets/builtin/worker_rules.md`. They are ~80% dev-scenario SOP. Split each into:

### Master
**KERNEL (stays in binary, scenario-agnostic, ah needs it):**
- Cutover/revival readiness ACK: read `$AH_MASTER_HANDOFF`; after takeover run `ah master ack-ready --cutover-id "$AH_CUTOVER_ID"`; don't claim takeover before ACK succeeds.
- Orchestration contract: dispatch via `ah ask <agent_id> "<task>" [--wait]`; read results via `ah pend`/`ah watch`/`ah logs`; escalate via ah's escalation channel.
- "You orchestrate *through* ah; don't take out-of-band actions that break the orchestration (e.g. killing ah-managed panes/sessions)."

**SCENARIO (extracted → shipped default `master.md`, user-overridable):**
- "You are PM/CEO-lite, no ABC-choice questions to the user"
- Role mappings (analyst=Gemini, coder=Codex) ← note: Gemini is deprecated; this is exactly why it must NOT be in the kernel
- 严禁亲自写码 / 物理实证纪律 / Zoom-out 4 问 / 说人话(现状-根因-下一步)报告风格

### Worker
**KERNEL (stays in binary):**
- **NEVER self-dispatch** — must not run `ah ask` / hand work to other agents (no dispatch authority).
- Only perform the single task the current ah prompt assigns; otherwise idle for next dispatch.
- Sandbox safety: never modify host system paths (`/etc`, `/usr`, `~/.bashrc`); never bypass OAuth/auth.

**SCENARIO (extracted → shipped default worker doc, user-overridable):**
- grep-before-claim
- delivery = Unified Diff + `cargo test` green
- scope-anchoring (don't refactor outside the task)

**Borderline calls (user may move into kernel later):** master "physical-evidence discipline" and worker "only-assigned-task" — placed in scenario layer; trivially relocatable.

**a1 MUST verify before finalizing kernel:** `ah brief --raw-user` and `[NEW] ah notify escalate` are referenced in current rules — confirm they actually exist in the CLI (`src/bin/ah.rs` clap defs). The kernel must reference **only real, implemented** ah commands. Strip/replace any aspirational ones.

---

## Design 2 — Per-provider auto-injection mechanism

- **Authoring (normalized, kind-agnostic):** integrator writes `<project>/.ah/rules/<slot-id>.md` where `<slot-id>` = the agent id in `ah.toml` (`master`, `a1`, `a2`, ...). One plain-markdown file per slot; the author ignores provider kind.
- **Composition at sandbox prep:** for each slot, `final_doc = [role kernel] + [user .ah/rules/<slot-id>.md if present, else shipped default]`. Kernel is always present (fixed prefix); user/default content is the body.
- **Destination = provider-appropriate file** (reuse existing per-provider destinations in `src/provider/home_layout.rs`): claude → `.claude/CLAUDE.md`; antigravity → `.gemini/AGENTS.md`; codex → its `AGENTS.md`/rules file. ah maps slot → provider → destination; content is the same markdown, only the target file differs.
- **Source change:** today `materialize_builtin_rules` writes the compiled `MASTER_RULES`/`WORKER_RULES` to the destination (`src/provider/home_layout.rs:358-378`, `:181`). Change the *source* of the body from "compiled constant" to "compose(kernel, user-or-default file)"; keep the destination logic.
- **Master path** (`src/rpc/handlers/sessions.rs:292-297`) and worker path both route through the same compose step (role-keyed kernel: master-kernel vs worker-kernel).

---

## Design 3 — Provider typo = hard error

- Today an unknown provider name silently degrades to an interactive bash shell (`src/provider/manifest.rs:414-433`).
- Change: unknown provider → **hard config-validation error** at load/spawn (fail loud, name the bad provider + list valid ones). `bash` stays available only as an **explicitly-named** provider, never as a silent fallback.
- This is part of making the config contract trustworthy. (Related, lower-priority hardening noted by audit: no `deny_unknown_fields` → silently-ignored keys; `ah start` vs `ah up` field-parity drift. Capture as follow-ups; the typo-error is the v1 must.)

---

## Design 4 — README + one-line install

- Rewrite `README.md` (current one is stale: claims "no working binary yet" — false). Cover: what ah is (L2 orchestration daemon + CLI driven over JSON-RPC), build/install, `ah.toml` schema incl. the new `.ah/rules/` docs, start `ahd`, drive via CLI (`ah start/ask/watch/ps`) or socket, the integration model.
- **One-line install:** v1 = `cargo install --git https://github.com/SevenX77/ccbd-rust --bin ah --bin ahd` (needs Rust toolchain). After Design 5 lands, document the prebuilt-binary `curl … | sh` installer too.

## Design 5 — GitHub auto-release (cargo-dist)

- `cargo dist init` → generates `.github/workflows/release.yml`; tag push (`v0.x.y`) → CI cross-compiles binaries, creates a GitHub Release with artifacts + checksums + a one-line install script.
- v1: manual tag is fine. Optional later: release-please for version-bump/changelog automation.

---

## Where the extracted docs live (USER-FACING — the thing the user asked to see)

| What | Path | Who edits |
|---|---|---|
| Fixed ah-coordination kernel | `assets/builtin/master_kernel.md`, `assets/builtin/worker_kernel.md` (compiled in) | ah core only |
| Shipped default scenario docs (current dev SOP) | `assets/builtin/defaults/master.md`, `assets/builtin/defaults/worker.md` | maintainers (defaults) |
| **Integrator overrides (what the user edits to repurpose)** | **`<project>/.ah/rules/<slot-id>.md`** (e.g. `master.md`, `a1.md`, `a2.md`) | **the integrator** |

(Exact asset filenames are a1's to finalize; the user-facing override location `<project>/.ah/rules/<slot-id>.md` is the contract.)

---

## Test requirements (TDD — red first)

- Compose: `[kernel] + [user doc]` produced correctly; kernel always present; user doc overrides default; missing user doc → kernel + default.
- Per-provider destination: claude slot → `.claude/CLAUDE.md`, antigravity → `.gemini/AGENTS.md`, codex → its rules file — each receives the composed content.
- Provider typo → hard error (not bash); valid `bash` still works when explicitly named.
- Master path and worker path both honor `.ah/rules/<id>.md`.

## Suggested sequencing

- New branch off current `feat/ahd-persistent-service` HEAD (has latest code), or off `main` after #62 merges — master's call.
- Land in reviewable chunks: (1) kernel/default split + compose + per-provider inject + tests; (2) provider-typo error; (3) README + install; (4) cargo-dist.
