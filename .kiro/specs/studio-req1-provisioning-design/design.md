# Studio Req1: ah Runtime Prerequisite Provisioning

## Status

Design draft for PM decision. This is not an implementation spec.

## Problem

The installer currently delivers the `ah` and `ahd` binaries, but a usable Windows/WSL runtime still depends on OS and distro prerequisites that users must install manually. `ah doctor` already diagnoses several of these requirements and prints `suggestion:` text, but it does not perform fixes.

The request is to bring first-party provisioning for ah's own runtime prerequisites into the installation/onboarding flow, while keeping provider CLIs and provider authentication outside ah's ownership.

## Current Evidence

- `ah doctor` is a diagnostic-only command in the CLI enum; there is no `setup`, `install`, `provision`, or `bootstrap` user-facing subcommand today. See `src/bin/ah.rs:120`-`123` and dispatch at `src/bin/ah.rs:300`.
- `cmd_doctor` runs checks, prints them, and fails if any check is `Fail`; it has no fix path. See `src/bin/ah.rs:859`-`867`.
- Doctor already models checks as `{ name, status, detail, suggestion }`, which is a good contract surface for mapping checks to fixes. See `src/cli/doctor.rs:14`-`20`.
- Doctor always checks `tmux` and `systemd-run` as required system binaries. See `src/cli/doctor.rs:35`-`47`.
- Doctor adds WSL onboarding checks when running inside WSL. See `src/cli/doctor.rs:22`-`31` and `src/cli/wsl.rs:141`-`212`.
- WSL guidance already states the operational dependency: systemd user session must be available, and fixing WSL systemd requires editing `/etc/wsl.conf` then running `wsl --shutdown` from PowerShell. See `src/cli/wsl.rs:6`.
- WSL tmux guidance already states that every agent runs in tmux and suggests `sudo apt update && sudo apt install -y tmux`. See `src/cli/wsl.rs:8`.
- `ah start` already preflights WSL systemd/tmux and refuses to start if missing. See `src/cli/wsl.rs:223`-`247`.
- ahd already has systemd user service bootstrap logic once systemd user manager exists. See `src/cli/service_bootstrap.rs:72`-`82` and `src/cli/service_bootstrap.rs:95`-`140`.
- Provider auth checks are warnings only and say "provider may need login before use"; this must remain non-provisioned. See `src/cli/doctor.rs:70`-`101`.

## Goals

1. Provide an ah-owned way to provision ah runtime prerequisites that are required before agents can run.
2. Reuse the existing doctor checks as the user-visible contract and as the source of prerequisite definitions.
3. Make provisioning idempotent and resumable, including reboot and WSL shutdown boundaries.
4. Clearly separate ah runtime prerequisites from provider CLI installation/authentication.
5. Give installers a stable non-interactive entry point for "install binaries, then provision runtime".

## Non-Goals

- Do not install provider CLIs such as `claude`, `codex`, `gemini`, or `antigravity`.
- Do not manage provider authentication, OAuth, tokens, or login flows.
- Do not mirror host niceties into WSL in v1, such as Windows proxy settings, timezone, shell profiles, fonts, or editor integration.
- Do not hide Windows admin or Linux sudo prompts. The tool may orchestrate and explain them, but it must not bypass OS permission models.
- Do not make `ah doctor` mutate system state by default.

## Recommended Interface

Use a new top-level command:

```text
ah setup [--check] [--fix] [--yes] [--json] [--resume]
```

Recommended semantics:

- `ah setup` defaults to an interactive plan: run prerequisite checks, show which fixes are available, ask before mutating.
- `ah setup --fix` executes applicable fixes with prompts unless `--yes` is present.
- `ah setup --check` is a read-only alias for the setup-specific prerequisite subset, useful for installers that do not want all doctor checks.
- `ah setup --json` emits machine-readable step status for installers.
- `ah setup --resume` continues after reboot or `wsl --shutdown`; this should also be the default behavior when a saved setup state exists.

Why not only `ah doctor --fix`:

- `doctor` is currently diagnostic and broad. It includes provider auth warnings, stale tmux cleanup, legacy repo state, daemon connectivity, and permission checks. Some are not safe or appropriate to auto-fix.
- `doctor --fix` can be added later as a narrow alias that invokes the same prerequisite engine for checks tagged `FixOwner::AhRuntime`, but the first user-facing provisioning verb should be explicit and unsurprising.

