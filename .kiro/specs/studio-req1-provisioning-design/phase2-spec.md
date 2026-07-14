# Req1 Phase 2 Windows Host Provisioning Implementation Spec

## Goal

Deliver the Windows host helper for Req1 runtime provisioning: enable WSL2 host features, survive the required Windows reboot, install or select a WSL distro, handle first-launch user creation, install `ah` inside that distro, then invoke the distro-local `ah setup --resume --fix --json` path implemented in Phases 0/1.

Phase 2 gates Windows onboarding release readiness. Code may be developed incrementally, but Phase 2 is not ready to release until the mock Windows CI gates and the real-machine gates in this spec have both passed. Any part that cannot be verified with a credible real Windows/WSL run must remain marked not-ready-to-release.

## Dependencies And Order

Phase 2 depends on:

- Phase 0 JSON envelope and prerequisite contract.
- Phase 1 distro-local `ah setup --fix`, including `NeedsWslShutdown` handling.
- A PM decision on the Windows packaging surface before user-facing release integration.

Implementation order:

1. PowerShell helper contract, state file, dry-run command runner, and mocked Windows CI.
2. WSL feature probe/enable with isolated UAC elevation and `NeedsWindowsReboot`.
3. Post-reboot resume and WSL2 default version.
4. Distro install/select and first-launch detection.
5. In-distro `ah` install/update and version verification.
6. Distro-local `ah setup --resume --fix --json` invocation and WSL shutdown orchestration.
7. Release packaging integration after PM approval.

Release-readiness order:

- P2-0 through P2-5 can land as implementation increments when their `[windows-CI-mock]` gates pass.
- Phase 2 as a whole is not release-ready until the Phase-level real-machine validation owner runs the required runbook after P2-5 and records the result.
- P2-6 release packaging integration must not ship before PM resolves the packaging surface and the real-machine validation owner/machine/cadence decision.

## Decisions

### Feature Enablement

Use DISM/PowerShell optional-feature APIs for WSL feature enablement, not `wsl.exe --install`.

Required features:

- `Microsoft-Windows-Subsystem-Linux`
- `VirtualMachinePlatform`

Commands:

```powershell
dism.exe /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart
dism.exe /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart
```

Reasoning:

- DISM is idempotent when a feature is already enabled.
- `/norestart` lets the helper write resume state before returning `NeedsWindowsReboot`.
- DISM exposes lower-level exit status than the high-level `wsl --install` feature path.
- `wsl --install` behavior varies more across Windows 10/11 versions and Store policy states.

`wsl.exe --install -d <distro> --no-launch` remains the distro-install command after features are already enabled and any required reboot has completed.

Feature status classification:

| Probe status | Meaning | Helper action |
| --- | --- | --- |
| `Enabled` | Feature active | Continue. |
| `EnablePending` | Feature enablement has been requested and requires boot to finish | Return `needs_windows_reboot`; do not run DISM again and do not continue to WSL status, distro install, or in-distro setup. |
| `Disabled` | Feature missing | With `--fix`, elevate only the feature child and enable; without `--fix`, return plan/fail. |
| Unknown/probe failure | Cannot trust feature state | Return `fail` with stderr/probe detail; do not mutate unless the user reruns after fixing the probe failure. |

`EnablePending` is a hard boundary. Treating it as disabled can repeat DISM unnecessarily; treating it as enabled can send the helper into distro provisioning before the hypervisor platform is usable.

### UAC Boundary

Start the helper in the normal user context. Elevate only the feature-enable child process.

The standard-user helper owns:

- state file path under `%LOCALAPPDATA%`;
- feature status probes;
- post-reboot resume;
- WSL default-version command;
- distro install/select;
- first-launch detection;
- in-distro `ah` install;
- distro-local `ah setup` invocation.

The elevated child owns only:

- DISM calls for the two Windows optional features.

Do not run the whole helper as Administrator. Running the full flow elevated risks writing state under the Administrator profile, installing the distro for the wrong Windows user, or losing access to the intended user's `%LOCALAPPDATA%`.

Elevated child result contract:

- The standard-user helper launches the child with `Start-Process -Verb RunAs -Wait -PassThru`.
- The child also writes a result JSON file to a standard-user-readable temp path created by the parent, e.g. `%LOCALAPPDATA%\ah\setup-elevated-result.<operation_id>.json`.
- The parent trusts the result file when present and uses the `-PassThru` process exit code only as a fallback for launch/cancel/fatal cases.
- The child result JSON includes `operation_id`, `features[]`, `dism_exit_codes`, `reboot_required`, `partial_enable`, `stderr_tail`, and `status`.

