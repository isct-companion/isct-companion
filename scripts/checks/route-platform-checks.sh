#!/usr/bin/env bash
set -euo pipefail

# Read NUL-separated git paths and print one `<job>=true|false` line for every
# Platform checks job, so the workflow and its result gate read one decision.
# Unclassified inputs run the baseline checks. The more expensive checks
# follow their source and build inputs.
#   --all   select every area without reading paths
#   --full  use the release suite, including native SDKs and simulator tests
#   --warm  select the caches affected by build input changes
all=false full=false warm=false
for arg; do
  case "$arg" in
    --all) all=true ;;
    --full) full=true ;;
    --warm) warm=true ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

selected=' '
areas=(frontend apple windows rust core-contracts ios android-config android campus-map app-store-tools)
quick_areas=(frontend apple windows rust ios android-config app-store-tools)
select_jobs() {
  selected+="$* "
}

if [[ $all == true ]]; then
  if [[ $warm == true ]]; then
    # The scheduled release suite warms the Android SDK and app compile.
    select_jobs "${quick_areas[@]}" core-contracts
  else
    select_jobs "${areas[@]}"
  fi
else
  while IFS= read -r -d '' path; do
    if [[ $warm == true ]]; then
      case "$path" in
        yarn.lock|.yarnrc.yml|package.json|apps/desktop/package.json)
          select_jobs frontend ;;
        apps/android/*.gradle*|apps/android/gradle.properties|apps/android/gradle/*|apps/android/gradlew*|.github/actions/android-build-tools/*)
          select_jobs android android-config ;;
        Cargo.lock|Cargo.toml|*/Cargo.toml|rust-toolchain.toml|.cargo/*)
          select_jobs apple windows rust core-contracts android ;;
        .github/workflows/platform-checks.yml|.github/workflows/native-sdk.yml|.github/workflows/warm-caches.yml|.github/actions/rust-caches/*|scripts/checks/route-platform-checks.sh)
          select_jobs "${areas[@]}" ;;
      esac
      continue
    fi
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
      apps/android/*|bindings/kotlin/*|scripts/bindings/build-android.mjs|.github/actions/android-build-tools/*|.github/workflows/android-release.yml)
        select_jobs android ;;
      scripts/release/attach-mobile-assets.sh)
        select_jobs ios android-config ;;
      apps/apple-widgets/*)
        select_jobs apple ios ;;
      crates/app-core/src/campus_map/*|crates/app-core/src/tests/campus_map__*.rs|.github/workflows/campus-map.yml)
        select_jobs rust core-contracts ios android-config campus-map ;;
      crates/app-ffi/*|scripts/bindings/*|.github/workflows/native-sdk.yml|.github/workflows/platform-checks.yml|scripts/checks/route-platform-checks.sh)
        select_jobs "${areas[@]}" ;;
      crates/*|Cargo.lock|Cargo.toml|rust-toolchain.toml|.cargo/*)
        select_jobs "${quick_areas[@]}" core-contracts
        if [[ $path == Cargo.lock || $path == Cargo.toml || $path == */Cargo.toml ]]; then
          select_jobs android
        fi ;;
      *)
        select_jobs "${quick_areas[@]}" ;;
    esac
  done
fi
if [[ $full == true ]] && [[ $selected != ' ' ]]; then
  select_jobs "${areas[@]}"
fi

has() {
  [[ "$selected" == *" $1 "* ]] && echo true || echo false
}
quick() {
  [[ $full == false ]] && has "$1" || echo false
}
native_ios=false
native_android=$(has android)
android_config=false
if [[ $full == true ]]; then native_ios=$(has ios); fi
# Kotlin compilation evaluates Gradle itself. During prewarming, populate the
# separate lightweight configuration cache when its inputs have changed too.
if [[ $full == false && ( $native_android == false || $warm == true ) ]]; then
  android_config=$(has android-config)
fi
native_sdk=false
if [[ $native_ios == true || $native_android == true ]]; then native_sdk=true; fi

printf '%s=%s\n' \
  frontend "$(has frontend)" \
  apple "$(has apple)" \
  windows "$(has windows)" \
  app-store-tools "$(has app-store-tools)" \
  rust "$(quick rust)" \
  core-contracts "$(quick core-contracts)" \
  ios-syntax "$(quick ios)" \
  android-config "$android_config" \
  native-sdk "$native_sdk" \
  native-ios "$native_ios" \
  native-android "$native_android" \
  campus-map "$([[ $full == true ]] && has campus-map || echo false)"
