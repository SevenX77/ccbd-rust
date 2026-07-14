# Windows-Native Support Design: First-Principles Architecture & Implementation Plan

This document establishes the first-principles architectural design for executing, monitoring, reaping, isolating, and identity-stamping agents on Windows natively without WSL. It evaluates the existing Windows spec files (`.kiro/specs/ah-windows-native/`), critiques their assumptions against verified breakages, surfaces critical security and concurrency gaps, and provides concrete execution and remediation mechanisms.

---

## 1. Executive Summary & First-Principles Stance

The core architecture of the Windows-native port pivots `ah` from its Unix-centric substrate (systemd scopes + tmux + Unix Domain Sockets) to a native Windows runtime. This native model is built on four core Win32 subsystems:
1. **ConPTY (Pseudo Console) & Virtual Terminal Grid**: Replaces tmux for interactive shell execution and programmatic output capture.
2. **Job Objects (Process Containment)**: Replaces systemd scopes for cascading process-tree termination and resource limits.
3. **Task Scheduler COM APIs**: Replaces systemd user units to provide non-privileged user-level background persistence.
4. **Named Pipes (IPC)**: Replaces Unix Domain Sockets (UDS) for low-latency, ACL-secured client-daemon communication.

### Verification of Existing Spec (Keep vs. Overturn)

