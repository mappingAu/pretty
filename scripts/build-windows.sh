#!/usr/bin/env bash
# Build and package the portable Windows zip for Pretty.
#
# Run on Windows (Git Bash / CI windows-latest), from anywhere:
#   scripts/build-windows.sh              # cargo build + zip
#   scripts/build-windows.sh --no-build   # package an existing release binary
#   scripts/build-windows.sh --features dx --no-build
#
# Artifacts: dist/pretty-<version>-windows-x86_64.zip
#            dist/SHA256SUMS-windows.txt
#
# The build uses `--features dx` by default so the Direct3D 12 backend is
# compiled in and `auto` prefers it on Windows.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

BUILD=1
FEATURES=${PRETTY_FEATURES:-dx}
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) BUILD=0 ;;
    --features) shift; FEATURES=${1:?--features needs a value} ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$VERSION" ] || { echo "cannot read the workspace version from Cargo.toml" >&2; exit 1; }

BINARY=${PRETTY_BINARY:-target/release/pretty.exe}
if [ "$BUILD" = 1 ]; then
  CARGO=${CARGO:-cargo}
  echo "==> cargo build --release -p pretty-app --features $FEATURES"
  "$CARGO" build --release -p pretty-app --features "$FEATURES" --locked
fi
[ -f "$BINARY" ] || {
  echo "missing release binary '$BINARY'; run: cargo build --release -p pretty-app --features dx" >&2
  exit 1
}

if command -v pwsh >/dev/null 2>&1; then
  PS=pwsh
elif command -v powershell >/dev/null 2>&1; then
  PS=powershell
else
  echo "PowerShell (pwsh) is required to create the Windows zip" >&2
  exit 1
fi

echo "==> staging portable zip with $PS"
"$PS" -NoProfile -ExecutionPolicy Bypass -File packaging/windows/package.ps1 \
  -Version "$VERSION" -Binary "$BINARY"
