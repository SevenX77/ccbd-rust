#!/usr/bin/env bash
# core-fixes R3 e2e: absolute_path + no-bwrap systemd spawn 校准
#
# 跑法: bash scripts/r3_e2e.sh
#
# 覆盖 atomic task:
#   T3.1.1 master 执行 pwd 输出 project root
#   T3.1.2 agent 执行 pwd 输出 project root (NO_SANDBOX)
#   T3.2.1 no-bwrap spawn 不再注入 /workspace bind
#   T3.2.2 agent 默认 cwd 为 project root
#   T3.2.3 bwrap .git ro-bind 已 obsolete
#   T3.2.4 bwrap additional_ro_binds 已 obsolete
#
# 模式: 两段 — Part 1 NO_SANDBOX (R3.1.x), Part 2 default no-bwrap systemd scope (R3.2.x)
# 真 LLM 覆盖: Part 1 用 1 codex (真 LLM) 验证 R3.1 cwd 路径生效 + 1 bash (pwd 直读)
#              Part 2 用 1 bash 验证 systemd scope spawn + project cwd; bwrap mount 断言已 obsolete

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
PROJECT_ROOT_ABS=$(pwd -P)
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
  pkill -f "target/release/ahd" 2>/dev/null || true
  sleep 1
  kill_ccbd_tmux_servers
  rm -f /tmp/r3-test-fixture-*.txt 2>/dev/null || true
  if [ -n "${DAEMON_LOG:-}" ] && [ -f "$DAEMON_LOG" ]; then
    echo "  daemon log tail:"
    tail -25 "$DAEMON_LOG" 2>/dev/null | sed 's/^/    /' || true
  fi
  rm -f "${TEST_CONFIG:-}" "${SANDBOX_CONFIG:-}" 2>/dev/null || true
  echo ""
  echo "=== R3 e2e summary: $PASS_COUNT pass / $FAIL_COUNT fail ==="
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

# tmux socket name (sha256 of canonical state_dir)
mkdir -p "$STATE_DIR"
SOCK_NAME="ccbd-$(printf '%s' "$(realpath "$STATE_DIR")" | sha256sum | awk '{print substr($1,1,16)}')"
TMUX_SOCK="/tmp/tmux-$(id -u)/$SOCK_NAME"
echo "  expected tmux socket: $TMUX_SOCK"

# Fixture: project root 下放一个文件供 default no-bwrap agent 读
FIXTURE_FILE="$PROJECT_ROOT_ABS/r3-fixture-$(date +%s).txt"
echo "r3-fixture-content-marker-$(date +%N)" > "$FIXTURE_FILE"
echo "  fixture: $FIXTURE_FILE"

# Legacy custom ro bind fixture. PR2 T4 删除 bwrap 后 additional_ro_binds 不再产生 mount.
RO_BIND_FILE=$(mktemp -t r3-ro-bind-XXXXXX.txt)
echo "ro-bind-marker-$(date +%N)" > "$RO_BIND_FILE"
echo "  ro_bind fixture: $RO_BIND_FILE"

############################
# Part 1: NO_SANDBOX (R3.1.x)
############################
echo ""
echo "==========================================="
echo "=== Part 1: NO_SANDBOX (R3.1.1 / R3.1.2) ==="
echo "==========================================="

TEST_CONFIG=$(mktemp -t r3-nosandbox-XXXXXX.toml)
cat > "$TEST_CONFIG" <<'EOF'
version = "1"
[master]
enabled = false

[agents.a1]
provider = "codex"

[agents.a2]
provider = "bash"
EOF

DAEMON_LOG=$(mktemp -t r3-ccbd-XXXXXX.log)
CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 ./target/release/ahd > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
echo "  daemon_pid=$DAEMON_PID  log=$DAEMON_LOG"

for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 1
  if CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah ping 2>&1 | grep -q "ok\|sessions="; then
    echo "  daemon ready"; break
  fi
done

START_OUT=$(CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 ./target/release/ah --config "$TEST_CONFIG" start --wait 2>&1 || echo "START_FAILED")
echo "$START_OUT" | head -8 | sed 's/^/  /'
SESSION_ID=$(echo "$START_OUT" | grep -oE 'session_id=[a-z0-9_-]+' | head -1 | cut -d= -f2 || true)

echo "  socket (precomputed): $TMUX_SOCK"

