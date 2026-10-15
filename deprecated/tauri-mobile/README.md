# Retired Tauri mobile adapters

This directory contains the old Tauri iOS/Android platform adapters, plugins,
icons, project patcher and release workflows. The generated projects in `gen/`
are local ignored build output, not native application source.

- `rust/`: mobile school WebView adapters. Target-gated module paths in the
  desktop crate retain the old source linkage during the native-shell transition.
- `plugins/`: Tauri Google sign-in and Android school-WebView plugins. Their
  workspace membership keeps path dependencies resolvable; desktop does not
  enable these target-specific dependencies.
- `tauri.mobile.conf.json`: retired iOS/Android bundle settings.
- `icons/`, `scripts/`, `workflows/`, `docs/`: archived mobile assets and build notes.
  Workflows outside `.github/workflows/` do not run in GitHub Actions.

New iOS and Android apps will live in `apps/ios/` and `apps/android/`, calling
`isct-core` through the planned `isct-ffi` Swift/Kotlin bindings. Add those
projects when they contain working source, rather than empty scaffolding.
Shared school login scripts, state machines and ports remain in `isct-core`.

The responsive compact React layout remains part of desktop because it is
also used by narrow desktop windows. It is not an archived native mobile shell.
