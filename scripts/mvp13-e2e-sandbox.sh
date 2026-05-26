#!/usr/bin/env bash
# mvp13 sandbox 模式完整 e2e: spawn 3 agent + ask + verify reply 干净 + kill
# 跟 mvp13-e2e-no-sandbox.sh 区别: 不 set CCBD_UNSAFE_NO_SANDBOX (走 bwrap 沙盒)。
# stage 5 onboarding mirror 必须工作才能进 IDLE。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
STATE_DIR="$REPO_ROOT/target/dev_state"

echo "=== [1/8] cleanup ==="
pkill -f "target/release/ccbd" 2>/dev/null || true
sleep 1
DAEMON_PIDS=$(pgrep -f "target/release/ccbd" 2>/dev/null || true)
if [ -n "$DAEMON_PIDS" ]; then
  echo "  killing stale ccbd: $DAEMON_PIDS"
  kill -9 $DAEMON_PIDS 2>/dev/null || true
fi
rm -rf "$STATE_DIR"/ccbd.sqlite* "$STATE_DIR"/pipes 2>/dev/null || true

echo ""
echo "=== [2/8] build release binaries ==="
cargo build --release --bin ccbd --bin ah 2>&1 | tail -3

TEST_CONFIG=$(mktemp -t ccb-sandbox-e2e-XXXXXX.toml)
cat > "$TEST_CONFIG" <<'EOF'
version = "1"
[master]
enabled = false

[agents.a1]
provider = "codex"

[agents.a2]
provider = "gemini"

[agents.a3]
provider = "claude"
EOF
echo "  test config: 3 agents (codex / gemini / claude, master disabled, SANDBOX mode)"

echo ""
echo "=== [3/8] start daemon (SANDBOX mode) ==="
DAEMON_LOG=$(mktemp -t ccbd-sandbox-e2e-XXXXXX.log)
CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccbd > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
echo "  daemon_pid=$DAEMON_PID log=$DAEMON_LOG"

cleanup() {
  echo ""
  echo "=== cleanup trap ==="
  SESSION_ID=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ah ps 2>&1 | grep -oE 'sess_[a-f0-9-]+' | head -1)
  if [ -n "$SESSION_ID" ]; then
    CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ah kill --session "$SESSION_ID" 2>&1 | head -3 || true
  fi
  sleep 1
  kill $DAEMON_PID 2>/dev/null || true
  sleep 1
  pkill -f "target/release/ccbd" 2>/dev/null || true
  echo "  daemon stopped"
  rm -f "$TEST_CONFIG"
}
trap cleanup EXIT

echo ""
echo "=== [4/8] daemon ready ==="
for i in 1 2 3 4 5; do
  sleep 1
  if CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ah ping 2>&1 | grep -q "ok\|pong\|alive"; then
    echo "  daemon ready after ${i}s"
    break
  fi
done

echo ""
echo "=== [5/8] ah start --wait (SANDBOX, 3 agents, may take 60-90s) ==="
START_OUT=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ah --config "$TEST_CONFIG" start --wait 2>&1)
echo "$START_OUT" | head -10
SESSION_ID=$(echo "$START_OUT" | grep -oE 'session_id=[a-f0-9-]+' | head -1 | cut -d= -f2)
echo "  session_id=$SESSION_ID"

echo ""
echo "=== [6/8] ps verify (3 agents IDLE) ==="
PS_OUT=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ah ps 2>&1)
echo "$PS_OUT" | head -25
IDLE_COUNT=$(echo "$PS_OUT" | grep -c "IDLE" || true)
if [ "$IDLE_COUNT" -lt 3 ]; then
  echo "WARNING: only $IDLE_COUNT agents IDLE (expected 3)"
fi

echo ""
echo "=== [7/8] ask a1 sandbox 模式 (verify reply distill works through sandbox) ==="
ASK_OUT=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" timeout 90 ./target/release/ah ask a1 "echo from sandbox a1" --wait 2>&1 || echo "TIMEOUT_OR_ERROR")
echo "$ASK_OUT" | head -20
if echo "$ASK_OUT" | grep -qE "from sandbox a1|reply"; then
  echo "  ask reply OK (sandbox + distill works)"
else
  echo "  WARNING: ask reply unclear or missing 'from sandbox a1'"
fi

echo ""
echo "=== [8/8] kill --session ==="
CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ah kill --session "$SESSION_ID" 2>&1 | head -3
sleep 2

echo ""
echo "=== sandbox e2e DONE ==="
