# Windows WSL2 Host Provisioning & Verification Strategy (Phase 2 Research)

This document establishes the technical feasibility, architectural options, and verification strategies for the Phase 2 Windows Host Provisioning Helper. This research serves as the fact base to guide the subsequent Phase 2 implementation specification.

---

## 1. WSL2 Feature Enablement Mechanism

To run WSL2 natively, two distinct Windows optional features must be enabled in the host OS kernel:
1. `Microsoft-Windows-Subsystem-Linux` (WSL core subsystem)
2. `VirtualMachinePlatform` (Hyper-V light-weight hypervisor platform)

### 1.1. Feature Enablement Approaches

We compared high-level commands against low-level servicing tools:

#### Option A: `wsl.exe --install` (High-Level Unified Command)
*   **Command**: `wsl.exe --install` (optionally `--no-distribution` or `--no-launch` to prevent immediate default distro creation).
*   **Privilege Requirement**: Requires local **Administrator** privileges; triggers a UAC prompt when run from a non-elevated terminal.
*   **Reboot Requirement**: Hard reboot **required** if features were not previously enabled.
*   **Idempotency**: Partially idempotent. If WSL is already installed, running it again may output help text or return success, but behavior varies across Windows 10 versions.
*   **Limitations**: On older Windows 10 versions (prior to Build 19041 / Version 2004), the `--install` parameter is entirely unsupported, making this method fragile for backward compatibility.

#### Option B: `dism.exe` / `Enable-WindowsOptionalFeature` (Low-Level Servicing)
*   **Command**:
    ```powershell
    # Native Win32 DISM Tool
    dism.exe /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart
    dism.exe /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart
    ```
    Or via PowerShell cmdlet:
    ```powershell
    Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Windows-Subsystem-Linux -All -NoRestart
    Enable-WindowsOptionalFeature -Online -FeatureName VirtualMachinePlatform -All -NoRestart
    ```
*   **Privilege Requirement**: Requires local **Administrator** privileges.
*   **Reboot Requirement**: Hard reboot **required** to load the Hyper-V kernel modules. By passing `/norestart` (or `-NoRestart` in PowerShell), immediate forced reboots are prevented, allowing the installer to write state gracefully before exiting.
*   **Idempotency**: Strictly idempotent. If features are already active, DISM returns code `0` immediately with no modifications made.

### 1.2. Recommendation & Verdict
Use **Option B (DISM / `Enable-WindowsOptionalFeature` via PowerShell)**. Low-level DISM commands provide superior error code visibility, strict idempotency, and controllable reboot boundaries, and they are fully backward compatible with older Windows 10/11 platforms.

---

## 2. Reboot Orchestration & Resume Strategy

Enabling kernel features necessitates a system reboot. How the provisioning flow resumes post-reboot is critical to user experience.

### 2.1. Comparison of Resume Mechanisms

#### Option 1: RunOnce Registry Key
*   **Mechanism**: Write a execution command to the Current User (`HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce`) or Local Machine (`HKLM:\Software\Microsoft\Windows\CurrentVersion\RunOnce`) registry path.
*   **Behavior**: Windows executes the registered command *exactly once* during the next user interactive logon, and automatically deletes the registry value prior to executing the target command.
*   **Pros**: Simple to configure; self-cleaning.
*   **Cons**: 
    - Fails if the user context changes post-reboot.
    - If the resumed process is interrupted or fails midway, the registry key is already gone, losing the ability to recover from unexpected errors.
    - Frequently flagged as suspicious behavior by anti-virus (AV) and Endpoint Detection and Response (EDR) software.

#### Option 2: Windows Scheduled Tasks (Task Scheduler)
*   **Mechanism**: Register a Task using `Register-ScheduledTask` with a trigger set to `AtLogon` or `AtStartup`.
*   **Behavior**: Runs when any specified user logs on, or when the OS boots.
*   **Pros**: Supports complex conditional triggers (e.g., "only run if network is available", "delay execution by 30 seconds to let system stabilize").
*   **Cons**: High administrative overhead; the script must explicitly execute `Unregister-ScheduledTask` to prevent executing again on subsequent reboots.

