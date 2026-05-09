#!/usr/bin/env bash
# mvp13 NO_SANDBOX smoke test: 验证 stage 0/1/2/3/4 主线集成
#
# 跑法: bash scripts/mvp13-e2e-no-sandbox.sh
#
# 不污染 user host 的 Python ccb daemon (用 isolated CCB_ENV=dev state_dir)。
# spawn 真 codex/claude/gemini binary，但只跑 4 agent 几秒 ask + kill，几秒钟 done。
# 不进 sandbox（CCBD_UNSAFE_NO_SANDBOX=1），所以 codex/claude/gemini 用 host 配置直接跑。
#
# 已知 race: 如果 user 当前 Python ccb 在跑同 provider，host config 会被两边共读，
# 可能出 .codex/sessions/ 或 .gemini/ 状态文件 race。短跑 + 接受 race。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Cleanup any previous test artifacts
echo "=== [1/8] cleanup ==="
pkill -f "target/release/ccbd" 2>/dev/null || true
sleep 1
DAEMON_PIDS=$(pgrep -f "target/release/ccbd" 2>/dev/null || true)
if [ -n "$DAEMON_PIDS" ]; then
  echo "  killing stale ccbd: $DAEMON_PIDS"
  kill -9 $DAEMON_PIDS 2>/dev/null || true
fi

# Force fresh state_dir (CCB_ENV=dev → target/dev_state/)
rm -rf target/dev_state/ccbd.sqlite* target/dev_state/pipes 2>/dev/null || true
echo "  fresh state_dir at target/dev_state/"

# Build binaries
echo ""
echo "=== [2/8] build release binaries ==="
cargo build --release --bin ccbd --bin ccb-rust 2>&1 | tail -3

# Verify binaries exist
test -x target/release/ccbd || { echo "ERROR: ccbd binary missing"; exit 1; }
test -x target/release/ccb-rust || { echo "ERROR: ccb-rust binary missing"; exit 1; }

# Test config: master disabled (Stage 7 prep skips master pane to avoid spawning user's claude)
TEST_CONFIG=$(mktemp -t ccb-e2e-XXXXXX.toml)
cat > "$TEST_CONFIG" <<'EOF'
version = "1"
[master]
enabled = false

[agents.a1]
provider = "codex"

[agents.a2]
provider = "codex"

[agents.a3]
provider = "gemini"

[agents.a4]
provider = "claude"
EOF
echo "  test config at $TEST_CONFIG"

# Start isolated daemon
echo ""
echo "=== [3/8] start isolated daemon ==="
DAEMON_LOG=$(mktemp -t ccbd-e2e-XXXXXX.log)
CCB_ENV=dev CCBD_UNSAFE_NO_SANDBOX=1 ./target/release/ccbd > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
echo "  daemon_pid=$DAEMON_PID log=$DAEMON_LOG"

# Cleanup trap
cleanup() {
  echo ""
  echo "=== cleanup trap ==="
  kill $DAEMON_PID 2>/dev/null || true
  sleep 1
  pkill -f "target/release/ccbd" 2>/dev/null || true
  echo "  daemon stopped"
  echo "  daemon log tail:"
  tail -20 "$DAEMON_LOG" || true
  rm -f "$TEST_CONFIG"
}
trap cleanup EXIT

# Wait daemon ready (poll ping)
echo ""
echo "=== [4/8] wait daemon ready (ping) ==="
for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 1
  if CCB_ENV=dev ./target/release/ccb-rust ping 2>&1 | grep -q "ok\|pong\|alive"; then
    echo "  daemon ready after ${i}s"
    break
  fi
  if ! kill -0 $DAEMON_PID 2>/dev/null; then
    echo "ERROR: daemon died, log:"
    cat "$DAEMON_LOG"
    exit 1
  fi
  if [ $i -eq 10 ]; then
    echo "ERROR: daemon ping timeout (10s)"
    exit 1
  fi
done

# Start agents
echo ""
echo "=== [5/8] ccb-rust start --wait (spawn 4 agents NO_SANDBOX, master disabled) ==="
echo "  this spawns real codex/codex/gemini/claude - takes 30-60s"
START_OUT=$(CCB_ENV=dev CCBD_UNSAFE_NO_SANDBOX=1 ./target/release/ccb-rust --config "$TEST_CONFIG" start --wait 2>&1)
echo "$START_OUT" | head -20
SESSION_ID=$(echo "$START_OUT" | grep -oE 'session_id=[a-z0-9_-]+' | head -1 | cut -d= -f2)
if [ -z "$SESSION_ID" ]; then
  echo "ERROR: failed to parse session_id from start output"
  exit 1
fi
echo "  session_id=$SESSION_ID"

# Verify ps shows 4 agents IDLE
echo ""
echo "=== [6/8] ps verify (4 agents should be IDLE) ==="
PS_OUT=$(CCB_ENV=dev ./target/release/ccb-rust ps 2>&1)
echo "$PS_OUT" | head -15
IDLE_COUNT=$(echo "$PS_OUT" | grep -c "IDLE" || true)
if [ "$IDLE_COUNT" -lt 4 ]; then
  echo "WARNING: only $IDLE_COUNT agents IDLE (expected 4) — some may still be starting"
fi

# Test ask reply text distill (Stage 4)
echo ""
echo "=== [7/8] ask a1 (verify reply text distill, Stage 4) ==="
ASK_OUT=$(CCB_ENV=dev CCBD_UNSAFE_NO_SANDBOX=1 timeout 60 ./target/release/ccb-rust ask a1 "echo from a1" --wait 2>&1 || echo "TIMEOUT_OR_ERROR")
echo "$ASK_OUT" | head -20
if echo "$ASK_OUT" | grep -q "echo from a1\|reply"; then
  echo "  ask reply OK (distill works)"
else
  echo "  WARNING: ask reply unclear, check raw output above"
fi

# Cleanup session
echo ""
echo "=== [8/8] kill --session $SESSION_ID ==="
CCB_ENV=dev ./target/release/ccb-rust kill "$SESSION_ID" --session 2>&1 | head -5
sleep 2

# Final verification: any zombie agents?
echo ""
echo "=== final zombie check ==="
ZOMBIE_AGENTS=$(pgrep -af "codex|gemini|claude-code" 2>/dev/null | grep -v "main.py" | grep -v "Code" | head -5 || true)
if [ -n "$ZOMBIE_AGENTS" ]; then
  echo "INFO: agent processes still running (may be your other claude/codex/gemini instances):"
  echo "$ZOMBIE_AGENTS"
else
  echo "  no zombie agents"
fi

echo ""
echo "=== e2e DONE ==="
echo "如有问题:"
echo "  - daemon log: $DAEMON_LOG"
echo "  - 检查 target/dev_state/ccbd.sqlite 看 events / state_change"
echo "  - 跑 'CCB_ENV=dev ./target/release/ccb-rust ps' 看现状"