Why installer integration should call `ah setup --fix`:

- Installers can remain thin: install `ah`/`ahd`, then invoke the same CLI path users can rerun manually.
- This avoids duplicating OS-specific logic across MSI/winget/shell installers.
- The setup command can print or return `NeedsReboot` / `NeedsWslShutdown` states that the installer can surface directly.

## Prerequisite Model

Introduce a shared prerequisite definition layer used by both doctor and setup:

```rust
struct Prerequisite {
    id: &'static str,
    owner: FixOwner,
    check: CheckFn,
    fix: Option<FixFn>,
    privileges: PrivilegeClass,
    boundary: ExecutionBoundary,
    restart: RestartRequirement,
}

enum FixOwner {
    AhRuntime,
    UserProject,
    ProviderExternal,
    DiagnosticOnly,
}
```

The important design point is not this exact Rust shape; it is that doctor and setup stop maintaining separate truth. Doctor renders all checks and suggestions. Setup filters to `FixOwner::AhRuntime` and executes only prerequisites with explicit fix actions.

Initial ah-runtime prerequisite mapping:

| Doctor / setup check | Existing evidence | Fix owner | v1 fix action |
| --- | --- | --- | --- |
| `wsl` / Windows WSL2 feature | WSL detection exists in `src/cli/wsl.rs:47`-`63`; Windows host detection is missing today | AhRuntime | Later phase: Windows host helper enables WSL2 feature and installs/validates a distro |
| `wsl:systemd-user` | Check exists at `src/cli/wsl.rs:170`-`198`; guidance requires `/etc/wsl.conf` + `wsl --shutdown` | AhRuntime | Edit `/etc/wsl.conf` in distro, then require `wsl --shutdown` from Windows host |
| `wsl:tmux` / `binary:tmux` | Checks at `src/cli/wsl.rs:200`-`209` and `src/cli/doctor.rs:35`-`47` | AhRuntime | Install `tmux` through supported distro package manager, initially apt |
| `binary:systemd-run` | Check at `src/cli/doctor.rs:35`-`47` | AhRuntime | Usually fixed by enabling WSL systemd or installing systemd package; do not separately install systemd until distro support is defined |
| daemon systemd user service | Bootstrap exists at `src/cli/service_bootstrap.rs:95`-`140` | AhRuntime | Existing `ah start` bootstrap remains; setup may validate but does not need to start a project daemon |
| provider auth warnings | `src/cli/doctor.rs:70`-`101` | ProviderExternal | No fix; keep suggestion only |
| stale tmux / legacy state cleanup | `src/cli/doctor.rs:120`-`183` | DiagnosticOnly or UserProject | No automatic v1 fix; potentially separate cleanup command later |
| basic network reachability | Not currently a concrete doctor check | AhRuntime or DiagnosticOnly | Add explicit check before adding a fix; likely diagnostic-only in v1 |

## Provisioning Flow

### Interactive user flow

1. User runs `ah setup`.
2. Setup runs the prerequisite checks and prints a plan grouped by boundary:
   - Windows host admin steps
   - Windows host user steps
   - WSL distro sudo steps
   - WSL distro user steps
3. Setup asks before each privilege boundary.
4. Setup executes idempotent fixes.
5. If a step requires reboot or WSL shutdown, setup writes a resume state and exits with a clear status:
   - `NeedsWindowsReboot`
   - `NeedsWslShutdown`
   - `NeedsDistroReopen`
6. User or installer resumes with `ah setup --resume`, or simply reruns `ah setup`.

### Installer flow

1. Installer installs `ah` and `ahd`.
2. Installer invokes `ah setup --fix --json`.
3. If setup returns `NeedsWindowsReboot`, installer surfaces a reboot-required result and exits successfully with "provisioning incomplete; rerun after reboot".
4. After reboot, installer or user reruns `ah setup --resume`.
5. If setup returns `NeedsWslShutdown`, the Windows helper runs `wsl --shutdown` or tells the user to do so; the next WSL launch resumes.

## Idempotency and Resumability

