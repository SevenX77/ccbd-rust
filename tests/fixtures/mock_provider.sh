#!/usr/bin/env bash
set -euo pipefail

printf 'mock_provider: ready\n'
printf '$ '

while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  printf 'mock_provider: received=%s\n' "$line"
  printf 'mock_provider: echo=%s\n' "$line"
  printf '$ '
done

printf 'mock_provider: done\n'
printf '$ '
