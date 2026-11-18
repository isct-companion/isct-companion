#!/bin/sh
# Cargo target runner for `yarn desktop:dev` on macOS (scripts/desktop/tauri.mjs).
# Signs a copy of the development app with the developer's Apple Development
# identity and a fixed identifier, so its designated requirement, and with it
# access to the keychain items it created, survives rebuilds; then runs it in
# this process. The copy keeps one name: WebKit keeps the bare executable's
# data under its process name. It is signed under a temporary name and renamed
# into place, so a signature still running from a run the watcher cancelled
# cannot rewrite the executable a later run started.
set -eu
binary=$1
shift
: "${ISCT_MACOS_DEV_SIGNING_IDENTITY:?set by scripts/desktop/tauri.mjs}"
directory=$(dirname "$binary")
signed="$directory/$(basename "$binary")-dev"
staging="$signed.$$"
rm -f "$signed".[0-9]*
trap 'rm -f "$staging"' EXIT
cp -c "$binary" "$staging"
/usr/bin/codesign --force --sign "$ISCT_MACOS_DEV_SIGNING_IDENTITY" \
  --identifier app.isctcompanion.dev --timestamp=none "$staging"
mv -f "$staging" "$signed"
trap - EXIT
exec "$signed" "$@"
