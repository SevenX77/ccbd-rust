# Studio Req1 Runtime Provisioning Implementation Spec

## Goal

Deliver first-party provisioning for ah's own runtime prerequisites, with a clear split between Windows host bootstrap and WSL distro-local fixes. The near-term implementation is Ubuntu/Debian apt-first: Phase 1 installs `tmux` and enables WSL systemd only in apt-based WSL distros; non-apt distros return `unsupported` plus exact manual guidance. Repeated runs must be safe, boundary exits must tell users exactly how to resume, and provider CLIs/authentication stay outside ah ownership.

This spec deepens `design.md` into implementation-level work. It does not change code by itself.

## Decisions

### Primary Interface

Use a new top-level Linux/WSL command:

```text
ah setup [--check] [--fix] [--yes] [--json] [--resume]
```

Semantics:

- `ah setup` is read-only: it runs checks and renders an interactive plan, but never mutates machine state.
- `ah setup --check` is read-only and exits nonzero when ah-runtime prerequisites are not satisfied.
- `ah setup --fix` is the only Linux-side setup mode that may apply supported ah-runtime fixes. Without `--yes`, it prompts before sudo/package-manager/file edits.
- `ah setup --fix --yes` may run non-interactively inside the distro, but only for already-supported package managers and deterministic file edits.
- `ah setup --json` emits structured step statuses and is stable for installers.
- `ah setup --resume` resumes any saved distro-local setup state; rerunning plain `ah setup` must also detect and resume saved state.

`ah doctor` stays read-only. Do not add `doctor --fix` in this work. Doctor and setup will share prerequisite definitions, but mutation lives behind `ah setup`.

Reasoning:

- `doctor` includes provider auth warnings, daemon liveness, legacy repo state, tmux orphan diagnostics, and project permission checks. Those are broader than runtime provisioning and should not become implicitly mutating.
- `ah setup` gives users and installers an explicit provisioning verb.
- Studio and other installers can call `ah setup --fix --json` after binaries exist inside WSL.

### Windows Bootstrap Boundary

Windows host prerequisites are owned by the Windows installer/helper, not the Linux `ah` binary.

Reasoning:

- Before WSL or a distro exists, the Linux `ah` binary cannot run.
- Enabling Windows optional features is a host-admin operation with reboot semantics. It must be handled from Windows PowerShell/exe/MSI/winget context.
- The installer can later invoke distro-local `ah setup --resume --fix --json` through `wsl.exe -d <distro> -- ah setup ...`.

Future Phase 2 boundary:

- A Windows host provisioning helper in the release/installer surface, e.g. `scripts/windows/provision-ah-wsl.ps1` or an equivalent cargo-dist installer hook, is the expected Phase 2 shape, but it requires the separate Phase 2 spec and PM packaging decision below.
- That helper would be the Phase 2 entry point for WSL feature enablement, distro selection/install, reboot resume, and invoking distro-local `ah setup`.
- The Linux `ah setup` command is still the canonical distro-local fixer and JSON contract.

## Shared Contract

Add a shared prerequisite layer used by `ah doctor` and `ah setup`.

Candidate files:

- `src/cli/prereq.rs`: shared prerequisite IDs, status model, owner, privilege, restart requirement, and plan rendering DTOs.
- `src/cli/setup.rs`: setup command orchestration and distro-local fix implementations.
- `src/cli/doctor.rs`: migrate existing `DoctorCheck` production to shared definitions while preserving current text output.
- `src/cli/wsl.rs`: keep WSL detection/probes here; expose probe results to prereq/setup.
- `src/bin/ah.rs`: add `Cmd::Setup` and flags.

Required data model:

```rust
struct RuntimePrerequisite {
    id: &'static str,
    owner: FixOwner,
    status: PrereqStatus,
    fix: Option<FixAction>,
    privilege: PrivilegeClass,
    boundary: ExecutionBoundary,
    restart: RestartRequirement,
}

enum FixOwner {
    AhRuntime,
    ProviderExternal,
    UserProject,
    DiagnosticOnly,
}

enum RestartRequirement {
    None,
    NeedsWslShutdown,
    NeedsWindowsReboot,
    NeedsDistroReopen,
    NeedsDistroFirstLaunch,
}
```