#### Option 3: User-Triggered Resume via State File (Recommended)
*   **Mechanism**: Save the state to `%LOCALAPPDATA%\ah\setup-state.json`. When the DISM script completes, it stops and prompts the user: `NeedsWindowsReboot`. Once the user reboots and re-runs the installer (or launches the `ah` CLI), the tool reads the state file, detects that the reboot stage was completed, and resumes.
*   **Pros**:
    - **Zero AV/EDR flags**: Avoids using registry keys or tasks, minimizing potential security false-positives.
    - **Safe & Declarative**: The user maintains total visibility and control over when the process continues.
    - **Consistent with POSIX setup**: Directly maps to the `ah setup --resume` command contract.
*   **Cons**: Requires a manual rerun step by the user (which can be automated if embedded within a GUI MSI installer).

### 2.2. Verdict
Standardize on **Option 3 (User-Triggered / Installer-Guided Resume with State File)**. This approach keeps the host footprint clean, avoids security software alerts, and matches the cross-platform setup contract.

---

## 3. Distro Installation & First-Launch Bootstrapping

Once features are active, we must install a WSL distro (defaulting to `Ubuntu`) and handle its initial launch.

### 3.1. Non-Interactive Installation
To install the distro without blocking on user setup prompts, use the `--no-launch` flag:
```powershell
wsl.exe --install -d Ubuntu --no-launch
```
This registers the distribution in the system but delays running the initial configuration shell.

### 3.2. Detecting the Uninitialized (First-Launch) State
When a new WSL distro is registered but has never been launched, it lacks a default Linux user.
*   **Registry Check**: Query the `HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss` GUID subkeys. If `DefaultUid` is missing, set to `0` (root), or if `DistributionName` matches the target but no default non-root user is mapped, the distro is uninitialized.
*   **Probe Command**: Querying the user database inside the distro:
    ```powershell
    # Returns 'root' instead of a standard user, or fails if OOBE is pending
    wsl.exe -d Ubuntu -- id -un
    ```
    If the command returns `root` and there is no standard user (UID 1000) inside `/etc/passwd`, the OOBE (Out-of-Box Experience) user creation process has not run.

### 3.3. User Creation Strategies

#### Option A: Interactive First-Launch (OOBE-native)
*   **Mechanism**: The helper pauses, returns exit code `12` (`NeedsDistroFirstLaunch`), and opens a new console window:
    ```powershell
    Start-Process wsl.exe -ArgumentList "-d", "Ubuntu" -Wait
    ```
    This prompts the user directly to enter a Linux username and password inside the Linux terminal.
*   **Pros**: Native Linux experience; secure password creation; matches official Microsoft guidelines.

#### Option B: Programmatic Sudo User Provisioning (Silent)
*   **Mechanism**: Execute setup as `root` directly using the `-u root` flag:
    ```powershell
    # Add user without interactive password prompt
    wsl.exe -d Ubuntu -u root adduser --gecos "" --disabled-password devuser
    # Add user to sudoers group
    wsl.exe -d Ubuntu -u root usermod -aG sudo devuser
    # Set as default user in wsl.conf
    wsl.exe -d Ubuntu -u root sh -c "echo '[user]\ndefault=devuser' >> /etc/wsl.conf"
    ```
*   **Pros**: Fully automated, zero-touch installation.
*   **Cons**: Bypasses Linux password creation, potentially leaving the standard user without a password (relying on passwordless sudo configuration or requiring manual password setup later).

### 3.4. Verdict
Implement **Option A (Interactive First-Launch)** as the default flow, returning `NeedsDistroFirstLaunch` and spawning a console window. Provide **Option B (Programmatic Sudo User)** as an opt-in parameter (e.g., `--non-interactive --username <name>`) to support enterprise headless deployments.

---

## 4. Privilege Elevation & UAC Isolation

WSL feature enablement requires administrative rights, whereas distro installation and WSL usage must run under the current standard user account.

### 4.1. Step Classification

