#!/bin/sh
# Captures the demo app's window as it is on screen: native 2x pixels, no
# shadow, rounded corners transparent (2560x1640 for the 1280x820 window).
#   scripts/screenshots/capture.sh <out-dir> <theme> <lang> <id>
# writes <out-dir>/<theme>/<lang>/<id>.png. The window-id helper is compiled
# once into the out dir. Move the pointer off the window first: hover states
# are captured too. See docs/screenshots/README.md.
set -eu
[ $# -eq 4 ] || { echo "usage: $0 <out-dir> <theme> <lang> <id>" >&2; exit 2; }
out=$1
here=$(cd "$(dirname "$0")" && pwd)
helper="$out/.window-id"
mkdir -p "$out/$2/$3"
[ -x "$helper" ] && [ "$helper" -nt "$here/window-id.swift" ] ||
  swiftc -O "$here/window-id.swift" -o "$helper"
id=$("$helper" "${ISCT_CAPTURE_APP:-IsctCompanion Demo}")
file="$out/$2/$3/$4.png"
screencapture -x -o -l"$id" "$file"
sips -g pixelWidth -g pixelHeight "$file" | tail -2 | tr -s ' ' | tr '\n' ' '
echo "$file"
