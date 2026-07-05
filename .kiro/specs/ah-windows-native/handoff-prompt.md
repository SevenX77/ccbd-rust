# Handoff — ah Windows Native Support

**Purpose of this doc:** hand off the Windows-native effort to a fresh master/session with zero prior context. It states the goal, what is already designed and built, what is NOT built, the approved roadmap, the hard constraints, and the exact next action. Read the referenced spec files for depth — this is the orientation layer, not a replacement.

---

## 1. Goal (user directive, do not re-litigate)

Make `ah` run **natively on Windows — no WSL**. The user (operator) will actually use it. WSL runs on a VM and is inconvenient to install/run, so native is the target. This is **top priority** (2026-07-03 directive). The antigravity Flash-3.5 impl-pipeline retest is explicitly parked **until Windows native lands** (user ordering).

Today's shipping state: `ah` v1.3.0 is public (`SevenX77/ah`), platform matrix = **Linux native · Windows via WSL2 · macOS roadmap**. This effort turns "Windows via WSL2" into "Windows native".

## 2. Design status — APPROVED, specs exist

All under `.kiro/specs/ah-windows-native/`:

| File | What it is |
|---|---|
| `research.md` (280 ln) | Feasibility research (antigravity-authored, a2/a4-reviewed). Verdict: **high feasibility; tmux→ConPTY is the 3-4 week hard core; 5-8 person-weeks core, 8-12+ for full parity.** |
| `design.md` (203 ln) | **Approved design v1** (a1-codex grounded rewrite after antigravity v1 was REJECTED for being ungrounded; 13 must-fixes landed; a2 APPROVE-WITH-CHANGES + a4 SOUND). User approved 2026-07-03. |
| `m0-spec.md` (599 ln) | M0 compile-gate spec (53-item Unix-only inventory + windows stub shapes). |
| `m1-spec.md` (417 ln) | M1 Win32 adaptors spec. **This is the next implementation target.** |
| `conpty-reference.md` (402 ln) | ConPTY API reference for M2. |
| `m0-acceptance-and-carryforward.md` (28 ln) | M0 done-verdict + the 5 tracked carry-forward items into M1/M2. |

**Locked decisions (do not re-open):** IPC = Named Pipes (Win) / UDS (Unix) · paths = `%LOCALAPPDATA%` · terminal grid = `alacritty_terminal` (pin pre-1.0 version) + vt100 marker coexist · release artifact = **MSVC** · service = **Task Scheduler via COM `ITaskService`** (not Windows Service; avoids admin/UAC) · cascade-kill = **Job Objects** `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` · process-wait = process HANDLE + `RegisterWaitForSingleObject`.

## 3. Completion status — foundation done, real impl NOT started

