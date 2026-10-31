#!/bin/sh
# Installer for the Pretty relocatable tarball.
set -eu
PREFIX="${PREFIX:-/usr/local}"
DESTDIR="${DESTDIR:-}"
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

install -d "$DESTDIR$PREFIX/bin" \
  "$DESTDIR$PREFIX/share/applications" \
  "$DESTDIR$PREFIX/share/man/man1" \
  "$DESTDIR$PREFIX/share/terminfo/p" \
  "$DESTDIR$PREFIX/share/doc/pretty/licenses" \
  "$DESTDIR$PREFIX/share/icons/hicolor/scalable/apps"
install -m 0755 "$ROOT/bin/pretty" "$DESTDIR$PREFIX/bin/pretty"
install -m 0644 "$ROOT/share/applications/pretty.desktop" "$DESTDIR$PREFIX/share/applications/pretty.desktop"
install -m 0644 "$ROOT/share/man/man1/pretty.1.gz" "$DESTDIR$PREFIX/share/man/man1/pretty.1.gz"
install -m 0644 "$ROOT/share/terminfo/p/pretty" "$DESTDIR$PREFIX/share/terminfo/p/pretty"
install -m 0644 "$ROOT/share/icons/hicolor/scalable/apps/pretty.svg" "$DESTDIR$PREFIX/share/icons/hicolor/scalable/apps/pretty.svg"
for size in 16 24 32 48 64 128 256 512; do
  install -d "$DESTDIR$PREFIX/share/icons/hicolor/${size}x${size}/apps"
  install -m 0644 "$ROOT/share/icons/hicolor/${size}x${size}/apps/pretty.png" \
    "$DESTDIR$PREFIX/share/icons/hicolor/${size}x${size}/apps/pretty.png"
done
for doc in README.md KITTY-COMPAT.md TEST-REPORT.md RENDERER.md ARCHITECTURE.md KNOWN-ISSUES.md PACKAGING.md; do
  [ -f "$ROOT/docs/$doc" ] && install -m 0644 "$ROOT/docs/$doc" "$DESTDIR$PREFIX/share/doc/pretty/$doc"
done
cp -a "$ROOT/docs/licenses/." "$DESTDIR$PREFIX/share/doc/pretty/licenses/"
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$DESTDIR$PREFIX/share/applications" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q "$DESTDIR$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi
printf 'Pretty installed to %s%s\n' "$DESTDIR" "$PREFIX"
printf 'Run: %s%s/bin/pretty\n' "$DESTDIR" "$PREFIX"
