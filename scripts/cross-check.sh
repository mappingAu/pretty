#!/usr/bin/env bash
# Cross-compilation checks for Windows, macOS and Android, plus host feature
# checks.
#
# Every check is executed for real and writes its command, stdout, stderr and
# exit code under test-artifacts/cross-check/<UTC timestamp>/<label>/. A check
# only becomes PASS when the compiler exits 0; missing Rust std is recorded as
# SKIPPED (never PASS). docs/CROSS-COMPILE.md is regenerated from those results
# and points at docs/CROSS-PLATFORM.md for prerequisites and runtime-test scope.
#
# The checks are compile-only. `cargo check` does not link, so a pass does not
# prove that a Windows/macOS system linker or SDK is installed, and no GUI or
# GPU device is touched. Native builds happen in .github/workflows/build.yml.
#
# Usage:
#   scripts/cross-check.sh                 # run every check
#   scripts/cross-check.sh --no-docs       # keep docs/CROSS-COMPILE.md untouched
#   CARGO=/path/to/cargo scripts/cross-check.sh
set -uo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

WRITE_DOCS=1
for arg in "$@"; do
  case "$arg" in
    --no-docs) WRITE_DOCS=0 ;;
    -h|--help)
      sed -n '2,20p' "$0"
      exit 0
      ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

# Prefer the rustup proxy: a distro cargo/rustc pair usually cannot see the
# targets installed in ~/.rustup, which would silently reduce every cross
# check to "std missing".
if [ -z "${CARGO:-}" ]; then
  if [ -x "$HOME/.cargo/bin/cargo" ]; then
    CARGO="$HOME/.cargo/bin/cargo"
  else
    CARGO=$(command -v cargo || true)
  fi
fi
if [ -z "${RUSTUP:-}" ]; then
  if [ -x "$HOME/.cargo/bin/rustup" ]; then
    RUSTUP="$HOME/.cargo/bin/rustup"
  else
    RUSTUP=$(command -v rustup || true)
  fi
fi
if [ -z "$CARGO" ] || ! command -v "$CARGO" >/dev/null 2>&1; then
  echo "error: cargo not found (set CARGO=/path/to/cargo)" >&2
  exit 2
fi

STAMP=$(date -u '+%Y%m%dT%H%M%SZ')
OUT="$ROOT/test-artifacts/cross-check/$STAMP"
n=1
while [ -e "$OUT" ]; do
  OUT="$ROOT/test-artifacts/cross-check/${STAMP}-$n"
  n=$((n + 1))
done
mkdir -p "$OUT"
RELDIR="test-artifacts/cross-check/${OUT##*/}"

# Host checks reuse the normal target dir; cross-target checks use a separate
# directory so they do not invalidate the host build (and vice versa). Neither
# directory is ever removed by this script.
HOST_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
CROSS_TARGET_DIR="${CROSS_TARGET_DIR:-$ROOT/target/cross}"
export CARGO_TERM_COLOR=never
export CARGO_INCREMENTAL=0

RESULTS="$OUT/results.tsv"
: > "$RESULTS"
printf 'label\tkind\ttarget\tfeatures\tstatus\texit\tlogdir\tcommand\n' >> "$RESULTS"

toolchain_summary() {
  {
    echo "generated_utc=$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    echo "host=$(uname -srm)"
    echo "cargo=$("$CARGO" --version 2>&1)"
    echo "host_target_dir=$HOST_TARGET_DIR"
    echo "cross_target_dir=$CROSS_TARGET_DIR"
    if [ -n "$RUSTUP" ] && command -v "$RUSTUP" >/dev/null 2>&1; then
      echo "rustup=$("$RUSTUP" --version 2>&1 | head -1)"
      echo "toolchains:"
      "$RUSTUP" toolchain list 2>&1 | sed 's/^/  /'
      echo "installed_targets:"
      "$RUSTUP" target list --installed 2>&1 | sed 's/^/  /'
    else
      echo "rustup=not found (cross targets in ~/.rustup may be invisible)"
      echo "toolchains: unknown"
      echo "installed_targets: unknown"
    fi
    echo "linkers_and_sdks:"
    for tool in x86_64-w64-mingw32-gcc x86_64-w64-mingw32-ld clang lld ld64.lld \
      aarch64-linux-android-clang llvm-ar; do
      if command -v "$tool" >/dev/null 2>&1; then
        echo "  $tool=$(command -v "$tool")"
      else
        echo "  $tool=missing"
      fi
    done
    for var in ANDROID_NDK_HOME ANDROID_NDK_ROOT ANDROID_HOME NDK_HOME; do
      if [ -n "${!var:-}" ]; then
        echo "  $var=${!var}"
      else
        echo "  $var=(unset)"
      fi
    done
    echo "git_head=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
    echo "git_dirty_files=$(git status --porcelain 2>/dev/null | wc -l)"
  } > "$OUT/environment.log" 2>&1
}