# T3.1.2: agent_a2 (bash) pwd
echo ""
echo "--- T3.1.2: agent (bash) pwd → project root ---"
sleep 2
tmux -L "$SOCK_NAME" send-keys -t agent_a2 "pwd" Enter
sleep 2
A2_PANE=$(tmux -L "$SOCK_NAME" capture-pane -p -t agent_a2 -S -20 2>/dev/null || true)
echo "  agent_a2 pane content:"
echo "$A2_PANE" | tail -10 | sed 's/^/    /'
if echo "$A2_PANE" | grep -qF "$PROJECT_ROOT_ABS"; then
  record_pass "T3.1.2 NO_SANDBOX agent pwd 含 project_root_abs"
else
  record_fail "T3.1.2 NO_SANDBOX agent pwd 未含 project_root" "expected $PROJECT_ROOT_ABS"
fi

# T3.1.1: master 跳过 (master.enabled=false 下不创建 master pane)
# 真验证: 改 config master.enabled=true 太重 (会拉真 claude),用 spawn_master_pane RPC 路径已被 src/rpc/handlers.rs:223 cover
# 这里改用 ps 看 sessions 表 absolute_path 字段
echo ""
echo "--- T3.1.1: master_cwd 路径 (via sqlite sessions.absolute_path) ---"
SQLITE_DB="target/dev_state/ahd.sqlite"
if [ -f "$SQLITE_DB" ]; then
  ABS_PATH=$(sql_query "$SQLITE_DB" "SELECT projects.absolute_path FROM sessions JOIN projects ON sessions.project_id=projects.id WHERE sessions.id='$SESSION_ID'" || true)
  echo "  sessions.absolute_path=$ABS_PATH"
  if [ "$ABS_PATH" = "$PROJECT_ROOT_ABS" ]; then
    record_pass "T3.1.1/T3.1.4 sessions.absolute_path 存为 project_root_abs"
  else
    record_fail "T3.1.1 absolute_path 不等于 project_root" "$ABS_PATH != $PROJECT_ROOT_ABS"
  fi
fi

# T3.1.2 verify codex (真 LLM) tmux session cwd
echo ""
echo "--- T3.1.2 (codex 真 LLM): codex pane cwd 设置 ---"
# tmux pane_current_path 反映 PTY 进程的 cwd
A1_CWD=$(tmux -L "$SOCK_NAME" display-message -t agent_a1 -p '#{pane_current_path}' 2>/dev/null || echo "n/a")
echo "  agent_a1 (codex) pane_current_path: $A1_CWD"
if [ "$A1_CWD" = "$PROJECT_ROOT_ABS" ]; then
  record_pass "T3.1.2 codex (真 LLM) pane_current_path = project_root_abs"
else
  record_fail "T3.1.2 codex pane_current_path 异常" "$A1_CWD != $PROJECT_ROOT_ABS"
fi

# Cleanup Part 1
echo ""
echo "--- Part 1 cleanup ---"
CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah kill "$SESSION_ID" --session 2>&1 | head -3 || true
sleep 2
kill -TERM "$DAEMON_PID" 2>/dev/null || true
sleep 2
PIDS=$(pidof ccbd 2>/dev/null || true)
for p in $PIDS; do kill -KILL "$p" 2>/dev/null || true; done
# Also explicitly kill the tmux server (in case daemon shutdown didn't get it)
tmux -L "$SOCK_NAME" kill-server 2>/dev/null || true
sleep 1
rm -f "$TEST_CONFIG"

############################
# Part 2: default no-bwrap systemd scope (R3.2.x)
############################
echo ""
echo "==========================================="
echo "=== Part 2: no-bwrap systemd scope (R3.2) ==="
echo "==========================================="

# Fresh state
rm -rf target/dev_state/ahd.sqlite* target/dev_state/pipes target/dev_state/sandboxes 2>/dev/null || true

SANDBOX_CONFIG=$(mktemp -t r3-sandbox-XXXXXX.toml)
cat > "$SANDBOX_CONFIG" <<EOF
version = "1"
[master]
enabled = false

[sandbox]
additional_ro_binds = ["$RO_BIND_FILE"]

[agents.b1]
provider = "bash"
EOF
echo "  no-bwrap config: 1 bash agent, legacy additional_ro_binds = $RO_BIND_FILE (obsolete)"
echo "  config: $SANDBOX_CONFIG"

DAEMON_LOG=$(mktemp -t r3-sb-ccbd-XXXXXX.log)
CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ahd > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
echo "  daemon_pid=$DAEMON_PID  log=$DAEMON_LOG"

for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 1
  if CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah ping 2>&1 | grep -q "ok\|sessions="; then
    echo "  daemon ready"; break
  fi
done

START_OUT=$(CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah --config "$SANDBOX_CONFIG" start --wait 2>&1 || echo "START_FAILED")
echo "$START_OUT" | head -10 | sed 's/^/  /'
SESSION_ID=$(echo "$START_OUT" | grep -oE 'session_id=[a-z0-9_-]+' | head -1 | cut -d= -f2 || true)

