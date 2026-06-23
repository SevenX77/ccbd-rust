#!/usr/bin/env bash
set -euo pipefail

# Mock provider for ah dogfood M1 tests.
# It accepts one dispatched message per line, emits visible work output, and
# ends each job with the protocol marker planned for the real completion path.

provider="${MOCK_DOGFOOD_PROVIDER:-claude}"
delay_ms="${FAKE_PROVIDER_DELAY_MS:-0}"
stuck_ms="${FAKE_PROVIDER_STUCK_MS:-0}"
slash_ack_text="${FAKE_PROVIDER_SLASH_ACK_TEXT:-}"
default_job_id="${AH_DISPATCHED_JOB_ID:-}"
prompt="${MOCK_DOGFOOD_PROMPT:-$ }"

sleep_delay() {
  local ms="$1"
  if [[ -z "$ms" || "$ms" == "0" ]]; then
    return 0
  fi
  python3 - "$ms" <<'PY'
import sys
import time

time.sleep(int(sys.argv[1]) / 1000.0)
PY
}

trim_cr() {
  local value="$1"
  printf '%s' "${value%$'\r'}"
}

extract_job_id() {
  local first_line="$1"
  local fallback="$2"

  if [[ -n "$fallback" ]]; then
    printf '%s' "$fallback"
    return 0
  fi

  case "$first_line" in
    job-id:*)
      first_line="${first_line#job-id:}"
      first_line="${first_line%%[[:space:]]*}"
      printf '%s' "$first_line"
      return 0
      ;;
    job_id:*)
      first_line="${first_line#job_id:}"
      first_line="${first_line%%[[:space:]]*}"
      printf '%s' "$first_line"
      return 0
      ;;
    *"job-id="*)
      first_line="${first_line#*job-id=}"
      first_line="${first_line%%[[:space:]]*}"
      printf '%s' "$first_line"
      return 0
      ;;
    *)
      printf 'missing-job-id'
      return 0
      ;;
  esac
}

emit_ready() {
  case "$provider" in
    claude)
      printf 'status Sonnet\n────────\n  ❯ '
      ;;
    codex)
      printf 'mock_dogfood_provider: codex ready\n  › '
      ;;
    *)
      printf 'mock_dogfood_provider: ready\n%s' "$prompt"
      ;;
  esac
}

emit_done_prompt() {
  case "$provider" in
    claude)
      printf '  ❯ '
      ;;
    codex)
      printf '  › '
      ;;
    *)
      printf '%s' "$prompt"
      ;;
  esac
}

emit_slash_ack() {
  local cmd="$1"
  if [[ -n "$slash_ack_text" ]]; then
    local ack="${slash_ack_text//\$cmd/$cmd}"
    printf '%s\n' "$ack"
  else
    printf '<<ah-slash-ack:cmd=%s>>\n' "$cmd"
  fi
}

emit_ready

while IFS= read -r raw_line; do
  line="$(trim_cr "$raw_line")"
  [[ -z "$line" ]] && continue

  if [[ "$line" == /* ]]; then
    printf '\nmock_dogfood_provider[%s]: slash cmd=%s\n' "$provider" "$line"
    emit_slash_ack "$line"
    emit_done_prompt
    continue
  fi

  job_id="$(extract_job_id "$line" "$default_job_id")"
  printf '\nmock_dogfood_provider[%s]: received=%s\n' "$provider" "$line"
  if [[ -n "$stuck_ms" && "$stuck_ms" != "0" ]]; then
    printf 'mock_dogfood_provider[%s]: Thinking...\n' "$provider"
    sleep_delay "$stuck_ms"
  fi
  sleep_delay "$delay_ms"
  printf 'mock_dogfood_provider[%s]: working job_id=%s\n' "$provider" "$job_id"
  printf 'mock_dogfood_provider[%s]: done job_id=%s\n' "$provider" "$job_id"
  printf '<<ah-idle:job-id=%s>>\n' "$job_id"
  emit_done_prompt
done

printf '\nmock_dogfood_provider[%s]: stdin closed\n' "$provider"