is_target_installed() {
  local target="$1"
  [ -n "$RUSTUP" ] && command -v "$RUSTUP" >/dev/null 2>&1 || return 1
  "$RUSTUP" target list --installed 2>/dev/null | grep -qx "$target"
}

# run_check <label> <host|target> <target|-> <features|-> <command...>
run_check() {
  local label="$1" kind="$2" target="$3" features="$4"
  shift 4
  local dir="$OUT/$label"
  mkdir -p "$dir"
  printf '%s\n' "$*" > "$dir/command.txt"
  {
    echo "label=$label"
    echo "kind=$kind"
    echo "target=$target"
    echo "features=$features"
    if [ "$kind" = target ]; then
      if is_target_installed "$target"; then
        echo "rust_std_installed=yes"
      else
        echo "rust_std_installed=no"
      fi
    fi
    if [ "$kind" = target ]; then
      echo "cargo_target_dir=$CROSS_TARGET_DIR"
    else
      echo "cargo_target_dir=$HOST_TARGET_DIR"
    fi
  } > "$dir/environment.txt"

  echo "=== $label: $*"
  local started=$SECONDS
  if [ "$kind" = target ]; then
    CARGO_TARGET_DIR="$CROSS_TARGET_DIR" "$@" >"$dir/stdout.log" 2>"$dir/stderr.log"
  else
    CARGO_TARGET_DIR="$HOST_TARGET_DIR" "$@" >"$dir/stdout.log" 2>"$dir/stderr.log"
  fi
  local code=$?
  local elapsed=$((SECONDS - started))
  echo "exit=$code elapsed=${elapsed}s log=$RELDIR/$label"

  local status="FAIL"
  if [ "$code" -eq 0 ]; then
    status="PASS"
  elif [ "$kind" = target ] && grep -Eq "target may not be installed|can't find crate for .(core|std)." \
    "$dir/stderr.log" "$dir/stdout.log"; then
    status="SKIP"
  fi
  {
    echo "exit_code=$code"
    echo "elapsed_seconds=$elapsed"
    echo "status=$status"
  } > "$dir/result.txt"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$label" "$kind" "$target" "$features" "$status" "$code" "$RELDIR/$label" "$*" >> "$RESULTS"
}

toolchain_summary

# --- host checks -----------------------------------------------------------
# The workspace check is the release gate; the feature-specific checks compile
# the Metal/DX gates and the reduced-feature configurations. The renderer unit
# tests execute the backend gate tests (including the wrong-OS typed errors).
run_check "host-workspace" host - - \
  "$CARGO" check --workspace
run_check "host-vulkan-only" host - vulkan \
  "$CARGO" check -p pretty-app --no-default-features --features vulkan
run_check "host-gles-only" host - gles \
  "$CARGO" check -p pretty-app --no-default-features --features gles
run_check "host-tests-renderer" host - default \
  "$CARGO" test -p pretty-renderer --lib
run_check "host-tests-metal" host - metal \
  "$CARGO" test -p pretty-renderer --lib --features metal
run_check "host-tests-dx" host - dx \
  "$CARGO" test -p pretty-renderer --lib --features dx

# --- cross-target checks ---------------------------------------------------
run_check "windows-msvc-dx" target x86_64-pc-windows-msvc dx \
  "$CARGO" check --target x86_64-pc-windows-msvc -p pretty-app --features dx --locked
run_check "windows-gnu-dx" target x86_64-pc-windows-gnu dx \
  "$CARGO" check --target x86_64-pc-windows-gnu -p pretty-app --features dx --locked
run_check "macos-x86_64-metal" target x86_64-apple-darwin metal \
  "$CARGO" check --target x86_64-apple-darwin -p pretty-app --features metal --locked
run_check "macos-aarch64-metal" target aarch64-apple-darwin metal \
  "$CARGO" check --target aarch64-apple-darwin -p pretty-app --features metal --locked
run_check "android-aarch64" target aarch64-linux-android android \
  "$CARGO" check --target aarch64-linux-android -p pretty-app --features android --locked

# --- summary + docs --------------------------------------------------------
fail_count=0
pass_count=0
skip_count=0
while IFS=$'\t' read -r label kind target features status code logdir command; do
  [ "$label" = label ] && continue
  case "$status" in
    PASS) pass_count=$((pass_count + 1)) ;;
    SKIP) skip_count=$((skip_count + 1)) ;;
    *) fail_count=$((fail_count + 1)) ;;
  esac
done < "$RESULTS"