Every fix must be a state transition from observed state to desired state, not a blind command.

### WSL2 feature and distro

Desired state:

- Windows optional features needed for WSL2 are enabled.
- A supported Linux distro is installed and runnable.
- `wsl.exe --status` and `wsl.exe -l -v` report usable state.

Idempotency:

- If features are already enabled, report `Pass`.
- If the distro already exists, do not reinstall or overwrite it.
- If feature enablement returns reboot required, persist setup state and stop.

Resume:

- Store setup state under ah's state directory or a Windows-side equivalent with the last completed step and target distro identity.
- After reboot, rerun checks instead of trusting stored state.

### WSL systemd enablement

Desired state inside distro:

```ini
[boot]
systemd=true
```

Idempotency:

- Parse `/etc/wsl.conf` if present.
- Preserve unrelated sections and keys.
- Add or update only `[boot].systemd`.
- If already true and `systemctl --user is-system-running` returns an accepted state, report `Pass`.
- If config is updated, mark `NeedsWslShutdown`.

Resume:

- After `wsl --shutdown` and distro reopen, rerun `wsl:systemd-user` check.
- Do not assume success from file content alone; the current probe requires `XDG_RUNTIME_DIR`, `systemd-run`, `systemctl`, and accepted `systemctl --user is-system-running` output (`src/cli/wsl.rs:263`-`303`).

### tmux installation

Desired state:

- `tmux` is on PATH and `tmux -V` succeeds, matching current check behavior (`src/cli/wsl.rs:305`-`324`).

Idempotency:

- If `tmux` exists, do nothing.
- If distro package manager is apt, run `sudo apt-get update` only when needed for install freshness policy, then `sudo apt-get install -y tmux`.
- If non-apt distro is detected, report `UnsupportedPackageManager` with manual command guidance until support is added.

Resume:

- Rerun the check after install.
- A failed sudo or package-manager command leaves no custom ah state; rerunning is safe.

### Network reachability

Desired state:

- Basic outbound network is usable enough for provider CLIs and package installs.

Design note:

- This is not currently an explicit doctor check. Do not invent a fixer before defining a check.
- v1 should add a diagnostic-only check such as DNS resolution plus HTTPS connect to a stable endpoint, with opt-out for offline environments.
- Do not mutate proxy settings in v1.

## Permission Boundaries

| Step | Boundary | Privilege |
| --- | --- | --- |
| Enable WSL / VM platform optional features | Windows host | Admin |
| Install Linux distro | Windows host | Usually user, may require Store/winget policy |
| Run `wsl --shutdown` | Windows host | User |
| Edit `/etc/wsl.conf` | WSL distro | sudo/root |
| Install `tmux` via apt | WSL distro | sudo/root |
| Validate `systemctl --user` | WSL distro | User |
| Bootstrap ahd user service | WSL distro | User systemd manager |
| Provider CLI install/login | External | Out of scope |

Cross-boundary coordination:

- Linux-side `ah setup` can handle distro-local fixes (`/etc/wsl.conf`, `tmux`) and can print an exact PowerShell command for `wsl --shutdown`.
- Windows-side installer or helper can handle host-level WSL feature/distro work and can invoke distro-side `ah setup --resume` through `wsl.exe -d <distro> -- ah setup --resume`.
- The shared prerequisite engine should distinguish host checks from distro checks so a Linux binary does not pretend it can enable Windows features by itself.

## Relationship to `ah doctor`

Doctor should remain read-only by default.

Recommended relationship:

- Move doctor checks toward shared prerequisite definitions.
- Add metadata to checks:
  - `id`
  - `owner`
  - `fix_available`
  - `manual_suggestion`
  - `requires_privilege`
  - `restart_requirement`
- `ah doctor` renders diagnostics as today.
- `ah doctor --json` can expose the metadata for external tooling.
- Optional later: `ah doctor --fix` invokes the same engine as `ah setup --fix`, but only for checks whose owner is `AhRuntime`.

Do not auto-fix every doctor warning:

- Provider auth warnings remain user/provider owned.
- Legacy repo state and stale tmux cleanup are not OS prerequisites and can be destructive if automated.
- Daemon not responding may mean "not started yet"; setup should not start arbitrary project daemons unless the user explicitly runs project startup.

