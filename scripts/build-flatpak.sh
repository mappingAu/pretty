#!/usr/bin/env bash
# Build a single-file Flatpak bundle from the already-built portable binary.
# This intentionally does not install or launch the bundle.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
VERSION=0.1.0
APP_ID=io.github.mappingAu.Pretty
BUILD="$ROOT/build/flatpak/$APP_ID"
REPO="$ROOT/build/flatpak-repo"
OUT="$ROOT/dist/pretty-${VERSION}-x86_64.flatpak"
BIN=${PRETTY_FLATPAK_BIN:-$ROOT/.portable-pretty}
if [ ! -x "$BIN" ]; then
  BIN="$ROOT/target/release/pretty"
fi
if [ ! -x "$BIN" ]; then
  echo "error: no portable Pretty binary; set PRETTY_FLATPAK_BIN" >&2
  exit 1
fi
command -v flatpak >/dev/null || { echo "error: flatpak is required" >&2; exit 1; }

rm -rf "$BUILD" "$REPO"
mkdir -p "$BUILD/files"/{bin,share/applications,share/icons/hicolor/scalable/apps,share/man/man1,share/terminfo,share/metainfo,share/doc/pretty/licenses,etc/fonts,share/fonts}
install -m 0755 "$BIN" "$BUILD/files/bin/pretty"
strip --strip-unneeded "$BUILD/files/bin/pretty" 2>/dev/null || true
install -m 0644 packaging/flatpak/io.github.mappingAu.Pretty.desktop "$BUILD/files/share/applications/"
install -m 0644 packaging/flatpak/io.github.mappingAu.Pretty.metainfo.xml "$BUILD/files/share/metainfo/"
install -m 0644 icons/pretty.svg "$BUILD/files/share/icons/hicolor/scalable/apps/pretty.svg"
for size in 16 24 32 48 64 128 256 512; do
  install -d "$BUILD/files/share/icons/hicolor/${size}x${size}/apps"
  install -m 0644 "icons/pretty-${size}.png" "$BUILD/files/share/icons/hicolor/${size}x${size}/apps/pretty.png"
done
install -m 0644 packaging/pretty.1.gz "$BUILD/files/share/man/man1/pretty.1.gz"
tic -x -o "$BUILD/files/share/terminfo" packaging/pretty.terminfo
install -m 0644 packaging/flatpak/fonts.conf "$BUILD/files/etc/fonts/fonts.conf"
install -m 0644 README.md LICENSE "$BUILD/files/share/doc/pretty/"
for doc in KITTY-COMPAT.md TEST-REPORT.md RENDERER.md ARCHITECTURE.md KNOWN-ISSUES.md PACKAGING.md; do
  install -m 0644 "docs/$doc" "$BUILD/files/share/doc/pretty/$doc"
done
cp -a assets/licenses/. "$BUILD/files/share/doc/pretty/licenses/"

# A terminal without a CJK font shows boxes for Chinese UI/terminal text. Bundle
# the host's Noto CJK collection when available; allow an explicit override.
CJK_FONT=${PRETTY_CJK_FONT:-}
if [ -z "$CJK_FONT" ]; then
  for f in \
    /usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc \
    /usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc \
    /usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc; do
    if [ -f "$f" ]; then CJK_FONT=$f; break; fi
  done
fi
if [ -n "$CJK_FONT" ] && [ -f "$CJK_FONT" ]; then
  install -m 0644 "$CJK_FONT" "$BUILD/files/share/fonts/NotoSansCJK-VF.ttc"
  echo "bundled CJK font: $CJK_FONT"
else
  echo "warning: no CJK font found; Chinese UI will show fallback boxes" >&2
fi

cat > "$BUILD/metadata" <<EOF
[Application]
name=$APP_ID
runtime=org.freedesktop.Platform/x86_64/25.08
sdk=org.freedesktop.Sdk/x86_64/25.08
command=pretty
EOF
# Permissions are written into the exported metadata by build-finish.
flatpak build-finish "$BUILD" \
  --command=pretty \
  --share=ipc \
  --share=network \
  --socket=wayland \
  --socket=fallback-x11 \
  --device=dri \
  --filesystem=home \
  --talk-name=org.freedesktop.Notifications \
  --env=TERM=pretty \
  --env=SHELL=/bin/sh \
  --env=FONTCONFIG_FILE=/app/etc/fonts/fonts.conf >/dev/null

mkdir -p "$ROOT/dist"
flatpak build-export "$REPO" "$BUILD" >/dev/null
flatpak build-bundle "$REPO" "$OUT" "$APP_ID" >/dev/null
echo "built: $OUT"
sha256sum "$OUT"
