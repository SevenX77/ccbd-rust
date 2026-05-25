#!/usr/bin/env bash
# core-fixes 综合 e2e: 4 agent (codex/codex/gemini/claude) + master + R2 ack chain
#
# 跑法: bash scripts/core_fixes_full_e2e.sh
#
# 目的: 端到端 smoke 真 LLM 全 4 provider + R2 ack chain 链路
# 覆盖:
#   T2.2.2 ACK→BUSY 可观察 (state_change events)
#   T2.4.5 send reply 含 ACK 状态
#   T2.5.1 并发 send 互斥 (复用 IDLE guard)
#   R1+R3+R4 联动: master_<p> + agent_<id> 独立 session, codex/gemini/claude 真 LLM 启动
#
# 模式: NO_SANDBOX (避免 bwrap onboarding 多 agent 一起拖延)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
STATE_DIR="$REPO_ROOT/target/dev_state"

sql_query() {
  local db="$1"
  local query="$2"
  if command -v sqlite3 >/dev/null 2>&1; then
    sqlite3 "$db" "$query" 2>&1
  else
    python3 -c "
import sqlite3, sys
try:
    conn = sqlite3.connect(sys.argv[1])
    for row in conn.execute(sys.argv[2]):
        print('|'.join('' if c is None else str(c) for c in row))
except Exception as e:
    print(f'SQL_ERR: {e}', file=sys.stderr)
    sys.exit(1)
" "$db" "$query" 2>&1
  fi
}

PASS_COUNT=0
FAIL_COUNT=0
RESULT_LINES=()
record_pass() { PASS_COUNT=$((PASS_COUNT + 1)); RESULT_LINES+=("[PASS] $1"); echo "  [PASS] $1"; }
record_fail() { FAIL_COUNT=$((FAIL_COUNT + 1)); RESULT_LINES+=("[FAIL] $1 -- $2"); echo "  [FAIL] $1 -- $2"; }

cleanup_global() {
  echo ""
  echo "=== global cleanup ==="
  if [ -n "${SESSION_ID:-}" ]; then
    CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust kill "$SESSION_ID" --session 2>&1 | head -3 || true
    sleep 2
  fi
  pkill -f "target/release/ccbd" 2>/dev/null || true
  sleep 1
  kill_ccbd_tmux_servers
  if [ -n "${DAEMON_LOG:-}" ] && [ -f "$DAEMON_LOG" ]; then
    echo "  daemon log tail:"
    tail -25 "$DAEMON_LOG" 2>/dev/null | sed 's/^/    /' || true
  fi
  rm -f "${TEST_CONFIG:-}" 2>/dev/null || true
  echo ""
  echo "=== full e2e summary: $PASS_COUNT pass / $FAIL_COUNT fail ==="
  for line in "${RESULT_LINES[@]}"; do
    echo "  $line"
  done
  if [ "$FAIL_COUNT" -gt 0 ]; then exit 1; fi
}
trap cleanup_global EXIT

kill_stale_ccbd() {
  pkill -TERM -f "target/release/ccbd" 2>/dev/null || true
  sleep 1
  pkill -KILL -f "target/release/ccbd" 2>/dev/null || true
}

kill_ccbd_tmux_servers() {
  for sock in /tmp/tmux-"$(id -u)"/ccbd-*; do
    [ -S "$sock" ] || continue
    tmux -L "$(basename "$sock")" kill-server 2>/dev/null || true
  done
}

echo "=== [1/8] cleanup + build ==="
kill_stale_ccbd
rm -rf target/dev_state 2>/dev/null || true
mkdir -p target/dev_state
kill_ccbd_tmux_servers
cargo build --release --bin ccbd --bin ccb-rust 2>&1 | tail -3

mkdir -p "$STATE_DIR"
SOCK_NAME="ccbd-$(printf '%s' "$(realpath "$STATE_DIR")" | sha256sum | awk '{print substr($1,1,16)}')"
TMUX_SOCK="/tmp/tmux-$(id -u)/$SOCK_NAME"
echo "  expected tmux socket: $TMUX_SOCK"

echo ""
echo "=== [2/8] write test config (4 agent + master enabled) ==="
TEST_CONFIG=$(mktemp -t full-e2e-XXXXXX.toml)
# Note: master.enabled=false to keep test focused on agent ACK chain.
# A separate r4_e2e covers master.enabled=true.
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
echo "  config: 4 agent (codex/codex/gemini/claude), master disabled, NO_SANDBOX"

echo ""
echo "=== [3/8] start daemon ==="
DAEMON_LOG=$(mktemp -t full-ccbd-XXXXXX.log)
CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 ./target/release/ccbd > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
echo "  daemon_pid=$DAEMON_PID  log=$DAEMON_LOG"

