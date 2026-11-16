#!/usr/bin/env bash
set -euo pipefail

# Read NUL-separated git paths. Shared and unclassified inputs conservatively
# select the full suite, so newly introduced build inputs cannot bypass CI.
selected=' '
jobs=(frontend apple windows ios android campus-map app-store-tools)
select_jobs() {
  selected+="$* "
}

if [[ ${1:-} == --all ]]; then
  select_jobs "${jobs[@]}"
else
  while IFS= read -r -d '' path; do
    case "$path" in
      docs/verification/*) select_jobs frontend ;;
      docs/*.md|README.md|README.ja.md|CONTRIBUTING.md) ;;
      apps/desktop/src/*|apps/desktop/tests/*|apps/desktop/public/*|apps/desktop/index.html)
        select_jobs frontend ;;
      apps/desktop/*|scripts/desktop/*|.github/workflows/desktop-release.yml)
        select_jobs frontend apple windows ;;
      apps/ios/appstore/*|scripts/app-store/*|resources/app-store/*|.github/workflows/app-store.yml)
        select_jobs app-store-tools ;;
      apps/ios/*|.github/workflows/ios-release.yml|.github/workflows/apple-signing-check.yml)
        select_jobs ios ;;
      apps/android/*|.github/actions/android-build-tools/*|.github/workflows/android-release.yml)
        select_jobs android ;;
      scripts/release/attach-mobile-assets.sh)
        select_jobs ios android ;;
      apps/apple-widgets/*)
        select_jobs apple ios ;;
      crates/app-core/src/campus_map/*|crates/app-core/src/tests/campus_map__*.rs|.github/workflows/campus-map.yml)
        select_jobs ios android campus-map ;;
      *)
        select_jobs "${jobs[@]}" ;;
    esac
  done
fi

for job in "${jobs[@]}"; do
  if [[ "$selected" == *" $job "* ]]; then
    printf '%s=true\n' "$job"
  else
    printf '%s=false\n' "$job"
  fi
done
