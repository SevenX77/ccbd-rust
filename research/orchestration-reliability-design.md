# ah Orchestration & Perception Layer Reliability: A First-Principles Re-design
**Author**: Antigravity, Lead Designer  
**Date**: 2026-07-09

---

## 1. Executive Summary & Design Thesis

The operator-drafted background document [perception-layer-first-principles.md](file:///home/sevenx/coding/ccbd-rust/research/perception-layer-first-principles.md) correctly identifies the five key states a hypervisor must deterministically track (F1–F5). However, its proposed solution — a linear signal hierarchy (T0 > T1 > T2 > T3), an imperative job completion protocol, and an event-driven outbox delivery model (R1–R5) — is built on assumptions that are fragile under real-world runtime conditions. 

Relying on transient events and imperative worker tool calls to drive state machine transitions introduces severe new failure modes. Specifically, it makes orchestration correctness dependent on LLM syntactic obedience, assumes platform-specific process behaviors that do not hold on macOS, and introduces a complex outbox journaling mechanism that cannot run reliably in sandboxed, transient sandboxes.

### The Rebel Thesis
Instead of attempting to construct a bulletproof, event-driven message queue on top of ephemeral sandboxes, we pivot from an **Event-Driven Reactive Model** to a **Declarative State Reconciler & Multi-Modal Sensor Fusion Model**. Rather than trusting individual signal channels or demanding absolute tool-call obedience from LLMs, the hypervisor must continuously reconcile desired state against the physical reality of the sandbox (OS process trees, I/O streams, and filesystem artifacts).

---

## 2. Critique of the Operator's "North Star"

### Critique 1: The Imperative Done Protocol (`ah job done`) is a LLM-Behavioral Failure Trap (F3 / G2 / R2)
*   **The Operator's Stance**: G2/R2 claims that task completion (F3) must be solved by worker agents explicitly calling `ah job done <id>` and that the system should block LLMs from finishing if this is not called.
*   **The First-Principles Refutation**: This is an imperative action trap. It couples the hypervisor's state machine to the LLM's behavioral and syntactic accuracy. If the agent runs out of tokens (context exhaustion), experiences API rate limits, hallucinates the tool name, or enters an infinite loop, it will never call `ah job done`. Under the operator's design, this triggers a false-positive watchdog alarm ("stopped without declaring done"), halting the pipeline.
*   **Real Code Grounding**: In [state_machine.rs:L1223](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L1223), the codebase implements `evidence_denial_for_job`. Rather than verifying the filesystem directly, it queries DB-recorded evidence flags (inserted via RPC `handle_evidence_insert` in [evidence.rs:L9](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/evidence.rs#L9)) like `mtime_changed`, `diff_generated`, or `test_passed`. If the hooks/events recording these flags are lost or dropped, the DB check fails.
*   **Corrected Mechanism**: We must classify jobs (see Section 3.2). For jobs that produce changes, the hypervisor-side Reconciler must perform a direct filesystem scan (e.g. `git status` or file ctime checks on the workspace) to generate local evidence. For artifact-less tasks, we must support fallback completions triggered by `end_turn` combined with watchdog alarms.

### Critique 2: "T0 OS Truth" (pidfd) is a Narrow, Platform-Bound Illusion (F1 / T0)
*   **The Operator's Stance**: The background document designates T0 (pidfd/cgroup scope/exit code) as the absolute, unforgeable source of truth for F1 (agent process alive/dead).
*   **The First-Principles Refutation**: A pidfd only monitors the immediate child process wrapper spawned by the daemon. However, agents routinely execute sub-processes (compilers, servers, remote runners) that escape the parent. If the wrapper process crashes or exits prematurely, the pidfd watcher confirms it as `Dead` and triggers cleanup, but orphan children remain running, locking files and binding ports.
*   **Real Code Grounding**: In [agent_watch.rs:L33-L66](file:///home/sevenx/coding/ccbd-rust/src/monitor/agent_watch.rs#L33-L66), the pidfd readiness task loops on `AsyncFd::readable()`. Once dead, it immediately calls `mark_agent_crashed_with_exit` in [agents_lifecycle.rs:L143](file:///home/sevenx/coding/ccbd-rust/src/db/agents_lifecycle.rs#L143) (async wrapper at [agents_lifecycle.rs:L399](file:///home/sevenx/coding/ccbd-rust/src/db/agents_lifecycle.rs#L399)) and invokes `cleanup` in [agent_watch.rs:L105](file:///home/sevenx/coding/ccbd-rust/src/monitor/agent_watch.rs#L105). This leaves orphaned sub-processes un-reaped because systemd scopes are not stopped on this path, and macOS has no systemd support.
*   **Correct Mechanism**: F1 must track the entire **Process Group (PGID)** or Session ID (SID) rather than just the root PID. Reaping must recursively kill the group or cgroup.

### Critique 3: Hook Outbox (G1/R1) assumes Client-Side Persistence and Blocks Execution
*   **The Operator's Stance**: R1 introduces a hook-side outbox with ACK and journaling to guarantee delivery.
*   **The First-Principles Refutation**: Hooks are executed as short-lived, transient CLI wrapper calls within the agent's sandboxed environment. If `ahd` is restarting or down, a transient CLI process cannot perform a reliable retry loop without blocking the shell execution and causing timeouts. 
*   **Durable Boundary (Sandbox Persistence)**: The agent home layout in [home_layout.rs:L1632](file:///home/sevenx/coding/ccbd-rust/src/provider/home_layout.rs#L1632) is cached under `.cache/ah/sandboxes` and is preserved during the agent's lifetime and across recovery-eligible crashes. Therefore, a filesystem journal in the sandbox is durable against daemon crashes. However, G1 is genuinely lossy because:
    1.  The HTTP/RPC `handle_agent_notify` endpoint in [agent.rs:L745](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/agent.rs#L745) returns synchronously, but if `ahd`'s socket queue is full or down, connection timeouts fail the hook.
    2.  The internal message dispatcher uses the broadcast channel `EVENT_FRAMES.send(frame)` in [pubsub.rs:L55](file:///home/sevenx/coding/ccbd-rust/src/orchestrator/pubsub.rs#L55), which is fire-and-forget.
*   **Correct Mechanism**: Treat T1 hooks as an optimistic fast-path notification, but rely on a continuous hypervisor-side reconcile loop as the fallback. Instead of making the client-side reliable, make the server-side resilient to lost events.

### Critique 4: The T3 Pane Diff "No Lifecycle Inference" Ban is a Logical Contradiction (F4 / G3 / R3)
*   **The Operator's Stance**: G3/R3 states that pane text diffs must only drive known dialogs and never infer lifecycle state, due to past prompt-scanner bugs.
*   **The First-Principles Refutation**: For many standard CLI tools (e.g. interactive git ssh prompts, apt confirmations), there is no structured IPC or log event. The terminal pane is the *only* output channel. If we ban lifecycle inference from T3, we cannot detect when the agent is waiting for interactive input (F4). The agent will remain in `BUSY` until it is terminated by the watchdog timer.
*   **Real Code Grounding**: In [state_machine.rs:L286](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L286), the state machine updates the agent's state to `PROMPT_PENDING`. Pane-diff stuck scans are processed at [state_machine.rs:L436-L488](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L436-L488).
*   **Correct Mechanism**: T3 pane diffs must be kept for lifecycle state inference, but they must follow a strict corroboration rule (see Section 3.3).

---

## 3. The Proposed Architecture: Declarative State Reconciler & Multi-Modal Sensor Fusion

We propose replacing the reactive, event-driven model with a declarative loop modeled after the Kubernetes Controller pattern.

```mermaid
graph TD
    subgraph Hypervisor Daemon (ahd)
        DB[(State Database)]
        Reconciler[Continuous Reconcile Loop]
        StateFS[Multi-Modal Sensor Fusion Engine]
    end
    
    subgraph Sandbox Environment
        AgentProc[Agent Process Tree / PGID]
        FIFO[FIFO Stdout Stream]
        FS[Filesystem Diffs & Artifacts]
    end

    Reconciler -->|1. Query Desired State| DB
    Reconciler -->|2. Pull Actual State| StateFS
    StateFS -->|T0: Waitid/PGID check| AgentProc
    StateFS -->|T2: Read Stream/Log tail| FIFO
    StateFS -->|T1: Verify Evidence/Tests| FS
    Reconciler -->|3. Converge & Reconcile| DB
    Reconciler -->|4. Clean/Reap PGID| AgentProc
```

### 3.1 The Continuous Reconcile Loop
Instead of only performing startup reconciliation in [system.rs:L526](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs#L526), a background task runs continuously inside `ahd` (e.g., every 5 seconds). It targets all agents in non-terminal states (`STATE_BUSY`, `STATE_WAITING_FOR_ACK`, `STATE_PROMPT_PENDING`).

For each target agent, it gathers all active sensors:
1.  **OS Tree Probe (T0)**: Checks if the PGID (Linux/macOS) contains active processes.
2.  **I/O Idle Probe (T2)**: Checks stdout FIFO stream modification time and CPU usage.
3.  **Physical Evidence Probe (T1)**: Runs direct local checks (e.g., checking workspace directory via host-side `git status` or scanning file modification times since `dispatched_at`) to verify physical artifacts.
4.  **UI Pane Scanner (T3)**: Scans terminal buffers for interactive prompt indicators.

### 3.2 Classified Job Completion Model
To support both artifact-producing and conversational/artifact-less tasks (resolving D3):

1.  **Evidence-Gated Jobs**: Require physical evidence (code changes, test runs). 
    *   *Completion Rule*: Reconciler validates filesystem changes (T1). If the required changes are present, it auto-resolves the job to completed and transitions the agent to `IDLE` (even if the `stop` hook was dropped).
2.  **Artifact-less / Conversational Jobs**: (Q&A, reviews). 
    *   *Completion Rule*: Relies on `end_turn` or explicit `ah job done` tool call as an optimistic trigger.
    *   *Watchdog Rule*: If the agent stops outputting text (FIFO is quiet) and remains in `BUSY` without declaring completion, the Reconciler triggers a watchdog `WARN` (e.g., `STUCK`) and notifies the operator instead of auto-failing the job immediately.

### 3.3 T3 Pane Softening Corroboration Rule (Resolving D4-b)
To prevent "ghost text" from incorrectly trapping agents in `PROMPT_PENDING`:
*   **The Corroboration Rule**: A T3 "prompt detected" match is ignored unless it is corroborated by T0 (Process Group state is sleeping/idle, e.g. blocked on syscall read) AND T2 (stdout FIFO has seen zero writes for at least `ui_completion_stable_ticks`). An uncorroborated T3 hint is discarded and the agent remains in `BUSY`.

---

## 4. Safety-Critical Mechanisms & Code Grounding

### Mechanism 1: Exact Marker Ownership & Provenance Gate (Resolving D1)
To prevent a sibling or newly started daemon from accidentally killing or modifying running agents (which occurred in the 6 live agent stack crashes):
1.  **Provenance Gate**: Integrate with the PR4 provenance flags `DaemonMarkerProvenance { Explicit, Ambient }`. Any destructive state transitions or `stop-class` actions (killing PGIDs, stopping systemd scopes) must be rejected if the daemon's active provenance is `Ambient`.
2.  **Identity Match**: Before executing a reap:
    *   For Linux: The reconciler verifies that the target scope description contains `@{daemon_marker}` (ensuring it was spawned by this daemon session).
    *   For macOS/Windows: The reconciler verifies that the target process is registered in the in-memory `TMUX_PANE_MAP` in [registry.rs:L111](file:///home/sevenx/coding/ccbd-rust/src/agent_io/registry.rs#L111) with a matching `expected_pid` and `socket_name`.
    *   **Anti-Recycling Proof**: To prevent PID wrap-around (reaping a recycled process PID), we compare the ctime of `/proc/<pid>` against the agent's database `spawned_at` timestamp. If they mismatch, the reap is aborted.
        > [!IMPORTANT]
        > **Prerequisite Schema Change (Required Additive Migration)**:
        > The current database schema lacks the `spawned_at` column. Implementing this anti-recycling check requires a schema migration to add `spawned_at` to the `agents` table:
        > - **Column Name**: `spawned_at`
        > - **Type/Source**: `INTEGER` (storing UTC microsecond-precision epoch timestamp).
        > - **Capture Mechanism**: Captured via the host's authoritative system clock (`std::time::SystemTime`) immediately after the process is spawned (before yielding control to the agent loop).
        > This is a required additive migration; the current schema does not have this column, so downstream implementation must treat this as real database schema change work.

### Mechanism 2: Process Tree Reaping & Cross-Platform Ownership (Resolving D2)
To clean up orphaned sub-processes safely:
*   **On Linux**: The reconciler invokes `stop_unit` in [scope.rs:L24](file:///home/sevenx/coding/ccbd-rust/src/platform/linux/scope.rs#L24) during cleanup in [registry.rs:L106](file:///home/sevenx/coding/ccbd-rust/src/agent_io/registry.rs#L106). Since systemd scopes are described as `ccbd-agent-{agent_id}@{daemon_marker}`, this reaps the entire cgroup scope.
*   **On macOS / Unix Fallback**: 
    1.  At agent spawn time, processes are launched in their own process group via `setpgid(0, 0)` (or using `process_group(0)` command extensions in Rust).
    2.  During cleanup, the reconciler verifies ownership (Mechanism 1) and sends `SIGKILL` to the negative process group ID (`kill(-pgid, SIGKILL)`). This terminates all descendant processes spawned within that agent group.

### Mechanism 3: Prevent Concurrency Race via State Version CAS
Because the Reconciler, the Hook RPC handler, and the Pidfd Watcher run asynchronously, they can race to write agent state.
*   **Code Location**: [state_machine.rs:L933](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L933).
*   **Mechanism**: Every state modification query must include `state_version` CAS gating:
    ```sql
    UPDATE agents 
    SET state = ?state, state_version = state_version + 1, updated_at = unixepoch() 
    WHERE id = ?id AND state_version = ?expected_version;
    ```
    If `changes == 0`, the CAS failed (state changed concurrently). The event is swallowed or retried, preventing race conditions between the continuous Reconciler and reactive hooks.

### Mechanism 4: FS-Scanner Freshness Gating (Preventing Premature Completion)
To eliminate the "premature-completion" failure family where stale or partial filesystem artifacts from prior steps false-complete a working agent, we enforce strict temporal validation on all scanned evidence:
1.  **Clock Source & Authoritative Reference**:
    - The database-recorded `dispatched_at` timestamp of the current job is the authoritative source of truth for the dispatch moment.
    - All evidence is compared against this timestamp. Evidence is considered fresh if and only if it is proven to post-date this moment.
    - The reference timestamp is compared against:
      - The filesystem modification time (`mtime`) of the generated artifact.
      - The database insert timestamp (`inserted_at`) of the corresponding evidence record.
2.  **Mtime Precision & Same-Second Race Resolution**:
    - Granularity varies by filesystem (e.g., ext4 supports sub-second/nanosecond resolution, but older file systems or virtualized sandbox mount layers may truncate to whole seconds).
    - To prevent same-second races, the Reconciler evaluates both raw filesystem `mtime` and DB `inserted_at` timestamps. If a conflict/race is possible (same-second timestamps), the Reconciler relies on the DB-recorded evidence insert time (`inserted_at`), which is generated by the database engine's transaction log.
    - **Gating Constraint**: The decision rule requires a strictly greater-than constraint (`>`) instead of a greater-than-or-equal (`>=`). The evidence timestamp $T_{\text{evidence}}$ must satisfy:
      $$T_{\text{evidence}} > T_{\text{dispatch}}$$
      Any evidence where $T_{\text{evidence}} \le T_{\text{dispatch}}$ is classified as stale and ignored.
3.  **Timezone & Clock-Domain Pitfalls**:
    - **UTC Epoch Normalization**: To avoid timezone mismatches, the DB `dispatched_at`, DB `inserted_at`, and file `mtime` must all be normalized to UTC microsecond-precision epochs. Local timestamps are banned.
    - **Daemon-vs-Sandbox Clock Skew**: If the agent's sandbox runs in a separate clock domain (e.g., virtualized VM or remote container), guest-kernel clock drift can corrupt `mtime`. To remain correct under skew:
      - The daemon evaluates filesystem `mtime` exclusively from the host operating system's view of the mounted filesystem, ignoring any guest-generated file timestamps.
      - If remote or virtualized clocks are inevitable, a safety skew window $\epsilon_{\text{drift}}$ (e.g., 1 second) is added to the dispatch timestamp:
        $$T_{\text{evidence}} > T_{\text{dispatch}} + \epsilon_{\text{drift}}$$
        preventing same-second clock-skew leaks.
4.  **The Decision Rule**:
    - Scanned evidence counts toward job completion **only if** its authoritative timestamp is strictly after the current job's dispatch moment.
    - If the timestamp is older or equal, the evidence is ignored. The agent remains in the `BUSY` state, allowing the task to either proceed or eventually trigger the watchdog timer.

---

## 5. Revised Reconstruct Road Map (R1'-R5')

We reverse the reconstruction order. Instead of building a complex hook transport (R1) first, we build the state reconciliation core which handles all signal losses by default.

### Phase 1 (R1'): Process Group Tracking & Continuous Reconciler Core
*   Implement PGID-based liveness tracking inside [agent_watch.rs](file:///home/sevenx/coding/ccbd-rust/src/monitor/agent_watch.rs).
*   Write the continuous `reconcile_active_agents_loop` inside the orchestrator.
*   *Acceptance Criteria*: Terminating the daemon, spawning orphan child processes, or restarting `ahd` results in zero zombie processes and automatic recovery of state representation within 5 seconds.

### Phase 2 (R2'): Classified Evidence-Gated Completion (F3)
*   Integrate direct filesystem and log scanners into the Reconciler.
*   Support fallback completions for conversational/artifact-less tasks.

### Phase 3 (R3'): Multi-Modal Sensor Fusion Engine with T3 Corroboration
*   Redefine `pane_diff` as a speculative state provider that injects state hints into the Reconciler.
*   Enforce the T3 corroboration rule to prevent ghost text locks.

### Phase 4 (R4'): Fast-Path Hook Optimization
*   Implement simple, non-blocking hooks. Hook failure is treated as a minor latency penalty (the system will take up to 5 seconds to reconcile via the background loop) rather than a system failure.

### Phase 5 (R5'): Unified Telemetry
*   Collect token usage, context limits, and process metrics via the reconcile loop.
