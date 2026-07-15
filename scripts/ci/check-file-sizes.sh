#!/usr/bin/env bash
# God-file size guard (refactor backlog T10).
#
# Fails if any committed Rust source file exceeds MAX_LINES. Keeping modules
# focused around one concern is an architecture rule (see CLAUDE.md "Architecture
# Hygiene"); this guard prevents silent regrowth of the god-files the refactor
# eliminated. Run locally with: scripts/ci/check-file-sizes.sh
set -euo pipefail

MAX_LINES="${MAX_LINES:-2000}"

# First-party committed .rs files only — `git ls-files` skips every (nested)
# target/ build dir and the generated WIT bindings under OUT_DIR (never
# committed). Vendored security/compatibility backports under patches/ retain
# their upstream layout and are not subject to ThinClaw's module-size policy.
violations=0
while IFS= read -r f; do
  [ -f "$f" ] || continue
  [[ "$f" == patches/* ]] && continue
  n=$(wc -l < "$f")
  if [ "$n" -gt "$MAX_LINES" ]; then
    printf '  %6d  %s\n' "$n" "$f"
    violations=$((violations + 1))
  fi
done < <(git ls-files '*.rs')

if [ "$violations" -gt 0 ]; then
  echo
  echo "✗ ${violations} Rust file(s) exceed ${MAX_LINES} lines (god-file threshold)."
  echo "  Decompose by concern into a directory module with a façade mod.rs."
  echo "  See CLAUDE.md → Architecture Hygiene and docs/refactor/."
  exit 1
fi

echo "✓ No Rust file exceeds ${MAX_LINES} lines."