| Component | Spec M0/M1 Stance | Evaluation & First-Principles Critique | Verdict |
| :--- | :--- | :--- | :--- |
| **Terminal Substrate** | ConPTY via `portable-pty` + `alacritty_terminal` grid. | ConPTY is the correct and only modern native substrate. Direct pipes fail for interactive tools (shells, ssh), and WinPTY is deprecated. We keep ConPTY but introduce a mandatory **DSR Cursor Query Responder** to prevent initialization stalls. | **KEEP** (with additions) |
| **Command Spawn** | Return raw `Vec<String>` where first elements are environment prefix strings (`KEY=VALUE`). | Grounded breakage (**P0#1**): Windows executes the first array element as the program; prepending `KEY=VALUE` or `env` fails. We overturn this approach and establish an explicit **Env-Stripping Command Spawner** in the multiplexer. | **OVERTURN** |
| **Tree Reaping & Wait** | Job Objects with no breakaway. `RegisterWaitForSingleObject` for async wait. | Unsafe wait callback: raw pointer context passing is highly vulnerable to use-after-free and double-free during drop/cancellation. We overturn the unsafe FFI context and design a memory-safe `Arc<Mutex>` context boundary. | **OVERTURN** (wait safety) |
| **PID Reuse Fence** | `pidfd_open` opens process via PID. | PID Recycling Race: opening a process by PID after it exits can open a recycled process. We introduce a thread-safe `ACTIVE_PROCESS_REGISTRY` to map `PID -> Raw HANDLE` immediately on spawn. | **OVERTURN** (PID lookup) |
| **Sandbox Isolation** | No filesystem or network isolation designed for Windows. | Gap: fails to match Linux's `BindReadOnlyPaths` ([src/platform/linux/scope.rs:341-348](file:///home/sevenx/coding/ccbd-rust/src/platform/linux/scope.rs#L341-L348)). We introduce a **Restricted Token Job Object** design for native filesystem and network sandboxing. | **OVERTURN** (isolation gap) |

---

## 2. Windows execution & Command Spawning (P0#1)

### The Breakage (Grounded in Code)
In [src/platform/windows/scope.rs:164-190](file:///home/sevenx/coding/ccbd-rust/src/platform/windows/scope.rs#L164-L190), the stub `command_with_env_prefix` constructs a command vector by pushing environment variables directly into the command arguments list:
```rust
fn command_with_env_prefix(...) -> Vec<String> {
    let mut cmd = Vec::new();
    for (key, value) in collect_spawn_env(manifest, extra_env_vars) {
        cmd.push(format!("{key}={value}")); // Unix env-prefix pattern
    }
    cmd.extend(manifest.command.iter().map(|part| (*part).to_string()));
    ...
}
```
This is passed to the spawner in [src/rpc/handlers/agent.rs:182-195](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/agent.rs#L182-L195) and [src/rpc/handlers/sessions.rs:517-539](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L517-539). Because Windows lacks a shell-level prepended environment syntax, passing this array directly to a process spawner causes the OS to look for an executable literally named `"KEY=VALUE"`, crashing immediately.

### First-Principles Spawning Mechanism
To maintain the cross-platform signatures of `wrap_command` and `spawn_window(..., cmd: &[&str])` without breaking shared caller code, Windows command execution must utilize an **Env-Stripping Spawner** inside the `WinPtyMultiplexer` / `TmuxServer` Windows implementation:

1. **Uniform Command Prepending**: We align the Windows `command_with_env_prefix` in [src/platform/windows/scope.rs](file:///home/sevenx/coding/ccbd-rust/src/platform/windows/scope.rs) to prepend `"env"`, matching the Linux behavior in [src/platform/linux/scope.rs:381-447](file:///home/sevenx/coding/ccbd-rust/src/platform/linux/scope.rs#L381-L447). The returned vector will be:
   `["env", "KEY1=VAL1", "KEY2=VAL2", "executable", "arg1", "arg2", ...]`
2. **Multiplexer Env Extraction**: When `spawn_window` is invoked on Windows with the command array, the multiplexer:
   - Verifies if `cmd[0] == "env"`.
   - Iterates through subsequent elements. If they contain `=`, it splits them on the first `=` into a key-value pair and registers them as environment variables via `CommandBuilder::env` or `std::process::Command::env`.
   - The first element that does not contain `=` is recognized as the target executable. All subsequent elements are arguments.
3. **Structured vs. Shell Execution**:
   - **Agent Commands** (defined in manifests like `["python", "main.py"]`): Spawning must be direct (no shell wrapper) via `CommandBuilder` to avoid command-line quoting/escaping bugs on Windows.
   - **Master Commands** (typically shell strings like `"claude"`): Since they may refer to command scripts (`.cmd`, `.bat`) or run shell utilities, they should be spawned explicitly under `cmd.exe /d /q /c "<command_string>"`.

---

## 3. Process Monitoring, Wait Registry, & Reaping (P0#2)

Process monitoring on Windows must reconcile with the Unix group-based reaping design (PGID `setpgid` + `kill(-pgid, SIGKILL)` in [research/orchestration-reliability-design.md](file:///home/sevenx/coding/ccbd-rust/research/orchestration-reliability-design.md) Section 4 Mechanism 2) while resolving the stubbed process APIs in [src/platform/windows/process.rs:35-50](file:///home/sevenx/coding/ccbd-rust/src/platform/windows/process.rs#L35-L50).

### 3.1. The PID Recycling Race, Registry Fence, & Handle Lifecycle (D2)
Windows process handles are kernel objects. The OS kernel is guaranteed to *never* reuse a process ID (PID) as long as at least one open `HANDLE` to that process remains in the system. 
However, looking up a process by calling `OpenProcess(..., pid)` *after* it has been spawned is race-prone: the process could have exited, and the OS could have already reassigned that PID to an unrelated program.

To eliminate this race, we introduce a thread-safe global **`ACTIVE_PROCESS_REGISTRY`** mapping `PID -> Raw HANDLE`.
1. When `WinPtyMultiplexer` spawns a process, it immediately obtains the child process `HANDLE` and its numeric `PID` from `CreateProcessW` / `portable-pty`.
2. It inserts the raw `HANDLE` into `ACTIVE_PROCESS_REGISTRY` mapping `PID -> Raw HANDLE`.
3. When [src/monitor/mod.rs](file:///home/sevenx/coding/ccbd-rust/src/monitor/mod.rs) calls `pidfd_open(pid)`:
   - The Windows implementation checks `ACTIVE_PROCESS_REGISTRY`. If the PID is found, it calls `DuplicateHandle` to create an independent, owned process handle for `MonitorHandle` and returns it. This guarantees that `ah` is monitoring the exact process it spawned.
   - If the PID is not in the registry (e.g., checking an external process), it falls back to `OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE, FALSE, pid)`.

#### 3.1.1. Handle Lifecycle & Fence Release across Exit Paths (D2)
To prevent memory leaks and indefinite PID fencing (which would block PIDs from reuse indefinitely), the registry must strictly bound the lifecycle of every process `HANDLE` across all exit paths:

1. **Clean Exit / Agent Crash (Normal Execution)**:
   - The FFI wait callback (or blocking wait thread) detects process termination via `WaitForSingleObject` signaling.
   - It fires and notifies the coordinator loop, which updates the agent's database status to a terminal state (e.g., `STATE_CRASHED` or `STATE_IDLE`).
   - The cleanup sequence in [src/agent_io/registry.rs:98-150](file:///home/sevenx/coding/ccbd-rust/src/agent_io/registry.rs#L98-L150) is called. It retrieves the raw handle from `ACTIVE_PROCESS_REGISTRY`, removes the entry, and calls `CloseHandle`. This terminates the PID fence, allowing the OS to reuse the PID.
2. **Unclean Exit / Orphaned Session (Missed Cleanup / Disconnects)**:
   - The reconciler loop (which runs every 5 seconds) queries the liveness of active database processes.
   - It checks the registry handle status: `WaitForSingleObject(handle, 0) == WAIT_OBJECT_0` indicates the process has exited.
   - If the process is dead but its cleanup was missed, the reconciler transitions the agent state to `CRASHED` in the database, removes the entry from `ACTIVE_PROCESS_REGISTRY`, and closes the raw `HANDLE`. This acts as an automated cleanup sweep, guaranteeing that the registry size is strictly bounded by active/running processes.
3. **Daemon Crash & Restart**:
   - A daemon crash (e.g., `kill -9`) automatically clears the entire `ACTIVE_PROCESS_REGISTRY` because it lives in the daemon's memory space. All open handles owned by the daemon process are closed by the OS kernel, releasing all PID fences.
   - Upon restart, the registry starts empty.
   - For each active session/agent loaded from the database, the daemon attempts to reopen the process via `OpenProcess` using the recorded PID *only if* the process is verified to be correct:
     - It checks if the process creation time matches the database `spawned_at` timestamp (preventing same-PID recycling hijack).
     - Or it checks if the process is a member of the reopened named Job Object `Local\ah-job-<scope_id>`.
     - If verified, it inserts the duplicated handle back into `ACTIVE_PROCESS_REGISTRY`, re-establishing the PID fence. If verification fails, the daemon triggers database cleanup and does not fence the recycled PID.

### 3.2. Asynchronous Wait Handle Safety (Memory-Safe Callback Context)
`RegisterWaitForSingleObject` runs a callback on a Win32 threadpool thread when the process handle is signaled.
The callback must send the exit code back to Tokio. If the parent task cancels the watch or drops the `MonitorHandle` while the callback is firing or about to fire, passing a raw pointer to a Rust state struct creates a use-after-free or double-free condition.

#### Safe Wait Mechanism
To prevent memory corruption, the wait registration context must use a thread-safe `Arc` and `Mutex` container:
```rust
struct WaitSharedState {
    tx: Option<tokio::sync::oneshot::Sender<i32>>,
}

struct MonitorWaitHandle {
    wait_handle: HANDLE,
    shared_state: Arc<Mutex<WaitSharedState>>,
}
```
1. **Callback Execution**: The FFI callback receives a raw pointer to `Arc::into_raw(shared_state.clone())`. It locks the mutex, extracts and consumes the `tx` sender, sends the process exit code (obtained via `GetExitCodeProcess`), and cleans up the Arc clone.
2. **Cancellation**: When `MonitorHandle` is dropped:
   - It calls `UnregisterWaitEx(wait_handle, INVALID_HANDLE_VALUE)`, which blocks until any pending callbacks complete.
   - It locks the mutex and clears the `tx` sender. If the callback did not run, the sender is dropped, safely closing the channel.
   - This ensures that under no circumstances does a callback access freed memory.

### 3.3. Job Objects, Suspended Spawning, & Daemon Revival (D1)
To contain and clean up process trees, Windows uses native **Job Objects**. Unlike Unix, where process groups are managed via PGID flags, Windows groups processes inside a Job kernel object.

#### 3.3.1. Reconciling Containment with the Revival Model (D1 Alignment)
A naive configuration setting `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` causes the OS to automatically terminate all processes in the Job when the last handle to the Job is closed. Because all open handles are automatically closed when a process exits, a daemon crash (e.g., `kill -9`) or a normal restart of `ahd` would instantly kill all running worker and master processes. This violates the core agent/master survival contract.

To ensure agents survive daemon restarts while remaining reapable on demand:
1. **Drop Close-on-Kill**: We explicitly do **not** set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
2. **Named Job Persistence**: We create each Job Object with a unique name, e.g., `Local\ah-job-<scope_id>` via `CreateJobObjectW`. In the Windows kernel, a named Job Object remains alive and active even if the daemon process closes its handle, so long as at least one member process is still running inside the Job.
3. **Explicit Reaping**: When `ahd` explicitly decides to reap or terminate an agent/master, it calls `TerminateJobObject(job_handle, exit_code)` against the Job Object, cascading the exit signal to all descendant processes in the tree.
4. **Suspended Spawn Sequence**: To prevent children from spawning grandchildren before assignment, the spawn sequence is:
   - Call `CreateProcessW` with `CREATE_SUSPENDED`.
   - Call `AssignProcessToJobObject(job_handle, child_process_handle)`.
   - Call `ResumeThread(main_thread_handle)`.
5. **Breakaway Control**: To ensure maximum containment, we do **not** set `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`. If specific developer tools (like `msbuild.exe`) require breakaway, we support configuring `JOB_OBJECT_LIMIT_BREAKAWAY_OK` (explicit breakaway) so they can run, while ensuring standard untracked orphans are reaped.

#### 3.3.2. Daemon Crash (`kill -9`) & Re-attach Lifecycle
When the `ahd` process is abruptly terminated (`kill -9`) and then restarted, the daemon must safely rebuild its in-memory process tracking and re-establish containment boundaries without misclassifying running agents as dead.

##### 1. The Authority Order
**Per-process liveness is the absolute, authoritative revival signal**. The recorded numeric PID, when verified against its creation timestamp, serves as the liveness oracle. 
Re-opening the Named Job Object via `OpenJobObjectW` is merely a low-cost **containment optimization** to cheaply re-associate processes; its success or failure does NOT indicate whether the underlying process tree is alive or dead. `OpenJobObjectW` failure must never, by itself, result in marking agents as dead or triggering database state cleanups.

##### 2. Re-connection & Robust Fallback Path
Upon restart, the daemon queries the SQLite database for active sessions/agents and executes the following recovery sequence:
1. **Attempt Optimization Re-open**: The daemon tries to open the named Job Object via `OpenJobObjectW(JOB_ALL_ACCESS, FALSE, name)`.
2. **Per-Process Liveness Fallback (Triggered on Open Failure or Verification)**:
   If `OpenJobObjectW` fails (returning `ERROR_FILE_NOT_FOUND`), the daemon must NOT conclude the process tree has died. It executes a fallback liveness check:
   - It reads the DB-recorded child `PID` and the microsecond-precision `spawned_at` timestamp.
   - It calls `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE | PROCESS_TERMINATE, FALSE, pid)`.
   - **PID Recycling Fence**: If `OpenProcess` succeeds, it queries the process creation time via `GetProcessTimes` and compares the `lpCreationTime` against the DB-recorded `spawned_at` timestamp.
     - **Match**: The process is confirmed as the correct, live child. The daemon inserts this verified handle back into `ACTIVE_PROCESS_REGISTRY` to re-fence the PID.
     - **Mismatch / Open Failure**: The process has either exited, or the PID has been recycled for an unrelated process. The daemon marks the agent as dead, skips registration, and schedules database cleanup.
3. **Restoring Containment**:
   If the fallback verified that the child process is alive but the Job Object was lost, the daemon creates a new named Job Object and calls `AssignProcessToJobObject` with the child's handle to restore tree containment (noting Windows 8+ nested job limits if the process is already associated with another untracked job).

##### 3. Empirical Spike Flag (M0.5 Task)
Whether a Named Job Object reliably survives the closure of the daemon's handle while its member processes are still active is a system-dependent behavior that must be empirically verified as part of the M0.5 conpty-spike task list. Until this behavior is verified on the target target runner version, the **Per-Process Liveness Fallback Path** is the safe, default recovery mechanism.

### 3.4. Reliability Alignment & Fail-Closed Ownership Gate (D3)
To reconcile the Windows process model with the frozen cross-platform reliability spec ([research/orchestration-reliability-design.md](file:///home/sevenx/coding/ccbd-rust/research/orchestration-reliability-design.md) Section 4 Mechanism 1 & 2), the daemon must enforce a strict **Provenance and Ownership Gate** before performing any destructive or termination operations (such as reaping, sending signals, or calling `TerminateJobObject`).

#### 3.4.1. The Fail-Closed Rule
A reap or kill command targeting an agent/session must fail-closed. If ownership of the target process cannot be definitively proven, the daemon MUST abort the reap operation, write a high-severity warning log, and leave the process untouched. This protects external host processes from accidental termination.

#### 3.4.2. Cross-Platform Provenance Mapping
The ownership verification mechanism is mapped across the three supported platforms as follows:
- **Linux**: The reconciler verifies that the systemd scope description contains `@{daemon_marker}` (representing the current daemon session) before calling `stop_unit` in [src/platform/linux/scope.rs:56](file:///home/sevenx/coding/ccbd-rust/src/platform/linux/scope.rs#L56).
- **macOS**: The reconciler verifies that the target process is registered in the in-memory `TMUX_PANE_MAP` in [src/agent_io/registry.rs:17-18](file:///home/sevenx/coding/ccbd-rust/src/agent_io/registry.rs#L17-L18) with a matching `expected_pid` and `socket_name` before calling pane-kill APIs.
- **Windows**: The reconciler verifies that the target process PID is present in `ACTIVE_PROCESS_REGISTRY` and belongs to the named Job Object `Local\ah-job-<scope_id>` owned by the current daemon.
If any of these platform-specific proofs are missing, the gate closes and termination is aborted.

---

## 4. Terminal & Pane Substrate: ConPTY Multiplexer (M2)

Because Windows has no native `tmux`, the daemon `ahd` must act as the multiplexer host using Windows ConPTY.

```mermaid
graph TD
    subgraph Client CLI (ah.exe)
        CLI[ah CLI]
    end
    subgraph Daemon (ahd.exe)
        NamedPipe[Named Pipe Server]
        Mux[WinPtyMultiplexer]
        VT[Virtual Terminal Emulator Grid]
        VT100[vt100::Parser]
        ConPTY[portable-pty / ConPTY Host]
    end
    subgraph Sandbox
        Cmd[Process Tree]
    end

    CLI -->|JSON-RPC: send_keys / capture_pane| NamedPipe
    NamedPipe --> Mux
    Mux -->|stdin write| ConPTY
    ConPTY --> Cmd
    Cmd -->|stdout/stderr stream| ConPTY
    ConPTY -->|ANSI stream| Mux
    Mux -->|Feed| VT
    Mux -->|Feed| VT100
```

### 4.1. The Gap Analysis: tmux vs. ConPTY
- **No Native `capture-pane`**: ConPTY is a transient pipe stream and has no memory of screen history. `ahd` must parse the stdout ANSI stream and maintain an in-memory 2D character grid buffer representing the screen.
- **No Native `send-keys`**: Windows lacks cross-process terminal key injection. The daemon must hold the `MasterPty` writer handle and write input bytes directly to the PTY stdin pipe.

### 4.2. Terminal Grid and ANSI Dual Plumbing
To ensure prompt detection and marker scanning ([src/agent_io/reader.rs:39-55](file:///home/sevenx/coding/ccbd-rust/src/agent_io/reader.rs#L39-L55)) function correctly:
1. **Grid Engine**: We use `alacritty_terminal`'s grid buffer to maintain screen lines, cursor states, and scrollback history. `capture_pane` reads the last 200 lines from this scrollback buffer.
2. **Dual-Plumbing**: The byte stream from the ConPTY reader is fed simultaneously to:
   - The `alacritty_terminal` parser to update the visual grid.
   - The existing `vt100::Parser` registered in [src/marker/parser_registry.rs](file:///home/sevenx/coding/ccbd-rust/src/marker/parser_registry.rs) to ensure prompt matches (`MatchResult` in [src/agent_io/reader.rs:180-213](file:///home/sevenx/coding/ccbd-rust/src/agent_io/reader.rs#L180-L213)) remain behaviorally consistent with Unix.

### 4.3. DSR Cursor-Position Query Responder (D6)
During startup, the Windows console host or command shells emit a Device Status Report (DSR) cursor query `\x1b[6n` to detect terminal size. If the PTY reader loop swallows this query and never replies, the shell execution stalls in initialization, causing commands to hang and teardowns to return `0xC000013A` (Control-C exit).

The ConPTY read loop in `ahd` must intercept DSR queries:
- Scan incoming byte chunks for `\x1b[6n`.
- Upon detection, write a cursor response back to the PTY writer. While a fixed `\x1b[1;1R` response is safe for initial shell detection, interactive CLI tools that query cursor locations can misbehave if coordinates mismatch. To ensure complete compatibility, the responder queries the `alacritty_terminal` grid cursor coordinates and dynamically formats the response as `\x1b[<row>;<col>R` (e.g. `\x1b[24;80R`).

---

## 5. Scope & Sandbox Isolation

To achieve functional parity with systemd scope sandboxing (such as `BindReadOnlyPaths` in [src/platform/linux/scope.rs:341-348](file:///home/sevenx/coding/ccbd-rust/src/platform/linux/scope.rs#L341-L348)), Windows-native agents must be isolated without requiring administrative privileges.

We implement **Restricted Token Job Objects**:
1. **Restricted Token Creation**:
   - Call `OpenProcessToken` on the current daemon process.
   - Call `CreateRestrictedToken`, specifying:
     - `DISABLE_MAX_PRIVILEGE` (disables administrative privileges).
     - Restricted SIDs (denying access to sensitive user directories and network resources).
2. **Job Security Assignment**:
   - Call `SetInformationJobObject` with `JobObjectSecurityLimitInformation`.
   - Pass the restricted token to the Job Object. Any process spawned inside or assigned to this job will automatically run under this restricted security boundary.
3. **Filesystem Sandboxing**:
   - Access control is enforced natively by Windows NTFS: the restricted token lacks access to directories outside the sandboxed workspace.
   - Write access to `%USERPROFILE%` is blocked, except for the explicit sandbox cache folder (e.g., `%LOCALAPPDATA%\ah\sandboxes\<id>`).

---

## 6. Process Identity & Environment Stamp (PR5) (D4)

The v1 release review confirmed that inherited environment variables like `AH_AGENT_ID` can leak from the daemon to the master or workers if they are not explicitly scrubbed from the spawned process environment.

### 6.1. The Map-Only Approach Failure & Correct Locus (D4)
The helper functions in [src/process_identity.rs:9-23](file:///home/sevenx/coding/ccbd-rust/src/process_identity.rs#L9-L23) operate exclusively on a `HashMap` (e.g., `inject_worker_identity(env: &mut HashMap<String, String>, ...)`). Simply removing a key from this `HashMap` (as done via `env.remove(AH_AGENT_ID)`) is insufficient; it only prevents the variable from being explicitly *added* by the map. It does not prevent the child process from *inheriting* the variable if it already exists in the daemon's own parent process environment.

Therefore, the correct locus for scrubbing inherited environment variables is the **Spawn Command Builder Boundary** at the scope and command assembly layer:
- **On Windows**: Inside the multiplexer spawner (e.g., `WinPtyMultiplexer`), when constructing the `CommandBuilder` or `std::process::Command` before execution.
- **On Linux/macOS**: In the scope/command builder layer ([src/platform/linux/scope.rs](file:///home/sevenx/coding/ccbd-rust/src/platform/linux/scope.rs) and [src/platform/macos/scope.rs](file:///home/sevenx/coding/ccbd-rust/src/platform/macos/scope.rs)), when building the arguments array or systemd properties.

### 6.2. The Identity Scrub Mechanism
At the Spawn Command Builder boundary, we perform the following steps:
1. **Inherited Variable Scrubbing**: Call `Command::env_remove` (or `CommandBuilder::env_remove` on Windows, and prepending `env -u KEY` on Unix scopes where applicable) for all three identity keys:
   - `AH_AGENT_ID`
   - `AH_ROLE`
   - `AH_SESSION_ID`
2. **Explicit Inject Overrides**:
   - For a **Master** process: set `AH_ROLE = "master"` and `AH_SESSION_ID = session_id` via the builder env method. Do not set `AH_AGENT_ID`, ensuring it remains completely scrubbed.
   - For a **Worker** process: set `AH_ROLE = "worker"`, `AH_SESSION_ID = session_id`, and `AH_AGENT_ID = agent_id` via the builder env method.
This ensures that even if `ahd` was launched from a terminal environment containing these environment variables, they are never leaked to child processes.

---

## 7. Packaging, Installation, & Service Persistence

### 7.1. Non-Admin Task Scheduler Persistence
Because registering a Windows Service via the Service Control Manager (SCM) requires administrative privileges (causing UAC prompts), the user daemon persistence is implemented using **Windows Task Scheduler COM APIs** in [src/platform/windows/service.rs](file:///home/sevenx/coding/ccbd-rust/src/platform/windows/service.rs):
1. **COM Apartment Configuration**: Since Task Scheduler is COM-based, COM operations must run on a dedicated thread using `CoInitializeEx(None, COINIT_MULTITHREADED)`.
2. **Task Definition**:
   - Create a Logon Trigger (`ITriggerCollection::Create(TriggerTypeLogon)`) tied to the active user's SID.
   - Create an Action (`IActionCollection::Create(ActionTypeExecute)`) pointing to `ahd.exe` with arguments specifying the state directory (`AH_STATE_DIR`).
   - Configure Restart Settings via `ITaskSettings` (`put_RestartCount(3)` and `put_RestartInterval("PT1M")`) to auto-restart the daemon on crash.
3. **Hashed Task Names**: The task is registered under a hashed folder path `\ah\ahd-<hash>` to support multiple users running independent daemon instances.

### 7.2. Statically Compiled Release Binaries Verification (3.8.1) (D5)
To ensure the "one-line install" runs on clean Windows machines without requiring manual software installation, we verify that the existing static configuration in `Cargo.toml` and `Cargo.lock` correctly covers the Windows target:
- **SQLite Verification**: The target block in `Cargo.toml` already configures `rusqlite = { version = "0.32", features = ["bundled"] }` under `[target.'cfg(windows)'.dependencies]`, which compiles SQLite from source and links it statically.
- **TLS Stack Verification**: The Cargo lockfile already utilizes a pure-Rust TLS stack (`rustls` with `ring`), ensuring no runtime dependency on OpenSSL dynamic libraries (`.dll`) is introduced on Windows.
No additional implementation work is required for binary static bundling; verification of target builds is completed.

---

## 8. Implementation Phases & Action Plan

We prioritize the development roadmap by ranking tasks according to what actually blocks a functional Windows `ah` build.

### Phase 1: Win32 Adaptors & Primitives (Milestone M1)
- **Task 1: Command Spawn Fix**: Adjust `platform/windows/scope.rs` to output `"env"` prefixes, and implement env-stripping in process spawning.
- **Task 2: Handle Wait & PID Registry**: Implement `ACTIVE_PROCESS_REGISTRY` to prevent PID reuse races, and code the safe `Arc<Mutex>` FFI wait callback wrapper.
- **Task 3: Job Objects Process Containment**: Implement process spawning with `CREATE_SUSPENDED`, assign to Job Object, and resume main thread.
- **Task 4: Task Scheduler COM Persistence**: Implement `ITaskService` COM helper functions for user service install/uninstall.
- **Task 5: Named Pipe IPC**: Implement the Named Pipe transport seam in `rpc/mod.rs` and `cli/rpc_client.rs` with current-user DACL ACL constraints.

### Phase 2: ConPTY Multiplexer MVP (Milestone M2)
- **Task 1: ConPTY Host Spawning**: Integrate `portable-pty` for Master/Slave PTY management.
- **Task 2: Screen Buffer & Dual Plumbing**: Feed ConPTY output into both the `alacritty_terminal` grid buffer (for `capture-pane`) and the `vt100::Parser` (for prompt matching).
- **Task 3: Keystroke Injection**: Wire JSON-RPC keysym/enter calls to write directly to the MasterPty stdin pipe.
- **Task 4: DSR Scanner**: Intercept console query `\x1b[6n` and respond with `\x1b[1;1R` to prevent shell boot hang.

### Phase 3: Advanced Features & Parity (Milestone M3+)
- **Task 1: Interactive Attach**: Implement client-daemon VT stream proxying for `ah attach`.
- **Task 2: Resize Propagation**: Pass client terminal window resize signals to the ConPTY PTY sizing API.
- **Task 3: Recovery / State Re-attach**: Re-read process state and reconstruct terminal buffers upon daemon restart.
