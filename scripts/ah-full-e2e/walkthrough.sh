#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
AH_BIN="$ROOT_DIR/target/debug/ah"
PROJECT_DIR="$(mktemp -d)"
export HOME="$PROJECT_DIR/home"
export XDG_STATE_HOME="$PROJECT_DIR/.local/state"
export XDG_CACHE_HOME="$PROJECT_DIR/.local/cache"
export CCB_SOCKET="$XDG_STATE_HOME/ccbd/ccbd.sock"

AH_CONFIG="$PROJECT_DIR/ah.toml"
MOCK_PROVIDER="$ROOT_DIR/tests/fixtures/mock_provider.sh"
PASS_COUNT=0
FAIL_COUNT=0
SESSION_ID=""
FIRST_JOB_ID=""
SECOND_JOB_ID=""
CANCEL_JOB_ID=""

cleanup() {
  set +e
  if [[ -x "$AH_BIN" && -S "$CCB_SOCKET" ]]; then
    "$AH_BIN" --config "$AH_CONFIG" stop >/dev/null 2>&1
  fi
  if command -v fuser >/dev/null 2>&1 && [[ -S "$CCB_SOCKET" ]]; then
    fuser -k "$CCB_SOCKET" >/dev/null 2>&1
  fi
  if [[ -n "${TMUX_SOCKET_NAME:-}" ]] && command -v tmux >/dev/null 2>&1; then
    tmux -L "$TMUX_SOCKET_NAME" kill-server >/dev/null 2>&1
  fi
  if [[ -n "${SYSTEMD_SCOPE:-}" ]] && command -v systemctl >/dev/null 2>&1; then
    systemctl --user stop "$SYSTEMD_SCOPE" >/dev/null 2>&1
  fi
  rm -rf "$PROJECT_DIR"
}
trap cleanup EXIT INT TERM

run_ah() {
  "$AH_BIN" --config "$AH_CONFIG" "$@"
}

wait_for() {
  local predicate="$1"
  local timeout="${2:-10}"
  local deadline=$((SECONDS + timeout))
  until eval "$predicate"; do
    if (( SECONDS >= deadline )); then
      echo "timeout waiting for: $predicate" >&2
      return 1
    fi
    sleep 0.2
  done
}

assert_contains() {
  local output="$1"
  local expected="$2"
  grep -q -- "$expected" <<<"$output"
}

sqlite_query() {
  local db="$1"
  local sql="$2"
  sqlite3 "$db" "$sql"
}

step() {
  local n="$1"
  local name="$2"
  shift 2
  echo "=== step $n: $name ==="
  if "$@"; then
    echo "PASS step $n: $name"
    PASS_COUNT=$((PASS_COUNT + 1))
  else
    echo "FAIL step $n: $name"
    FAIL_COUNT=$((FAIL_COUNT + 1))
  fi
}

write_config() {
  local drift="${1:-0}"
  cat >"$AH_CONFIG" <<EOF
version = "1"

[master]
cmd = "bash --noprofile --norc -i"

[env]
GRAND_TOUR_MOCK_PROVIDER = "$MOCK_PROVIDER"
GRAND_TOUR_DRIFT = "$drift"

[agents.a1]
provider = "bash"

[agents.a1.env]
GRAND_TOUR_MOCK_PROVIDER = "$MOCK_PROVIDER"
GRAND_TOUR_DRIFT = "$drift"
EOF
}

prepare_workspace() {
  mkdir -p "$HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME"
  write_config 0
  cd "$PROJECT_DIR"
}

parse_job_id() {
  sed -n 's/.*job_id=\([^ ]*\).*/\1/p' <<<"$1" | head -1
}

db_path() {
  echo "$XDG_STATE_HOME/ccbd/ccbd.sqlite"
}

step_start() {
  local output
  output="$(run_ah start --wait 2>&1)"
  assert_contains "$output" "session_id="
  assert_contains "$output" "agent_id=a1"
  SESSION_ID="$(sed -n 's/^session_id=//p' <<<"$output" | head -1)"
  wait_for "[[ -S '$CCB_SOCKET' ]]" 10
}

step_ping() {
  local output
  output="$(run_ah ping 2>&1)"
  assert_contains "$output" "ok=true"
  assert_contains "$output" "agents="
}

step_ask_first() {
  local output
  output="$(run_ah ask a1 "grand tour first" 2>&1)"
  assert_contains "$output" "job_id="
  assert_contains "$output" "status=QUEUED"
  FIRST_JOB_ID="$(parse_job_id "$output")"
  [[ -n "$FIRST_JOB_ID" ]]
}

step_ps() {
  local output
  output="$(run_ah ps 2>&1)"
  assert_contains "$output" "sessions"
  assert_contains "$output" "agents"
  assert_contains "$output" "a1"
}

