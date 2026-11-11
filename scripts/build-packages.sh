#!/usr/bin/env bash
# Build deb, rpm and tar.gz from one staged, verified release tree.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
[ "$ARCH" = x86_64 ] || { echo "packaging script expects x86_64, got $ARCH" >&2; exit 1; }
CARGO=${CARGO:-cargo}
PACKAGE_BINARY=${PRETTY_PACKAGE_BINARY:-}
BUILD="$ROOT/build"
DIST="$ROOT/dist"
STAGE="$BUILD/stage"
# If the caller supplied a binary inside build/, preserve it before cleaning.
if [ -n "${PACKAGE_BINARY:-}" ] && [ -x "$PACKAGE_BINARY" ]; then
  case "$(readlink -f "$PACKAGE_BINARY")" in
    "$ROOT"/build/*) cp "$PACKAGE_BINARY" "$ROOT/.pretty-package-binary"; PACKAGE_BINARY="$ROOT/.pretty-package-binary" ;;
  esac
fi
rm -rf "$BUILD" "$DIST"
mkdir -p "$BUILD" "$DIST" "$STAGE"

echo "==> building release binary with $($CARGO --version)"
if [ -n "$PACKAGE_BINARY" ]; then
  echo "==> using portable package binary: $PACKAGE_BINARY"
  BIN_SRC="$PACKAGE_BINARY"
  [ -x "$BIN_SRC" ] || { echo "missing package binary $BIN_SRC" >&2; exit 1; }
else
  $CARGO build --release -p pretty-app
  BIN_SRC="$ROOT/target/release/pretty"
fi

echo "==> staging package tree"
install -d "$STAGE/usr/bin" "$STAGE/usr/share/applications" \
  "$STAGE/usr/share/man/man1" "$STAGE/usr/share/doc/pretty/licenses" \
  "$STAGE/usr/share/icons/hicolor/scalable/apps" "$STAGE/usr/share/terminfo"
install -m 0755 "$BIN_SRC" "$STAGE/usr/bin/pretty"
strip --strip-unneeded "$STAGE/usr/bin/pretty" 2>/dev/null || true
install -m 0644 packaging/pretty.desktop "$STAGE/usr/share/applications/pretty.desktop"
install -m 0644 packaging/pretty.1.gz "$STAGE/usr/share/man/man1/pretty.1.gz"
tic -x -o "$STAGE/usr/share/terminfo" packaging/pretty.terminfo
install -m 0644 icons/pretty.svg "$STAGE/usr/share/icons/hicolor/scalable/apps/pretty.svg"
for size in 16 24 32 48 64 128 256 512; do
  install -d "$STAGE/usr/share/icons/hicolor/${size}x${size}/apps"
  install -m 0644 "icons/pretty-${size}.png" "$STAGE/usr/share/icons/hicolor/${size}x${size}/apps/pretty.png"
done
install -m 0644 README.md "$STAGE/usr/share/doc/pretty/README.md"
for doc in KITTY-COMPAT.md TEST-REPORT.md RENDERER.md ARCHITECTURE.md KNOWN-ISSUES.md PACKAGING.md CONFIG.md CROSS-PLATFORM.md CROSS-COMPILE.md TEST-REPORT-V0.2.0.md AUDIT-020.md; do
  install -m 0644 "docs/$doc" "$STAGE/usr/share/doc/pretty/$doc"
done
cp -a assets/licenses/. "$STAGE/usr/share/doc/pretty/licenses/"
cp LICENSE "$STAGE/usr/share/doc/pretty/"
install -m 0644 packaging/deb/copyright "$STAGE/usr/share/doc/pretty/copyright"
find "$STAGE" -type f -exec sha256sum {} + | sort -k2 > "$BUILD/package-sha256.txt"

echo "==> building deb"
DEBROOT="$BUILD/deb-root"
mkdir -p "$DEBROOT/DEBIAN"
cp -a "$STAGE/." "$DEBROOT/"
sed -e "s/^Version: .*/Version: $VERSION/" \
    -e "s/^Installed-Size: .*/Installed-Size: $(du -sk "$STAGE" | cut -f1)/" \
    packaging/deb/control > "$DEBROOT/DEBIAN/control"
dpkg-deb --root-owner-group --build "$DEBROOT" "$DIST/pretty-terminal_${VERSION}_amd64.deb" >/dev/null
echo "    $DIST/pretty-terminal_${VERSION}_amd64.deb"

echo "==> building rpm"
RPMTOP="$BUILD/rpmbuild"
mkdir -p "$RPMTOP"/{SOURCES,SPECS,BUILD,BUILDROOT,RPMS,SRPMS}
tar -czf "$RPMTOP/SOURCES/pretty-stage.tar.gz" -C "$BUILD" stage
cp packaging/rpm/pretty-terminal.spec "$RPMTOP/SPECS/"
rpmbuild -bb --define "_topdir $RPMTOP" --define "dist .fc44" \
  "$RPMTOP/SPECS/pretty-terminal.spec" > "$BUILD/rpmbuild.log"
find "$RPMTOP/RPMS" -name '*.rpm' -exec cp {} "$DIST/" \;
echo "    $(find "$DIST" -name '*.rpm' -printf '%f\n')"

echo "==> building tar.gz"
TARROOT="$BUILD/pretty-$VERSION-linux-x86_64"
mkdir -p "$TARROOT"
cp -a "$STAGE/usr" "$TARROOT/"
mkdir -p "$TARROOT/docs/licenses"
for doc in KITTY-COMPAT.md TEST-REPORT.md RENDERER.md ARCHITECTURE.md KNOWN-ISSUES.md PACKAGING.md CONFIG.md CROSS-PLATFORM.md CROSS-COMPILE.md TEST-REPORT-V0.2.0.md AUDIT-020.md; do cp "docs/$doc" "$TARROOT/docs/$doc"; done
cp README.md "$TARROOT/docs/README.md"
cp -a assets/licenses/. "$TARROOT/docs/licenses/"
cp LICENSE "$TARROOT/"
cp packaging/install.sh "$TARROOT/install.sh"
chmod +x "$TARROOT/install.sh"
# Relocatable tar layout: bin/ and share/ at top level.
mv "$TARROOT/usr/bin" "$TARROOT/bin"
mv "$TARROOT/usr/share" "$TARROOT/share"
rmdir "$TARROOT/usr"
cat > "$TARROOT/README.txt" <<'TXT'
Pretty terminal emulator (relocatable tarball)
==============================================
Run directly:      ./bin/pretty
Install locally:   ./install.sh
Install system:    sudo ./install.sh
Custom prefix:     PREFIX=$HOME/.local ./install.sh
DESTDIR staging:   DESTDIR=/tmp/root PREFIX=/usr ./install.sh

The tarball includes a compiled terminfo entry in share/terminfo. If you run
./bin/pretty directly, Pretty detects ../share/terminfo and sets TERMINFO for
its child shell. Otherwise installing the tarball installs the terminfo entry.

Docs:     docs/README.md, docs/CONFIG.md, docs/KITTY-COMPAT.md, docs/CROSS-PLATFORM.md, docs/TEST-REPORT.md
Licenses: docs/licenses/
TXT
tar -czf "$DIST/pretty-${VERSION}-linux-x86_64.tar.gz" -C "$BUILD" "pretty-$VERSION-linux-x86_64"
echo "    $DIST/pretty-${VERSION}-linux-x86_64.tar.gz"

echo "==> package artifacts"
(cd "$DIST" && sha256sum ./* > SHA256SUMS && ls -lh)
