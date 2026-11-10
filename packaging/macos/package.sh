#!/usr/bin/env bash
# Build the macOS artifacts for Pretty from an existing release binary:
#
#   dist/pretty-<version>-macos-<arch>.app.tar.gz   Pretty.app bundle
#   dist/pretty-<version>-macos-<arch>.tar.gz       standalone CLI tree
#   dist/SHA256SUMS-macos.txt
#
# Run on macOS (CI uses macos-latest / Apple Silicon):
#   cargo build --release -p pretty-app --features metal
#   packaging/macos/package.sh
#
# scripts/build-macos.sh wraps both steps. Environment overrides:
#   PRETTY_VERSION  version label (default: workspace version from Cargo.toml)
#   PRETTY_ARCH     arch label (default: uname -m)
#   PRETTY_BINARY   release binary (default: target/release/pretty)
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

VERSION=${PRETTY_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}
[ -n "$VERSION" ] || { echo "cannot read the workspace version from Cargo.toml" >&2; exit 1; }
ARCH=${PRETTY_ARCH:-$(uname -m)}
BINARY=${PRETTY_BINARY:-target/release/pretty}

[ -x "$BINARY" ] || {
  echo "missing release binary '$BINARY'; run: cargo build --release -p pretty-app --features metal" >&2
  exit 1
}

STAGE="$ROOT/build/macos"
APP="$STAGE/Pretty.app"
rm -rf "$STAGE"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$ROOT/dist"

# --- .app bundle -----------------------------------------------------------
install -m 0755 "$BINARY" "$APP/Contents/MacOS/pretty"
sed -e "s|@VERSION@|$VERSION|g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
install -m 0644 LICENSE "$APP/Contents/Resources/LICENSE"

if packaging/macos/make-icns.sh "$STAGE/pretty.icns"; then
  install -m 0644 "$STAGE/pretty.icns" "$APP/Contents/Resources/pretty.icns"
else
  echo "warning: packaging without a .icns icon (sips/iconutil unavailable)" >&2
fi

# Ad-hoc signing keeps the bundle launchable on Apple Silicon (the cargo-built
# binary is already ad-hoc signed by the linker; this signs the bundle). It is
# not Developer ID signing and not notarisation.
if command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$APP" >/dev/null 2>&1 \
    && echo "ad-hoc signed: $APP" \
    || echo "warning: ad-hoc codesign failed; the bundle is still usable locally" >&2
fi

# --- standalone CLI tree ---------------------------------------------------
CLI="$STAGE/pretty-$VERSION-macos-$ARCH"
mkdir -p "$CLI/docs"
install -m 0755 "$BINARY" "$CLI/pretty"
install -m 0644 README.md LICENSE "$CLI/"
for doc in "$ROOT"/docs/*.md; do
  [ -f "$doc" ] && install -m 0644 "$doc" "$CLI/docs/"
done
cat > "$CLI/README-MACOS.txt" <<'TXT'
Pretty for macOS
================
The .app bundle is a normal macOS application: double-click Pretty.app.

The standalone `pretty` binary includes the Metal backend and can be run from
a terminal:

    ./pretty --print-info
    ./pretty --renderer metal
    ./pretty --renderer auto

Settings: ~/Library/Application Support/org.pretty.pretty/config.toml
(override with --config or PRETTY_CONFIG).

The archive is ad-hoc signed, not Developer ID signed or notarised. On first
launch macOS Gatekeeper may require: right-click -> Open (or
`xattr -d com.apple.quarantine` for the downloaded file).

This build is produced by the native GitHub Actions macOS runner; the GUI was
not executed on physical Mac hardware as part of this release.
TXT

# --- archives --------------------------------------------------------------
tar -czf "dist/pretty-$VERSION-macos-$ARCH.app.tar.gz" -C "$STAGE" Pretty.app
tar -czf "dist/pretty-$VERSION-macos-$ARCH.tar.gz" -C "$STAGE" "pretty-$VERSION-macos-$ARCH"

if command -v shasum >/dev/null 2>&1; then
  (cd "$ROOT/dist" && shasum -a 256 ./*.tar.gz > SHA256SUMS-macos.txt)
else
  (cd "$ROOT/dist" && sha256sum ./*.tar.gz > SHA256SUMS-macos.txt)
fi

echo "bundle:   $APP"
ls -lh "$ROOT/dist"