echo "  socket (precomputed): $TMUX_SOCK"
sleep 3  # let agent settle / spawn cmd to be flushed

# Verify daemon spawn cmd uses systemd scope directly; bwrap argv is obsolete after PR2 T4.
SPAWN_CMD_LINE=$(grep -E "spawn cmd:.*systemd-run" "$DAEMON_LOG" 2>/dev/null | head -1 || true)

echo ""
echo "--- T3.2.1 / T3.2.2: no-bwrap systemd spawn + provider command direct ---"
if [ -z "$SPAWN_CMD_LINE" ]; then
  record_fail "T3.2.1/.2.2 daemon log 未见 systemd-run spawn cmd" "(spawn 失败或日志未捕获)"
else
  echo "  spawn cmd extract:"
  echo "$SPAWN_CMD_LINE" | grep -oE -- 'systemd-run|--user|--scope|-- env|bash --noprofile --norc -i|bwrap' | head -10 | sed 's/^/    /'
  if echo "$SPAWN_CMD_LINE" | grep -qF -- "systemd-run" && echo "$SPAWN_CMD_LINE" | grep -qF -- "--scope"; then
    record_pass "T3.2.1 systemd-run --scope wrapper 保留"
  else
    record_fail "T3.2.1 systemd scope wrapper 缺失" "$SPAWN_CMD_LINE"
  fi
  if echo "$SPAWN_CMD_LINE" | grep -qF -- "bwrap"; then
    record_fail "T3.2.1 spawn cmd 不应再包含 bwrap" "$SPAWN_CMD_LINE"
  else
    record_pass "T3.2.1 spawn cmd 无 bwrap"
  fi
  if echo "$SPAWN_CMD_LINE" | grep -qF -- "-- env" && echo "$SPAWN_CMD_LINE" | grep -qF -- "bash --noprofile --norc -i"; then
    record_pass "T3.2.2 provider command 直接位于 systemd '-- env' 后"
  else
    record_fail "T3.2.2 provider direct command 形态异常" "$SPAWN_CMD_LINE"
  fi
fi

echo ""
echo "--- T3.2.3: obsolete - bwrap .git ro-bind 已删除 ---"
record_pass "T3.2.3 obsolete: PR2 T4 删除 bwrap 后不再验证 .git ro-bind"

echo ""
echo "--- T3.2.4: obsolete - bwrap additional_ro_binds 已删除 ---"
record_pass "T3.2.4 obsolete: PR2 T4 删除 bwrap 后 additional_ro_binds 不再产生 mount argv"

# Run-time verification: bash pane should start in project root; no /workspace bind exists.
echo ""
echo "--- T3.2.2 run-time: pane 内 pwd = project root ---"
tmux -L "$SOCK_NAME" send-keys -t agent_b1 "pwd" Enter
sleep 2
B1_PANE=$(tmux -L "$SOCK_NAME" capture-pane -p -t agent_b1 -S -30 2>/dev/null || true)
echo "  agent_b1 pane (last 8 lines):"
echo "$B1_PANE" | tail -8 | sed 's/^/    /'
if echo "$B1_PANE" | grep -qF "$PROJECT_ROOT_ABS"; then
  record_pass "T3.2.2 run-time no-bwrap pwd = project root"
else
  record_fail "T3.2.2 run-time pwd 非 project root" "$(echo "$B1_PANE" | tail -2)"
fi
echo ""
echo "--- T3.2.1 run-time: cat project fixture 可读 ---"
FIXTURE_BASENAME=$(basename "$FIXTURE_FILE")
tmux -L "$SOCK_NAME" send-keys -t agent_b1 "cat ./$FIXTURE_BASENAME 2>&1 | head -3" Enter
sleep 2
B1_PANE=$(tmux -L "$SOCK_NAME" capture-pane -p -t agent_b1 -S -40 2>/dev/null || true)
echo "  agent_b1 pane (cat):"
echo "$B1_PANE" | tail -10 | sed 's/^/    /'
if echo "$B1_PANE" | grep -q "r3-fixture-content-marker"; then
  record_pass "T3.2.1 run-time no-bwrap 可读 project fixture"
else
  record_fail "T3.2.1 run-time fixture 读失败" ""
fi

# Final cleanup Part 2
CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" ./target/release/ah kill "$SESSION_ID" --session 2>&1 | head -3 || true
sleep 1
kill -TERM "$DAEMON_PID" 2>/dev/null || true
sleep 1
rm -f "$FIXTURE_FILE" "$RO_BIND_FILE"

exit 0