The exact Rust shape can vary, but the JSON output must expose equivalent fields:

Top-level JSON envelope, used by every phase and every exit status:

- `schema_version`: integer, initially `1`
- `operation_id`: stable UUID/string for the current setup attempt or resumed operation
- `overall_status`: `pass | warn | fail | fixed | needs_wsl_shutdown | needs_windows_reboot | needs_distro_reopen | needs_distro_first_launch | unsupported | permission_denied`
- `phase`: `phase0_check | phase1_distro | phase2_windows_host`
- `selected_distro`: WSL distro name when known, else `null`
- `next_action`: object with `kind`, `message`, and `command` fields; `command` is the exact next user/installer command when action is required
- `resume_command`: exact command to continue, or `null` when no boundary/resume is pending
- `steps`: array of step objects

Each `steps[]` object must expose:

- `id`
- `status`: `pass | warn | fail | skipped | fixed | needs_wsl_shutdown | needs_windows_reboot | needs_distro_reopen | needs_distro_first_launch | unsupported | permission_denied`
- `owner`
- `fix_available`
- `privilege`
- `boundary`
- `restart`
- `detail`
- `suggestion`
- `resume_token` when setup stops at an external boundary

Exit codes:

- `0`: all selected ah-runtime prerequisites pass, or fixes succeeded.
- `1`: checks failed and no mutation was requested, or a supported fix failed.
- `2`: unsupported environment or unsupported package manager.
- `10`: `NeedsWslShutdown`.
- `11`: `NeedsWindowsReboot`.
- `12`: `NeedsDistroReopen`, `NeedsDistroFirstLaunch`, or distro installation complete but not yet bootstrapped.

Boundary output contract:

Every boundary exit must print the same facts in human text and JSON before exiting:

1. What was just changed, including files, packages, feature names, or distro names. Example: `Updated /etc/wsl.conf to set [boot] systemd=true`.
2. The exact next command the user or installer must run. Examples: `powershell.exe -NoProfile -Command "wsl --shutdown"` followed by `ah setup --resume --check`, or `reboot Windows, then rerun the Studio installer`.
3. How to inspect current status without mutation: `ah setup --check` or `ah setup --check --json`.

When suggesting `wsl --shutdown`, the message must state that it stops all running WSL distros, not only the selected distro. Phase 1 uses `wsl --shutdown` because enabling systemd is a WSL VM boundary and the command is the documented, reliable way to restart WSL globally; the prompt must offer the impact explicitly. If a future Windows helper can safely use `wsl --terminate <distro>` for a narrower flow, that must be specified and tested separately.

## Doctor Mapping

Doctor remains broad and read-only. Setup filters to `FixOwner::AhRuntime` and `fix_available=true`.

`run_doctor(client, project_dir: Option<&Path>)` now gates project-scoped checks on `project_dir` (`src/cli/doctor.rs:22-39`): `legacy repo state`, `permissions:cwd`, and `permissions:.ccb` are absent when doctor runs without an explicit project directory. Setup must keep the same distinction and must not invent an ambient CWD project.

