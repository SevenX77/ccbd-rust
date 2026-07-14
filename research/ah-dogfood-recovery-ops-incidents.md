# ah dogfood — recovery-session operational incidents (2026-06-28)

Logged by the rebuilt Master PM during the post-OOM recovery session that finished
tasks 2 and 3. These are **infrastructure observations / candidate new bugs** in the
ah orchestration layer itself, surfaced while dogfooding. They did not block task
delivery (worked around), but each may be a real defect worth its own investigation.

Environment: isolated dogfood instance `AH_STATE_DIR=/home/sevenx/.local/state/ah/pm-dogfood2`,
tmux `-L ahd-45ce5b09f5615229`, workers a1/a2=codex, a3=antigravity, a4=claude.

---

## Incident 1 — claude worker (a4) wedged in `PROMPT_PENDING`: prompt-delivery zombie

**Symptom.** Midway through the session, worker **a4 (claude)** stopped accepting
dispatched jobs. `ah ask a4 "<task>"` returned `status=QUEUED`, but the prompt text
was **never injected into a4's tmux pane** — the pane showed the previous turn's
completed response with an **empty input box** (`❯ ` with no text). `ah ps` reported
a4 stuck in state `PROMPT_PENDING` (sub_state `HookEvent`) and it never left that
state.

**Reproduction during session.** a4 had successfully completed an earlier audit
(task-2). When dispatched the task-3 audit:
- 1st `ah ask a4` → `QUEUED`, a4 → `PROMPT_PENDING`, no pane delivery.
- Waited ~30–40 min; still `PROMPT_PENDING`, empty input box.
- 2nd `ah ask a4` (re-send) → `QUEUED`; still no delivery, still `PROMPT_PENDING`.
- Worked around by re-dispatching the audit to a3 (antigravity), which completed it.

At session end a4 was **still** `PROMPT_PENDING` with 2–3 queued-but-undelivered jobs.

**Why it matters.** This is a *prompt-delivery zombie*: a worker that is alive
(pid present, pane healthy, last turn done) but to which the daemon can no longer
deliver new work. It is a different mechanism from the task-3 revival zombies (those
were *first-run interactive gates*; this is *the daemon's own prompt injection /
`PROMPT_PENDING` state machine* failing to send keys to a clean, ready pane). It
silently strands a worker — the job sits `QUEUED` forever with no error surfaced.

**Suspected area (not yet root-caused).** The `PROMPT_PENDING` → inject (tmux
send-keys) → `BUSY` transition for claude workers; whether `PROMPT_PENDING` can latch
when a prior prompt/gate detection never clears, blocking subsequent delivery. Worth
checking the prompt-handler/ack path that gates injection (e.g. whether a stale
pending marker or hook event blocks `send_keys`).

**Repro/diagnosis pointers for follow-up.**
- `ah ps` showing `PROMPT_PENDING` that never advances after `ah ask`.
- `tmux -L ahd-45ce5b09f5615229 capture-pane -t agent_a4 -p` shows empty input box.
- Inspect ahd.log for this session around the queued job IDs and the a4 pane's
  inject attempts.

---

## Incident 2 — per-worker Rust toolchain inconsistency (cargo not fungible)

**Symptom.** Only **a1's** sandbox had a working Rust toolchain; **a2's** did not.
- a1 (first cargo dispatch) reported the sandbox initially had *no* toolchain and it
  installed a minimal `stable` into its sandbox HOME
  (`/home/sevenx/.cache/ah/sandboxes/<id>/.rustup`), after which all its builds worked.
- a2, dispatched the 3b fix, wrote the code but **could not compile/test**:
  `error: rustup could not choose a version of cargo to run ... no installed toolchains`.
- The **Master** sandbox (`.../sandboxes/7de86f9fa85d`) also has no toolchain
  (`rustup toolchain list` → none), so the PM cannot run cargo directly either.

**Consequence / workaround.** cargo work is **not fungible across codex workers** —
all build/test had to be routed to **a1**. a2's 3b changes had to be handed to a1 for
compile+test verification. This serializes all cargo through one worker and defeats
load-balancing across a1/a2.

**Why it matters.** Sandbox provisioning does not deterministically seed a Rust
toolchain into each worker's isolated HOME. For a Rust project this makes most
workers unable to do the core inner-loop (build/test). Likely a sandbox-home
materialization gap (toolchain/`$CARGO_HOME`/`$RUSTUP_HOME` not linked or seeded the
way provider configs are). Candidate fix area: the same home-layout materialization
that seeds provider state could also link/seed the shared toolchain, or the sandbox
should inherit the host `~/.rustup`/`~/.cargo` via the existing link mechanism.

---

## Deferred verification gate — kill-master revival e2e (task-3) on clean CI

The natural end-to-end proof for task-3 ("kill the master / a codex worker, confirm
the revived agent skips the theme wizard / codex update prompt and reaches working
state") was **deliberately NOT run on this live stack**, for two reasons:

1. **False negative.** The currently-running `ahd`/`ah` binaries are the *pre-fix*
   build; task-3 lives on the unmerged `feat/revive-skip-interactive-gates` branch.
   Killing the master now would just reproduce the *old* zombie — it would not
   exercise the fix.
2. **OOM-cascade risk.** Standing up a fresh instance with the new binary on this
   already-OOM'd shared host risks another stack-wide OOM cascade — the same reason
   the heavy real-systemd / real-LLM e2e suites were excluded from the task-2/3 gates.

**Action:** run the kill-master revival e2e (with a task-3-built `ahd`/`ah`) in a
**clean CI environment or a dedicated low-load window**, not on the live dogfood box.
Until then, task-3 is verified at the unit level only (3a `master_watch`, 3b
`home_layout`, both green) plus independent static audit — but lacks an end-to-end
revival proof.

---

## Branch state at this milestone (none merged, none pushed — by directive)

| Task | Branch | Commits |
| --- | --- | --- |
| 1 (REVIVE_IDLE, untouched) | `feat/revive-idle` | `30434bd`, `5b66073` |
| 2 (persistent ahd unit) | `feat/ahd-persistent-unit` | `5e3e3ad`, `a475f45` |
| 3 (revival skip interactive gates) | `feat/revive-skip-interactive-gates` | `c7f14a4`, `c9706a0` |

Merge decision retained by the user.
