# Antigravity CLI (agy 1.1.0) Hooks Research

This document outlines the findings regarding the hook extensibility framework supported by the Antigravity CLI (`agy` v1.1.0 / v1.0.7) to address premature turn-end and self-polling issues with background tasks.

---

## 1. Supported Hook Events (Question 1)

Based on the official hooks reference documentation and Go binary symbol inspection, `agy` supports the following hook events:

1. **`SessionStart`**: Runs when a new session is started or an existing session is resumed.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:67](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L67), [docs/agent-cli-knowledge-base/codex/hooks-official.md:285](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L285).
2. **`PreToolUse`**: Intercepts tool calls (e.g. `Bash` commands, `apply_patch` edits, and MCP tool calls) before they execute.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:79](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L79), [docs/agent-cli-knowledge-base/codex/hooks-official.md:311](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L311).
3. **`PermissionRequest`**: Invoked when the agent is about to ask for approval/permission (e.g., shell escalations).
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:91](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L91), [docs/agent-cli-knowledge-base/codex/hooks-official.md:366](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L366).
4. **`PostToolUse`**: Runs after a tool executes, exposing the tool response/output to the hook.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:103](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L103), [docs/agent-cli-knowledge-base/codex/hooks-official.md:423](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L423).
5. **`UserPromptSubmit`**: Invoked when the user submits a new prompt but before the agent processes it.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:115](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L115), [docs/agent-cli-knowledge-base/codex/hooks-official.md:479](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L479).
6. **`Stop`**: Fires at the end of every conversation turn when the agent is about to stop and yield to the user.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:125](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L125), [docs/agent-cli-knowledge-base/codex/hooks-official.md:517](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L517).

### Binary Symbol Verification
Go Symbol dump of `/home/sevenx/.local/bin/agy` confirms the existence of the core hook handling structures:
- `jsonhook.ParseHooksFile`
- `jsonhook.(*Caller).CallHook`
- `jsonhook.(*HookHandler).Execute`
- `jsonhook.JSONHookSpec.IsEnabled`
- *Source Citation*: Symbol table extraction from `/home/sevenx/.local/bin/agy`.

---

## 2. Hook Veto/Blocking Capabilities (Question 2)

Hooks are **not** observe-only; they can actively veto, block, or force continuation of the agentic loop.

1. **`PreToolUse` / `PermissionRequest` Veto**:
   - The hook can block a tool call by writing a specific JSON shape to `stdout` containing `{"decision": "block", "reason": "..."}` or returning `permissionDecision: "deny"`.
   - Alternatively, it can exit with code `2` and write the failure reason to `stderr`.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:338-360](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L338-L360).
2. **`PostToolUse` Interception**:
   - The hook can return `{"decision": "block", "reason": "..."}` or `continue: false` to replace the tool result with custom feedback and force the model to proceed from the hook's feedback message.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:451-474](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L451-L474).
3. **`Stop` Hook Turn Block & Continuation**:
   - At the turn-end `Stop` event, if the hook outputs `{"decision": "block", "reason": "[reason]"}` or exits with code `2` (writing reason to `stderr`), the agent will **not** yield the turn to the user. Instead, the CLI automatically generates a new continuation prompt using `reason` as the prompt text and forces the agent to start a new turn to process it.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:532-547](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L532-L547).

---

## 3. Config Entry Point (Question 3)

Hooks for `agy` are configured through two primary mechanisms:

1. **`hooks.json` File**:
   - Located in the `.gemini/config/` directory inside the agent sandbox or the repository root.
   - *Active Config Path*: `~/.gemini/config/hooks.json` (or `.gemini/config/hooks.json` relative to the sandbox home root).
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:33-47](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L33-L47), [research/a1-antigravity-hook-research.md:42](file:///home/sevenx/coding/ccbd-rust/research/a1-antigravity-hook-research.md#L42).
2. **`config.toml` Inline Table**:
   - Inline `[[hooks.Stop]]` configurations inside the core `config.toml` file.
   - *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:150-173](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L150-L173).

### Required Feature Gate
`agy` requires hooks to be explicitly enabled via a configuration feature flag. Without this, the hooks will be quietly ignored (`skipping hooks.json`).
- **`config.json` / `settings.json` Gate**: The setting `"enableJsonHooks": true` must be present.
- **`config.toml` Gate**:
  ```toml
  [features]
  codex_hooks = true
  ```