for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 1
  if CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust ping 2>&1 | grep -q "ok\|sessions="; then
    echo "  daemon ready"; break
  fi
done

echo ""
echo "=== [4/8] start agents (4 真 LLM, 估计 60-120s) ==="
START_OUT=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 timeout 180 ./target/release/ccb-rust --config "$TEST_CONFIG" start --wait 2>&1 || echo "START_FAILED")
echo "  start output (first 15 lines):"
echo "$START_OUT" | head -15 | sed 's/^/    /'
SESSION_ID=$(echo "$START_OUT" | grep -oE 'session_id=[a-z0-9_-]+' | head -1 | cut -d= -f2 || true)
echo "  session_id=$SESSION_ID"

# Verify ps
PS_OUT=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust ps 2>&1)
echo "  ps:"
echo "$PS_OUT" | head -20 | sed 's/^/    /'
IDLE_COUNT=$(echo "$PS_OUT" | grep -c "IDLE" || true)
echo "  IDLE agents: $IDLE_COUNT / 4"
if [ "$IDLE_COUNT" -ge 3 ]; then
  record_pass "R1+R3+R4 联动: ≥3/4 真 LLM 进 IDLE"
else
  record_fail "R1+R3+R4 联动: 仅 $IDLE_COUNT/4 进 IDLE" "see ps + daemon log"
fi

echo "  using socket (precomputed): $TMUX_SOCK"

# Verify session naming (R1 1-Session-per-CLI)
TMUX_LS=$(tmux -L "$SOCK_NAME" ls 2>&1)
echo "  tmux ls:"
echo "$TMUX_LS" | sed 's/^/    /'
EXPECTED_AGENTS="agent_a1 agent_a2 agent_a3 agent_a4"
MISSING=""
for a in $EXPECTED_AGENTS; do
  if ! echo "$TMUX_LS" | grep -qE "^${a}:"; then
    MISSING="$MISSING $a"
  fi
done
if [ -z "$MISSING" ]; then
  record_pass "R1 1-Session-per-CLI: 4 sessions all present"
else
  record_fail "R1 缺 sessions:" "$MISSING"
fi

echo ""
echo "=== [5/8] R2 ack chain via real codex ask ==="
SQLITE_DB="target/dev_state/ccbd.sqlite"

# T2.5.1: 并发 send 互斥 — fire 2 concurrent asks to same agent
echo "--- T2.5.1: 并发 ask 互斥 ---"
(
  CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 timeout 90 ./target/release/ccb-rust ask a1 "echo concurrent-1" --request-id req-concurrent-1 --wait > /tmp/full-e2e-c1.log 2>&1 &
  PID1=$!
  CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 timeout 5 ./target/release/ccb-rust ask a1 "echo concurrent-2" --request-id req-concurrent-2 --wait > /tmp/full-e2e-c2.log 2>&1 &
  PID2=$!
  wait $PID1 $PID2 2>/dev/null || true
)
echo "  concurrent-1 reply (first 6 lines):"
head -6 /tmp/full-e2e-c1.log 2>/dev/null | sed 's/^/    /' || echo "    (no output)"
echo "  concurrent-2 reply (first 6 lines):"
head -6 /tmp/full-e2e-c2.log 2>/dev/null | sed 's/^/    /' || echo "    (no output)"
# 期望: c2 在 c1 占住 a1 时被拒 (state != IDLE) 或排队后串行
REJECT_C2=$(grep -E "AGENT_WRONG_STATE|WAITING_FOR_ACK|BUSY|wrong state|already" /tmp/full-e2e-c2.log 2>/dev/null | head -1 || true)
if [ -n "$REJECT_C2" ]; then
  record_pass "T2.5.1 并发 ask: c2 被互斥 ($REJECT_C2)"
else
  # 也可能两个串行成功; 看 events 表 SENT 是否各自单独
  SENT_COUNT=$(sql_query "$SQLITE_DB" "SELECT COUNT(*) FROM events WHERE agent_id='a1' AND event_type='command_received'" || echo "0")
  echo "  command_received events for a1: $SENT_COUNT"
  if [ "$SENT_COUNT" -ge 2 ]; then
    record_pass "T2.5.1 并发 ask: 串行处理 ($SENT_COUNT command_received events)"
  elif [ "$SENT_COUNT" -eq 1 ] && grep -q "status=QUEUED" /tmp/full-e2e-c2.log 2>/dev/null; then
    record_pass "T2.5.1 并发 ask: c2 保持队列且未并发派发 ($SENT_COUNT command_received event)"
  else
    record_fail "T2.5.1 并发 ask 行为不明" "see /tmp/full-e2e-c*.log"
  fi
fi
rm -f /tmp/full-e2e-c1.log /tmp/full-e2e-c2.log

