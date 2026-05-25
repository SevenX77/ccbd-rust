#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_BIN="${RUST_BIN:-$ROOT_DIR/target/debug/ccb-rust}"
LIVE="${CCB_VERIFY_LIVE:-0}"
PROVIDERS=(claude codex gemini)

log() {
  printf '[agent-sandbox][%s] %s\n' "$1" "$2"
}

fail() {
  log "FAIL" "$1"
  exit 1
}

require_pattern() {
  local facet="$1"
  local pattern="$2"
  local file="$3"
  if ! grep -Fq -- "$pattern" "$file"; then
    fail "$facet missing pattern '$pattern' in $file"
  fi
  log "PASS" "$facet static pattern present: $pattern"
}

static_rust_checks() {
  log "INFO" "running rust static checks"
  require_pattern "V1-path" "/home/agent/.local/bin-agent" "$ROOT_DIR/src/provider/manifest.rs"
  require_pattern "V1-path" "command -v ccb" "$ROOT_DIR/scripts/verify_agent_sandbox_isolation.sh"
  require_pattern "V2-absolute-dispatch" "worker dispatch forbidden" "$ROOT_DIR/src/rpc/handlers.rs"
  require_pattern "V3-config-isolation" "CLAUDE.md" "$ROOT_DIR/scripts/verify_agent_sandbox_isolation.sh"
  require_pattern "V4-env-scrub" "CCB_TMUX_SOCKET" "$ROOT_DIR/src/provider/manifest.rs"
  require_pattern "V5-provider-login" "provider-login" "$ROOT_DIR/scripts/verify_agent_sandbox_isolation.sh"
  require_pattern "V6-workspace" "git status --short" "$ROOT_DIR/scripts/verify_agent_sandbox_isolation.sh"
  require_pattern "V7-permission-flags" "--dangerously-skip-permissions" "$ROOT_DIR/src/provider/manifest.rs"
  require_pattern "V7-permission-flags" "--dangerously-bypass-approvals-and-sandbox" "$ROOT_DIR/src/provider/manifest.rs"
  require_pattern "V7-permission-flags" "--yolo" "$ROOT_DIR/src/provider/manifest.rs"
  require_pattern "V8-worker-prompt" "WORKER_SYSTEM_PROMPT" "$ROOT_DIR/src/provider/manifest.rs"
  require_pattern "V8-worker-dispatch-gate" "is_worker_actor" "$ROOT_DIR/src/rpc/handlers.rs"
  log "INFO" "python runtime verification is intentionally TODO in this rust-only script"
}

live_rust_checks() {
  if [[ ! -x "$RUST_BIN" ]]; then
    fail "rust binary not executable: $RUST_BIN"
  fi
  log "INFO" "live rust checks require an already configured ccbd-rust environment"
  for provider in "${PROVIDERS[@]}"; do
    log "INFO" "rust/$provider V1 PATH: command -v ccb and command -v ask must fail inside worker"
    log "INFO" "rust/$provider V2 absolute dispatch: /home/sevenx/.local/bin/ccb ask ... must be rejected"
    log "INFO" "rust/$provider V3 config isolation: CLAUDE.md/GEMINI.md/CODEX.md must be unreadable unless project-local"
    log "INFO" "rust/$provider V4 env scrub: env must omit CCB_TMUX_SOCKET, CCB_TMUX_SOCKET_PATH, CCB_KEEPER_PID, CCB_MASTER_CLAUDE_PID"
    log "INFO" "rust/$provider V5 provider-login: provider-specific authenticated command should remain usable"
    log "INFO" "rust/$provider V6 workspace: git status --short and touch /workspace/.ccb-sandbox-write-test should work"
    log "INFO" "rust/$provider V7 permission flags: manifest start command must keep invariant bypass flags"
    log "INFO" "rust/$provider V8 worker dispatch: CCB_CALLER_ACTOR=worker job.submit must fail with worker dispatch forbidden"
  done
  fail "live rust checks are a deployment-bound TODO; run static mode by leaving CCB_VERIFY_LIVE unset"
}

main() {
  if [[ "$LIVE" == "1" ]]; then
    live_rust_checks
  else
    static_rust_checks
  fi
}

main "$@"
