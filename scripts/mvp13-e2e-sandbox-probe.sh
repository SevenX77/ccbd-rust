#!/usr/bin/env bash
# mvp13 sandbox e2e probe: 暴露 sandbox 模式下 first-run 弹窗的真实形态
#
# 跟 mvp13-e2e-no-sandbox.sh 不同：这次**带 sandbox** (不 set CCBD_UNSAFE_NO_SANDBOX)，
# 让 codex/claude/gemini 在隔离的 /home/agent 里 first-run，触发实际弹窗。
#
# 不 ask、不等 IDLE，只 spawn → sleep 60s → capture-pane 看弹窗。
#
# 目的：让 master Claude 看到真 first-run 屏幕状态，针对性补 Stage 5 onboarding。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
STATE_DIR="$REPO_ROOT/target/dev_state"

echo "=== [1/7] cleanup ==="
pkill -f "target/release/ccbd" 2>/dev/null || true
sleep 1
DAEMON_PIDS=$(pgrep -f "target/release/ccbd" 2>/dev/null || true)
if [ -n "$DAEMON_PIDS" ]; then
  echo "  killing stale ccbd: $DAEMON_PIDS"
  kill -9 $DAEMON_PIDS 2>/dev/null || true
fi
rm -rf "$STATE_DIR"/ccbd.sqlite* "$STATE_DIR"/pipes 2>/dev/null || true
echo "  fresh state_dir at $STATE_DIR/"

echo ""
echo "=== [2/7] build release binaries ==="
cargo build --release --bin ccbd --bin ccb-rust 2>&1 | tail -3

TEST_CONFIG=$(mktemp -t ccb-sandbox-probe-XXXXXX.toml)
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
echo "  test config: 3 agents (1 codex + 1 gemini + 1 claude, master disabled)"

echo ""
echo "=== [3/7] start daemon (SANDBOX mode, NO CCBD_UNSAFE_NO_SANDBOX) ==="
DAEMON_LOG=$(mktemp -t ccbd-sandbox-probe-XXXXXX.log)
RUST_LOG=ccbd=debug,info CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccbd > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
echo "  daemon_pid=$DAEMON_PID log=$DAEMON_LOG"

cleanup() {
  echo ""
  echo "=== cleanup trap ==="
  CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust kill --session "$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust ps 2>&1 | grep -oE 'sess_[a-z0-9-]+' | head -1)" 2>&1 | head -3 || true
  sleep 1
  kill $DAEMON_PID 2>/dev/null || true
  sleep 2
  pkill -f "target/release/ccbd" 2>/dev/null || true
  echo "  daemon stopped"
  echo "  daemon log tail:"
  tail -15 "$DAEMON_LOG" 2>&1 || true
  rm -f "$TEST_CONFIG"
}
trap cleanup EXIT

echo ""
echo "=== [4/7] daemon ready ==="
for i in 1 2 3 4 5; do
  sleep 1
  if CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust ping 2>&1 | grep -q "ok\|pong\|alive"; then
    echo "  daemon ready after ${i}s"
    break
  fi
  if ! kill -0 $DAEMON_PID 2>/dev/null; then
    echo "ERROR: daemon died, log:"
    cat "$DAEMON_LOG"
    exit 1
  fi
done

echo ""
echo "=== [5/7] start agents (SANDBOX, no --wait) ==="
START_OUT=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust --config "$TEST_CONFIG" start 2>&1 || echo "START_FAILED")
echo "$START_OUT" | head -10

echo ""
echo "=== [6/7] sleep 60s for agents first-run UI to materialize ==="
for i in 10 20 30 40 50 60; do
  sleep 10
  echo "  T+${i}s"
done

echo ""
echo "=== [7/7] ccb-rust ps + capture-pane each agent ==="
echo "--- ps ---"
PS_OUT=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust ps 2>&1)
echo "$PS_OUT"

# tmux socket: daemon uses -L <name> with name in dev_state. Find from "tmux -L ..." hint in ps output.
TMUX_LABEL=$(echo "$PS_OUT" | grep -oE 'ccbd-[a-z0-9]+' | head -1)
echo "  tmux_label=$TMUX_LABEL"

# List panes via -L label
echo ""
echo "--- tmux list-panes (via -L $TMUX_LABEL) ---"
if [ -n "$TMUX_LABEL" ]; then
  PANE_IDS=$(tmux -L "$TMUX_LABEL" list-panes -a -F '#{pane_id}' 2>&1 | head -10)
  echo "  pane_ids: $PANE_IDS"
  for PANE in $PANE_IDS; do
    echo ""
    echo "=== pane $PANE content (last 30 lines) ==="
    tmux -L "$TMUX_LABEL" capture-pane -t "$PANE" -p 2>&1 | tail -30
  done
fi

echo ""
echo "=== [8/8] state_change events (last 30) ==="
sqlite3 target/dev_state/ccbd.sqlite \
  "SELECT seq_id, agent_id, event_type, substr(payload, 1, 180) FROM events WHERE event_type='state_change' ORDER BY seq_id DESC LIMIT 30" 2>&1 | tail -40

echo ""
echo "=== sandbox probe DONE ==="
echo "查看 daemon log: $DAEMON_LOG"
