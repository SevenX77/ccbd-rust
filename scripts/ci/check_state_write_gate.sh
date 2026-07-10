#!/usr/bin/env bash
# CI grep rule: bans direct UPDATE agents SET state/status outside db/perception/gate.rs

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT="${1:-$script_dir/../../src}"

if [ ! -e "$ROOT" ]; then
    echo "Error: Target path '$ROOT' does not exist." >&2
    exit 1
fi

# Baseline validation function
check_baseline() {
    local path="$1"
    local content="$2"

    case "$path" in
        "rpc/handlers/ack.rs")
            [[ "$content" == *"UPDATE agents SET state = 'STUCK', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state = 'WAITING_FOR_ACK' AND state_version = ?"* ]] && return 0
            ;;
        "rpc/handlers/sessions.rs")
            [[ "$content" == *"UPDATE agents SET state = 'KILLED', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ?1 AND session_id = ?2 AND state NOT IN ('CRASHED', 'KILLED')"* ]] && return 0
            ;;
        "orchestrator/mod.rs")
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', state_version = state_version + 1 WHERE id = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'BUSY', state_version = state_version + 1 WHERE id = ?"* ]] && return 0
            ;;
        "prompt_handler/integration.rs")
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', sub_state = 'PromptIdleSelfHealed', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state = 'PROMPT_PENDING' AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'PROMPT_PENDING', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('IDLE', 'SPAWNING') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'PROMPT_PENDING', state_version = 5 WHERE id = 'a1'"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', state_version = 6 WHERE id = 'a1'"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', sub_state = 'ManualResolve', state_version = state_version + 1 WHERE id = ?"* ]] && return 0
            ;;
        "prompt_handler/resolve.rs")
            [[ "$content" == *"UPDATE agents SET state = ? WHERE id = 'a1'"* ]] && return 0
            ;;
        "marker/timer.rs")
            [[ "$content" == *"UPDATE agents SET state = 'BUSY', state_version = state_version + 1 WHERE id = ?"* ]] && return 0
            ;;
        "db/jobs.rs")
            [[ "$content" == *"UPDATE agents SET state = 'STUCK' WHERE id = 'a1'"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = ? WHERE id = 'a1'"* ]] && return 0
            ;;
        "db/job_state.rs")
            [[ "$content" == *"UPDATE agents SET state = ? WHERE id = ?"* ]] && return 0
            ;;
        "db/agents_lifecycle.rs")
            [[ "$content" == *"UPDATE agents SET state = 'KILLED', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state NOT IN ('CRASHED', 'KILLED')"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'CRASHED', exit_code = ?, error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state NOT IN ('CRASHED', 'KILLED', 'PROMPT_PENDING')"* ]] && return 0
            ;;
        "db/state_machine.rs")
            [[ "$content" == *"UPDATE agents SET state = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'PROMPT_PENDING', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('IDLE', 'SPAWNING') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', sub_state = 'Matched', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY', 'STUCK') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', sub_state = 'Matched', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', sub_state = 'HookEvent', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('WAITING_FOR_ACK', 'BUSY', 'STUCK') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', sub_state = 'LogEvent', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('WAITING_FOR_ACK', 'BUSY', 'STUCK') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'BUSY', sub_state = 'Deferred', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('WAITING_FOR_ACK', 'BUSY', 'STUCK') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'STUCK', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('BUSY', 'WAITING_FOR_ACK') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'UNKNOWN', error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY') AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = ?, error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state = ? AND state_version = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = ?, state_version = state_version + 1 WHERE id = ?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'IDLE' WHERE id = 'a_cancel'"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'BUSY' WHERE id = 'a_cancel'"* ]] && return 0
            ;;
        "db/agents.rs")
            [[ "$content" == *"UPDATE agents SET state = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state != 'CRASHED'"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'CRASHED' WHERE id = 'a1'"* ]] && return 0
            ;;
        "db/system.rs")
            [[ "$content" == *"UPDATE agents SET state = 'CRASHED', error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY', 'IDLE')"* ]] && return 0
            ;;
        "db/state_machine_assert.rs")
            [[ "$content" == *"UPDATE agents SET state='IDLE', sub_state='Asserted', state_version=state_version+1, updated_at=unixepoch() WHERE id=? AND state IN ('UNKNOWN', 'WAITING_FOR_ACK') AND state_version=?"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state='BUSY' WHERE id='a_assert'"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state='UNKNOWN' WHERE id='a_assert'"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = ? WHERE id = 'a_ack_assert'"* ]] && return 0
            ;;
        "monitor/master_watch.rs")
            [[ "$content" == *"UPDATE agents SET state = 'IDLE' WHERE id = 'a_worker_gate'"* ]] && return 0
            [[ "$content" == *"UPDATE agents SET state = 'IDLE', pid = 20 WHERE id = 'a_lifecycle_happy'"* ]] && return 0
            ;;
    esac

    return 1
}

# Resolve list of files to check
files=()
if [ -f "$ROOT" ]; then
    files+=("$ROOT")
else
    # Recursively find all Rust source files
    while IFS= read -r -d '' file; do
        files+=("$file")
    done < <(find "$ROOT" -type f -name "*.rs" -print0 2>/dev/null)
fi

ABS_ROOT=$(realpath "$ROOT")
errors=0

for file in "${files[@]}"; do
    ABS_FILE=$(realpath "$file")
    if [ -d "$ROOT" ]; then
        rel_path=${ABS_FILE#$ABS_ROOT/}
    else
        rel_path=$(basename "$ABS_FILE")
    fi

    # Exempt the perception gate file (always allowed to issue direct writes)
    if [[ "$rel_path" == *"db/perception/gate.rs" ]]; then
        continue
    fi

    # Look for matching SQL queries
    matches=$(grep -nE "UPDATE[[:space:]]+agents[[:space:]]+SET[[:space:]]+\b(state|status)\b" "$file" || true)
    if [ -n "$matches" ]; then
        while IFS= read -r match_line; do
            [ -z "$match_line" ] && continue
            line_num=$(echo "$match_line" | cut -d: -f1)
            content=${match_line#*:}
            trimmed=$(echo "$content" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

            if [[ "$trimmed" == "//"* ]]; then
                continue
            fi

            if ! check_baseline "$rel_path" "$trimmed"; then
                echo "VIOLATION: Direct state/status write found at $rel_path:$line_num" >&2
                echo "  Line: $trimmed" >&2
                errors=$((errors + 1))
            fi
        done <<< "$matches"
    fi
done

if [ $errors -gt 0 ]; then
    echo "Error: $errors direct state/status write violation(s) detected outside db/perception/gate.rs" >&2
    exit 1
fi

echo "All checks passed successfully."
exit 0