| Operation | Scope | Privilege Level | Native Execution Engine |
| --- | --- | --- | --- |
| Check feature status | Machine | User (Read-only) | `Get-WindowsOptionalFeature` |
| Enable WSL features | Machine | **Administrator** | `dism.exe` / `Enable-WindowsOptionalFeature` |
| Set default WSL version | Machine | User | `wsl.exe --set-default-version 2` |
| Install Linux Distro | User | User | `wsl.exe --install -d Ubuntu --no-launch` |
| Manage WSL State | User | User | `wsl.exe --shutdown` / `wsl.exe --terminate` |
| Run in-distro `ah setup` | VM | User / Guest Sudo | `wsl.exe -d Ubuntu -- ah setup` |

### 4.2. UAC Isolation Design
To avoid running the entire script as Administrator (which can corrupt path references like user home directories or Registry keys stored under the Administrator profile instead of the installing user's profile):

1.  **Start as Standard User**: The bootstrap helper (`install.ps1`) is launched in the standard user's context.
2.  **Verify & Elevate Subset**: If features are missing, the script spawns an elevated PowerShell process to execute *only* the DISM commands:
    ```powershell
    $Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$PSScriptRoot\enable-features.ps1`""
    Start-Process powershell.exe -ArgumentList $Arguments -Verb RunAs -Wait
    ```
3.  **Handoff to User Context**: Once the elevated process completes and the machine reboots, all subsequent tasks (default version selection, distro downloading, user configuration, in-distro binary mapping) run strictly in the non-privileged standard user context.

---

## 5. Packaging & Distribution Surface

`ah` currently distributes a POSIX shell script. The host helper must align with this flow for Windows users.

### 5.1. Evaluation of Packaging Options

*   **Option 1: `cargo-dist` PowerShell Installer Hook**
    *   **Description**: Custom scripting hooks within `cargo-dist`'s native PowerShell installation script.
    *   **Pros**: Provides a seamless one-liner install: `irm https://ah.dev/install.ps1 | iex`. Matches the macOS/Linux UX.
    *   **Cons**: Script size increases; requires strict modularization to avoid polluting the core installer script.
*   **Option 2: Standalone PowerShell Module (`scripts/windows/provision-ah-wsl.ps1`)**
    *   **Description**: A dedicated script maintained in the codebase and packaged with releases.
    *   **Pros**: Highly customizable, easy to test, and can be invoked directly by power users or external tools.
    *   **Cons**: Requires manual download/execution instructions if not integrated into a main installer.
*   **Option 3: MSI Package / WIX Installer**
    *   **Description**: Standard Windows installation wizard wrapping the executables and invoking the provisioning script.
    *   **Pros**: Clean GUI experience for enterprise users.
    *   **Cons**: Introduces packaging overhead; debugging errors during Custom Actions in MSI is notoriously difficult.

### 5.2. Verdict
**Primary**: **Option 1 (PowerShell Installer script generated by `cargo-dist`)** as the primary entry point for quick onboarding.  
**Secondary**: Package the underlying logic as **Option 2 (Standalone PowerShell script)**, shipping it under `scripts/windows/` for direct invocations and troubleshooting.

---

## 6. Verification & Test Matrix

This section establishes how the Windows-side provisioning code can be reliably tested given the resource limitations of standard CI runners.

### 6.1. GitHub-Hosted Runner Capabilities & Limitations

*   **Nested Virtualization**: **Not Supported** on standard GitHub-hosted `windows-latest` / `windows-2022` runners. (Running WSL2 or Hyper-V virtual machines will fail with virtualization errors).
*   **Reboot Cycles**: **Not Supported**. A `Restart-Computer` command shuts down the runner instance, terminating the CI job immediately.
*   **Interactive Session**: **Not Supported**. Runners execute as non-interactive Session 0 Windows services. UAC popups (`Start-Process -Verb RunAs`) will hang indefinitely waiting for user input.

### 6.2. CI Automation Testing (Mocked Dry-Run)
We can statically and dynamically test the helper's state machine on CI without executing real OS mutations:

1.  **Pester Unit Testing Framework**: Windows runner images come pre-packaged with **Pester** (the standard testing framework for PowerShell).
2.  **Mocking Command Invocations**: Use Pester's `Mock` capability to override physical system calls.
    *   Mock `Get-WindowsOptionalFeature` to return both `Enabled` and `Disabled` states.
    *   Mock `dism.exe` and `wsl.exe` to intercept arguments and return mock standard outputs / exit codes.
    *   Mock Registry access paths (`HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss`).
3.  **Assertions**: Verify that the helper script paths execute correctly based on the mock inputs:
    - Assert that if features are mock-disabled, the script attempts to invoke the elevated sub-process with the exact parameters and exits with `NeedsWindowsReboot`.
    - Assert that if features are mock-enabled but no distro is registered, the correct `wsl --install` parameters are assembled.
    - Assert that setup state JSON schema serializes and deserializes accurately.

### 6.3. Real-Machine E2E Validation Strategy
To achieve high-fidelity validation of feature enablement, reboot recovery, and distro launch, we must implement an out-of-band verification plan:

#### Method A: Windows Sandbox E2E Testing (Local Developer Environment)
Windows Sandbox is a lightweight, isolated desktop environment. Because it is disposable, it is ideal for testing provisioning scripts:
*   **Scripted Launch**: Create a `.wsb` (Windows Sandbox configuration file) mapping the local source directory.
*   **Execution**: Inside the sandbox, run the installation script.
*   **Verification**: Ensure the script correctly detects the sandbox environment, executes features installation, prompts for reboot, and handles state files. (Note: Sandbox supports nested virtualization, allowing WSL to run inside it if enabled in host settings).

#### Method B: Dedicated Vagrant / VirtualBox Test Suite
*   **Vagrant Setup**: Maintain a local Vagrant file targeting a clean Windows 11 base box (e.g., `generic/windows11`).
*   **Automation Script**: A local test runner script (`tests/windows/run-e2e.ps1`) boots the VM, copies the current branch scripts, executes the installation, triggers a VM reboot, and asserts the final configuration state.

---

### 6.4. Phase 2 Test Matrix Summary

| Target Component | Test Environment | Test Mechanism | Assertions / Verification Gates |
| --- | --- | --- | --- |
| **Script Syntax & Style** | GitHub CI (`windows-latest`) | `PSScriptAnalyzer` | Zero rules violations, warnings, or format syntax errors. |
| **State File Parser** | GitHub CI (`windows-latest`) | Pester Unit Test | Verify JSON serialization/deserialization of `%LOCALAPPDATA%\ah\setup-state.json`. |
| **Logic & State Transitions** | GitHub CI (`windows-latest`) | Pester with Mocking | Assert correct command construction and exit code generation under mocked registry and feature states. |
| **Elevation Guard** | Windows Sandbox / Local VM | Manual / Automated Test | Script correctly triggers UAC elevation only for feature changes and falls back to standard user otherwise. |
| **Reboot Recovery** | Windows Sandbox / Local VM | Manual / Automated Test | Script resumes state from setup-state.json post-reboot and skips completed steps. |
| **Distro Provisioning** | Windows Sandbox / Local VM | Manual / Automated Test | Installs Ubuntu, detects if uninitialized, pops interactive WSL console, and configures non-root defaults. |

---

## References

1.  **GitHub Actions Runners Specifications**: Documentation regarding virtualization support and Session 0 limitations.  
    *Source: GitHub Actions Virtual Environments Documentation (github.com/actions/runner-images)*
2.  **WSL Installation & Distribution Guidelines**: Command-line reference for `--install`, `--no-launch`, and registry structures.  
    *Source: Microsoft Learn - Windows Subsystem for Linux (learn.microsoft.com)*
3.  **DISM Application Servicing Command-Line Options**: Technical reference for optional feature enablement.  
    *Source: Microsoft Learn - Deployment Image Servicing and Management (learn.microsoft.com)*
4.  **Pester Testing Framework**: Best practices for mocking OS binaries and testing PowerShell scripts.  
    *Source: Pester Documentation (pester.dev)*