| Doctor check | Current source | Owner | Setup behavior |
| --- | --- | --- | --- |
| `binary:tmux` | `src/cli/doctor.rs:42` | AhRuntime | In WSL, same fix as `wsl:tmux`: install `tmux` via supported package manager. Outside WSL, fail with manual install guidance in Phase 1. |
| `binary:systemd-run` | `src/cli/doctor.rs:42` | AhRuntime | In WSL, resolved by enabling WSL systemd and reopening distro. Do not separately install systemd in Phase 1. Outside WSL, diagnostic/manual only. |
| `wsl` | `src/cli/wsl.rs:163` | AhRuntime | Pass-only distro check. Windows host helper owns WSL feature/distro creation before Linux `ah` can run. |
| `wsl:systemd-user` | `src/cli/wsl.rs:193` | AhRuntime | Edit `/etc/wsl.conf` to `[boot] systemd=true`, then stop with `NeedsWslShutdown`; pass only after user-manager probe succeeds. |
| `wsl:tmux` | `src/cli/wsl.rs:200` | AhRuntime | Install `tmux` in apt-based distros; unsupported package managers get deterministic manual guidance. |
| `daemon` | `src/cli/doctor.rs:56` | DiagnosticOnly | No setup fix. `ah setup` must not start project daemons. Existing `ah start` owns daemon bootstrap. |
| `tmux server orphans` | `src/cli/doctor.rs:127` | DiagnosticOnly | No setup fix. Keep suggestion or future explicit cleanup command. |
| `tmux legacy shared session` | `src/cli/doctor.rs:159` | DiagnosticOnly | No setup fix. Potentially future explicit cleanup command. |
| `legacy repo state` | `src/cli/doctor.rs:22-39`, `src/cli/doctor.rs:171` | UserProject | No setup fix. Present only when `project_dir` is supplied. Deleting repo files is user-owned. |
| `provider:*` | `src/cli/doctor.rs:77` | ProviderExternal | No setup fix. Do not install provider CLIs or manage auth. |
| `provider:home` | `src/cli/doctor.rs:77` | ProviderExternal | No setup fix. User environment issue. |
| `permissions:cwd` | `src/cli/doctor.rs:22-39`, `src/cli/doctor.rs:111` | UserProject | No setup fix. Present only when `project_dir` is supplied. Project permissions are user-owned. |
| `permissions:.ccb` | `src/cli/doctor.rs:22-39`, `src/cli/doctor.rs:111` | UserProject | No setup fix. Present only when `project_dir` is supplied. Project state dir creation remains daemon/project behavior. |

Setup's ah-runtime set for Phase 1 is exactly:

- `wsl:systemd-user`
- `wsl:tmux`
- `binary:tmux` when WSL is detected
- `binary:systemd-run` when WSL is detected

Phase 2 adds Windows host-only checks:

- `windows:wsl-feature`
- `windows:virtual-machine-platform`
- `windows:wsl-version`
- `windows:wsl-distro`

These host checks must be visible in installer JSON and can later be mirrored into `ah setup --json` when invoked from Windows-native packaging, but they are not executed by the Linux `ah` binary.

## State And Resume

### Distro-Local State

Store distro-local setup state under the existing ah state root:

```text
<state_dir>/setup/state.json
```

The `state_dir` must come from the same neutral state layout used by no-config commands unless `--config` is explicitly supplied. Setup is host/runtime provisioning, not project state.

State fields:

- `schema_version`
- `operation_id`
- `phase`
- `boundary`: `distro`
- `distro_name` from `WSL_DISTRO_NAME` when present
- `last_completed_step`
- `pending_restart`: `none | wsl_shutdown | distro_reopen`
- `wsl_conf_hash_before` and `wsl_conf_hash_after` when edited
- `created_at`, `updated_at`
- `last_error`

Resume rule:

- Stored state is advisory. On every run, re-run probes and derive current state from the machine.
- If file state says `NeedsWslShutdown` but `wsl:systemd-user` now passes, clear the pending state.
- If file state says a package install was attempted, still re-run `tmux -V`.

### Windows Host State

Store Windows host setup state in the installing user's profile:

```text
%LOCALAPPDATA%\ah\setup-state.json
```

If an all-users MSI later requires machine-wide resume state, use:

```text
%ProgramData%\ah\setup-state.json
```

State fields:

- `schema_version`
- `operation_id`
- `boundary`: `windows-host`
- `selected_distro`
- `requested_default_wsl_version`
- `feature_steps`: map of optional feature names to observed/requested state
- `pending_restart`: `none | windows_reboot | distro_install | distro_first_launch | distro_setup`
- `last_completed_step`
- `created_at`, `updated_at`
- `last_error`

Resume after reboot:

