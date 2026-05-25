#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STATE_DIR="$ROOT/target/dev_state"
SOCKET="$STATE_DIR/ccbd.sock"
LOG="/tmp/ccbd-ac-e2e.log"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

rm -f "$SOCKET" "$LOG" "$STATE_DIR"/ccbd.sqlite "$STATE_DIR"/ccbd.sqlite-*
cargo build --release --quiet
CCB_ENV=dev CCBD_STATE_DIR="$STATE_DIR" target/release/ccbd >"$LOG" 2>&1 &
DAEMON_PID=$!
for _ in {1..40}; do
  if [[ -S "$SOCKET" ]]; then
    break
  fi
  sleep 0.25
done

if [[ ! -S "$SOCKET" ]]; then
  cat "$LOG" >&2 || true
  exit 1
fi
python3 - <<'PY'
import json
import os
import signal
import socket
import sqlite3
import subprocess
import time

socket_path = "target/dev_state/ccbd.sock"
db_path = "target/dev_state/ccbd.sqlite"
next_id = 1

def rpc(method, params):
    global next_id
    req = {"jsonrpc": "2.0", "method": method, "params": params, "id": next_id}
    next_id += 1
    sock = socket.socket(socket.AF_UNIX)
    sock.connect(socket_path)
    sock.sendall((json.dumps(req) + "\n").encode())
    raw = sock.recv(65536).decode().strip()
    sock.close()
    obj = json.loads(raw)
    if "error" in obj:
        raise RuntimeError(f"{method} failed: {obj}")
    return obj["result"]

session = rpc("session.create", {
    "project_id": "p1",
    "absolute_path": "/tmp/ccbd-ac-e2e",
    "master_pid": 999,
})
session_id = session["session_id"]

spawn = rpc("agent.spawn", {
    "session_id": session_id,
    "agent_id": "ag_1",
    "provider": "bash",
})
assert spawn["state"] == "IDLE", spawn

time.sleep(0.2)
ps = subprocess.check_output(["ps", "aux"], text=True)
assert "bash" in ps, "bash process not visible in ps aux"

first = rpc("agent.send", {
    "agent_id": "ag_1",
    "text": "echo hello\n",
    "request_id": "req-1",
})
second = rpc("agent.send", {
    "agent_id": "ag_1",
    "text": "echo hello\n",
    "request_id": "req-1",
})
assert first["seq_id"] == second["seq_id"], (first, second)

conn = sqlite3.connect(db_path)
count = conn.execute(
    "SELECT COUNT(*) FROM events WHERE agent_id='ag_1' AND event_type='command_received' AND request_id='req-1'"
).fetchone()[0]
assert count == 1, count
pid = conn.execute("SELECT pid FROM agents WHERE id='ag_1'").fetchone()[0]
conn.close()

deadline = time.time() + 5
read = {"events": []}
while time.time() < deadline:
    read = rpc("agent.read", {"agent_id": "ag_1", "since_event_id": 0})
    if any("hello" in event["payload"] for event in read["events"]):
        break
    time.sleep(0.2)
assert any("hello" in event["payload"] for event in read["events"]), read
last_seq = max((event["seq_id"] for event in read["events"]), default=0)

os.kill(pid, signal.SIGKILL)
deadline = time.time() + 5
crashed = None
while time.time() < deadline:
    after = rpc("agent.read", {"agent_id": "ag_1", "since_event_id": last_seq})
    for event in after["events"]:
        if event["event_type"] == "state_change" and '"to":"CRASHED"' in event["payload"]:
            crashed = event
            break
    if crashed:
        break
    time.sleep(0.2)
assert crashed, "missing CRASHED state_change"
print("AC1-AC5 ok")
PY
