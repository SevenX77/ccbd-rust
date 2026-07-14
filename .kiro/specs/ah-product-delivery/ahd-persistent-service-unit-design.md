# ahd Persistent User Systemd Service Unit Design (GAP-2)

## Context

`ah start` currently bootstraps `ahd` through a transient user unit. The command builder emits `systemd-run --user --unit=ahd.service` plus restart, start-limit, OOM, and environment settings in `src/cli/start.rs:42-67`. The actual daemon bootstrap path lives in `ensure_daemon_running`: it checks the socket, removes stale sockets, locates the sibling `ahd` binary, creates the state dir, and then either runs the transient unit or falls back to direct spawn (`src/bin/ah.rs:494-565`). The recursion guard skips systemd bootstrap when the current process is already inside a detected daemon service unit (`src/cli/start.rs:69-70`, `src/systemd_unit.rs:7-18`).

Transient units do not create an installed unit file. After `systemctl --user stop ahd.service` or user manager restart, the unit definition is gone; without an installed `[Install]` section it also cannot be enabled for boot/login startup.

Goal: replace the transient bootstrap path with an installed per-user service unit under the user systemd config directory, make `ah start` idempotently install/enable/start it, and keep the existing direct-spawn fallback for systems where user systemd is unavailable.

Non-goal: this design does not change provider sandboxing, agent scope semantics, RPC protocol, or source code in this task.

## Proposed Unit File

Install an ordinary user service file at:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/<unit_name>.service
```

Use a generated unit rather than a checked-in static unit because `ExecStart`, `AH_STATE_DIR`, and pass-through environment values are runtime/user specific.

Template shape:

```ini
[Unit]
Description=ah daemon
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=simple
ExecStart=/absolute/path/to/ahd
Restart=on-failure
RestartSec=1s
OOMScoreAdjust=-900
Environment=AH_STATE_DIR=/absolute/path/to/state-dir
Environment=KEY=value