1. Windows installer/helper starts and reads `%LOCALAPPDATA%\ah\setup-state.json`.
2. It re-runs host probes: optional features, `wsl.exe --status`, `wsl.exe -l -v`.
3. If reboot completed feature enablement, it continues to distro selection/install.
4. If distro exists but first-launch user initialization has not completed, it stops at `NeedsDistroFirstLaunch` with exact instructions to open the distro once and create the Linux username/password.
5. If distro exists and ah binaries are installed inside it, it invokes:

   ```powershell
   wsl.exe -d <distro> -- ah setup --resume --fix --json
   ```

6. If distro-local setup returns `NeedsWslShutdown`, the Windows helper may run `wsl.exe --shutdown` after user confirmation, then relaunch the distro-side resume.

Never rely only on the state file to skip checks. It exists to know what the previous operation intended, not to prove the current machine state.

## Phase 0: Shared Contract And Read-Only Planning

Scope:

- Add `Cmd::Setup` and flags in `src/bin/ah.rs`.
- Add `src/cli/prereq.rs` and `src/cli/setup.rs`.
- Move or wrap doctor-owned checks into stable prerequisite IDs without changing default `ah doctor` text.
- Implement `ah setup --check` and `ah setup --json` as read-only plan renderers.
- Add JSON tests for stable IDs and owner/fix metadata.

Files:

- `src/bin/ah.rs`
- `src/cli/mod.rs`
- `src/cli/doctor.rs`
- `src/cli/wsl.rs`
- `src/cli/prereq.rs`
- `src/cli/setup.rs`

Exit gates:

- [auto-CI] `ah doctor` output remains compatible for existing checks.
- [auto-CI] `ah setup --check --json` always emits the top-level envelope: `schema_version`, `operation_id`, `overall_status`, `phase`, `selected_distro`, `next_action`, `resume_command`, and `steps[]`.
- [auto-CI] `ah setup --check --json` reports `wsl:systemd-user` and `wsl:tmux` with the same pass/fail statuses as doctor on injected probes.
- [auto-CI] Provider warnings appear in doctor but are absent from the setup fix plan.
- [auto-CI] Unit tests cover JSON schema, check ownership, and non-mutating default/`--check`.
- [doctor-selfcheck] `ah setup --check --json` exit code is the acceptance substitute for real machine state in Phase 0.
- [real-machine] No Windows machine is required for Phase 0; real WSL runs are optional smoke only.

## Phase 1: Distro-Local Fixes

Scope:

- Implement `ah setup --fix` inside WSL.
- Add apt-based tmux install:

  ```text
  sudo apt-get update
  sudo apt-get install -y tmux
  ```

- Implement `/etc/wsl.conf` parser/writer for `[boot] systemd=true`.
- Preserve unrelated sections and keys in `/etc/wsl.conf`.
- Normal systemd-enable edits are idempotent in-place updates and do not create a backup; `ah setup` has no automatic rollback/undo for this change.
- Backup malformed or overwritten config only after user confirmation:

  ```text
  /etc/wsl.conf.ah-backup.<timestamp>
  ```

- Stop with `NeedsWslShutdown` after editing `/etc/wsl.conf`.
- Add resume state under `<state_dir>/setup/state.json`.

Package-manager policy:

- Phase 1 supports Ubuntu/Debian apt-based distros only.
- Detect apt by `apt-get` on PATH and `/etc/os-release` ID/ID_LIKE containing `ubuntu` or `debian`.
- Non-apt distros return `UnsupportedPackageManager` with manual guidance; do not run unknown commands.

Privilege policy:

- Reading probes: no privilege.
- Installing tmux: distro sudo/root.
- Editing `/etc/wsl.conf`: distro sudo/root.
- `--yes` allows non-interactive sudo only if the user's sudo policy allows it. Do not ask for or store passwords.

Failure modes:

