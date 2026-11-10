#!/usr/bin/env bash
# Generate a macOS .icns icon from icons/pretty-512.png using sips + iconutil.
#
# Usage: packaging/macos/make-icns.sh <output.icns>
#
# Only works on macOS with the Xcode command line tools (sips, iconutil).
# Exits non-zero when the tools are unavailable so callers can package without
# an icon instead of failing.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
[ $# -ge 1 ] || { echo "usage: $0 <output.icns>" >&2; exit 2; }
OUT=$1
SRC="$ROOT/icons/pretty-512.png"

if ! command -v sips >/dev/null 2>&1 || ! command -v iconutil >/dev/null 2>&1; then
  echo "sips/iconutil not available; skipping .icns generation" >&2
  exit 1
fi
[ -f "$SRC" ] || { echo "missing icon source: $SRC" >&2; exit 1; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
ICONSET="$TMP/Pretty.iconset"
mkdir -p "$ICONSET"

# size name (iconset naming: <size>x<size>[@2x])
while read -r size name; do
  sips -z "$size" "$size" "$SRC" --out "$ICONSET/icon_${name}.png" >/dev/null
done <<'SIZES'
16 16x16
32 16x16@2x
32 32x32
64 32x32@2x
128 128x128
256 128x128@2x
256 256x256
512 256x256@2x
512 512x512
1024 512x512@2x
SIZES

mkdir -p "$(dirname -- "$OUT")"
iconutil -c icns "$ICONSET" -o "$OUT"
echo "wrote $OUT"
