#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Install side-by-side ccbd-rust wrappers without touching an existing Python ccb.

set -euo pipefail

bin_dir="${CCB_RS_BIN_DIR:-/home/sevenx/.local/bin}"
default_home="/home/sevenx/coding/ccbd-rust/target/release"

mkdir -p "$bin_dir"

cat >"${bin_dir}/ccb-rs" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail

ccbd_rs_home="${CCBD_RS_HOME:-/home/sevenx/coding/ccbd-rust/target/release}"
ccb_bin="${ccbd_rs_home}/ccb"
ccbd_bin="${ccbd_rs_home}/ccbd"

if [ ! -x "$ccb_bin" ] || [ ! -x "$ccbd_bin" ]; then
  echo "ccb-rs: ccbd-rust release binaries not found in ${ccbd_rs_home}" >&2
  echo "ccb-rs: first run cargo build --release" >&2
  exit 127
fi

state_home="${CCB_RS_STATE_HOME:-${PWD}/.ccb-rs}"
log_path="${state_home}/ccbd-rs.log"

export XDG_STATE_HOME="$state_home"
unset CCB_ENV
unset CCB_SOCKET

needs_daemon=1
case "${1:-}" in
  version|config)
    needs_daemon=0
    ;;
esac

socket_path="${state_home}/ccbd/ccbd.sock"

daemon_ready() {
  [ -S "$socket_path" ] && "$ccb_bin" ping >/dev/null 2>&1
}

if [ "$needs_daemon" -eq 1 ] && ! daemon_ready; then
  mkdir -p "$state_home"
  "$ccbd_bin" >"$log_path" 2>&1 &
  for _ in $(seq 1 80); do
    if daemon_ready; then
      break
    fi
    sleep 0.1
  done
  if ! daemon_ready; then
    echo "ccb-rs: ccbd-rust daemon did not become ready at ${socket_path}" >&2
    echo "ccb-rs: log: ${log_path}" >&2
    exit 1
  fi
fi

exec "$ccb_bin" "$@"
WRAPPER

cat >"${bin_dir}/ccbd-rs" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail

ccbd_rs_home="${CCBD_RS_HOME:-/home/sevenx/coding/ccbd-rust/target/release}"
ccbd_bin="${ccbd_rs_home}/ccbd"

if [ ! -x "$ccbd_bin" ]; then
  echo "ccbd-rs: ccbd-rust release binary not found in ${ccbd_rs_home}" >&2
  echo "ccbd-rs: first run cargo build --release" >&2
  exit 127
fi

export XDG_STATE_HOME="${CCB_RS_STATE_HOME:-${PWD}/.ccb-rs}"
unset CCB_ENV
unset CCB_SOCKET

exec "$ccbd_bin" "$@"
WRAPPER

chmod +x "${bin_dir}/ccb-rs" "${bin_dir}/ccbd-rs"

cat <<EOF
Installed:
  ${bin_dir}/ccb-rs
  ${bin_dir}/ccbd-rs

Default CCBD_RS_HOME:
  ${default_home}
EOF