[Install]
WantedBy=default.target
```

Mapping from current transient flags:

| Current transient argument | Installed unit field |
| --- | --- |
| `--unit=ahd.service` | filename and unit name, recommended to become state-dir derived |
| `--property=Restart=on-failure` | `[Service] Restart=on-failure` |
| `--property=RestartSec=1s` | `[Service] RestartSec=1s` |
| `--property=StartLimitIntervalSec=60` | `[Unit] StartLimitIntervalSec=60` |
| `--property=StartLimitBurst=5` | `[Unit] StartLimitBurst=5` |
| `--property=OOMScoreAdjust=-900` | `[Service] OOMScoreAdjust=-900` |
| `--setenv AH_STATE_DIR=<dir>` | `[Service] Environment=AH_STATE_DIR=<dir>` |
| `--setenv <ENV_PASSTHROUGH>` | one `[Service] Environment=<key>=<value>` per present env |
| final `<ahd_bin>` argv | `[Service] ExecStart=<escaped absolute ahd path>` |

The generator should reuse `ENV_PASSTHROUGH` from `src/provider/manifest.rs:240-296`, not duplicate the list. Only variables present in the current environment should be written, preserving today's behavior in `src/cli/start.rs:30-39` and `src/cli/start.rs:59-64`.

Values must be rendered using systemd-safe escaping, not shell quoting. `ExecStart` and `Environment=` are not shell scripts; spaces, `%`, quotes, backslashes, and newlines need explicit validation/escaping. Reject environment values containing newline or NUL. Use absolute, canonical paths where possible; if canonicalization fails, keep the original absolute path and surface a clear error only when the path is relative.

Recommended logging: keep journald as the authoritative systemd log. Continue writing `ahd.log` only if implementation keeps the current stdout/stderr redirection behavior by adding `StandardOutput=append:<state-dir>/ahd.log` and `StandardError=append:<state-dir>/ahd.log`. If portability across older systemd versions is a concern, omit file append from the unit and rely on `journalctl --user -u <unit>`.

## Why Installed Unit Instead Of Transient

Installed unit advantages:

- Survives user manager restart and reboot because the unit file is on disk.
- Can be enabled through `systemctl --user enable <unit>`.
- Gives an inspectable contract for PM/user debugging: `systemctl --user cat <unit>`.
- Allows deterministic migration and uninstall semantics.

Transient unit advantages:

- No config-file writes.
- Natural one-shot bootstrap for ad hoc direct state dirs.
- Current implementation already exists and has tests around argv generation.

Recommendation: make installed units the default systemd path for `ah start`; keep transient only as a compatibility fallback during rollout if writing the unit file or daemon-reload fails for a recoverable reason. Direct spawn remains the final fallback when user systemd is unavailable, as it is today in `src/bin/ah.rs:546-562`.

## Unit Naming And State-Dir Isolation

The current hard-coded `ahd.service` collides across isolated state dirs. Dogfood already runs per-state-dir isolated instances, and `src/systemd_unit.rs:16-18` already treats both `ahd.service` and `ah-*.service` as daemon service units.

Recommendation: use a deterministic state-dir derived unit name by default:

```text
ah-<short_hash>.service
```

Where `<short_hash>` is derived from the normalized absolute state dir. A 12 or 16 hex character SHA-256 prefix is enough for practical collision resistance and keeps unit names readable. Do not include raw path segments in the unit name; state paths can contain private data and systemd escaping would still produce long, unstable names.

Keep `ahd.service` as a legacy alias only for migration from the current default state dir if PM wants command compatibility. New installs should not reuse `ahd.service` unless the selected state dir maps to an explicitly configured singleton mode.

Consequences:

- Multiple `AH_STATE_DIR` values can run independently.
- `systemctl --user enable ah-<hash>.service` enables exactly that state dir.
- Agent/tmux scope binding should bind to the detected current daemon unit, not a hard-coded `ahd.service`. Existing detection supports `ah-*.service` (`src/systemd_unit.rs:7-18`); call sites that still pass literal `ahd.service` should be audited in implementation.

## Enable And Boot Startup

After writing or updating the unit file:

```text
systemctl --user daemon-reload
systemctl --user enable <unit>
systemctl --user start <unit>
```

Use `enable` rather than only `start` because the GAP is about persistence and reboot/login startup. `WantedBy=default.target` is the correct install target for user services.

Linger:

- `systemctl --user enable` starts the unit when the user manager starts.
- Without linger, the user manager is usually tied to login sessions; services may stop when the user logs out and may not start at machine boot before login.
- `loginctl enable-linger <user>` requires appropriate privileges/polkit on many systems and changes host-level user behavior.

Recommendation: do not automatically enable linger by default in `ah start`. Instead:

- Detect linger status best-effort via `loginctl show-user <user> -p Linger`.
- If disabled, print a clear note: enabled user service will start with the user manager, but true boot-before-login and logout survival require `loginctl enable-linger <user>`.
- Add an explicit future flag or command, for example `ah service enable-linger`, if PM wants ah to manage linger.

If PM decides linger is in scope, the operation must be idempotent:

- If `Linger=yes`, do nothing.
- If `Linger=no`, run `loginctl enable-linger <user>`.
- Failure should not roll back unit install/start; report that boot-before-login is not guaranteed.

## `ah start` Flow

Current flow:

1. Check daemon socket and return if accepting (`src/bin/ah.rs:494-497`).
2. Remove stale socket (`src/bin/ah.rs:499-500`).
3. Locate `ahd`, create state dir, prepare log (`src/bin/ah.rs:503-527`).
4. If user systemd is available and not already inside daemon service, reset failed and run transient systemd unit (`src/bin/ah.rs:530-545`).
5. Fall back to direct spawn on systemd failure (`src/bin/ah.rs:546-562`).

Proposed flow:

1. Keep the initial socket fast path unchanged.
2. Keep stale socket removal, `ahd` lookup, state-dir creation, dev db cleanup, and readiness wait unchanged.
3. Compute `unit_name` from state dir.
4. If `systemd_user_bootstrap_available()` is false, use direct spawn as today.
5. Read `/proc/self/cgroup`; if `should_skip_systemd_bootstrap_for_cgroup` is true, use direct spawn to avoid recursive systemd bootstrap. This keeps the existing guard in `src/cli/start.rs:69-70`.
6. Generate desired unit content for `(unit_name, ahd_bin, state_dir, current ENV_PASSTHROUGH values)`.
7. Ensure `${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user` exists.
8. If an old unit file exists and content differs, write atomically:
   - write temp file in same directory
   - fsync temp file if local helper pattern exists
   - rename over target
   - optionally fsync parent directory
9. Run `systemctl --user daemon-reload` only when the file changed or when first installing. Running it always is simpler and acceptable; tests should allow either policy if implementation documents it.
10. Run `ahd_reset_failed_is_best_effort(&unit_name)` before start, preserving the start-limit recovery behavior from `src/cli/start.rs:73-83`.
11. Run `systemctl --user enable <unit_name>`.
12. Run `systemctl --user start <unit_name>` or `restart <unit_name>` depending on whether material content changed while the service is already active.
13. Wait for the socket to accept, same as today.

Start vs restart rule:

- If socket was not accepting and the unit is inactive, `start`.
- If unit file content changed while the unit is active but the socket was stale/not accepting, prefer `restart` to ensure new `ExecStart` and environment take effect.
- If socket accepts at the beginning, return early and do not rewrite/restart by default. This preserves a cheap idempotent `ah start`.
- Add a future explicit `ah service reconcile` if PM wants running daemons to pick up changed env/unit fields even when already healthy.

Idempotency:

- Existing identical unit file: no content rewrite required.
- Existing enabled unit: `systemctl --user enable <unit>` is harmless.
- Existing running unit: initial socket accepting path returns success.
- Existing failed unit: `reset-failed` best-effort clears systemd start-limit state before `start`.
- Existing unit for same name but different state dir: impossible with hash naming except hash collision. If detected via a generated comment like `# AH_STATE_DIR=...`, treat as fatal and do not overwrite silently.

## Transient Migration

Migration must avoid two `ahd` processes for the same state dir.

When bootstrapping a state dir whose legacy unit name would have been `ahd.service`:

1. Check whether `ahd.service` is active.
2. If active and it is a transient unit, stop it before installing the persistent unit:

```text
systemctl --user stop ahd.service
```

3. Remove stale socket if it remains and is not accepting.
4. Write the installed unit file for the new state-dir-derived unit.
5. `daemon-reload`, `enable`, and `start` the new unit.

Detecting transient vs installed:

- Best effort: `systemctl --user show ahd.service -p FragmentPath -p LoadState -p ActiveState`.
- Transient units usually have empty or non-config `FragmentPath`.
- Installed units should have a `FragmentPath` under the user systemd config tree or another systemd user unit search path.

If `ahd.service` is active and the new state-dir-derived unit is different, stopping the legacy transient unit is safe only when it owns the same socket/state dir. Prefer verifying via socket path and/or `Environment` from `systemctl show -p Environment` if available. If verification is unavailable, stop only when the target socket is stale/not accepting and the legacy unit name is the current known bootstrap unit.

Compatibility for an already installed legacy `ahd.service`:

- Do not delete or overwrite user-authored installed `ahd.service` automatically.
- If it serves the same state dir, either keep using it for this run or emit a migration warning and require explicit uninstall/migrate command.
- If it serves a different state dir, state-dir-derived naming avoids collision.

## Stop, Rollback, And Uninstall

Current `ah stop` sends `system.shutdown` through RPC and prints shutdown text (`src/bin/ah.rs:738-741`). That should remain the default meaning: stop the current daemon/session, not remove persistent service configuration.

Recommended command semantics:

- `ah stop`: call daemon shutdown as today. If the daemon is managed by a persistent user unit, also `systemctl --user stop <unit>` after successful RPC or when the socket is no longer accepting, to prevent `Restart=on-failure` ambiguity. A clean daemon exit should not restart under `Restart=on-failure`, but explicitly stopping the unit gives clearer systemd state.
- `ah service uninstall` or future equivalent: `systemctl --user disable --now <unit>`, delete the generated unit file, `systemctl --user daemon-reload`, and optionally `systemctl --user reset-failed <unit>`.
- Uninstall must only delete files marked as ah-generated and matching the expected unit path. Never remove arbitrary user-authored unit files.
- Rollback on failed install: if file write succeeds but `daemon-reload` or `enable` fails, leave the file in place and report the exact failing command. Do not attempt direct deletion unless the file was newly created in the same operation and PM explicitly wants transactional rollback.
- Rollback on failed start: keep the installed/enabled unit so the user can inspect it. Fall back to direct spawn only if preserving current CLI behavior is more important than surfacing service install errors. My recommendation is to fall back only when systemd is unavailable; for installed-unit command failures, return a clear error.

## Testing Strategy

Unit-level tests:

- Generate unit content from `(ahd_bin, state_dir, env)` and assert exact fields:
  - `[Unit] StartLimitIntervalSec=60`
  - `[Unit] StartLimitBurst=5`
  - `[Service] ExecStart=<ahd_bin>`
  - `[Service] Restart=on-failure`
  - `[Service] RestartSec=1s`
  - `[Service] OOMScoreAdjust=-900`
  - `[Service] Environment=AH_STATE_DIR=<state_dir>`
  - present `ENV_PASSTHROUGH` values are included, absent ones are not
  - `[Install] WantedBy=default.target`
- Escaping tests for spaces, `%`, quotes, and backslashes in paths/env values.
- Reject newline/NUL in env values.
- Unit-name derivation tests:
  - same normalized state dir gives same `ah-<hash>.service`
  - different state dirs give different names
  - unit name matches `src/systemd_unit.rs:16-18` detection pattern
- Config dir resolution tests:
  - `XDG_CONFIG_HOME` set
  - fallback to `$HOME/.config`
  - missing HOME/XDG reports a clear error

Flow tests with fake command runner/filesystem tempdir:

- Fresh install runs write -> daemon-reload -> reset-failed -> enable -> start.
- Repeated start with identical unit and inactive service does not rewrite but still can enable/start.
- Repeated start with accepting socket returns early and invokes no systemd commands.
- Changed unit content triggers daemon-reload and restart/start policy as specified.
- `reset-failed` failure is ignored and logged.
- `enable`/`start` failure returns a clear error and does not silently direct-spawn unless PM approves compatibility fallback.
- Existing generated file for same unit but different state-dir marker is fatal.

Migration tests with fake `systemctl show`:

- Active transient `ahd.service` for same state dir is stopped before new unit start.
- Active installed user-authored `ahd.service` is not deleted.
- New state-dir-derived unit avoids collision when legacy `ahd.service` is unrelated.

Enable/reboot semantics without real reboot:

- Assert `systemctl --user enable <unit>` is invoked and the unit has `[Install] WantedBy=default.target`.
- In an integration test environment with user systemd available, run `systemctl --user is-enabled <unit>` after install.
- Verify persistence by `systemctl --user daemon-reload`, `systemctl --user stop <unit>`, `systemctl --user start <unit>` without rewriting the file.
- Simulate user manager restart in CI only if the runner supports it; otherwise treat `is-enabled` plus installed file plus manual start after `daemon-reload` as the non-reboot proxy.
- Linger test should be command-construction/detection only by default; do not require CI privileges for `loginctl enable-linger`.

## Decisions For PM

1. Unit naming:
   - Recommendation: default to `ah-<state_dir_hash>.service`; keep `ahd.service` only as legacy migration input.
   - Tradeoff: solves multi-instance dogfood isolation, but user-facing systemctl commands become less obvious.

2. Linger management:
   - Recommendation: do not auto-enable linger from `ah start`; detect and warn/note. Add explicit linger command later if desired.
   - Tradeoff: avoids privileged host-level mutation, but enabled service may not survive logout or boot before login.

3. Start failure fallback:
   - Recommendation: if installed-unit systemctl commands fail, return a hard error instead of direct-spawning.
   - Tradeoff: exposes service configuration problems early, but changes current "systemd failed, direct spawn anyway" behavior.

4. `ah stop` scope:
   - Recommendation: stop current daemon and stop its unit, but do not disable or delete the unit. Put disable/delete behind explicit uninstall.
   - Tradeoff: preserves enable-on-next-login semantics, but users need a separate operation to fully uninstall persistence.

5. Running daemon reconciliation:
   - Recommendation: if the socket is healthy, `ah start` returns early and does not rewrite/restart the unit.
   - Tradeoff: maximum idempotence and low surprise, but changed env/path fields require an explicit reconcile/restart path.

6. Legacy `ahd.service` alias:
   - Recommendation: migrate transient `ahd.service` to hashed unit; do not auto-overwrite installed `ahd.service`.
   - Tradeoff: safe for user-authored units, but leaves possible legacy units that need manual cleanup.

## PM Decisions (pinned 2026-06-28)

All six decision points are pinned to a2's recommendations. Implementation follows these as fixed; do not re-ask.

1. **Unit naming** — ACCEPT `ah-<state_dir_hash>.service` default (12–16 hex SHA-256 prefix of normalized absolute state dir). Dogfood itself proves the multi-instance need (this PM runs on a per-state-dir instance). `ahd.service` kept only as legacy migration input. `ah start` must print the resolved unit name so `systemctl --user` commands are discoverable despite the hash.
2. **Linger** — ACCEPT: `ah start` does NOT auto-run `loginctl enable-linger`. Detect via `loginctl show-user <user> -p Linger`, and if disabled print a clear one-line note about boot-before-login/logout survival. A separate `ah service enable-linger` command is a future, out-of-scope follow-up.
3. **Start-failure fallback** — ACCEPT hard-error as the TARGET, with a bounded rollout fallback: (a) if `systemd_user_bootstrap_available()` is false → direct spawn, unchanged from today; (b) if user systemd IS available but unit-file write / `daemon-reload` fails recoverably → fall back to the EXISTING transient `systemd-run` path and log a clear warning (transitional, documented); (c) if `enable`/`start` of the installed unit fails → return a clear hard error, do NOT silently direct-spawn. Rationale: never mask the very persistence breakage GAP-2 targets, but don't regress a working bootstrap during rollout.
4. **`ah stop` scope** — ACCEPT: `ah stop` does RPC shutdown as today AND `systemctl --user stop <unit>` (to avoid `Restart=on-failure` ambiguity), but does NOT disable/delete. disable + file delete + daemon-reload live behind an explicit `ah service uninstall` (future), which only removes ah-generated files matching the expected path.
5. **Running-daemon reconciliation** — ACCEPT: healthy/accepting socket → `ah start` returns early, no rewrite/restart (cheap idempotent start preserved). Picking up changed env/path on a healthy daemon is deferred to a future explicit `ah service reconcile`.
6. **Legacy alias** — ACCEPT: migrate an active *transient* `ahd.service` for the same state dir (stop → install hashed unit → enable → start), but NEVER auto-overwrite or delete a user-authored *installed* `ahd.service`. Detect transient vs installed via `systemctl --user show <unit> -p FragmentPath -p LoadState -p ActiveState`; only stop the legacy unit when the target socket is stale/not-accepting and ownership is verifiable.

Hard implementation invariants (do not regress): systemd-safe escaping for `ExecStart`/`Environment=` (reject newline/NUL in env values); atomic unit-file write (temp + rename in same dir); reuse `ENV_PASSTHROUGH` from `src/provider/manifest.rs` (no duplicate list); preserve the existing recursion guard (`should_skip_systemd_bootstrap_for_cgroup`) and `ahd_reset_failed_is_best_effort`; agent/tmux scope binding must use the DETECTED current unit, not literal `ahd.service`.

## Review Amendments (a3, pinned 2026-06-28)

Design review by a3 added the following REQUIRED amendments before/within implementation (all accepted, verified against code):

- **A1 — `$` escaping (MAJOR). [CORRECTED 2026-06-29 by a4 empirical round-trip — original rule below was WRONG for `Environment=`.]**
  - **Original (incorrect) rule:** "systemd `Environment=` performs `$VAR`/`${VAR}` expansion; every literal `$` in an env value MUST be escaped to `$$`."
  - **Correction (physical evidence, supersedes the above):** `man systemd.exec` → `Environment=`, verbatim: *"Variable expansion is not performed inside the strings and the `$` character has no special meaning. Specifier expansion is performed."* a4 confirmed via `systemd-run --user` round-trip that `Environment=K="tok$$en"` yields the literal value `tok$$en` (double-dollar, **corrupted**), while `Environment=K="tok$en"` yields the correct `tok$en`. `$VAR` expansion happens only in **`ExecStart=` command lines** (`man systemd.service` → COMMAND LINES: "To pass a literal dollar sign, use `$$`"), NOT in `Environment=`.
  - **Resulting rule:** the two contexts share `%`→`%%`, quote/backslash/whitespace quoting, and control-char rejection, but differ ONLY on `$`:
    - `escape_systemd_exec_token` (for `ExecStart=`): escape `$`→`$$`.
    - `escape_systemd_env_value` (for ALL `Environment=`, incl. `AH_STATE_DIR` and passthrough): emit `$` **literally** (do NOT double).
  - Tests must assert true round-trip equality (env value in == value the process receives), not the doubled form.
- **A2 — recursion guard precise match (MAJOR).** `should_skip_systemd_bootstrap_for_cgroup(cgroup)` (`src/cli/start.rs:69-70`) currently returns true for ANY daemon-unit cgroup (`detect_current_service_unit_from_cgroup(cgroup).is_some()`). With hashed names this wrongly skips bootstrap for a DIFFERENT instance: starting `ah-B` from inside `ah-A`'s cgroup degrades `ah-B` to direct-spawn and breaks its persistence. Refactor the guard to take the TARGET `unit_name` and skip ONLY when the current cgroup unit name equals the target (`detect_current_service_unit_from_cgroup` already returns the name). Add tests: same unit → skip; different unit → do NOT skip.
- **A3 — real DBus/user-manager availability probe (MAJOR/precondition).** `systemd_user_bootstrap_available()` must actually probe user-manager/DBus connectivity (e.g. `systemctl --user is-system-running` succeeds / `XDG_RUNTIME_DIR` + DBus reachable), not merely that `systemd-run` exists. On no-DBus/headless, degrade safely to direct spawn.
- **A4 — drop `StandardOutput=append:` (precondition).** Omit `StandardOutput=append:`/`StandardError=append:` from the unit for older-systemd compatibility; rely on journald as authoritative (`journalctl --user -u <unit>`). If `ahd.log` is still wanted, the daemon writes it internally — out of scope for the unit file.
- **A5 — stale-unit GC (MAJOR, best-effort).** A deleted `state_dir` leaves an enabled `ah-<hash>.service` that systemd keeps trying to start at boot/login → log spam + cruft. On `ah start` (and on uninstall), best-effort scan `~/.config/systemd/user/ah-*.service`; if a unit's `AH_STATE_DIR` no longer exists on disk, `disable --now` it and delete the (ah-generated) file. GC must only touch ah-generated files (verify the generated marker comment), never user-authored units. May land as its own commit/slice if it grows.
- **A6 — thread derived unit name to scope binding + adapt tests (NIT→required).** Real call sites and tests pass literal `"ahd.service"` to the already-parameterized scope builders: `src/tmux/scope.rs:112,124,125` (`BindsTo=`/`PartOf=`) and many `Some("ahd.service")` sites in `src/sandbox/systemd.rs` (tests + `master_command`). After hashed naming, agent/master scopes MUST `BindsTo=`/`PartOf=` the DERIVED current unit or they bind to a non-existent unit. Thread the detected/derived unit name through these call sites and parameterize the affected tests. **Naming (A2/decision 1) and scope-binding (A6) are COUPLED — implement together; do not rename the unit without updating scope binding.**

Implementation may land in auditable slices, but any slice that changes the unit name must include A6 in the same slice to avoid breaking scope binding.