- `sudo` denied: `PermissionDenied`, no further mutation.
- apt lock held: retryable failure with exact command and apt stderr.
- tmux install exits nonzero: failure, rerun safe.
- `/etc/wsl.conf` malformed: stop before write unless user approves backup+rewrite.
- systemd file changed: `NeedsWslShutdown`, do not claim success until the user-manager probe passes after reopen.
- `NeedsWslShutdown` text must state that `wsl --shutdown` terminates all running distros and must offer the exact command plus `ah setup --check`/`--json` status check.

Exit gates:

- [auto-CI] Unit tests cover wsl.conf parse/write idempotency, preservation of unrelated sections, and backup decision logic.
- [auto-CI] Unit tests cover apt plan generation and failure classification with fake command runner.
- [auto-CI] Non-apt distro fixtures return `unsupported` with manual guidance and no mutation plan.
- [auto-CI] Boundary-output tests verify `NeedsWslShutdown` includes what changed, exact next command, `wsl --shutdown` all-distro impact text, and `ah setup --check` status command.
- [doctor-selfcheck] On any real or fake supported distro after setup, `ah setup --check --json` exit code `0` is the machine-readable acceptance signal.
- [real-machine] Fresh Ubuntu WSL: `ah setup --fix` installs tmux, writes systemd config, returns `NeedsWslShutdown`.
- [real-machine] After `wsl --shutdown` and reopen, `ah setup --resume --check` passes `wsl:systemd-user` and `wsl:tmux`.
- [real-machine] Rerunning `ah setup --fix` after success is a no-op.

## Phase 2: Windows Host Helper Gate

Phase 2 is not ready for implementation in this spec. Before coding, PM must approve the Windows packaging surface and an independent Phase 2 implementation spec. That spec must choose the delivery vehicle (`MSI`, `winget`, standalone `.exe`, PowerShell script, or cargo-dist hook) and decide whether reboot resume is automatic (`RunOnce`/scheduled task/installer continuation) or explicitly user-triggered.

The content below is a required design backlog for that future spec, not an implementation-ready task list for the current Phase 0/1 work.

Required Phase 2 spec content:

- Add a Windows-side installer/helper path that can run before WSL exists.
- Detect and enable required host features:
  - Microsoft-Windows-Subsystem-Linux
  - VirtualMachinePlatform
- Ensure WSL2 is the target default:

  ```powershell
  wsl.exe --set-default-version 2
  ```

- Detect installed distros with:

  ```powershell
  wsl.exe -l -v
  ```

- Install or select a supported distro. Phase 2 default is Ubuntu unless the installer is passed an explicit distro.
- Persist `%LOCALAPPDATA%\ah\setup-state.json`.
- Stop with `NeedsWindowsReboot` immediately when DISM reports reboot required.
- After reboot, resume host checks, then invoke distro-local setup.
- Define how the helper installs or updates `ah` inside the selected distro before invoking `wsl.exe -d <distro> -- ah setup --resume --fix --json`. A newly installed distro will not have `ah` on `PATH`; the Phase 2 spec must pin install path, target Linux user, PATH update strategy, version verification, upgrade/downgrade policy, and failure/resume behavior. Acceptable approaches include invoking the ah release shell installer inside the distro or copying a release artifact into a deterministic path, but the spec must choose one.
- Model first-launch user initialization explicitly. A new WSL distro may require the user to open it once and create a Linux username/password before `sudo` or `ah setup` can work. The Phase 2 spec must add a `NeedsDistroFirstLaunch`/`UserInit` state, probe it with commands such as `wsl.exe -d <distro> -- id -un` and sudo capability checks, and treat first-launch username/password creation as an external boundary with exact resume instructions.
- Capture the no-revert policy. Enabling WSL features, installing a distro, and enabling systemd in `/etc/wsl.conf` have no general `ah setup --undo` path. The helper may provide manual rollback guidance later, but Phase 2 must not promise automatic rollback.

Windows commands:

- Probe feature state with PowerShell/DISM:

  ```powershell
  Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Windows-Subsystem-Linux
  Get-WindowsOptionalFeature -Online -FeatureName VirtualMachinePlatform
  ```

