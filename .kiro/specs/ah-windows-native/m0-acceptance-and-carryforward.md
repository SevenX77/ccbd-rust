# M0 Acceptance & M1/M2 Carry-Forward

Status: **M0 compile gate ACHIEVED** — `cargo check --all-targets --target x86_64-pc-windows-msvc` green in CI (PR#91, merged to main `377a1d7`). Convergence: 3 lib errors → 2 test-surface `geteuid` misses → green (3 CI round-trips).

## Acceptance verdict (a2 audit, master-verified)

- **53-item Unix-only inventory (m0-spec §5.1): fully covered, no silent skips.** nix moved to `[target.'cfg(unix)'.dependencies]`; monitor rows via `MonitorHandle`/`BorrowedMonitorHandle`; db/rpc/agent rows cfg-gated/stubbed.
- **`src/platform/windows/*` stubs: signature-complete, correct shape.** service exports `ServiceUnitError`/`escape_systemd_env_value`/`escape_systemd_exec_token`; scope `RecoverySpawn`/`ScopeUnit` fields + `SystemctlRunner` aligned to Linux; `MonitorHandle`/`BorrowedMonitorHandle` placeholder types present.
- **CI gate present:** `.github/workflows/ci.yml` windows-msvc-check with the spec command.

M0 is **done for its purpose** (compile gate). The items below are **M1/M2 entry-work**, not M0 defects — deferred deliberately because they are better built with the real implementation in hand than speculatively.

## Carry-forward (tracked, do NOT silently drop)

### → M1 (Win32 adaptors)

1. **IPC transport seam** — `src/rpc/mod.rs:26` Windows entry returns `Unsupported`; Unix accept/dispatch loop is entirely inside `#[cfg(unix)]` (`rpc/mod.rs:35`), `bind_rpc_listener` returns `UnixListener` (`rpc/mod.rs:107`); CLI `RpcClient` trait exists but Windows `rpc_call`/`rpc_stream_first` return `Unsupported` (`cli/rpc_client.rs:119`). **M1 task:** extract the shared connection/dispatch logic into a transport seam and slot in a Named Pipe listener. (Deferred from spec M0-5 on purpose — building the abstraction without the Named Pipe impl risks the wrong seam.)
2. **`MonitorHandle::try_clone()` ownership** — `src/platform/windows/process.rs:22` is `Ok(Self { raw: self.raw })` (raw copy). Harmless in M0 (`pidfd_open` always `Err`, never executed). **M1 task:** when real `OpenProcess`/owned HANDLE lands, change to `DuplicateHandle` or shared ownership to avoid double-close/dangling.
3. **`Win32_System_TaskScheduler` feature** — missing from `Cargo.toml` windows-sys features (has Foundation/Security/Threading/JobObjects/Com/Console/Storage_FileSystem). Unused in M0. **M1 task (service):** add the feature as a prerequisite of the Task Scheduler COM adaptor.

### → M2 (ConPTY multiplexer)

4. **Agent IO stream reader boundary** — `src/agent_io/reader.rs:39` still takes a `File`; Windows path is a no-op task (`reader.rs:46`). Doesn't affect M1. **M2 task:** re-split reader signature for ConPTY stream capture (per m0-spec M0-7 "future-compatible stream reader boundary").
5. **`test_no_legacy_pty_dependency` guard narrowing** — `tests/mvp6_acceptance.rs:173` still the old substring guard. Not triggered (M0 added no root portable-pty). **M2 task (before adding root Windows PTY dep):** implement the intent-aware option-(a) whitelist per m0-spec §11, else M2's Windows PTY dep will trip the old guard.

## Operational note

- **dispatch-ACK cancel-race incident** (this session): cancelling a busy worker's job left a subsequently-queued job stuck in QUEUED (never DISPATCHED) while the work sat uncommitted in the tree. Recovery: cancel stuck job → new short "verify+commit+push, don't re-implement" task. Reliability backlog candidate: the dispatch-ACK hardening (PR#89) does not cover the cancel-race path.