### ✅ DONE (shipped in v1.3.0)
- **M0.5 ConPTY spike** (throwaway proof-of-core): `tests/windows_conpty_spike/` — `portable-pty` spawns ConPTY shell, `alacritty_terminal` parses the grid, asserts READY. Ran on windows-latest CI. Proves the multiplexer core is viable.
- **M0 compile gate ACHIEVED** (PR#91, merged `377a1d7`): `cargo check --all-targets --target x86_64-pc-windows-msvc` is **green in CI** (`.github/workflows/ci.yml` job `windows-msvc-check`). 53-item Unix-only surface was cfg-gated; `nix` moved to `[target.'cfg(unix)'.dependencies]`.
- **`src/platform/windows/*` — signature-complete STUBS** (not real implementations): `process.rs` `scope.rs` `service.rs` `identity.rs` `proc_info.rs` `mod.rs`. Shapes match the Linux free-function surface so the tree compiles for windows-msvc. **They do nothing yet** (e.g. `pidfd_open` always `Err`, `rpc` Windows entry returns `Unsupported`).

### ❌ NOT DONE (this is the actual product work)
- **M1 — Win32 adaptors (2-3 wk):** real impls behind the stubs — `process` (OpenProcess HANDLE + RegisterWaitForSingleObject), `scope` (Job Objects: no-breakaway + suspend-assign-resume), `service` (Task Scheduler COM), `identity`/`proc_info`, and the **Named Pipe IPC seam** (extract shared connect/dispatch out of the `#[cfg(unix)]` UDS loop, slot in a Named Pipe listener).
- **M2 — ConPTY multiplexer (4-6 wk) = FIRST USABLE MVP:** a `TerminalMultiplexer` trait abstracting tmux, and `WinPtyMultiplexer` covering the full `TmuxServer` surface — capture-pane = read the in-memory VT grid, send-keys = write PTY input, incl. scrollback (`-S -200`) + keysym Enter.
- **M3+ — parity (4-8+ wk):** attach / resize / scrollback paging / crash recovery.

**Bottom line:** a Windows user cannot run `ah` natively yet. The runtime still requires tmux (Linux/WSL2). M2 is the milestone that makes it usable.

### Carry-forward items (from `m0-acceptance-and-carryforward.md` — do NOT silently drop)
1. IPC transport seam — `src/rpc/mod.rs:26` Win returns `Unsupported`; extract seam + Named Pipe listener (M1).
2. `MonitorHandle::try_clone()` — `src/platform/windows/process.rs:22` raw copy; switch to `DuplicateHandle` when real HANDLE lands (M1).
3. `Win32_System_TaskScheduler` feature — missing from `Cargo.toml` windows-sys features; add for the Task Scheduler adaptor (M1).
4. Agent IO reader boundary — `src/agent_io/reader.rs:39` takes `File`; re-split for ConPTY stream capture (M2).
5. `test_no_legacy_pty_dependency` guard — `tests/mvp6_acceptance.rs:173` old substring guard; narrow before adding a root Windows PTY dep (M2).

## 4. Hard constraint (decides how work is verified)

Development runs on a **Linux VPS**. Windows code (`#[cfg(windows)]`) can be **written + cross-checked** here (`cargo check --target x86_64-pc-windows-{gnu,msvc}`), but **real running/testing only happens on GitHub Actions `windows-latest`** (free on the public repo `SevenX77/ah`). No local msvc toolchain/linker on the dev machine.

→ Every M1/M2 task must ship with a windows-latest CI job that actually runs the behavior. "cross-check compiles" is necessary but NOT sufficient — it's the假绿 trap. Each phase's spec must name "what's locally verifiable / what must run on Windows CI".

## 5. Execution path & next action

Roadmap order: **M0.5 spike ✅ → M0 compile gate ✅ → M1 adaptors (NEXT) → M2 multiplexer (first MVP) → M3+ parity.**

**Immediate next step:** start **M1** from `m1-spec.md`. Two spec loose-ends to close first (flagged in the approved design): (a) `wrap_command*` signature needs a spawn-plan type carrying Job Object metadata; (b) `service.rs` must export `ServiceUnitError`/`escape_*` symbols or M0 compile breaks — already stubbed, keep exported when implementing.

**Who does what (proven division of labor):**
- **codex (a1/a2)** — grounded implementation + rigorous review. M1 is real Win32 systems code → codex-led, TDD, serial cargo. antigravity's design v1 was REJECTED by a2 for being ungrounded (no file:line); codex v2 grounded it.
- **antigravity (a3)** — architecture/decision exploration (its strength; it produced the sound research + decision skeleton). Weak at grounding real code — do not hand it the impl.
- **a4 (claude)** — audit / second review.
- **Multi-agent review gate is mandatory** — it caught the plausible-but-ungrounded v1 design. Keep the审 gate on every phase's design + impl.

**Discipline reminders:** serial cargo (`CARGO_BUILD_JOBS=1`, tests `--test-threads=1`, OOM guard on the VPS). Branch per phase, PR, CI green (windows-latest job included) before merge. Don't accept "unrelated failure" claims without a baseline diff.

## 6. What "done" looks like per phase (acceptance)

- **M1 done:** windows-latest CI spawns a process via the HANDLE path, a Job Object cascade-kills children on close, a Task Scheduler task registers/runs without admin, and Named Pipe IPC round-trips an RPC — all asserted on Windows CI, not mocked.
- **M2 done (MVP):** on windows-latest, `ah start` brings up a master + worker with no tmux; capture-pane reads real agent output from the VT grid; send-keys delivers a prompt; a full dispatch→reply cycle completes. This is the first build a Windows user can actually use.
- **M3+ done:** attach/resize/scrollback/recovery reach Linux parity.

---

*Companion memory:* `project_ah_windows_native_top_priority` (priority + research verdict + constraints). Related: `project_ah_release_sync_dev_to_public_repo` (platform matrix), `project_ah_v1_public_release`.
