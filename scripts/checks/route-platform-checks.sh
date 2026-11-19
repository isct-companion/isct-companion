#!/usr/bin/env bash
set -euo pipefail

# Read NUL-separated git paths and print one `<job>=true|false` line for every
# Platform checks job, so the workflow and its result gate read one decision.
# Shared and unclassified inputs conservatively select every area, so newly
# introduced build inputs cannot bypass CI.
#   --all   select every area without reading paths
#   --full  the release suite: native SDKs and campus map replace the quick
#           Rust, Swift and Gradle checks
all=false full=false
for arg; do
  case "$arg" in
    --all) all=true ;;
    --full) full=true ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

selected=' '
areas=(frontend apple windows rust ios android campus-map app-store-tools)
select_jobs() {
  selected+="$* "
}

if [[ $all == true ]]; then
  select_jobs "${areas[@]}"
else
  while IFS= read -r -d '' path; do
    case "$path" in
      docs/verification/*) select_jobs frontend ;;
      docs/*.md|README.md|README.ja.md|CONTRIBUTING.md|AGENTS.md|CLAUDE.md|STATUS.md|.mcp.json|.project/events.jsonl|.project/compiled.json|.project/layout.json) ;;
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
        select_jobs rust ios android campus-map ;;
      *)
        select_jobs "${areas[@]}" ;;
    esac
  done
fi

has() {
  [[ "$selected" == *" $1 "* ]] && echo true || echo false
}
quick() {
  [[ $full == false ]] && has "$1" || echo false
}
release() {
  [[ $full == true ]] && has "$1" || echo false
}
native_sdk=false
if [[ $(release ios) == true || $(release android) == true ]]; then native_sdk=true; fi

printf '%s=%s\n' \
  frontend "$(has frontend)" \
  apple "$(has apple)" \
  windows "$(has windows)" \
  app-store-tools "$(has app-store-tools)" \
  rust "$(quick rust)" \
  ios-syntax "$(quick ios)" \
  android-config "$(quick android)" \
  native-sdk "$native_sdk" \
  native-ios "$(release ios)" \
  native-android "$(release android)" \
  campus-map "$(release campus-map)"
