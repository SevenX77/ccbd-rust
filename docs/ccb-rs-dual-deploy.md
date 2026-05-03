# ccb-rs Dual Deploy

## Boundary

- `ccb` stays unchanged and keeps using the upstream Python daemon/state.
- `ccb-rs` runs ccbd-rust `target/release/ccb`.
- `ccbd-rs` runs ccbd-rust `target/release/ccbd`.
- `ccb-rs` sets `XDG_STATE_HOME=$PWD/.ccb-rs`.
- ccbd-rust state lives in `$PWD/.ccb-rs/ccbd`.
- RPC socket: `$PWD/.ccb-rs/ccbd/ccbd.sock`.
- tmux socket name is derived from that state dir, so it is separate.

## Install

```bash
(cd ~/coding/ccbd-rust && cargo build --release --all-targets)
bash ~/coding/ccbd-rust/scripts/install_ccb_rs.sh
```

Wrappers install to `~/.local/bin/ccb-rs` and `~/.local/bin/ccbd-rs`; `CCB_RS_BIN_DIR` overrides the install directory.
`CCBD_RS_HOME` overrides the binary directory; default:

`/home/sevenx/coding/ccbd-rust/target/release`

## Use

Rust canary:

```bash
cd projectX && ccb-rs start && ccb-rs ps
```

Existing Python tool:

```bash
cd projectX && ccb start
```

Switch per project by changing directory and using either `ccb` or `ccb-rs`.

## Limitation

Do not run both tools against the same project at the same time. Their daemon state is separate, but the project files and provider sessions are shared workspace context.

## Upgrade

```bash
(cd ~/coding/ccbd-rust && cargo build --release --all-targets)
```

## Uninstall

```bash
rm ~/.local/bin/ccb-rs ~/.local/bin/ccbd-rs
```