Status mapping:

| Elevated outcome | Parent status |
| --- | --- |
| User cancels UAC, access denied, or child never starts | `permission_denied` |
| All requested features enabled or `EnablePending`, reboot required | `needs_windows_reboot` |
| One feature enabled/pending and another failed | `needs_windows_reboot` with `partial_enable=true`; after reboot/resume, observed-state probes decide whether to retry the failed feature |
| DISM exits nonzero without enabling/pending any feature | `fail` |
| Result file missing but process exit code reports success | Re-probe features; observed state wins |

The child must not block waiting on the parent. The parent must not continue to WSL/distro work until it has re-probed feature state after the elevated child returns.

### Resume Strategy

Use user-triggered resume through a state file:

```text
%LOCALAPPDATA%\ah\setup-state.json
```

Do not use RunOnce or Scheduled Tasks for the default flow.

Reasoning:

- RunOnce deletes its key before the resumed command runs; a crash loses recovery.
- Scheduled Tasks add cleanup and policy complexity.
- User-triggered resume avoids AV/EDR suspicion and keeps the user in control across reboot.

If a future GUI installer wants automatic continuation, it may wrap this state file and rerun the same helper after reboot, but the helper itself must remain resumable through an explicit command.

### Distro Policy

Default distro is `Ubuntu` unless the installer/helper is passed an explicit distro name.

Install command:

```powershell
wsl.exe --install -d Ubuntu --no-launch
```

If the distro already exists, skip install and use the existing distro. If Windows policy blocks Store-backed distro install, return `UnsupportedDistroInstall` with exact manual instructions. Do not guess alternate Store, winget, or tar import behavior in Phase 2 unless PM approves that distribution path separately.

Existing distro WSL version classification:

- Parse `wsl.exe -l -v` and record the selected distro's version.
- `wsl.exe --set-default-version 2` only affects future distro installs; it does not convert an existing WSL1 distro.
- If the selected distro exists and reports version `1`, stop before first-launch or in-distro setup.
- With `--fix`, run or prompt for:

  ```powershell
  wsl.exe --set-version Ubuntu 2
  ```

- Without `--fix`, return a plan/fail explaining that the selected distro must be converted to WSL2 before ah provisioning can continue.

WSL1-to-WSL2 conversion can take time and can fail if the distro is corrupt or disk space is low. The helper must print backup guidance before conversion: export or back up important distro data first. If conversion fails, leave state resumable and do not attempt in-distro `ah` install.

### First Launch

Use interactive WSL first-launch as the default user-creation path.

When the selected distro exists but has no initialized non-root Linux user, return `NeedsDistroFirstLaunch` and open or instruct the user to open:

```powershell
wsl.exe -d Ubuntu
```

The user creates the Linux username/password through the distro's native OOBE prompt. Programmatic user creation is not part of the default Phase 2 path. An enterprise zero-touch option can be a later feature with its own security review.

### In-Distro `ah` Install

The Windows helper must install or update `ah` inside the selected distro before it invokes distro-local setup. A newly installed distro will not have `ah` on `PATH`.

Decision:

- Invoke the existing POSIX release installer inside the selected distro under the distro's default non-root user.
- Install to the user's home bin directory, initially `$HOME/.local/bin`.
- Use the cargo-dist shell installer's documented generated environment contract: set `AH_INSTALL_DIR="$HOME/.local"` so its cargo-home layout installs binaries into `$HOME/.local/bin`.
- Also set `AH_NO_MODIFY_PATH=1`; the helper uses absolute paths and must not rely on or edit shell startup files for this flow.
- Do not rely on an undocumented cargo-dist generated installer flag.
- Do not assume `$HOME/.local/bin` is already on the non-interactive WSL `PATH`; the helper uses the absolute path for its own calls.
- Verify with the absolute path:

  ```sh
  "$HOME/.local/bin/ah" --version
  ```

- Invoke distro-local setup through the absolute path:

  ```sh
  "$HOME/.local/bin/ah" setup --resume --fix --json
  ```

- If a user-facing PATH update is needed, report it as guidance or warning; do not block provisioning when the absolute path works.

The helper must pin an expected `ah` version from the release package metadata. If the installed version does not match the helper's expected version, retry install/update once, then return a resumable `AhInstallFailed` state with stderr and manual command guidance.

Install-dir contract:

- Probe the target Linux home with:

  ```powershell
  wsl.exe -d Ubuntu -- sh -lc 'printf %s "$HOME"'
  ```

