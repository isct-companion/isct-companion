#!/bin/sh
# Cargo target runner for `yarn desktop:dev` on macOS (scripts/desktop/tauri.mjs).
# Signs a copy of the development app with the developer's Apple Development
# identity and its fixed identifier (ISCT_MACOS_DEV_IDENTIFIER: the dev or the
# demo app's), so its designated requirement, and with it access to the
# keychain items it created, survives rebuilds; then runs it in this process.
# The copy keeps one name: WebKit keeps the bare executable's data under its
# process name, so the dev and demo apps each keep their own
# (`isct-companion-dev`, `isct-companion-demo`). It is signed under a temporary
# name and renamed into place, so a signature still running from a run the
# watcher cancelled cannot rewrite the executable a later run started.
set -eu
binary=$1
shift
: "${ISCT_MACOS_DEV_SIGNING_IDENTITY:?set by scripts/desktop/tauri.mjs}"
: "${ISCT_MACOS_DEV_IDENTIFIER:?set by scripts/desktop/tauri.mjs}"
directory=$(dirname "$binary")
signed="$directory/$(basename "$binary")-${ISCT_MACOS_DEV_IDENTIFIER##*.}"
staging="$signed.$$"
rm -f "$signed".[0-9]*
trap 'rm -f "$staging"' EXIT
cp -c "$binary" "$staging"
/usr/bin/codesign --force --sign "$ISCT_MACOS_DEV_SIGNING_IDENTITY" \
  --identifier "$ISCT_MACOS_DEV_IDENTIFIER" --timestamp=none "$staging"
mv -f "$staging" "$signed"
trap - EXIT
exec "$signed" "$@"