{
  echo "Cross-check run: $RELDIR"
  echo "generated_utc=$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "cargo=$("$CARGO" --version 2>&1)"
  echo "pass=$pass_count skip=$skip_count fail=$fail_count"
  echo
  column -t -s $'\t' "$RESULTS" 2>/dev/null || cat "$RESULTS"
} > "$OUT/summary.txt"

if [ "$WRITE_DOCS" = 1 ]; then
  REPORT="$ROOT/docs/CROSS-COMPILE.md"
  {
    echo "# Cross-compilation status (generated)"
    echo
    echo "Generated: $(date -u '+%Y-%m-%dT%H:%M:%SZ') by \`scripts/cross-check.sh\`."
    echo
    echo "Per-platform prerequisites, the implemented/compiled/runtime-tested"
    echo "breakdown and the release limitations live in"
    echo "[CROSS-PLATFORM.md](CROSS-PLATFORM.md). This file is the machine-generated"
    echo "result table for one run on this host."
    echo
    echo "**Scope:** every PASS below is a compile-only \`cargo check\`/\`cargo test\`"
    echo "result. \`cargo check\` does not link or run a binary, and no physical"
    echo "Windows, macOS or Android hardware was used. A SKIP (missing Rust std) is"
    echo "never reported as a PASS."
    echo
    echo "Run log: \`$RELDIR\`"
    echo
    echo "## Host feature checks"
    echo
    echo "| Check | Features | Result | Exit | Log |"
    echo "|---|---|---|---|---|"
    while IFS=$'\t' read -r label kind target features status code logdir command; do
      [ "$label" = label ] && continue
      [ "$kind" = host ] || continue
      echo "| \`$label\` | $features | $status | $code | [\`$logdir\`](../$logdir) |"
    done < "$RESULTS"
    echo
    echo "## Cross-target checks"
    echo
    echo "| Target | Features | Result | Exit | Log |"
    echo "|---|---|---|---|---|"
    while IFS=$'\t' read -r label kind target features status code logdir command; do
      [ "$label" = label ] && continue
      [ "$kind" = target ] || continue
      if [ "$status" = PASS ]; then
        display="COMPILE-ONLY PASS"
      elif [ "$status" = SKIP ]; then
        display="SKIPPED (Rust std missing)"
      else
        display="FAIL"
      fi
      echo "| \`$target\` | $features | $display | $code | [\`$logdir\`](../$logdir) |"
    done < "$RESULTS"
    echo
    echo "## Environment"
    echo
    echo '```text'
    sed -n '1,80p' "$OUT/environment.log"
    echo '```'
    echo
    echo "## Check details"
    echo
    while IFS=$'\t' read -r label kind target features status code logdir command; do
      [ "$label" = label ] && continue
      echo "### \`$label\` — $status (exit $code)"
      echo
      echo '```bash'
      echo "$command"
      echo '```'
      echo
      if [ "$code" -ne 0 ]; then
        echo "Representative errors:"
        echo
        echo '```text'
        grep -Eh '^error(\[|:)|target may not be installed|note: the .* target' \
          "$OUT/$label/stderr.log" "$OUT/$label/stdout.log" 2>/dev/null | head -12 \
          || echo "(no error line found)"
        echo '```'
        echo
      fi
      echo "Full output: [\`$logdir/stdout.log\`](../$logdir/stdout.log),"
      echo "[\`$logdir/stderr.log\`](../$logdir/stderr.log),"
      echo "[\`$logdir/result.txt\`](../$logdir/result.txt)."
      echo
    done < "$RESULTS"
    echo "## Native CI builds"
    echo
    echo "Native Windows/macOS release binaries are built and unit-tested by"
    echo "\`.github/workflows/build.yml\` (DX12 feature on Windows, Metal feature on"
    echo "macOS, default Vulkan+GLES on Linux). CI does not launch the GUI, so it"
    echo "also does not prove runtime GPU support on those platforms."
    echo
    echo "## Notes"
    echo
    echo "- A target can PASS \`cargo check\` even when its system linker, MSVC"
    echo "  build tools, Xcode SDK or Android NDK is absent, because \`cargo check\`"
    echo "  stops before linking. Do not read that as proof of a working build."
    echo "- Host feature tests execute the renderer's backend-gate unit tests with"
    echo "  the \`metal\` and \`dx\` features compiled on Linux; the wrong-OS typed"
    echo "  errors are exercised there, not on Apple/Windows hardware."
    echo "- Android is compile-checked only. The repository has an"
    echo "  \`android_main\` NativeActivity entry point but no Gradle/NDK APK"
    echo "  packaging; see CROSS-PLATFORM.md."
  } > "$REPORT"
  echo "Cross-compilation report written to docs/CROSS-COMPILE.md"
fi

echo
echo "Artifacts: $OUT"
echo "Summary: pass=$pass_count skip=$skip_count fail=$fail_count"
[ "$fail_count" -eq 0 ] || exit 1
