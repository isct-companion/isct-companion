#!/usr/bin/env bash
set -euo pipefail

tag=$1
shift
for asset in "$@"; do
  test -f "$asset" || { echo "Missing release asset: $asset" >&2; exit 1; }
done

# Desktop release creates the draft as soon as the tag is pushed.
for _ in $(seq 1 60); do
  if gh release view "$tag" > /dev/null 2>&1; then
    gh release upload "$tag" "$@" --clobber
    exit 0
  fi
  sleep 20
done
echo "Desktop release opened no release for $tag within 20 minutes. Rerun its draft job, then rerun this upload job." >&2
exit 1
