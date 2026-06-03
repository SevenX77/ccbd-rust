#!/usr/bin/env bash
set -euo pipefail

prompt=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --prompt)
      prompt="${2:-}"
      shift 2
      ;;
    -h|--help)
      echo "usage: $0 --prompt codex_update|codex_update_ready|trust_path|unknown_eula|transient_ready|claude_try_ready|stable_unknown"
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

case "$prompt" in
  codex_update)
    printf '\033[?1049h\033[2J\033[H'
    cat <<'EOF'
Update available! 0.129.0 -> 0.130.0
Run `npm install -g @openai/codex` to update.

1) Update now
2) Skip for now
EOF
    ;;
  codex_update_ready)
    printf '\033[?1049h\033[2J\033[H'
    cat <<'EOF'
Update available! 0.129.0 -> 0.130.0
Run `npm install -g @openai/codex` to update.

1) Update now
2) Skip for now
EOF
    ;;
  trust_path)
    printf '\033[?1049h\033[2J\033[H'
    cat <<'EOF'
Do you trust this directory?

1) Yes, trust this workspace
2) No, exit
EOF
    ;;
  unknown_eula)
    printf '\033[?1049h\033[2J\033[H'
    cat <<'EOF'
New provider EULA requires review before continuing.

1) Accept terms and continue
2) Decline and exit
EOF
    ;;
  transient_ready)
    printf '\033[?1049h\033[2J\033[H'
    cat <<'EOF'
Loading provider shell...
New transient startup panel still rendering
EOF
    sleep 1
    printf '\033[2J\033[H'
    printf 'mock_prompt_provider: ready\n'
    printf '\033[60;1H  › '
    stty raw -echo 2>/dev/null || true
    while true; do
      probe=""
      while IFS= read -rsn1 probe; do
        [[ "$probe" != $'\r' && "$probe" != $'\n' ]] && break
      done
      [[ -z "$probe" ]] && break
      [[ "$probe" == $'\177' || "$probe" == $'\b' ]] && continue
      printf '%s' "$probe"
      sleep 0.05
      printf '\033[D \033[D'
    done
    stty sane 2>/dev/null || true
    sleep 30
    exit 0
    ;;
  claude_try_ready)
    printf '\033[?1049h\033[2J\033[H'
    printf 'Claude Code\n'
    printf 'Opus 4.8 (1M context)\n'
    printf '\033[60;1H❯ Try "fix lint errors"'
    stty raw -echo 2>/dev/null || true
    while true; do
      probe=""
      while IFS= read -rsn1 probe; do
        [[ "$probe" != $'\r' && "$probe" != $'\n' ]] && break
      done
      [[ -z "$probe" ]] && break
      [[ "$probe" == $'\177' || "$probe" == $'\b' ]] && continue
      printf '%s' "$probe"
      sleep 0.05
      printf '\033[D \033[D'
    done
    stty sane 2>/dev/null || true
    sleep 30
    exit 0
    ;;
  stable_unknown)
    printf '\033[?1049h\033[2J\033[H'
    cat <<'EOF'
Mystery provider startup panel
No known prompt or seed readiness marker is visible.
EOF
    sleep 30
    exit 0
    ;;
  *)
    echo "missing or invalid --prompt value" >&2
    exit 2
    ;;
esac

IFS= read -r answer
printf '\033[2J\033[H'
printf 'mock_prompt_provider: selected=%s\n' "$answer"
printf 'mock_prompt_provider: done\n'
if [[ "$prompt" == "codex_update_ready" ]]; then
  printf '\033[60;1H  › '
  stty raw -echo 2>/dev/null || true
    while true; do
      probe=""
      while IFS= read -rsn1 probe; do
        [[ "$probe" != $'\r' && "$probe" != $'\n' ]] && break
      done
      [[ -z "$probe" ]] && break
      [[ "$probe" == $'\177' || "$probe" == $'\b' ]] && continue
      printf '%s' "$probe"
      sleep 0.05
      printf '\033[D \033[D'
    done
  stty sane 2>/dev/null || true
else
  printf '\033[60;1H$ '
fi
sleep 30