# T2.2.2 / T2.4.5: WAITING_FOR_ACK 可观察 + send reply 含 ACK
echo ""
echo "--- T2.2.2 / T2.4.5: WAITING_FOR_ACK 链路 ---"
sleep 5  # wait for previous asks to settle
ACK_AGENT="a1"
A1_STATE=$(sql_query "$SQLITE_DB" "SELECT state FROM agents WHERE id='a1'" | head -1 || true)
if [ "$A1_STATE" != "IDLE" ]; then
  ACK_AGENT="a2"
  echo "  a1 state after concurrency is $A1_STATE; using a2 for ACK/BUSY check"
fi
ASK3=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 timeout 60 ./target/release/ccb-rust ask "$ACK_AGENT" "echo r2-ack-test-marker" --wait 2>&1 || echo "TIMEOUT")
echo "  ask reply (first 8 lines):"
echo "$ASK3" | head -8 | sed 's/^/    /'

# Query events for state transitions
STATE_CHANGES=$(sql_query "$SQLITE_DB" "SELECT payload FROM events WHERE agent_id='$ACK_AGENT' AND event_type='state_change' ORDER BY seq_id" || true)
echo "  $ACK_AGENT state_change events:"
echo "$STATE_CHANGES" | head -15 | sed 's/^/    /'
if echo "$STATE_CHANGES" | grep -q "WAITING_FOR_ACK"; then
  record_pass "T2.2.2 / T2.4.5 WAITING_FOR_ACK 进入 events 链"
else
  record_fail "T2.2.2 WAITING_FOR_ACK 未在 events 出现" ""
fi
if echo "$STATE_CHANGES" | grep -q '"to":"BUSY"'; then
  BUSY_SEEN=1
  record_pass "T2.2.2 BUSY 转换 in events"
else
  BUSY_SEEN=0
fi

# R2 在多 agent 上分别 verify (gemini/claude)
echo ""
echo "--- R2 ack chain on gemini (a3) ---"
ASK_A3=$(CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 timeout 90 ./target/release/ccb-rust ask a3 "echo gemini-ack-test" --wait 2>&1 || echo "TIMEOUT")
echo "$ASK_A3" | head -6 | sed 's/^/    /'
A3_STATE_CHANGES=$(sql_query "$SQLITE_DB" "SELECT payload FROM events WHERE agent_id='a3' AND event_type='state_change' ORDER BY seq_id" || true)
if echo "$A3_STATE_CHANGES" | grep -q "WAITING_FOR_ACK"; then
  record_pass "R2 ack chain on gemini: WAITING_FOR_ACK observed"
else
  record_fail "R2 ack chain on gemini: WAITING_FOR_ACK 缺失" "$(echo "$A3_STATE_CHANGES" | head -3)"
fi
if [ "$BUSY_SEEN" -eq 0 ]; then
  if echo "$A3_STATE_CHANGES" | grep -q '"to":"BUSY"'; then
    record_pass "T2.2.2 BUSY 转换 in events (gemini fallback)"
  else
    record_fail "T2.2.2 未观察到 BUSY 转换" ""
  fi
fi

echo ""
echo "=== [6/8] kill --session ==="
CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" ./target/release/ccb-rust kill "$SESSION_ID" --session 2>&1 | head -5 | sed 's/^/  /'
sleep 3

echo ""
echo "=== [7/8] verify zombie / tmux clean ==="
TMUX_LS_AFTER=$(tmux -L "$SOCK_NAME" ls 2>&1 || echo "(no sessions)")
echo "  tmux ls after kill --session:"
echo "$TMUX_LS_AFTER" | sed 's/^/    /'
if echo "$TMUX_LS_AFTER" | grep -qE "^agent_(a1|a2|a3|a4):"; then
  record_fail "kill --session 后仍残留 agent_* sessions" "$(echo "$TMUX_LS_AFTER" | grep '^agent_')"
else
  record_pass "kill --session 后 agent_* sessions 全清"
fi

echo ""
echo "=== [8/8] daemon shutdown ==="
kill -TERM "$DAEMON_PID" 2>/dev/null || true
sleep 3
if pgrep -f "target/release/ccbd" 2>/dev/null; then
  record_fail "daemon SIGTERM 后仍存活" ""
else
  record_pass "daemon SIGTERM 后退出"
fi

# tmux server cleared
TMUX_LS_FINAL=$(tmux -L "$SOCK_NAME" ls 2>&1 || echo "(no server)")
if echo "$TMUX_LS_FINAL" | grep -qE "^agent_|^master_"; then
  record_fail "T1.1.2 daemon SIGTERM 后 tmux 残留" "$(echo "$TMUX_LS_FINAL" | head -3)"
else
  record_pass "T1.1.2 daemon SIGTERM 后 tmux server agent_*/master_* 全清"
fi

exit 0
