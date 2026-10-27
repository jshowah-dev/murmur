#!/usr/bin/env bash
# Scans what a push adds for secrets and work terms (CLAUDE.md, "Hard line").
# Prints the paths of flagged files on stdout, never the matched text, so a hit
# isn't copied into a run log or an issue. Exits 1 when anything is flagged.
#
# Usage: land-check.sh <base> <head>
# Work terms come from $DENYLIST (the VAULT_DENYLIST Actions secret): one
# extended regex per line, matched case-insensitively against added lines and paths.

set -uo pipefail

base=$1
head=$2

# Known key formats only. Each needs its vendor prefix, so notes about keys don't trip it.
secrets='AKIA[0-9A-Z]{16}|-----BEGIN [A-Z ]*PRIVATE KEY-----|sk-ant-[A-Za-z0-9_-]{20,}|sk-(proj|svcacct|admin)-[A-Za-z0-9_-]{20,}|(^|[^A-Za-z0-9_-])sk-[A-Za-z0-9]{40,}|gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,}|xox[baprs]-[A-Za-z0-9-]{10,}|[sr]k_live_[0-9A-Za-z]{24,}|AIza[0-9A-Za-z_-]{35}|(^|[^A-Za-z0-9])SK[0-9a-f]{32}([^A-Za-z0-9]|$)'

terms=$(printf '%s\n' "${DENYLIST:-}" | tr -d '\r' | sed '/^[[:space:]]*$/d')
if [ -z "$terms" ]; then
  echo "VAULT_DENYLIST is not set; checking for secrets only." >&2
else
  grep -Eiq -f <(printf '%s\n' "$terms") </dev/null
  if [ $? -eq 2 ]; then
    echo "VAULT_DENYLIST has a pattern grep can't read." >&2
    exit 1
  fi
fi

hits=()
while IFS= read -r -d '' path; do
  added=$(git diff --unified=0 --no-color --no-renames "$base" "$head" -- "$path" |
    tr -d '\000' | awk '/^@@/ { body = 1; next } body && /^\+/ { print substr($0, 2) }')
  if printf '%s\n' "$added" | grep -Eq -- "$secrets"; then
    hits+=("secret: $path")
  fi
  if [ -n "$terms" ] && printf '%s\n%s\n' "$path" "$added" | grep -Eiq -f <(printf '%s\n' "$terms"); then
    hits+=("work term: $path")
  fi
done < <(git diff --name-only -z --no-renames --diff-filter=AM "$base" "$head")

if [ ${#hits[@]} -gt 0 ]; then
  printf '%s\n' "${hits[@]}"
  exit 1
fi
echo "Clean." >&2
