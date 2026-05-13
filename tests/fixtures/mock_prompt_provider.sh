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
      echo "usage: $0 --prompt codex_update|trust_path|unknown_eula"
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
  *)
    echo "missing or invalid --prompt value" >&2
    exit 2
    ;;
esac

IFS= read -r answer
printf '\033[2J\033[H'
printf 'mock_prompt_provider: selected=%s\n' "$answer"
printf 'mock_prompt_provider: done\n'
printf '\033[60;1H$ '
sleep 30