## Phasing

### Phase 0: Shared prerequisite contract

- Define stable prerequisite IDs for existing doctor checks.
- Tag checks by fix ownership.
- Add setup plan rendering with no mutation.
- Add JSON output so installers can consume the same plan.

Exit gate:

- `ah setup --check` and `ah doctor` agree on statuses for tmux/systemd WSL prerequisites.
- Provider auth warnings are visible in doctor but excluded from setup fix plan.

### Phase 1: Distro-local fixes

Scope:

- `ah setup --fix` inside WSL can:
  - install `tmux` for apt-based distros;
  - edit `/etc/wsl.conf` to enable systemd;
  - stop with `NeedsWslShutdown` and clear instructions.

Why first:

- It covers the most common handoff pain with the least Windows-host complexity.
- It exercises idempotency and resume without requiring Windows admin.

Exit gate:

- Fresh Ubuntu WSL distro: `ah setup --fix` installs tmux, updates `/etc/wsl.conf`, requests shutdown, then after reopen passes `wsl:systemd-user` and `wsl:tmux`.
- Re-running `ah setup --fix` is a no-op when already provisioned.
- Non-apt distro reports unsupported package manager with manual guidance, not a partial mutation.

### Phase 2: Windows host helper for WSL feature and distro

Scope:

- Add a Windows-side helper path for:
  - detecting WSL optional features;
  - enabling WSL2 prerequisites with admin elevation;
  - installing or selecting a supported distro;
  - surfacing reboot-required state;
  - invoking distro-side setup after reboot.

Why later:

- It crosses admin, reboot, Windows Store/winget policy, and distro selection boundaries.
- It likely belongs partly in installer packaging rather than only the Linux `ah` binary.

Exit gate:

- Clean Windows machine with no WSL can run installer provisioning to a deterministic "reboot required" or "distro ready" state.
- After reboot, setup resumes without losing which distro was selected.

### Phase 3: Network diagnostics and expanded distro support

Scope:

- Add basic network diagnostic checks.
- Add package-manager support beyond apt if PM chooses.
- Consider optional host niceties only if Studio makes them product requirements.

Exit gate:

- Network check is useful but non-invasive.
- Additional package managers have real distro tests.

## Failure Modes

- Windows feature enablement requires reboot: stop immediately with `NeedsWindowsReboot`; do not continue into distro fixes that may fail due to stale host state.
- `/etc/wsl.conf` changed: stop with `NeedsWslShutdown`; do not claim systemd is usable until the user reopens WSL and the user-manager probe passes.
- sudo unavailable or denied: report `PermissionDenied` with the exact command that would have run.
- Package manager lock held: report retryable failure.
- Unsupported distro/package manager: do not guess; emit manual guidance and retain failed setup status.
- Existing `/etc/wsl.conf` is malformed: do not overwrite blindly; write a backup only with user confirmation and report the parse issue.
- Provider CLI missing or unauthenticated: show doctor warning only; setup must not try to install or log in.
- Network blocked: report diagnostic warning; do not change proxy settings in v1.

## Open Questions for PM

1. Should the primary command be `ah setup`, or should installer UX hide it and expose only installer-level provisioning?
2. Which Windows packaging surfaces must call provisioning: MSI, winget, shell script, or all of them?
3. What distro matrix is first-party supported in Phase 1: Ubuntu only, Debian too, or any apt-based distro?
4. Should setup install a distro if none exists, or only configure a distro the user has already selected?
5. Where should resume state live on Windows before WSL exists?
6. Should `ah setup --fix --yes` be allowed to run sudo/package-manager commands non-interactively, or should distro-local privileged actions always prompt?
7. What network endpoint is acceptable for a built-in reachability check without creating privacy or availability concerns?
8. Should stale tmux cleanup ever be auto-fixable, or remain a separate explicit cleanup command?

## Recommendation

Build Phase 0 and Phase 1 first. They align directly with the existing doctor contract, avoid Windows admin and reboot complexity for the first delivery, and cover the immediate user-facing failures that block `ah start` inside WSL. Keep provider CLI/auth out of scope. Add Phase 2 only after PM decides the Windows installer should own WSL feature/distro installation rather than merely guide it.
