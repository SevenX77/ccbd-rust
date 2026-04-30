#!/usr/bin/env bash
# MVP9 E2E smoke (launcher path):
# - ccb config validate examples/ccb.toml
# - ccbd start fresh, then ccb start --config to launch 3 bash agents
# - ccb ps shows the launched session (table rendering)
# - ccb doctor exits 0
#
# NOTE: in CCBD_UNSAFE_NO_SANDBOX=1 mode, spawned processes are NOT ccbd's
# children (they're tmux's), so ccbd's pidfd_watch immediately fires "agent
# pidfd ready" and the agents quickly transition to KILLED state. This is
# expected for the unsafe path — production usage runs under bwrap +
# systemd-run scope where the child relationship is correct, and the full
# cancel/kill lifecycle is exercised in tests/mvp9_acceptance.rs.
#
# This smoke only validates that the CLI launcher path emits the right
# session.create + N agent.spawn + session.apply_layout RPC calls.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STATE_DIR="$ROOT/target/dev_state"
SOCKET="$STATE_DIR/ccbd.sock"
LOG="/tmp/ccbd-mvp9-smoke.log"
PROJECT="/tmp/ccbd-mvp9-smoke-project"
CONFIG="$PROJECT/ccb.toml"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

mkdir -p "$STATE_DIR" "$PROJECT"
rm -f "$SOCKET" "$LOG" "$STATE_DIR"/ccbd.sqlite*

cat > "$CONFIG" <<'TOML'
version = "1"
layout = "grid"

[env]
SMOKE = "mvp9"

[agents.b1]
provider = "bash"

[agents.b2]
provider = "bash"

[agents.b3]
provider = "bash"
TOML

cargo build --release --quiet --bin ccbd --bin ccb
CCB="$ROOT/target/release/ccb"

echo "[smoke] ccb config validate ..."
"$CCB" config validate --config "$CONFIG"

echo "[smoke] starting ccbd ..."
CCB_ENV=dev CCBD_UNSAFE_NO_SANDBOX=1 "$ROOT/target/release/ccbd" >"$LOG" 2>&1 &
DAEMON_PID=$!

for _ in {1..40}; do
  [[ -S "$SOCKET" ]] && break
  sleep 0.25
done
[[ -S "$SOCKET" ]] || { cat "$LOG" >&2; exit 1; }
echo "[smoke] ccbd up at $SOCKET"

echo "[smoke] ccb doctor ..."
CCB_ENV=dev CCBD_UNSAFE_NO_SANDBOX=1 "$CCB" doctor

echo "[smoke] ccb start --config $CONFIG ..."
START_OUT=$(CCB_ENV=dev "$CCB" start --config "$CONFIG")
echo "$START_OUT"

# Verify the start output mentions session_id, layout, and 3 agents
echo "$START_OUT" | grep -q "session_id="  || { echo "FAIL: no session_id"; exit 1; }
echo "$START_OUT" | grep -q "layout=grid"  || { echo "FAIL: layout not grid"; exit 1; }
echo "$START_OUT" | grep -q "agent_id=b1"  || { echo "FAIL: b1 not started"; exit 1; }
echo "$START_OUT" | grep -q "agent_id=b2"  || { echo "FAIL: b2 not started"; exit 1; }
echo "$START_OUT" | grep -q "agent_id=b3"  || { echo "FAIL: b3 not started"; exit 1; }

echo "[smoke] ccb ps (table rendering check) ..."
CCB_ENV=dev "$CCB" ps | grep -q "b1\|b2\|b3" || { echo "FAIL: agents missing from ps"; exit 1; }

echo "[smoke] MVP9 launcher E2E OK ✓"
echo "  - config validate: PASS"
echo "  - ccbd boot: PASS"
echo "  - doctor: PASS"
echo "  - ccb start (3 agents + grid layout): PASS"
echo "  - ccb ps: PASS"
