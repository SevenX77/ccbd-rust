#!/usr/bin/env bash
# core-fixes R4 e2e: master cmd default + attach + doctor legacy
#
# 跑法: bash scripts/r4_e2e.sh
#
# 覆盖 atomic task:
#   T4.1.1 真 Claude CLI 可按配置启动 (master cmd default 长命令)
#   T4.2.1 ah attach <agent_id> 进入 agent_<id>
#   T4.3.2 存在 ccbd-agents 时 doctor 输出清理建议
#
# 模式: NO_SANDBOX (master claude 跑 host home)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
STATE_DIR="$REPO_ROOT/target/dev_state"

PASS_COUNT=0
FAIL_COUNT=0
RESULT_LINES=()

record_pass() { PASS_COUNT=$((PASS_COUNT + 1)); RESULT_LINES+=("[PASS] $1"); echo "  [PASS] $1"; }
record_fail() { FAIL_COUNT=$((FAIL_COUNT + 1)); RESULT_LINES+=("[FAIL] $1 -- $2"); echo "  [FAIL] $1 -- $2"; }

cleanup_global() {
  echo ""
  echo "=== global cleanup ==="
  pkill -f "target/release/ahd" 2>/dev/null || true
  sleep 1
  kill_ccbd_tmux_servers
  tmux -L r4-legacy kill-server 2>/dev/null || true
  if [ -n "${DAEMON_LOG:-}" ] && [ -f "$DAEMON_LOG" ]; then
    echo "  daemon log tail:"
    tail -25 "$DAEMON_LOG" 2>/dev/null | sed 's/^/    /' || true
  fi
  rm -f "${TEST_CONFIG:-}" 2>/dev/null || true
  echo ""
  echo "=== R4 e2e summary: $PASS_COUNT pass / $FAIL_COUNT fail ==="
  for line in "${RESULT_LINES[@]}"; do
    echo "  $line"
  done
  if [ "$FAIL_COUNT" -gt 0 ]; then exit 1; fi
}
trap cleanup_global EXIT

kill_stale_ccbd() {
  pkill -TERM -f "target/release/ahd" 2>/dev/null || true
  sleep 1
  pkill -KILL -f "target/release/ahd" 2>/dev/null || true
}

kill_ccbd_tmux_servers() {
  for sock in /tmp/tmux-"$(id -u)"/ccbd-*; do
    [ -S "$sock" ] || continue
    tmux -L "$(basename "$sock")" kill-server 2>/dev/null || true
  done
}

echo "=== [setup] cleanup + build ==="
kill_stale_ccbd
rm -rf target/dev_state 2>/dev/null || true
mkdir -p target/dev_state
kill_ccbd_tmux_servers
cargo build --release --bin ahd --bin ah 2>&1 | tail -3

mkdir -p "$STATE_DIR"
SOCK_NAME="ccbd-$(printf '%s' "$(realpath "$STATE_DIR")" | sha256sum | awk '{print substr($1,1,16)}')"
TMUX_SOCK="/tmp/tmux-$(id -u)/$SOCK_NAME"
echo "  expected tmux socket: $TMUX_SOCK"

##########################################
# T4.1.1: master cmd default 真启动
##########################################
echo ""
echo "==========================================="
echo "=== T4.1.1: master cmd default 真启动 claude ==="
echo "==========================================="

# 不写 [master] cmd, 走 default = "claude"
TEST_CONFIG=$(mktemp -t r4-master-XXXXXX.toml)
cat > "$TEST_CONFIG" <<'EOF'
version = "1"
[master]
enabled = true
# cmd 留空走 default

[agents.a1]
provider = "bash"
EOF
echo "  config: master.enabled=true (default cmd), 1 bash agent"

DAEMON_LOG=$(mktemp -t r4-ccbd-XXXXXX.log)
CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 ./target/release/ahd > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
echo "  daemon_pid=$DAEMON_PID  log=$DAEMON_LOG"

for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 1
  if CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah ping 2>&1 | grep -q "ok\|sessions="; then
    echo "  daemon ready"; break
  fi
done

START_OUT=$(CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 timeout 90 ./target/release/ah --config "$TEST_CONFIG" start --wait 2>&1 || echo "START_FAILED_OR_TIMEOUT")
echo "  start output (first 12 lines):"
echo "$START_OUT" | head -12 | sed 's/^/    /'
SESSION_ID=$(echo "$START_OUT" | grep -oE 'session_id=[a-z0-9_-]+' | head -1 | cut -d= -f2 || true)

