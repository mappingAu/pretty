#!/usr/bin/env bash
# Build and package the macOS .app bundle and tarballs for Pretty.
#
# Run on macOS (CI macos-latest), from anywhere:
#   scripts/build-macos.sh              # cargo build + bundle + tar.gz
#   scripts/build-macos.sh --no-build   # package an existing release binary
#   scripts/build-macos.sh --features vulkan,metal --no-build
#
# Artifacts: dist/pretty-<version>-macos-<arch>.app.tar.gz
#            dist/pretty-<version>-macos-<arch>.tar.gz
#            dist/SHA256SUMS-macos.txt
#
# The build compiles Vulkan and Metal (but not GLES) so `auto` is
# Metal -> Vulkan on macOS.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

BUILD=1
FEATURES=${PRETTY_FEATURES:-vulkan,metal}
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) BUILD=0 ;;
    --features) shift; FEATURES=${1:?--features needs a value} ;;
    -h|--help)
      sed -n '2,13p' "$0"
      exit 0
      ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

if [ "$BUILD" = 1 ]; then
  CARGO=${CARGO:-cargo}
  echo "==> cargo build --release -p pretty-app --no-default-features --features $FEATURES"
  "$CARGO" build --release -p pretty-app --no-default-features --features "$FEATURES" --locked
fi

echo "==> staging Pretty.app and tarballs"
PRETTY_BINARY=${PRETTY_BINARY:-target/release/pretty} bash packaging/macos/package.sh
