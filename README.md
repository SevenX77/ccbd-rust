# ah

`ah` is an L2 orchestration daemon and CLI for running multiple AI agent CLIs in isolated tmux-backed workspaces. The daemon (`ahd`) owns state, sessions, workers, recovery, and event streams; the CLI (`ah`) drives it over JSON-RPC on a Unix socket.

Use it when you want one project-level control plane to coordinate provider agents such as Codex, Claude, Antigravity, or an explicit shell provider.

## Install

v1 installs from source with Cargo:

```bash
cargo install --git https://github.com/SevenX77/ccbd-rust --bin ah --bin ahd
```

This requires a Rust toolchain. To build from a checkout:

```bash
cargo build --release
```

The binaries are written to:

```text
target/release/ah
target/release/ahd
```

Prebuilt release archives and a `curl ... | sh` installer are planned for the cargo-dist release chunk. There is no prebuilt release URL documented in this chunk.

## Minimal Project

Create `ah.toml` in a project directory:

```toml
version = "1"

[agents.a1]
provider = "codex"

[agents.a2]
provider = "claude"
```

Start the project:

```bash
ah start
```

`ah start` locates `ah.toml`, starts `ahd` if needed, creates a session, and spawns the configured agents. Use `--config <path>` to point at a specific config file and `--wait` to wait until agents are ready:

```bash
ah --config ./ah.toml start --wait
```

`ahd` can also be run directly, but the normal entry point is `ah start` because it performs daemon bootstrap and then drives the project config.

## CLI

Implemented top-level commands:

```text
ah ping
ah version
ah ps
ah start [--wait]
ah up [--force]
ah ask <agent_id> <text> [--wait] [--request-id <id>]
ah pend <job_id>
ah cancel <job_id>
ah kill <target_id> [--session] [--force]
ah watch <agent_id> [--since-event-id <id>]
ah logs <agent_id> [--since <id>]
ah attach <target> [subject] [--session <session_id>]
ah stop
ah doctor
ah config validate --config <path>
ah config migrate
ah prompt resolve <agent_id> [--action <value> | --keys <value>] [--save-to-kb]
ah master cutover [--wait] [--print-attach]
ah master ack-ready [--cutover-id <id>]
ah agent notify --agent-id <id> --event <event> [--provider <name>] [--event-id <id>] [--hook-json] [--hook-debug-log <path>] [--socket <path>]
```

Common workflow:

```bash
ah start --wait
ah ask a1 "Inspect the failing test and propose the smallest fix" --wait
ah ps
ah watch a1
ah logs a1
ah attach agent a1
```

## `ah.toml`

The config schema is defined by `src/cli/config.rs`.

Top-level fields:

| Field | Type | Notes |
|---|---|---|
| `version` | string | Must be `"1"`. |
| `agents` | table | Required. At least one `[agents.<id>]` entry. |
| `master` | table | Optional. Defaults are applied when missing. |
| `completion` | table | Optional. |
| `daemon` | table | Optional, currently empty. |
| `env` | table of strings | Optional project environment values. |
| `sandbox` | table | Optional sandbox settings. |

Agent fields:

```toml
[agents.a1]
provider = "codex"

[agents.a1.env]
FOO = "bar"
```

| Field | Type | Notes |
|---|---|---|
| `provider` | string | Required. Valid values: `codex`, `claude`, `antigravity`, `bash`. Misspellings are hard errors. |
| `env` | table of strings | Optional extra environment for the agent. |
| `hooks` | table | Optional provider hook config. |
| `plugins` | array of strings | Optional provider plugin names. |

Master fields:

```toml
[master]
enabled = true
cmd = "claude --dangerously-skip-permissions --continue /remote-control"
readiness_timeout_s = 120
plugins = []
```

| Field | Type | Notes |
|---|---|---|
| `enabled` | bool | Defaults to `true`. |
| `cmd` | string | Defaults to `claude --dangerously-skip-permissions --continue /remote-control`. Empty string normalizes to `claude`. |
| `provider` | optional string | Present in config parsing, but v1 master spawning still uses Claude for the sandbox rules path. |
| `readiness_timeout_s` | integer | Defaults to `120`. |
| `hooks` | table | Optional. |
| `plugins` | array of strings | Optional. |

Completion fields:

```toml
[completion]
hook_push_enabled = false
```

Sandbox fields:

```toml
[sandbox]
additional_ro_binds = ["/opt/tools"]
```

## Editable Agent Rules

ah injects a rules document into each provider home at sandbox preparation time. The source is:

```text
[fixed ah coordination kernel] + [project .ah/rules/<slot-id>.md if present, otherwise built-in default]
```

The slot id is the agent id from `ah.toml`, such as `a1` or `a2`. The master slot id is `master`.

Provider destinations are selected automatically:

| Provider | Injected file |
|---|---|
| `claude` | `.claude/CLAUDE.md` |
| `antigravity` | `.gemini/AGENTS.md` |
| `codex` | `.codex/AGENTS.md` |

Editable examples in this repository:

```text
.ah/rules/master.md
.ah/rules/a1.md
```

Built-in defaults live at:

```text
assets/builtin/defaults/master.md
assets/builtin/defaults/worker.md
```

To customize an agent, create a file matching its slot:

```bash
mkdir -p .ah/rules
$EDITOR .ah/rules/a1.md
```

ah always prepends its fixed coordination kernel, so project rules can focus on scenario-specific behavior.

## Provider Names

Valid provider names are:

```text
codex
claude
antigravity
bash
```

`bash` is a real explicit provider. Unknown provider names such as `claud` or `coddex` fail config validation and do not silently fall back to bash.

## Integration Model

External integrators typically:

1. Write an `ah.toml` with one `[agents.<id>]` table per slot.
2. Add `.ah/rules/<slot-id>.md` files to define scenario-specific behavior.
3. Start the daemon/session with `ah start`.
4. Drive work through the CLI (`ah ask`, `ah pend`, `ah watch`, `ah logs`, `ah ps`, `ah attach`) or by speaking JSON-RPC to the Unix socket used by `ah`.

The daemon stores state in SQLite under the resolved ah state directory and uses tmux for provider panes. The CLI is the supported public control surface for v1.

## License

TBD.