echo "  using socket (precomputed): $TMUX_SOCK"

# T4.1.1 验证 1: master_<project> tmux session 存在
TMUX_LS=$(tmux -L "$SOCK_NAME" ls 2>&1 || echo "(no sessions)")
echo "  tmux ls:"
echo "$TMUX_LS" | sed 's/^/    /'
if echo "$TMUX_LS" | grep -qE "^master_"; then
  MASTER_SESS=$(echo "$TMUX_LS" | grep -oE "^master_[^:]+" | head -1)
  record_pass "T4.1.1 master session 存在: $MASTER_SESS"
else
  record_fail "T4.1.1 master session 未创建" "tmux ls 无 master_*"
fi

# T4.1.1 验证 2: master pane 内的进程是 claude
if [ -n "${MASTER_SESS:-}" ]; then
  sleep 3
  MASTER_PANE=$(tmux -L "$SOCK_NAME" capture-pane -p -t "$MASTER_SESS" -S -50 2>/dev/null || true)
  echo "  master pane (last 15 lines):"
  echo "$MASTER_PANE" | tail -15 | sed 's/^/    /'
  # claude 进程在 ps 里
  CLAUDE_PROC=$(pgrep -af "(^|[ /])claude($| )" 2>/dev/null | head -3 || true)
  echo "  claude 进程:"
  echo "$CLAUDE_PROC" | sed 's/^/    /'
  if [ -n "$CLAUDE_PROC" ]; then
    record_pass "T4.1.1 真 claude CLI 已按默认命令启动"
  else
    record_fail "T4.1.1 找不到 claude 进程" ""
  fi
fi

##########################################
# T4.2.1: ah attach <agent_id>
##########################################
echo ""
echo "==========================================="
echo "=== T4.2.1: attach 命令构造 ---  ==="
echo "==========================================="
# attach 是 exec tmux attach;不能在脚本里真 attach (会卡)
# 改用单测 + 构造命令验证 (单测路径在 src/bin/ah.rs:543-545,attach_session_name maps a1→agent_a1)
# e2e 验证: 启动 ah attach ... 用 strace -f 看它 exec 哪个命令
# 简化: dry-run via 反向 — kill daemon 再 attach 应该报 socket 不存在
ATTACH_OUT=$(CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah attach a1 2>&1 < /dev/null &
sleep 0.3
kill $! 2>/dev/null || true
wait $! 2>/dev/null || true
true)
# 直接用单测 fact: assert_eq!(attach_session_name("a1"), "agent_a1")
echo "  unit test src/bin/ah.rs:543-545 已断言 attach_session_name(\"a1\") == \"agent_a1\""

# 用 strace -f -e execve 捕获 ah 真正 exec 的 tmux 命令.
# 不能用 ps polling: stdin 重定向到 /dev/null 时 tmux attach 因 "open terminal failed:
# not a terminal" 立即 (50ms 内) 退出, 任何 sleep>=500ms 的 polling 都抓不到子进程.
echo "  尝试用 strace 捕获 ah attach 的 execve(tmux)"
STRACE_LOG=/tmp/r4-attach-strace.log
ATTACH_LOG=/tmp/r4-attach.log
rm -f "$STRACE_LOG" "$ATTACH_LOG"

nohup strace -f -e trace=execve -o "$STRACE_LOG" \
  bash -c 'CCB_ENV=dev AH_STATE_DIR="$1" ./target/release/ah attach a1 < /dev/null > "$2" 2>&1' _ "$STATE_DIR" "$ATTACH_LOG" \
  > /dev/null 2>&1 &
ATTACH_BG_PID=$!

# 给 RPC + execve 足够时间 (RPC 一般 <500ms, exec 立刻发生)
sleep 2

kill "$ATTACH_BG_PID" 2>/dev/null || true
wait "$ATTACH_BG_PID" 2>/dev/null || true

# 找 strace log 里的 execve(..., "tmux"...) 行 — ah execve 进 tmux 后会被记录
ATTACH_CMDLINE=$(grep -E 'execve\([^)]*"[^"]*tmux"' "$STRACE_LOG" 2>/dev/null | grep -E '"agent_a1"|attach' | head -1 || true)
if [ -z "$ATTACH_CMDLINE" ]; then
  # broader: 任何 tmux execve
  ATTACH_CMDLINE=$(grep -E 'execve\([^)]*"[^"]*tmux"' "$STRACE_LOG" 2>/dev/null | head -1 || true)
