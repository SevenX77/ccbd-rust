# ah Dual Deploy

## Boundary

- `ccb` stays unchanged and keeps using the upstream Python daemon/state.
- `ah` runs ccbd-rust `target/release/ah`.
- `ccbd-rs` runs ccbd-rust `target/release/ccbd`.
- `ah` uses the project `ah.toml` to derive a per-project state directory.
- ccbd-rust state lives under `~/.local/state/ah/<project_id>/` by default.
- RPC socket: `~/.local/state/ah/<project_id>/ccbd.sock`.
- tmux socket name is derived from that state dir, so it is separate.

## Install

```bash
(cd ~/coding/ccbd-rust && cargo build --release --all-targets)
bash ~/coding/ccbd-rust/scripts/install_ah.sh
```

Wrappers install to `~/.local/bin/ah` and `~/.local/bin/ccbd-rs`; `AH_BIN_DIR` overrides the install directory.
`AH_HOME` overrides the binary directory; default:

`/home/sevenx/coding/ccbd-rust/target/release`

## Use

Rust canary:

```bash
cd projectX && ah start && ah ps
```

Existing Python tool:

```bash
cd projectX && ccb start
```

Switch per project by changing directory and using either `ccb` or `ah`.

## Limitation

Do not run both tools against the same project at the same time. Their daemon state is separate, but the project files and provider sessions are shared workspace context.

## Upgrade

```bash
(cd ~/coding/ccbd-rust && cargo build --release --all-targets)
```

## Uninstall

```bash
rm ~/.local/bin/ah ~/.local/bin/ccbd-rs
```
