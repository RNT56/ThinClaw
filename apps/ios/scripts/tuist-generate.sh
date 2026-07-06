#!/usr/bin/env bash
#
# tuist-generate.sh — resolve dependencies and generate the ThinClaw Xcode
# workspace from the Tuist manifests, robustly enough to run on a fresh CI
# checkout.
#
# Why this exists (the CI failure it fixes):
#   Project.swift declares `resources: ["App/Resources/**"]` for the app target.
#   The repo carries no committed files under App/Resources (git does not track
#   empty directories), so on a fresh checkout that directory is ABSENT. Tuist
#   treats a resource glob whose parent directory does not exist as a hard error
#   during "Loading and constructing the graph": `tuist generate` prints the
#   section header, then a bare log reference, and exits 1 with nothing on
#   stderr. Locally the directory happens to exist (empty), so the same command
#   only warns and succeeds — which is why the failure only ever showed up on
#   the GitHub runner. Reproduced by deleting App/Resources locally: exit 1.
#
#   The fix is to guarantee every declared-but-empty resource directory exists
#   before generation. We simply create App/Resources here (an EMPTY directory
#   is enough — Tuist only warns for an empty glob, it errors for a missing
#   one). We intentionally write no placeholder file: an empty directory is
#   invisible to git, so this leaves no untracked noise for local developers,
#   touches no app source, and is a no-op once the directory holds real
#   resources.
#
# It also runs `tuist install` (plugins) and lets `tuist generate` perform the
# xcodebuild-backed SPM resolution for the external packages (swift-openapi-*,
# GRDB, etc.) declared transitively by the local Packages/*.
#
# Usage:
#   apps/ios/scripts/tuist-generate.sh
#
# `tuist` must be on PATH. In CI and locally it is provided by mise; call this
# script under `mise x --` (e.g. `mise x -- apps/ios/scripts/tuist-generate.sh`)
# or with the mise shims on PATH.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IOS_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${IOS_DIR}"

# Guarantee declared-but-empty resource directories exist so the resource globs
# in Project.swift resolve during graph construction. Extend this list if the
# manifest gains more `resources:` paths that are not yet populated.
DECLARED_RESOURCE_DIRS=(
  "App/Resources"
)
for dir in "${DECLARED_RESOURCE_DIRS[@]}"; do
  if [ ! -d "${IOS_DIR}/${dir}" ]; then
    echo "==> creating missing declared resource dir: ${dir}"
    mkdir -p "${IOS_DIR}/${dir}"
  fi
done

if ! command -v tuist >/dev/null 2>&1; then
  echo "error: 'tuist' not found on PATH. Run under mise, e.g.:" >&2
  echo "       mise x -- apps/ios/scripts/tuist-generate.sh" >&2
  exit 127
fi

echo "==> tuist install (plugins)"
tuist install

echo "==> tuist generate (constructs graph, resolves SPM via xcodebuild)"
tuist generate --no-open

echo "==> workspace generated: ${IOS_DIR}/ThinClaw.xcworkspace"
