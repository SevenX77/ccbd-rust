#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Install side-by-side ccbd-rust wrappers without touching an existing Python ccb.

set -euo pipefail

bin_dir="${AH_BIN_DIR:-/home/sevenx/.local/bin}"
default_home="/home/sevenx/coding/ccbd-rust/target/release"

mkdir -p "$bin_dir"

cat >"${bin_dir}/ah" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail

ah_home="${AH_HOME:-/home/sevenx/coding/ccbd-rust/target/release}"
ah_bin="${ah_home}/ah"
ccbd_bin="${ah_home}/ccbd"

if [ ! -x "$ah_bin" ] || [ ! -x "$ccbd_bin" ]; then
  echo "ah: ccbd-rust release binaries not found in ${ah_home}" >&2
  echo "ah: first run cargo build --release" >&2
  exit 127
fi

unset CCB_ENV
unset CCB_SOCKET

exec "$ah_bin" "$@"
WRAPPER

cat >"${bin_dir}/ccbd-rs" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail

ah_home="${AH_HOME:-/home/sevenx/coding/ccbd-rust/target/release}"
ccbd_bin="${ah_home}/ccbd"

if [ ! -x "$ccbd_bin" ]; then
  echo "ccbd-rs: ccbd-rust release binary not found in ${ah_home}" >&2
  echo "ccbd-rs: first run cargo build --release" >&2
  exit 127
fi

unset CCB_ENV
unset CCB_SOCKET

exec "$ccbd_bin" "$@"
WRAPPER

chmod +x "${bin_dir}/ah" "${bin_dir}/ccbd-rs"

cat <<EOF
Installed:
  ${bin_dir}/ah
  ${bin_dir}/ccbd-rs

Default AH_HOME:
  ${default_home}
EOF