- Enable missing features from an elevated process:

  ```powershell
  dism.exe /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart
  dism.exe /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart
  ```

- Install distro through `wsl.exe --install -d Ubuntu` when available. If Windows policy blocks Store-backed install, return `UnsupportedDistroInstall` with manual instructions instead of guessing winget/Store behavior.

Elevation policy:

- The helper detects whether it is elevated before DISM.
- If not elevated, it relaunches itself elevated with the same arguments and operation ID.
- It does not bypass UAC or credentials.
- User-facing and JSON output must make admin requirements explicit.

Reboot policy:

- If any feature enablement reports reboot required, write state and exit `NeedsWindowsReboot`.
- Do not attempt distro install before reboot if the feature state is pending.
- After reboot, re-probe feature state before continuing.

WSL shutdown policy:

- If distro-local `ah setup` returns `NeedsWslShutdown`, Windows helper asks before running:

  ```powershell
  wsl.exe --shutdown
  ```

- After shutdown, it restarts the selected distro and reruns distro-local `ah setup --resume --fix --json`.
- The confirmation text must say `wsl --shutdown` terminates all running WSL distros. A narrower `wsl.exe --terminate <distro>` flow requires separate proof that systemd enablement and distro-local setup resume correctly.

Future Phase 2 exit gates:

- [real-machine] Clean Windows machine with no WSL reaches one of:
  - deterministic `NeedsWindowsReboot` with resume state written;
  - WSL2 feature enabled, distro selected/installed, distro-local setup invoked.
- [real-machine] After reboot, helper resumes from `%LOCALAPPDATA%\ah\setup-state.json` and does not repeat completed feature enablement.
- [real-machine] Fresh distro first launch is detected as `NeedsDistroFirstLaunch`; after user initialization, helper resumes and can install/verify `ah` inside the distro.
- [real-machine] Repeated runs skip already-enabled features and already-installed distro.
- [real-machine] If distro install is unsupported by host policy, helper exits with actionable manual guidance and does not corrupt state.
- [auto-CI] PowerShell unit tests or golden tests cover command planning, state serialization, resume decision logic, first-launch state classification, and in-distro ah install command construction.
- [doctor-selfcheck] Once distro-local ah is installed, `wsl.exe -d <distro> -- ah setup --check --json` exit code is the acceptance substitute for distro-local readiness.

Verification boundary:

- Actual DISM, reboot, Store/WSL install, first-launch user creation, and distro launch require a real Windows/WSL test machine. Do not claim Linux developer machines can validate this end to end.

## Phase 3: Network Diagnostics And Expansion

Scope:

- Add diagnostic-only network checks after PM approves endpoint policy.
- Add additional package managers only with real distro test coverage.
- Consider host niceties only if product requirements demand them.

Network policy:

- Phase 3 may add DNS and HTTPS reachability checks.
- Do not mutate Windows proxy, WSL mirrored networking, timezone, locale, shell profiles, fonts, or editor integration.
- Network failures are warnings unless a specific ah-owned runtime download is being attempted.

Exit gates:

- [auto-CI] Network check rendering is useful and non-invasive in fake probe fixtures.
- [auto-CI] Additional package-manager support has unit tests.
- [real-machine] Additional package-manager support has at least one real distro verification path.

## Idempotency Rules

Every fix is a state transition from observed state to desired state.

- If WSL features are enabled, skip DISM.
- If reboot is pending, stop and resume later.
- If distro exists, do not reinstall it.
- If `/etc/wsl.conf` already has `[boot] systemd=true`, do not rewrite it.
- If `/etc/wsl.conf` needs a normal systemd-enable edit, do the minimal merge in place; do not promise an `ah setup --undo` path.
- If tmux exists and `tmux -V` succeeds, do not run apt.
- If `ah setup --resume` finds no pending state, run normal checks.
- If saved state and observed state disagree, observed state wins and state is updated.

No fix may depend on elapsed time, a blind "last step succeeded" marker, or a deleted temp directory.

## Permission Boundaries

