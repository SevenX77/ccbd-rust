# Changelog

All notable changes to `ah` are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.3.4] - 2026-07-06

### Added
- `ah events` runtime snapshots now include a `starting` runtime_state for the
  cold-start window before master/worker tmux runtime has been recorded.
  Consumers such as Studio should clean up only `degraded` runtimes; `starting`
  means startup is still in progress and must be left alone.

### Fixed
- Claude workers spawned into an ah sandbox HOME with
  `--dangerously-skip-permissions` now receive `IS_SANDBOX=1` directly from the
  daemon's provider spawn path, so sandbox identity no longer depends on the
  harness config template carrying a duplicate `[env] IS_SANDBOX` entry.

## [1.3.3] - 2026-07-06

### Fixed
- `ah events` no longer exits when the daemon closes the subscription stream
  (`ah stop` or a daemon restart). It now emits a local inactive snapshot so
  consumers see the runtime go down, then keeps reconnecting — a GUI
  supervisor would otherwise freeze on the last active snapshot. The local
  fingerprint resets after a live connection so the down-edge is never
  dedup-suppressed, while pure connect-failure loops stay quiet.

## [1.3.2] - 2026-07-06

### Added
- `CLAUDE_CODE_OAUTH_TOKEN` joined the daemon env passthrough whitelist, so a
  host launcher can hand a long-lived `claude setup-token` credential to the
  daemon and every master/worker it spawns inherits it — without persisting
  the token into config files, the sqlite inventory, or spawn-cmd logs.

### Fixed
- `ah events` no longer filters runtime inventory by the config file's parent
  directory. Sessions record the project's absolute path (the `ah start`
  cwd), while the config may live elsewhere (Studio keeps transient configs
  under the OS temp dir), so the filter matched nothing and every snapshot
  reported an inactive runtime even while master and workers were alive.
  The daemon's state dir is already scoped to the config, so the
  subscription reports that daemon's full inventory.

## [1.3.1] - 2026-07-06

### Added
- `ah events --format json`, a stable runtime lifecycle event source for
  GUI and service integrations. The command writes an initial full snapshot,
  then full JSONL snapshots whenever ahd inventory, master tmux, or worker
  tmux state changes.
- Runtime snapshot schema v1 with ahd inventory, tmux socket/server health,
  master liveness, worker liveness, session summaries, and agent summaries.

### Changed
- Runtime state changes are now broadcast from daemon-owned paths: session
  inventory, master runtime, worker lifecycle, recovery, and state machine
  transitions. Clients can subscribe instead of polling `ah ps` or probing
  tmux directly.
- If ahd is absent, `ah events` emits an inactive snapshot and keeps retrying
  the daemon stream.

## [1.3.0] — 2026-07-05

### Added
- `ah tell master "<text>"` — an async command for the operator to send an
  instruction to the master agent. It delivers into the master's pane and
  returns immediately without blocking on the master's turn. Master
  observability is now first-class: a `UserPromptSubmit` hook flips
  `master_state` to `BUSY` (a real "started working" signal, not merely
  "delivered") and a `Stop` hook flips it back to `IDLE`; both events are
  written to the daemon log and `master_state` is surfaced by `ah ps`.
- Studio provisioning for Windows/WSL2 — PowerShell provisioning that
  enables WSL2, installs the distro, runs an in-distro `ah` install and
  first-launch checks, with idempotent re-runs and bare-invocation resume.
- Configurable installer landing directory via `AH_INSTALL_DIR`.
- Opt-in tmux "follow terminal" sizing.
- Windows compile scaffolding (M0) and a ConPTY spike. Foundational only —
  the runtime still targets Linux and Windows-via-WSL2; native Windows is
  not yet shipped.

### Fixed
- Dispatch-ACK race that could leave a job marked DISPATCHED while its
  prompt was never delivered, then later misjudged as STUCK.
- Health-check false-positive STUCK for tasks that were long-running but
  still alive.
- Studio handoff: the default master command is now plain `claude`, and
  no-config socket resolution is isolated to avoid ambient cwd state.

## [1.2.0] — 2026-07-02

### Added
- Plugin bundle system completed across providers — antigravity bundle
  adaptation plus the bundle CLI and bundle-aware realign/recovery, so a
  project's skills/hooks/plugins are materialized into each provider's
  native layout on spawn and re-aligned on `ah up`.

### Fixed
- Antigravity premature completion — a deferred background-work gate now
  prevents a worker from being reported COMPLETE before its real work
  (including post-response background tasks) has actually finished.

### Changed
- `agent.notify` Stop-hook receipts are now logged (both receive and
  outcome), so daemon logs show whether a provider's completion push
  actually fired — previously an invisible blind spot during incidents.

## [1.1.0] — 2026-07-02

### Added
- Plugin/skill bundle foundation — agent skills injected from `ah.toml`,
  the Claude plugin-bundle spine, cross-provider MCP translation, and
  Codex bundle adaptation.
- macOS groundwork — a platform abstraction layer (OS-specific behavior
  moved behind traits) and a kqueue-based process watcher. Release binaries
  remain Linux-only; native macOS support is on the roadmap.
- Windows (WSL2) onboarding preflight checks.
- README — Requirements table and a full Windows (WSL2) setup guide.

### Fixed
- Completion-detection fallbacks hardened.
- A revived master now resolves its Claude config directory correctly.

## [1.0.0] — 2026-07-01

First public release. `ah` is a Linux-native L2 orchestration daemon
(`ahd`) and CLI (`ah`) for running multiple AI agent CLIs — Codex, Claude,
Antigravity, or an explicit shell provider — in isolated tmux-backed
workspaces. The daemon owns state, sessions, workers, recovery, and event
streams; the CLI drives it over JSON-RPC on a Unix socket.

[1.3.1]: https://github.com/SevenX77/ah/releases/tag/v1.3.1
[1.3.0]: https://github.com/SevenX77/ah/releases/tag/v1.3.0
[1.2.0]: https://github.com/SevenX77/ah/releases/tag/v1.2.0
[1.1.0]: https://github.com/SevenX77/ah/releases/tag/v1.1.0
[1.0.0]: https://github.com/SevenX77/ah/releases/tag/v1.0.0
