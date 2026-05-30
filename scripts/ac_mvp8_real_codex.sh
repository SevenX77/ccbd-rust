#!/usr/bin/env bash
# MVP8 AC6 real-codex E2E smoke:
# - spawn real codex agent under ccbd-rust
# - submit job via ccb ask "write a python hello world"
# - wait for completion via ccb pend
# - assert reply non-empty + contains a python hint (print/def/hello)
#
# Requires: ~/.codex/auth.json present and valid.
# Uses CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" → target/dev_state isolation.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STATE_DIR="$ROOT/target/dev_state"
SOCKET="$STATE_DIR/ahd.sock"
LOG="/tmp/ccbd-mvp8-smoke.log"
PROJECT="/tmp/ccbd-mvp8-smoke-project"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

mkdir -p "$STATE_DIR" "$PROJECT"
rm -f "$SOCKET" "$LOG" "$STATE_DIR"/ahd.sqlite*

cargo build --release --quiet --bin ahd --bin ccb
CCB_ENV=dev AH_STATE_DIR="$STATE_DIR" CCBD_UNSAFE_NO_SANDBOX=1 target/release/ahd >"$LOG" 2>&1 &
DAEMON_PID=$!

for _ in {1..40}; do
  [[ -S "$SOCKET" ]] && break
  sleep 0.25
done
[[ -S "$SOCKET" ]] || { cat "$LOG" >&2; exit 1; }
echo "[smoke] ccbd up at $SOCKET (pid=$DAEMON_PID)"

python3 - "$SOCKET" "$PROJECT" <<'PY'
import json, socket, sys, time

socket_path, project_path = sys.argv[1], sys.argv[2]
next_id = 1

def rpc(method, params, timeout=120):
    global next_id
    req = {"jsonrpc": "2.0", "method": method, "params": params, "id": next_id}
    next_id += 1
    sock = socket.socket(socket.AF_UNIX)
    sock.settimeout(timeout)
    sock.connect(socket_path)
    sock.sendall((json.dumps(req) + "\n").encode())
    buf = b""
    while True:
        chunk = sock.recv(65536)
        if not chunk: break
        buf += chunk
        try:
            obj = json.loads(buf.decode().strip())
            break
        except json.JSONDecodeError:
            continue
    sock.close()
    if "error" in obj:
        raise RuntimeError(f"{method} failed: {obj['error']}")
    return obj["result"]

session = rpc("session.create", {
    "project_id": "p_mvp8_smoke",
    "absolute_path": project_path,
    "master_pid": 99,
})
session_id = session["session_id"]
print(f"[smoke] session={session_id}")

spawn = rpc("agent.spawn", {
    "session_id": session_id,
    "agent_id": "ag_codex",
    "provider": "codex",
})
print(f"[smoke] codex spawn state={spawn.get('state')}")

# Wait for IDLE via system.dump (codex CLI takes ~5-15s to render fresh prompt)
deadline = time.time() + 60
last = None
while time.time() < deadline:
    dump = rpc("system.dump", {})
    last = next((a for a in dump.get("agents", []) if a["id"] == "ag_codex"), None)
    if last and last.get("state") == "IDLE":
        print(f"[smoke] codex IDLE")
        break
    time.sleep(1)
else:
    raise RuntimeError(f"codex not IDLE in 60s, last={last}")

submit = rpc("job.submit", {
    "agent_id": "ag_codex",
    "text": "Write a python hello world program. Just the code, no explanation.\n",
})
job_id = submit["job_id"]
print(f"[smoke] job_id={job_id}")

result = rpc("job.wait", {"job_id": job_id, "timeout": 120}, timeout=180)
status = result.get("status")
reply = result.get("reply_text") or ""
print(f"[smoke] status={status}")
print("[smoke] reply preview (first 600 chars):")
print(reply[:600])

assert status == "COMPLETED", f"expected COMPLETED, got {result}"
keywords = ["print", "hello", "Hello", "def", "world", "World"]
matched = [k for k in keywords if k in reply]
assert matched, f"no python hello-world keywords in reply: {reply!r}"
print(f"[smoke] AC6 OK — keywords matched: {matched}")
PY