- Set:

  ```sh
  AH_INSTALL_DIR="$HOME/.local"
  AH_NO_MODIFY_PATH=1
  ```

- Run the release installer in the same shell with that environment set.
- Use the probed `$HOME` value only inside the selected distro. Do not hardcode the developer machine's sandbox path into the spec or script. If the probed home is `/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7`, the resulting install dir is `/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7/.local/bin`; on a normal Ubuntu user it is `/home/<user>/.local/bin`.
- Verify and invoke by shell-expanded absolute path:

  ```sh
  "$HOME/.local/bin/ah" --version
  "$HOME/.local/bin/ah" setup --resume --fix --json
  ```

This closes the path/PATH/version loop: installer location is controlled by `AH_INSTALL_DIR`, helper calls do not depend on PATH, and version verification decides whether the state can advance.

Interim release URL contract:

- P2-4 must be able to start before P2-6 packaging is finalized.
- Until the cargo-dist PowerShell hook is approved, the helper takes an explicit `--ah-install-url <url>` or reads `AH_SETUP_INSTALL_URL` from the environment.
- CI/mock tests use a fixture URL and never download it.
- Real-machine dogfood uses the current release shell installer URL supplied by PM/release owner for that run.
- P2-6 later replaces this with release metadata from the approved packaging surface; P2-4 must not hardcode a final production URL.

### Packaging Recommendation And PM Decision

Recommendation:

- Primary release surface: cargo-dist PowerShell installer hook, once PM approves adding a Windows PowerShell installer surface.
- Shared implementation unit: standalone script/module under `scripts/windows/`, invoked by the installer hook and directly usable for troubleshooting.

Rationale:

- The standalone script keeps the logic testable with Pester and easy to run in dogfood.
- A cargo-dist PowerShell hook provides the eventual one-liner onboarding UX.
- MSI/winget can wrap the same script later, but they add packaging overhead and are not required for the first Phase 2 implementation.

PM decision required before release integration:

- Whether to ship a PowerShell installer hook in the official release surface.
- The public command name and download URL.
- Whether standalone script invocation is an officially supported interface or a support/debug escape hatch.

## Components

Candidate files and components:

- `scripts/windows/provision-ah-wsl.ps1`: standard-user entry point and resume command.
- `scripts/windows/enable-ah-wsl-features.ps1`: elevated child that only runs DISM feature enablement.
- `scripts/windows/AhProvisioning.psm1`: shared state, command-runner, JSON envelope, and probe helpers.
- `tests/windows/Req1Phase2.Tests.ps1`: Pester mocked state-machine tests.
- `.github/workflows/windows-provisioning.yml` or an existing workflow job: run PSScriptAnalyzer and Pester on `windows-latest`.
- cargo-dist installer hook file: only after PM approves the packaging surface.

The implementation may choose fewer files, but it must preserve the same isolation: standard-user orchestration, elevated feature child, reusable module, and mocked tests.

## Shared JSON Contract

Phase 2 must use the same top-level envelope as `ah setup --json`:

- `schema_version`
- `operation_id`
- `overall_status`
- `phase`: `phase2_windows_host`
- `selected_distro`
- `next_action`
- `resume_command`
- `steps[]`

Phase 2 `overall_status` values include:

- `pass`
- `fixed`
- `needs_windows_reboot`
- `needs_distro_first_launch`
- `needs_wsl_shutdown`
- `unsupported`
- `permission_denied`
- `fail`

`next_action.command` is the immediate command the user should run next. `resume_command` is the command that continues provisioning after an external boundary. They may differ.

Examples:

- Before reboot:
  - `next_action.command`: `Restart-Computer` or `reboot Windows, then rerun the installer`
  - `resume_command`: `powershell.exe -ExecutionPolicy Bypass -File .\provision-ah-wsl.ps1 --resume`
- Before distro first launch:
  - `next_action.command`: `wsl.exe -d Ubuntu`
  - `resume_command`: `powershell.exe -ExecutionPolicy Bypass -File .\provision-ah-wsl.ps1 --resume`

## State And Resume

State path:

```text
%LOCALAPPDATA%\ah\setup-state.json
```

Use `%ProgramData%\ah\setup-state.json` only if PM later chooses an all-users MSI flow. That is a packaging-specific change, not the default Phase 2 path.

State fields:

