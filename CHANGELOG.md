# Changelog

All notable changes to `ah` are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

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

[1.2.0]: https://github.com/SevenX77/ah/releases/tag/v1.2.0
[1.1.0]: https://github.com/SevenX77/ah/releases/tag/v1.1.0
[1.0.0]: https://github.com/SevenX77/ah/releases/tag/v1.0.0