| Step | Runner | Privilege | Notes |
| --- | --- | --- | --- |
| Enable WSL feature | Windows helper | Windows admin | DISM/UAC; stops at reboot boundary. |
| Enable VirtualMachinePlatform | Windows helper | Windows admin | DISM/UAC; stops at reboot boundary. |
| Set WSL default version | Windows helper | Windows user | Requires `wsl.exe`; no admin after features enabled. |
| Install/select distro | Windows helper | Windows user/admin varies | Store/winget policy dependent; must be resumable. |
| Run `wsl --shutdown` | Windows helper/user | Windows user | Requires user confirmation unless installer explicitly requested noninteractive. |
| Install tmux | `ah setup` in WSL | distro sudo/root | apt only in Phase 1. |
| Edit `/etc/wsl.conf` | `ah setup` in WSL | distro sudo/root | preserve existing file, backup on risky rewrite. |
| Probe systemd user manager | `ah setup`/doctor in WSL | distro user | uses existing `systemctl --user is-system-running` logic. |
| Provider CLI/auth | external | external | out of scope. |

## Non-Goals

- Do not install `claude`, `codex`, `gemini`, `antigravity`, or any provider CLI.
- Do not manage OAuth, tokens, provider login, or subscription state.
- Do not mutate host proxy, timezone, locale, fonts, editor integration, shell profiles, or Windows networking policy in this work.
- Do not auto-delete legacy repo state, stale tmux sessions, or user project files.
- Do not start arbitrary project daemons from setup.
- Do not bypass UAC, sudo, OAuth, or Windows Store/enterprise policy.

## Test Strategy

Automated in normal Linux CI:

- Prerequisite model and JSON schema tests.
- Doctor/setup mapping tests with fake probes.
- WSL detection classification tests using existing `src/cli/wsl.rs` fixture style.
- `/etc/wsl.conf` parse, merge, backup decision, and idempotent write tests using temp files.
- Apt command planning and error classification with fake runner.
- Resume state serialization and observed-state-wins reconciliation tests.

Future Phase 2 automated on Windows CI without reboot:

- PowerShell/helper plan rendering for feature states using mocked command output.
- JSON state read/write golden tests.
- Command construction tests for DISM, `wsl --set-default-version`, `wsl -l -v`, and distro-local `ah setup` invocation.

Future Phase 2 manual or dedicated real-machine validation:

- Clean Windows 11 machine with no WSL: feature enablement and `NeedsWindowsReboot`.
- Post-reboot resume through distro install/selection.
- Ubuntu WSL distro: `ah setup --fix` edits systemd config, requests `wsl --shutdown`, then passes after reopen.
- Apt tmux install on fresh Ubuntu/Debian.
- Enterprise/Store-blocked distro install failure path.

Validation boundary:

- The repository's Linux dev environment cannot validate Windows feature enablement, UAC, reboot, Store/winget policy, or WSL first-launch behavior.
- CI without real reboot can validate planning and serialization, not the end-to-end host mutation.

## Implementation Order

Current implementation scope is Phase 0 and Phase 1 only.

1. Phase 0 shared contract and read-only `ah setup --check --json`.
2. Phase 1 distro-local fixers and resume state.
3. Phase 2 independent spec and PM packaging decision; do not implement from this document alone.
4. Phase 3 diagnostics and distro expansion.

Do not implement Phase 2 before Phase 0 JSON/resume contracts exist and before the separate Phase 2 spec resolves packaging, reboot resume, in-distro ah installation, and WSL first-launch handling. The Windows helper must consume the same statuses the distro-side command produces.

## Review Checklist

- `ah doctor` remains read-only.
- `ah setup --check` performs no mutation.
- Provider checks remain external and non-fixable.
- Distro-local `--fix` is idempotent.
- Future Phase 2 Windows host helper writes resume state before any reboot boundary.
- Every external boundary returns a machine-readable status.
- Re-running after reboot or `wsl --shutdown` re-probes machine state before continuing.
- All privileged operations are explicit and auditable.