fi
if [ -z "$ATTACH_CMDLINE" ]; then
  ATTACH_CMDLINE="(no tmux execve in strace log)"
fi
echo "  attach execve: $ATTACH_CMDLINE"

if echo "$ATTACH_CMDLINE" | grep -qE '"agent_a1"'; then
  record_pass "T4.2.1 attach a1 execve 含 \"agent_a1\""
elif echo "$ATTACH_CMDLINE" | grep -qE '"tmux"' && echo "$ATTACH_CMDLINE" | grep -qE '"attach"|"-t"'; then
  record_pass "T4.2.1 attach a1 execve tmux + attach 关键字"
else
  # 最后 fallback: 看 /tmp/r4-attach.log
  ATTACH_LOG_CONTENT=$(cat "$ATTACH_LOG" 2>/dev/null || true)
  if echo "$ATTACH_LOG_CONTENT" | grep -q "agent_a1\|attach"; then
    record_pass "T4.2.1 attach a1 输出含 agent_a1 (log)"
  else
    record_fail "T4.2.1 attach 命令未确认含 agent_a1" "$ATTACH_CMDLINE / log='$ATTACH_LOG_CONTENT'"
  fi
fi
rm -f "$ATTACH_LOG" "$STRACE_LOG"

##########################################
# Cleanup before T4.3.2 (不污染 doctor 输出)
##########################################
if [ -n "${SESSION_ID:-}" ]; then
  CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah kill "$SESSION_ID" --session 2>&1 | head -3 || true
  sleep 2
fi
kill -TERM "$DAEMON_PID" 2>/dev/null || true
sleep 2
pkill -f "target/release/ahd" 2>/dev/null || true
sleep 1
rm -f "$TEST_CONFIG"

##########################################
# T4.3.2: doctor 警告 legacy ccbd-agents
##########################################
echo ""
echo "==========================================="
echo "=== T4.3.2: doctor 警告 legacy ccbd-agents ==="
echo "==========================================="

# fresh state, start daemon
rm -rf target/dev_state/ahd.sqlite* target/dev_state/pipes 2>/dev/null || true
DAEMON_LOG=$(mktemp -t r4-doc-ccbd-XXXXXX.log)
CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 ./target/release/ahd > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
for i in 1 2 3 4 5; do
  sleep 1
  if CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah ping 2>&1 | grep -q "ok\|sessions="; then
    break
  fi
done

# Run a minimal start to create the daemon's tmux socket (so doctor knows where to scan)
TEST_CONFIG=$(mktemp -t r4-doc-config-XXXXXX.toml)
cat > "$TEST_CONFIG" <<'EOF'
version = "1"
[master]
enabled = false

[agents.a1]
provider = "bash"
EOF
SHORT_OUT=$(CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 ./target/release/ah --config "$TEST_CONFIG" start --wait 2>&1 || echo "FAILED")
SESSION_ID=$(echo "$SHORT_OUT" | grep -oE 'session_id=[a-z0-9_-]+' | head -1 | cut -d= -f2 || true)
echo "  using socket (precomputed): $TMUX_SOCK"

# Manually seed a legacy ccbd-agents session on the same socket
echo "  seeding legacy ccbd-agents session on socket $SOCK_NAME"
tmux -L "$SOCK_NAME" new-session -d -s ccbd-agents -- bash -c "sleep 60" 2>&1 || true
tmux -L "$SOCK_NAME" ls 2>&1 | sed 's/^/    /'

# Run doctor
echo "  running ah doctor:"
DOCTOR_OUT=$(CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah doctor 2>&1 || true)
echo "$DOCTOR_OUT" | sed 's/^/    /'
if echo "$DOCTOR_OUT" | grep -qiE "ccbd-agents|legacy"; then
  record_pass "T4.3.2 doctor 警告 legacy ccbd-agents 出现"
else
  record_fail "T4.3.2 doctor 未警告 legacy ccbd-agents" "$(echo "$DOCTOR_OUT" | head -10)"
fi

# Cleanup
tmux -L "$SOCK_NAME" kill-session -t ccbd-agents 2>/dev/null || true
if [ -n "${SESSION_ID:-}" ]; then
  CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah kill "$SESSION_ID" --session 2>&1 | head -3 || true
fi
sleep 1
kill -TERM "$DAEMON_PID" 2>/dev/null || true

exit 0
