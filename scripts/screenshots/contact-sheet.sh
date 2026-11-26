#!/bin/sh
# One review sheet per theme from capture.sh's output: rows ja / zh / en,
# columns the screens in the order given (ImageMagick `magick`).
#   scripts/screenshots/contact-sheet.sh <out-dir> desktop-today desktop-lecture ...
# writes <out-dir>/contact-<theme>.png for each theme folder present.
set -eu
[ $# -ge 2 ] || { echo "usage: $0 <out-dir> <id>..." >&2; exit 2; }
out=$(cd "$1" && pwd)
shift
ids="$*" # screen ids hold no spaces
rows=$(mktemp -d)
trap 'rm -rf "$rows"' EXIT
for theme in light dark; do
  [ -d "$out/$theme" ] || continue
  rank=0
  for lang in ja zh en; do
    rank=$((rank + 1))
    [ -d "$out/$theme/$lang" ] || continue
    # shellcheck disable=SC2046,SC2086 # one word per screen id
    (cd "$out/$theme/$lang" &&
      magick $(for id in $ids; do echo "$id.png"; done) -resize 640x \
        -bordercolor "#808080" -border 12 +append "$rows/$theme-$rank-$lang.png")
  done
  magick "$rows/$theme"-*.png -append "$out/contact-$theme.png"
  echo "$out/contact-$theme.png"
done
