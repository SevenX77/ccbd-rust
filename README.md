# ccbd-rust

Rust rewrite of the [Claude Code Bridge (CCB)](https://github.com/bfly123/claude_code_bridge) daemon — a global multi-agent CLI orchestrator for AI-driven autonomous development.

This repository implements **L2 (the scheduling layer)** of a three-tier spec-driven development engine:

- **L3** — orchestration / spec pipeline (separate repo, Python, future)
- **L2** — Rust ccbd daemon (this repo)
- **L1** — agent CLIs (codex / claude / gemini, external)

## Status

Phase 2 startup. **No working binary yet.** Design only.

See [docs/DESIGN.md](docs/DESIGN.md) for the full architecture rationale, motivation for rewriting from Python, and milestone plan.

## Why a rewrite?

The Python predecessor accumulated ~7 patches in 6 days around isolation / lifecycle / kill issues — symptoms of "shotgun surgery" architectural debt. Health score 35/100 ([per a 4-round Gemini 3.1 Pro architecture review](docs/DESIGN.md#9-references)). Root cause: no central source of truth, file-system state masquerading as a database. This rewrite uses SQLite as SoT, Rust's ownership model as a compile-time race detector, and a Kubernetes-style reconciliation loop instead of a fragile external systemd timer.

## Quick start (developer)

```bash
# install rustup if missing
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"

# clone + build
git clone <this repo>
cd ccbd-rust
cargo build

# run in dev mode (state under target/dev_state/, socket under target/dev_sockets/)
CCB_ENV=dev cargo run

# tests
cargo test
```

## Layout

```
ccbd-rust/
├── docs/
│   ├── DESIGN.md      ← architecture document (start here)
│   └── (more contracts to come)
├── src/
│   └── main.rs        ← currently a `cargo init` stub
├── tests/
└── Cargo.toml
```

## Roadmap (Phase 2 milestones)

1. **M1** SQLite SoT
2. **M2** Tmux-backed subprocess management
3. **M3** JSON-RPC interface (spawn/kill/status)
4. **M4** Reconciliation loop on startup
5. **M5** STUCK detection + last_token_at
6. **M6** Auth-share + sandbox-isolation port from Phase 1-D Python
7. **M7** Cutover from old Python CCB

Target: 2-3 days at AI vibecoding pace.

## License

TBD (private project for now).

## Acknowledgments

- Original Python CCB by [bfly123](https://github.com/bfly123/claude_code_bridge)
- Architecture review and Rust pivot decision: 4-round consultative review with Gemini 3.1 Pro (2026-04-25 → 04-26)
