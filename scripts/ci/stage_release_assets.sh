#!/usr/bin/env bash
# Create/reuse a draft release and upload only missing or byte-identical assets.
set -euo pipefail

TAG="${RELEASE_TAG:?RELEASE_TAG is required}"
MODE="${RELEASE_MODE:?RELEASE_MODE is required}"
TARGET="${RELEASE_COMMIT:?RELEASE_COMMIT is required}"
ASSET_DIR="${RELEASE_ASSET_DIR:?RELEASE_ASSET_DIR is required}"
TITLE="${RELEASE_TITLE:-ThinClaw ${TAG}}"
NOTES_FILE="${RELEASE_NOTES_FILE:?RELEASE_NOTES_FILE is required}"
DRY_RUN="${RELEASE_DRY_RUN:-0}"
CONTROL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONTRACT="${RELEASE_CONTRACT:-$CONTROL_ROOT/release/non-apple-assets.json}"

python3 "$CONTROL_ROOT/scripts/ci/release_control.py" validate \
  --directory "$ASSET_DIR" --contract "$CONTRACT" >/dev/null

if [[ "$DRY_RUN" == "1" ]]; then
  echo "dry-run: would stage $(find "$ASSET_DIR" -maxdepth 1 -type f | wc -l | tr -d ' ') validated assets for $TAG at $TARGET"
  exit 0
fi

actual_commit="$(git rev-parse "${TAG}^{commit}")"
if [[ "$actual_commit" != "$TARGET" ]]; then
  echo "release tag $TAG resolves to $actual_commit, expected $TARGET" >&2
  exit 1
fi

if release_json="$(gh release view "$TAG" --repo "$GITHUB_REPOSITORY" --json isDraft,isPrerelease,tagName 2>/dev/null)"; then
  is_draft="$(jq -r '.isDraft' <<<"$release_json")"
  if [[ "$is_draft" != "true" ]]; then
    if [[ "$MODE" != "backfill" ]]; then
      echo "release $TAG is already public; only explicit backfill mode may return it to draft" >&2
      exit 1
    fi
    gh release edit "$TAG" --repo "$GITHUB_REPOSITORY" --draft --latest=false
  fi
else
  gh release create "$TAG" --repo "$GITHUB_REPOSITORY" --verify-tag --target "$TARGET" \
    --draft --title "$TITLE" --notes-file "$NOTES_FILE"
fi

release_api="$(gh api "repos/${GITHUB_REPOSITORY}/releases/tags/${TAG}")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

while IFS= read -r -d '' asset; do
  name="$(basename "$asset")"
  asset_id="$(jq -r --arg name "$name" '.assets[] | select(.name == $name) | .id' <<<"$release_api" | head -1)"
  if [[ -n "$asset_id" ]]; then
    gh api -H 'Accept: application/octet-stream' "repos/${GITHUB_REPOSITORY}/releases/assets/${asset_id}" > "$tmp/$name"
    local_sha="$(shasum -a 256 "$asset" | awk '{print $1}')"
    remote_sha="$(shasum -a 256 "$tmp/$name" | awk '{print $1}')"
    if [[ "$local_sha" != "$remote_sha" ]]; then
      echo "release asset $name already exists with different bytes; refusing to clobber" >&2
      exit 1
    fi
    echo "asset $name already exists with matching sha256; keeping immutable copy"
  else
    gh release upload "$TAG" "$asset" --repo "$GITHUB_REPOSITORY"
  fi
done < <(find "$ASSET_DIR" -maxdepth 1 -type f -print0 | sort -z)