- `schema_version`
- `operation_id`
- `helper_version`
- `phase`: `phase2_windows_host`
- `boundary`: `windows-host`
- `selected_distro`
- `requested_default_wsl_version`: `2`
- `feature_steps`: map from feature name to observed/requested state, last exit code, and reboot flag.
- `selected_distro_wsl_version`: observed `1`, `2`, or unknown from `wsl.exe -l -v`.
- `pending_restart`: `none | windows_reboot | distro_install | distro_first_launch | in_distro_ah_install | distro_setup | wsl_shutdown`
- `ah_install`: expected version, install source, install path, last observed version.
- `last_completed_step`
- `created_at`
- `updated_at`
- `last_error`

State writes must be atomic: write a temp file next to `setup-state.json`, then replace the old file. State must not contain tokens, passwords, provider credentials, or sudo passwords.

Observed state wins:

- If state says `windows_reboot` but both features are now enabled, proceed.
- If either feature is `EnablePending`, keep or set `windows_reboot` and stop even if state says a later step was reached.
- If the selected distro exists but reports WSL version `1`, stop at WSL2 conversion before first-launch or in-distro setup.
- If state says distro install is pending but `wsl.exe -l -v` shows the selected distro, proceed to first-launch probing.
- If state says `in_distro_ah_install` is complete but `"$HOME/.local/bin/ah" --version` fails, reinstall/update.
- If state says `wsl_shutdown` but distro-local `ah setup --check --json` passes, clear state.