- *Source Citation*: [docs/agent-cli-knowledge-base/codex/hooks-official.md:18-23](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md#L18-L23), [research/a1-antigravity-hook-research.md:15](file:///home/sevenx/coding/ccbd-rust/research/a1-antigravity-hook-research.md#L15).

### Concrete Configuration Example
To register a `Stop` hook via `~/.gemini/config/hooks.json`:
```json
{
  "hooks": {
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "python3 ~/.gemini/config/hooks/block_turn_end.py",
            "timeout": 30
          }
        ]
      }
    ]
  }
}
```

And in `~/.gemini/config/config.json`:
```json
{
  "enableJsonHooks": true
}
```

---

## 4. Feasibility Verdict (Question 4)

### Verdict: ~~FULLY FEASIBLE~~ — SUPERSEDED, see §7 (INFEASIBLE on this agy build; Codex-borrowed semantics refuted)

### Explanation
We can block turn-end while background tasks are still running. The CLI's native `Stop` event triggers at the end of every turn. If the hook command blocks, it halts turn completion and forces the agent to continue.

### Hook Mechanism Design Sketch
1. **Event Registration**: Register a command hook on the `Stop` event in `~/.gemini/config/hooks.json`.
2. **Task State Check**: The registered hook script (e.g., `block_turn_end.py`) does the following:
   - Queries the daemon's active tasks directory or database to check for running tasks.
   - Alternatively, checks if there are any active `.output` log files or running PIDs associated with the active session.
3. **Turn-End Blocking**:
   - **Active Tasks Found**: The script outputs the continuation block JSON payload on `stdout` and exits `0`:
     ```json
     {
       "decision": "block",
       "reason": "Wait for pending background tasks to finish..."
     }
     ```
     This instructs `agy` to start a new turn immediately, using `"Wait for pending background tasks to finish..."` as a new user prompt. The model will then run another turn (meaning it won't prematurely end the turn).
   - **No Active Tasks**: The script outputs `{}` and exits `0`, allowing `agy` to finish the turn and yield to the user.

---

## 5. Bonus: Auto-Feeding Task Outputs (Question 5)

The `Stop` hook surface **can** cure the self-polling/polling loop flaw.

### Mechanism
Rather than having the agent poll the status of background tasks, the `Stop` hook script can intercept the completion event:
1. When checking active tasks, if a task has transitioned from running to completed (e.g., exit code file generated), the `Stop` hook script reads the trailing lines of the task's output log.
2. It constructs the `reason` payload including the output of the completed task:
   ```json
   {
     "decision": "block",
     "reason": "Internal task [task-id] completed with output:\n\n```\n[Tail of output logs]\n```\nPlease evaluate these results."
   }
   ```
3. This injects the completed task output directly into the model's next turn prompt automatically. The agent receives the execution results without ever having to call `ManageTask(status)` or run a custom polling loop.

---

## 6. Viewed Files & Citations Reference

- **[hooks-official.md](file:///home/sevenx/coding/ccbd-rust/docs/agent-cli-knowledge-base/codex/hooks-official.md)**: Full specification of Codex/agy hooks events, config shapes, matchers, and input/output JSON schemas.
- **[a1-antigravity-hook-research.md](file:///home/sevenx/coding/ccbd-rust/research/a1-antigravity-hook-research.md)**: Previous binary and empirical audit logs confirming the `enableJsonHooks` gate requirement.
- **`/home/sevenx/.local/bin/agy` (Go Binary)**: Direct string symbol references confirming hook structures (`jsonhook.JSONHookSpec.IsEnabled`, etc.).
- **[home_layout.rs](file:///home/sevenx/coding/ccbd-rust/src/provider/home_layout.rs)**: Logic inside `ccbd-rust` verifying layout directories and symlinks created for hook setups.

---

## 7. Empirical Spike Findings: Turn-Continuation Viability (Round 2)

### 7.1 Spike Setup & Execution
To determine if `agy`'s `Stop` hook can successfully force a turn-continuation, we ran an empirical spike under an isolated sandbox (`$SPIKE_HOME` at `/home/sevenx/coding/ccbd-rust/agyspike_home`).
- **Feature Gate Activation**: Configured `"enableJsonHooks": true` in both `.gemini/config/config.json` and `.gemini/antigravity-cli/settings.json`.
- **Hook Configuration**: Mapped the `Stop` event in `hooks.json` to `/home/sevenx/coding/ccbd-rust/agyspike_home/hook.sh`.
- **Test Payloads**: Tested outputs containing `{"decision": "block", "reason": "..."}` (Codex schema) as well as `{"decision": "continue", "reason": "..."}` (schema matching strings found in the `agy` binary).

### 7.2 Key Findings & Evidence
1. **Successful Hook Pick-up**:
   The `agy` CLI successfully loads the configuration and logs the command execution:
   `I0709 11:05:06.564745 518546 json_hook_caller.go:145] JSON hook "jsonhook__hooks_Stop_0_0": executing command`
2. **Asynchronous Shell Execution (The Critical Defect)**:
   By configuring `hook.sh` to perform a `sleep 10` delay, we measured the execution time of the parent `agy` process:
   ```bash
   time HOME="/home/sevenx/coding/ccbd-rust/agyspike_home" agy --print "say done and stop"
   ```
   **Result**: The command completed successfully in **4.85 seconds**, shutting down the CLI store manager and exiting:
   ```text
   I0709 11:05:39.798963 json_hook_caller.go:145] JSON hook "jsonhook__hooks_Stop_0_0": executing command
   ... (1.16 seconds later) ...
   I0709 11:05:40.962379 manager.go:621] CLI store manager shutting down
   ```
   This is definitive empirical proof that **`agy` does not wait for the `Stop` hook script to complete**. It spawns the hook command in a background goroutine and immediately proceeds with shutdown and process termination.
3. **Silently Aborted Side-Effects**:
   Because `agy` exits immediately, the operating system kills the orphaned hook subprocess (or the process is terminated prematurely before completing). Consequently:
   - Any disk-based indicator files (e.g. `touch stop_hook_fired` or writing to a log file) are never executed.
   - Any background RPC calls (e.g. `ah agent notify`) are cut short, explaining the zero-RPC delivery issue recorded in `dogfood-evidence.md`.
4. **Impossibility of Hook-Driven Turn Continuation**:
   Since `agy` does not wait for the hook subprocess to finish, it is physically unable to read the script's `stdout`. Therefore, the output JSON `{"decision": "continue"}` or `{"decision": "block"}` has zero effect on the execution loop, and `agy` cannot be forced to continue the turn via the `Stop` hook interface.

### 7.3 Feasibility Verdict Update
- **Verdict**: **NOT VIABLE** (Reversing the Round 1 Verdict).
- **Reasoning**: The asynchronous lifecycle of `agy`'s native hook runner prevents the `Stop` hook from blocking turn-completion or returning any decision data to the engine.

### 7.4 Recommended Fallback Design
Given that the "block premature turn-end" solution via `Stop` hooks is dead, we recommend the following design directions:
1. **UDS/IPC Interception at Tool Level (`PreToolUse` / `PostToolUse`)**:
   Unlike the `Stop` hook, `PreToolUse` and `PostToolUse` hooks are *synchronous* blockers because the agent cannot proceed with tool execution without receiving a decision. If `ahd` intercepts tool usage, it can withhold tool execution or response completion while background tasks are running.
2. **`ah`-Side Monitor & UI-Pull Fallback**:
   Continue relying on the `ah`-side monitor to scan the workspace and agent logs. If the agent yields while tasks are still running, the hypervisor retains a `RUNNING` session state and issues a resume command (`agy` continuation) to the provider once background tasks complete or when new results need to be fed back, bypassing hook-level control entirely.


---

## 7. Empirical Verification (SUPERSEDES the §4 "FULLY FEASIBLE" verdict)

> Added by master 2026-07-09. §1–§3 (agy HAS a `jsonhook` framework, `enableJsonHooks`+`hooks.json`, agy-binary-confirmed) stand. But §2/§4's `{"decision":"block"}` → turn-continuation semantics were cited from `codex/hooks-official.md` (OpenAI **Codex**, not Antigravity) and are **REFUTED by agy-native evidence** from a prior 3-source root-cause investigation (a1 disassembly + a4 audit + master black-box, `/tmp/a1-agy-hook-rootcause.md`, `/tmp/a4-audit-agy-rootcause.md`). Today's isolated re-spike (`agyspike_home/`) did not complete cleanly; the prior investigation is authoritative and more rigorous.

### Verdict (Q4): **INFEASIBLE on this agy build** — a Stop hook cannot block turn-end.

**Mechanism (agy-native, verified):**
1. agy loads `hooks.json` and logs `json_hook_caller.go:144] … executing command` — the hook IS recognized.
2. But `jsonhook.(*HookHandler).Execute` runs the command as: `context.WithTimeoutCause(...)` → `os/exec.CommandContext(ctx, "sh", "-c", <command>)` → `.Output()`. (a1 disassembly at CallHook `0x65df32c`/Execute `0x65e0cd8→0x65e0d40→0x65e0f8c`; a4 audit APPROVED.)
3. The Stop hook fires **mid-generation** (~88–120 ms after a `streamGenerateContent` starts, NOT at turn-end), on a context tied to the generation step that is **cancelled near-instantly** → the hook subprocess is SIGKILL'd **before it runs to completion** → it never produces stdout, so agy never reads a `{"decision":...}` payload; agy swallows the cancel (no `command failed` log).
4. Master black-box tried **5 command forms** (env-prefix, absolute path, no-arg wrapper script, `setsid … &` detached, original) against a live agy — ALL logged "executing command" with **zero spawn / zero effect**; the identical command runs 100% manually.

**Consequence for the anti-false-completion goal:** the "block turn-end while an internal task is pending via a Stop-hook `decision:block`" scheme **cannot work** on this agy build — the hook can't reliably execute a command, let alone return a decision agy would honor, and it fires mid-generation rather than at turn-end. The Codex model does not port.

### Fallback (the actionable path)
Because the hook layer is proven dead here, the fix must be **AH-SIDE, not an agy hook**:
- **ah completion-detection must not treat a turn-end as job-complete when the reply/pane shows a pending-background-task signature** (e.g. `"I will wait for…"`, `"yield to wait"`, an active `ManageTask`/`… running · N task(s)`): keep the agent BUSY and re-prompt "continue", instead of latching the narration as a completed reply.
- This is exactly the perception-layer **G2** problem (turn-end ≠ task complete) already addressed by the frozen orchestration-reliability design (explicit-done protocol / evidence-gated completion + artifact-less watchdog). The antigravity false-completion is a concrete instance to fold into that work.
- (Neutralizing agy's `ManageTask` background tool would also help, but that is an agy-config lever, and hook-based enforcement is confirmed non-viable.)