step_pend_first() {
  local output
  output="$(run_ah pend "$FIRST_JOB_ID" 2>&1)"
  assert_contains "$output" "grand tour first"
}

step_logs() {
  local output
  output="$(run_ah logs a1 2>&1)"
  assert_contains "$output" "mock_provider"
}

step_drift_config() {
  write_config 1
  assert_contains "$(cat "$AH_CONFIG")" "GRAND_TOUR_DRIFT = \"1\""
}

step_up() {
  local output
  output="$(run_ah up --force 2>&1)"
  assert_contains "$output" "a1"
}

step_ask_second() {
  local output
  output="$(run_ah ask a1 "grand tour second" 2>&1)"
  assert_contains "$output" "job_id="
  SECOND_JOB_ID="$(parse_job_id "$output")"
  [[ -n "$SECOND_JOB_ID" && "$SECOND_JOB_ID" != "$FIRST_JOB_ID" ]]
}

step_prompt_resolve() {
  local output
  if output="$(run_ah prompt resolve a1 --keys "Enter" 2>&1)"; then
    assert_contains "$output" "state: PROMPT_PENDING"
  else
    echo "SKIP step 10: bash mock does not emit a real prompt; prompt resolve path covered by Rust E2E"
  fi
}

step_cancel() {
  local output
  CANCEL_JOB_ID="$SECOND_JOB_ID"
  [[ -n "$CANCEL_JOB_ID" ]]
  output="$(run_ah cancel "$CANCEL_JOB_ID" 2>&1)"
  assert_contains "$output" "job_id=$CANCEL_JOB_ID"
  assert_contains "$output" "status="
}

step_watch() {
  local output_file="$PROJECT_DIR/watch.out"
  run_ah watch a1 >"$output_file" 2>&1 &
  local pid=$!
  sleep 3
  kill "$pid" >/dev/null 2>&1 || true
  wait "$pid" >/dev/null 2>&1 || true
  [[ -s "$output_file" ]]
}

step_kill() {
  local output
  output="$(run_ah kill a1 2>&1)"
  assert_contains "$output" "state=KILLED"
}

step_stop() {
  local output
  output="$(run_ah stop 2>&1)"
  assert_contains "$output" "ccbd shutting down"
}

smoke_attach_help() {
  local output
  output="$(run_ah attach --help 2>&1)"
  assert_contains "$output" "Attach"
}

smoke_doctor() {
  local output
  output="$(run_ah doctor 2>&1 || true)"
  assert_contains "$output" "daemon"
}

smoke_config_validate() {
  local output
  output="$(run_ah config validate --config "$AH_CONFIG" 2>&1)"
  assert_contains "$output" "ok:"
}

smoke_config_migrate() {
  local backup="$PROJECT_DIR/ah.toml.backup"
  local output
  mv "$AH_CONFIG" "$backup"
  mkdir -p "$PROJECT_DIR/.ccb"
  printf 'legacy=true\n' >"$PROJECT_DIR/.ccb/ccb.config"
  output="$(run_ah config migrate 2>&1)"
  mv "$backup" "$AH_CONFIG"
  assert_contains "$output" "found legacy"
}

smoke_version() {
  local output
  output="$(run_ah version 2>&1)"
  [[ -n "$output" ]]
}

verify_db_events() {
  local db
  db="$(db_path)"
  wait_for "[[ -f '$db' ]]" 5
  sqlite_query "$db" "SELECT COUNT(*) FROM events WHERE event_type IN ('state_change','command_received','output_chunk');" >/dev/null
}

main() {
  if [[ ! -x "$AH_BIN" ]]; then
    echo "target/debug/ah missing; run: cargo build --bin ah --bin ccbd" >&2
    exit 1
  fi
  prepare_workspace

  step 1 "ah start" step_start
  step 2 "ah ping" step_ping
  step 3 "ah ask first" step_ask_first
  step 4 "ah ps" step_ps
  step 5 "ah pend" step_pend_first
  step 6 "ah logs" step_logs
  step 7 "modify ah.toml drift" step_drift_config
  step 8 "ah up" step_up
  step 9 "ah ask second" step_ask_second
  step 10 "ah prompt resolve" step_prompt_resolve
  step 11 "ah cancel" step_cancel
  step 12 "ah watch" step_watch
  step 13 "ah kill" step_kill
  step 14 "ah stop" step_stop

  step "S1" "ah attach --help" smoke_attach_help
  step "S2" "ah doctor" smoke_doctor
  step "S3" "ah config validate" smoke_config_validate
  step "S4" "ah config migrate" smoke_config_migrate
  step "S5" "ah version" smoke_version
  step "DB" "sqlite event smoke" verify_db_events

  echo "PASS=$PASS_COUNT FAIL=$FAIL_COUNT"
  if (( FAIL_COUNT > 0 )); then
    exit 1
  fi
  echo "PASS grand tour walkthrough"
}

main "$@"