Resume entry:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\provision-ah-wsl.ps1 --resume
```

Plain helper invocation must also detect pending state and offer to resume.

## Boundary Output Contract

Every boundary exit must print both human text and JSON with:

1. What just happened, including feature names, distro name, state path, or installed `ah` path.
2. The exact next command the user must run.
3. The exact status/check command.
4. The resume command.

Boundary-specific requirements:

- `NeedsWindowsReboot`: state path must be printed before exit; no distro install may run before reboot.
- `NeedsDistroFirstLaunch`: explain that the distro will ask for a Linux username/password; do not ask the user to enter credentials into `ah`.
- `NeedsWslShutdown`: state that `wsl.exe --shutdown` terminates all running WSL distros, not only the selected distro.

## Task P2-0: PowerShell Contract And Mockable Runner

Scope:

- Add the standard-user helper entry point.
- Add shared JSON envelope rendering.
- Add state read/write helpers.
- Add command-runner abstraction so Pester tests can mock all host mutations.
- Add wrapper functions for every host command. Production orchestration code must call wrappers, not raw `dism.exe`, `wsl.exe`, registry APIs, or `Start-Process` directly.
- Add `--check`, `--fix`, `--resume`, `--json`, `--yes`, `--distro <name>`, and `--dry-run` flags.

Files/components:

- `scripts/windows/provision-ah-wsl.ps1`
- `scripts/windows/AhProvisioning.psm1`
- `tests/windows/Req1Phase2.Tests.ps1`
- Windows CI workflow job

Exit gates:

- [windows-CI-mock] Pester tests verify JSON envelope fields for pass, fail, and boundary states.
- [windows-CI-mock] State file serialization/deserialization round-trips with unknown-field tolerance.
- [windows-CI-mock] `--dry-run` never invokes real DISM or WSL commands.
- [windows-CI-mock] Pester mocks wrapper functions such as `Get-AhWindowsOptionalFeature`, `Invoke-AhDismEnableFeature`, `Invoke-AhWsl`, `Read-AhLxssRegistry`, and `Start-AhElevatedFeatureChild`; tests do not mock raw `dism.exe`/`wsl.exe` from production code.
- [windows-CI-mock] Static or grep-based guard asserts production orchestration files do not directly call `dism.exe` or `wsl.exe` outside the wrapper module and elevated child.
- [windows-CI-mock] `next_action.command` and `resume_command` are distinct and correctly populated for reboot and first-launch boundaries.

Verification:

- Fully mockable on `windows-latest`; no real WSL, UAC, or reboot required.

## Task P2-1: WSL Feature Probe And Enablement

Scope:

- Probe both optional features.
- If both are enabled, skip DISM.
- If either is missing and `--fix` is absent, return a read-only plan with `overall_status=fail`.
- If either is missing and `--fix` is present, spawn the elevated child for only the missing features.
- Elevated child runs DISM with `/norestart`.
- Standard-user helper writes state and returns `NeedsWindowsReboot` when a feature was enabled or DISM reports reboot required.
- If either feature probes as `EnablePending`, return `NeedsWindowsReboot` immediately without rerunning DISM.
- Parent collects elevated child outcome through the result JSON file plus `Start-Process -PassThru` exit code fallback.

Files/components:

- `scripts/windows/provision-ah-wsl.ps1`
- `scripts/windows/enable-ah-wsl-features.ps1`
- `scripts/windows/AhProvisioning.psm1`

Commands:

```powershell
Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Windows-Subsystem-Linux
Get-WindowsOptionalFeature -Online -FeatureName VirtualMachinePlatform
Start-Process powershell.exe -ArgumentList ... -Verb RunAs -Wait
```

Exit gates:

- [windows-CI-mock] Disabled feature fixture builds the exact elevated child command and does not run distro install in the same pass.
- [windows-CI-mock] `EnablePending` fixture returns `needs_windows_reboot`, does not run DISM again, and does not call WSL/distro wrappers.
- [windows-CI-mock] Enabled feature fixture skips elevation and proceeds to WSL status checks.
- [windows-CI-mock] DISM argument tests assert `/norestart`, `/all`, and exact feature names.
- [windows-CI-mock] UAC cancel or denied launch maps to `permission_denied`.
- [windows-CI-mock] Elevated child DISM failure with no feature progress maps to `fail`.
- [windows-CI-mock] Partial enable maps to `needs_windows_reboot` with `partial_enable=true` and resumable state.
- [real-machine] Clean Windows machine with WSL disabled reaches `NeedsWindowsReboot`, writes `%LOCALAPPDATA%\ah\setup-state.json`, and prints resume instructions.
- [real-machine] Rerun before reboot reprints the pending reboot state instead of trying distro install.

Verification:

- CI proves command planning and state handling only.
- Actual UAC and feature enablement require real machine validation.

## Task P2-2: Post-Reboot Resume And WSL2 Default

Scope:

- On resume, re-probe optional features.
- Run `wsl.exe --status` and classify WSL availability.
- Set default WSL version to 2 once features are active:

  ```powershell
  wsl.exe --set-default-version 2
  ```

- Skip if the command reports version 2 already active.
- Parse existing distro version from `wsl.exe -l -v` before first-launch or in-distro setup.
- If selected distro is WSL1, stop for conversion. With `--fix`, run or prompt for `wsl.exe --set-version <distro> 2`; without `--fix`, return plan/fail.

Files/components:

- `scripts/windows/provision-ah-wsl.ps1`
- `scripts/windows/AhProvisioning.psm1`

Exit gates:

- [windows-CI-mock] State with `pending_restart=windows_reboot` and mocked enabled features proceeds to WSL status.
- [windows-CI-mock] Mocked WSL default-version command is called only after feature probes pass.
- [windows-CI-mock] Failed `wsl --status` returns actionable `fail` or `unsupported` with stderr.
- [windows-CI-mock] Existing selected distro with version `1` returns WSL2 conversion plan and does not proceed to first-launch or in-distro setup.
- [windows-CI-mock] With `--fix`, WSL1 selected distro fixture assembles `wsl.exe --set-version Ubuntu 2`, records a resumable conversion state, and handles conversion failure without continuing.
- [real-machine] After reboot, helper resumes from state, does not repeat DISM, and can run `wsl.exe --status`.
- [real-machine] Existing WSL1 Ubuntu is either converted to WSL2 or stops with exact backup/conversion guidance; no in-distro setup runs while version remains `1`.

Verification:

- CI verifies resume logic and command order.
- Reboot completion is real-machine only.

## Task P2-3: Distro Install, Selection, And First Launch

Scope:

- List existing distros with:

  ```powershell
  wsl.exe -l -v
  ```

- Select the requested distro if present.
- If missing, install the default distro:

  ```powershell
  wsl.exe --install -d Ubuntu --no-launch
  ```

- Detect uninitialized first-launch state.
- Return `NeedsDistroFirstLaunch` until a non-root default Linux user exists.
- Do not run first-launch probes if the selected distro is WSL1; WSL2 conversion in P2-2 must complete first.

First-launch probes:

- Lxss registry under `HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss` for matching `DistributionName` and `DefaultUid`.
- `wsl.exe -d <distro> -- id -un`.
- Optional sudo capability probe only after a non-root user exists.

Classification:

- Missing distro: install when `--fix`; plan only otherwise.
- Existing distro with version `1`: handled by P2-2 conversion; stop before first-launch classification.
- Distro exists and `DefaultUid` missing/zero or `id -un` returns `root`: `NeedsDistroFirstLaunch`.
- Distro exists with non-root user: continue to in-distro `ah` install.

Files/components:

- `scripts/windows/provision-ah-wsl.ps1`
- `scripts/windows/AhProvisioning.psm1`

Exit gates:

- [windows-CI-mock] Mocked missing distro assembles `wsl.exe --install -d Ubuntu --no-launch`.
- [windows-CI-mock] Mocked existing distro skips install.
- [windows-CI-mock] Mocked existing WSL1 distro does not run first-launch or in-distro install probes.
- [windows-CI-mock] Mocked `DefaultUid=0` or `id -un=root` returns `NeedsDistroFirstLaunch` with exact command `wsl.exe -d Ubuntu`.
- [windows-CI-mock] Store/policy install failure returns `UnsupportedDistroInstall` and leaves state resumable.
- [real-machine] Fresh distro install returns `NeedsDistroFirstLaunch`.
- [real-machine] After the user completes OOBE username/password creation, helper resumes and detects the non-root user.

Verification:

- CI validates registry and command-output classification through mocks.
- First-launch OOBE must be tested on a real Windows/WSL machine.

## Task P2-4: In-Distro `ah` Install Or Update

Scope:

- Run after selected distro has a non-root default user.
- Install/update `ah` and `ahd` inside the distro using the POSIX release installer.
- Target install directory: `$HOME/.local/bin`.
- Use absolute paths for verification and subsequent calls.
- Verify expected version.
- Return a resumable failure when install or version verification fails.

Command shape:

```powershell
wsl.exe -d Ubuntu -- sh -lc 'export AH_INSTALL_DIR="$HOME/.local"; export AH_NO_MODIFY_PATH=1; curl -fsSL "$AH_SETUP_INSTALL_URL" | sh; "$HOME/.local/bin/ah" --version'
```

The exact production installer URL and version metadata must come from the release packaging layer. Until P2-6 finalizes packaging, the helper accepts `--ah-install-url` or `AH_SETUP_INSTALL_URL`; this interim input is required for P2-4 dogfood and tests.

Version policy:

- If no `ah` exists, install expected version.
- If older/different `ah` exists, update to expected version.
- If the installer cannot verify the expected version, return `AhInstallFailed`.
- Do not downgrade unless the user passed an explicit release pin from the installer.

Files/components:

- `scripts/windows/provision-ah-wsl.ps1`
- `scripts/windows/AhProvisioning.psm1`
- packaging hook or release metadata provider

Exit gates:

- [windows-CI-mock] Install command construction quotes distro name and shell command safely.
- [windows-CI-mock] Install command sets `AH_INSTALL_DIR="$HOME/.local"` plus `AH_NO_MODIFY_PATH=1` and verifies `"$HOME/.local/bin/ah" --version`.
- [auto-CI] P2-6 packaging fixture runs the generated cargo-dist shell installer and proves `AH_INSTALL_DIR="$HOME/.local"` lands binaries in `$HOME/.local/bin`.
- [windows-CI-mock] Missing install URL returns plan/fail before invoking WSL.
- [windows-CI-mock] Missing `ah` fixture invokes install before setup.
- [windows-CI-mock] Version mismatch fixture invokes update and then verify.
- [windows-CI-mock] Install failure preserves state with `pending_restart=in_distro_ah_install` and emits manual retry command.
- [real-machine] Fresh Ubuntu distro installs `ah` into `$HOME/.local/bin` and verifies the expected version with the absolute path.
- [real-machine] Re-running helper skips install when the expected version is already present.

Verification:

- CI can verify command planning and state transitions.
- Real download/install inside WSL is a real-machine gate.

## Task P2-5: Invoke Distro-Local Setup

Scope:

- Invoke Phase 1 setup through the installed absolute path:

  ```powershell
  wsl.exe -d Ubuntu -- sh -lc '"$HOME/.local/bin/ah" setup --resume --fix --json'
  ```

- Parse the JSON envelope.
- If distro-local setup returns `needs_wsl_shutdown`, prompt before running:

  ```powershell
  wsl.exe --shutdown
  ```

- After shutdown, rerun the installed `ah setup --resume --fix --json`.
- If distro-local setup passes, clear Phase 2 state.

Files/components:

- `scripts/windows/provision-ah-wsl.ps1`
- `scripts/windows/AhProvisioning.psm1`

Exit gates:

- [windows-CI-mock] Distro-local `needs_wsl_shutdown` fixture prints all-distro shutdown impact and exact resume command.
- [windows-CI-mock] With `--yes`, helper may run mocked `wsl.exe --shutdown`; without `--yes`, it stops for confirmation.
- [windows-CI-mock] Passing distro-local JSON clears state.
- [real-machine] Fresh Ubuntu flow reaches distro-local `NeedsWslShutdown`, runs or instructs `wsl.exe --shutdown`, then resumes to pass.
- [real-machine] Final `wsl.exe -d Ubuntu -- sh -lc '"$HOME/.local/bin/ah" setup --check --json'` returns exit code `0`, or uses the already probed absolute Linux home path if the helper records one.

Verification:

- CI verifies JSON parsing and orchestration only.
- Actual WSL shutdown/reopen behavior is real-machine only.

## Task P2-6: Packaging Integration

Scope:

- Keep the implementation in the standalone script/module.
- Wire the approved release surface to call the standalone helper.
- Preserve a direct manual invocation path for support.

Recommended packaging:

- Do not ship a native Windows `ah`/`ahd` binary for this Req1 path; the native Windows port is still outside the current functional scope.
- Keep cargo-dist's native binary release Linux-only and continue generating the POSIX shell installer for the Linux `ah`/`ahd` used inside WSL.
- Ship `scripts/windows/provision-ah-wsl.ps1`, `scripts/windows/AhProvisioning.psm1`, and `scripts/windows/enable-ah-wsl-features.ps1` as release assets via cargo-dist `extra-artifacts`.
- Windows onboarding entry point is the downloaded `provision-ah-wsl.ps1` release asset. It enables WSL2, installs/selects the distro, then installs Linux `ah` inside WSL using the POSIX release installer URL.

PM decision for this task:

- cargo-dist PowerShell binary installer is not in scope for this release because it would install a native Windows `ah`/`ahd` stub.
- exact public Windows onboarding command points at the `provision-ah-wsl.ps1` release asset;
- support policy keeps standalone script invocation as the primary troubleshooting path.

P2-4/P2-6 URL handoff:

- Before P2-6, P2-4 uses explicit `--ah-install-url`/`AH_SETUP_INSTALL_URL`.
- P2-6 replaces that interim source with release metadata from the approved packaging surface.
- P2-6 must preserve an override for dogfood and support runs so real-machine validation can test a candidate build before public release.

Exit gates:

- [windows-CI-mock] Packaging smoke verifies the Windows helper files are configured as cargo-dist `extra-artifacts`.
- [windows-CI-mock] Standalone helper invocation remains documented and testable without cargo-dist.
- [real-machine] Downloaded `provision-ah-wsl.ps1` release asset can start the Phase 2 helper from a clean standard-user PowerShell session.

Verification:

- Packaging command wiring can be tested in CI.
- Real installer invocation on a clean Windows host is a real-machine gate.

## Verification Strategy

Phase 2 verification has two layers:

- `[windows-CI-mock]` proves the helper state machine, JSON contract, command construction, and wrapper boundaries.
- `[real-machine]` proves the OS behavior that GitHub-hosted runners cannot exercise.

Release-readiness requires both. Mock CI is necessary but not sufficient.

### Windows CI Mock Coverage

Run on `windows-latest` without requiring nested virtualization, reboot, or UAC interaction.

Required tools:

- Pester for mocked PowerShell tests.
- PSScriptAnalyzer or equivalent syntax/static checks.

Mocked commands and APIs:

- wrapper functions around `Get-WindowsOptionalFeature`
- wrapper functions around `dism.exe`
- wrapper functions around `Start-Process -Verb RunAs`
- wrapper functions around `wsl.exe`
- wrapper functions around registry reads under `HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss`
- filesystem reads/writes for `%LOCALAPPDATA%\ah\setup-state.json`

Required assertions:

- exact feature names and DISM flags;
- UAC elevation used only for the feature child;
- no distro install before reboot boundary;
- state file is written before boundary exit;
- resume uses observed probes rather than trusting state;
- first-launch classification;
- in-distro `ah` install command construction;
- JSON envelope schema for every boundary.
- production code only invokes host mutations through wrapper functions.

### Real-Machine Validation

Actual release readiness requires the named owner below to run the full runbook on the named real Windows machine after P2-5 lands.

DECIDED (owner/machine/cadence resolved by the user):

- Owner: the user (project owner), personally.
- Runner: the user's own Windows 11 + WSL2 machine — the Studio target environment, i.e. the most realistic validation bed. (A dedicated VM snapshot per the recommendation below may supplement but does not replace this run.)
- Cadence: after Phase 2 code and mock CI (DISM / reboot / elevation all mocked on windows CI) are fully green, PM-proxy assembles the runbook below into a clean, ready-to-follow checklist and hands it to the user; the user runs it once on the real machine and signs off. Sign-off = release-readiness reached; only then is the public-repo (`SevenX77/ah`) tag cut.
- Artifact: saved transcript/logs, final JSON envelopes, `setup-state.json` snapshots at each boundary, and before/after `wsl.exe -l -v` output.

Do not rely on Windows Sandbox as the sole planned runner unless PM has verified that the host's Sandbox configuration supports nested WSL for this scenario. Sandbox can be a disposable supplement, not the default bet.

Recommended runner:

- A dedicated Windows 11 VM with snapshot/restore support and WSL-capable virtualization enabled.
- Internal dogfood on a clean user machine may supplement the VM run but should not replace the reproducible VM baseline.

Minimum real-machine runbook:

1. Start from no enabled WSL optional features, or a clean VM snapshot.
2. Run the approved helper as a standard user.
3. Confirm UAC appears only for feature enablement.
4. Confirm `NeedsWindowsReboot` with state file and resume instructions.
5. Reboot.
6. Rerun helper with `--resume`.
7. Install/select Ubuntu.
8. Complete distro first-launch username/password creation.
9. Rerun helper.
10. Confirm `ah` is installed inside WSL and distro-local setup reaches `NeedsWslShutdown`.
11. Run/approve `wsl.exe --shutdown`.
12. Confirm final `ah setup --check --json` passes in the distro.
13. Rerun helper and confirm no-op idempotency.

Owner deliverable:

- The owner attaches the runbook result to the PR/release checklist.
- Each failed step records whether the issue is a helper bug, machine policy limitation, packaging issue, or manual operator error.
- Release readiness remains blocked until all required real-machine steps pass or PM explicitly scopes the failed path out of the release.

No part of DISM feature enablement, UAC, reboot recovery, Store-backed distro install, first-launch OOBE, or WSL shutdown may be marked ready-to-release based on mock CI alone.

### Phase-Level Release Gate

After P2-5:

- [windows-CI-mock] All P2-0 through P2-5 mock tests pass.
- [real-machine] Named owner runs the full runbook on the named runner and records artifacts.
- [real-machine] Final in-distro status command succeeds:

  ```powershell
  wsl.exe -d Ubuntu -- sh -lc '"$HOME/.local/bin/ah" setup --check --json'
  ```

- [real-machine] Re-running the helper is a no-op or prints only already-satisfied status.

After P2-6:

- [windows-CI-mock] Packaging hook dry-run calls the standalone helper.
- [real-machine] The approved release package launches the helper from a clean standard-user PowerShell session.
- [real-machine] The named owner reruns at least the packaging-start through final-check subset against the approved package.

## Risks

- GitHub-hosted Windows CI cannot validate nested virtualization, reboot, or UAC prompts.
- Phase 2 has no release-ready real-machine validation until PM assigns an owner, runner, cadence, and artifact location.
- Windows Sandbox nested WSL support is environment-dependent; it must not be the only planned validation path.
- Windows Store and enterprise policy can block distro install.
- Distro first launch is intentionally user-interactive and may be abandoned mid-flow.
- The POSIX release installer must be reachable from inside the distro; corporate proxy/offline environments can fail here.
- P2-4 depends on an install URL that P2-6 eventually owns; the interim `--ah-install-url`/`AH_SETUP_INSTALL_URL` override is required to avoid blocking implementation on packaging.
- Elevation boundary mistakes can write state under the wrong Windows profile.
- `wsl.exe --shutdown` affects all running WSL distros and can interrupt unrelated work.
- The packaging surface is not yet approved; implementation should keep script logic separate from release integration.

## Non-Goals

- Do not install provider CLIs or manage provider auth.
- Do not bypass UAC, Windows credentials, Linux passwords, sudo prompts, OAuth, or provider credential flows.
- Do not mutate Windows proxy, timezone, locale, fonts, shell profiles, or editor integration.
- Do not promise automatic rollback for WSL feature enablement, distro install, or `/etc/wsl.conf` edits.
- Do not build MSI/winget packaging before PM chooses that surface.

## Rollback Policy

Phase 2 does not promise automatic rollback for host feature enablement, distro install, WSL1-to-WSL2 conversion, in-distro `ah` install, or Phase 1 `/etc/wsl.conf` edits.

This is acceptable only because every step must be forward-idempotent:

- probes always run before mutation;
- `EnablePending` and reboot boundaries stop rather than guessing;
- observed machine state wins over saved state;
- failed or partial states remain resumable;
- rerunning the helper never depends on a one-shot temp file or deleted resume hook;
- user-facing output includes exact status and resume commands.

If any implementation step creates a half-state that blocks rerun, that step violates this spec and must add either a safe forward repair path or a manual recovery boundary before release.
