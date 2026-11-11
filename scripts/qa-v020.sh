#!/usr/bin/env bash
#
# Reproducible, GUI-free QA checks for Pretty v0.2.0.
#
# This script never opens a window and never touches a user configuration:
# CLI checks run against throw-away files under a mktemp directory. Test
# output is written to test-artifacts/qa-v020-<timestamp>/.
#
# Usage:
#   scripts/qa-v020.sh                 # core + workspace tests + CLI smoke
#   scripts/qa-v020.sh --core-only     # only core tests + CLI smoke
#   PRETTY_BIN=target/debug/pretty scripts/qa-v020.sh
#
# Exit status: 0 when every check passed, 1 otherwise.
set -uo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$REPO_ROOT" || exit 2

CORE_ONLY=0
for arg in "$@"; do
    case "$arg" in
        --core-only) CORE_ONLY=1 ;;
        -h|--help)
            sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

STAMP=${QA_STAMP:-$(date +%Y%m%d-%H%M%S)}
OUT=${QA_OUT:-"test-artifacts/qa-v020-$STAMP"}
mkdir -p "$OUT" || exit 2
SUMMARY="$OUT/summary.txt"
: >"$SUMMARY"

PASS=0
FAIL=0
SKIP=0

note() {
    printf '%s\n' "$*" | tee -a "$SUMMARY"
}

# run_check <name> <command...>
# Runs a command, stores stdout+stderr in <OUT>/<name>.log and records the
# exact command and exit code in the summary.
run_check() {
    local name=$1
    shift
    local log="$OUT/$name.log"
    note "=== $name ==="
    note "cmd: $*"
    "$@" >"$log" 2>&1
    local rc=$?
    note "$name exit=$rc"
    if [ "$rc" -eq 0 ]; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
    return "$rc"
}

# expect_contains <name> <file> <needle>
expect_contains() {
    local name=$1 file=$2 needle=$3
    if grep -Fq -- "$needle" "$file"; then
        note "$name: contains '$needle' -> PASS"
        PASS=$((PASS + 1))
    else
        note "$name: missing '$needle' -> FAIL (see $file)"
        FAIL=$((FAIL + 1))
    fi
}

# expect_absent <name> <file> <needle>
expect_absent() {
    local name=$1 file=$2 needle=$3
    if grep -Fq -- "$needle" "$file"; then
        note "$name: unexpected '$needle' -> FAIL (see $file)"
        FAIL=$((FAIL + 1))
    else
        note "$name: no '$needle' -> PASS"
        PASS=$((PASS + 1))
    fi
}

# expect_nonzero <name> <exit-code>
expect_nonzero() {
    local name=$1 rc=$2
    if [ "$rc" -ne 0 ]; then
        note "$name: exit=$rc (expected non-zero) -> PASS"
        PASS=$((PASS + 1))
    else
        note "$name: exit=0 (expected non-zero) -> FAIL"
        FAIL=$((FAIL + 1))
    fi
}

note "QA run: $(date -Is)"
note "host: $(uname -srm)"
note "rustc: $(rustc --version 2>/dev/null || echo unavailable)"
note "cargo: $(cargo --version 2>/dev/null || echo unavailable)"
note "session: XDG_SESSION_TYPE=${XDG_SESSION_TYPE:-?} WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-none} DISPLAY=${DISPLAY:-none}"
note "artifacts: $OUT"

# ---------------------------------------------------------------------------
# Rust test suites
# ---------------------------------------------------------------------------
run_check core-tests-debug cargo test -p pretty-core
run_check core-tests-release cargo test -p pretty-core --release
if [ "$CORE_ONLY" -eq 0 ]; then
    run_check workspace-tests cargo test --workspace
fi

if cargo clippy --version >/dev/null 2>&1; then
    run_check clippy cargo clippy --workspace --all-targets -- -D warnings
else
    note "clippy: not installed (cargo-clippy unavailable) -> SKIP"
    SKIP=$((SKIP + 1))
fi

# ---------------------------------------------------------------------------
# CLI / UX smoke tests (no GUI window)
# ---------------------------------------------------------------------------
BIN=${PRETTY_BIN:-target/release/pretty}
if [ ! -x "$BIN" ]; then
    if [ -x target/debug/pretty ]; then
        BIN=target/debug/pretty
    else
        note "CLI smoke: no binary at target/release/pretty or target/debug/pretty -> SKIP"
        note "build one with: cargo build -p pretty-app"
        SKIP=$((SKIP + 1))
        BIN=""
    fi
fi

if [ -n "$BIN" ]; then
    BIN_ABS=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
    note "binary: $BIN_ABS"
    note "binary mtime: $(stat -c '%y' "$BIN_ABS" 2>/dev/null || stat -f '%Sm' "$BIN_ABS" 2>/dev/null || echo unknown)"

    run_check cli-version "$BIN_ABS" --version
    expect_contains cli-version "$OUT/cli-version.log" "pretty 0.2.0"

    run_check cli-help "$BIN_ABS" --help
    expect_contains cli-help "$OUT/cli-help.log" "Usage: pretty"

    if command -v timeout >/dev/null 2>&1; then
        run_check cli-print-info timeout 180 "$BIN_ABS" --print-info
    else
        run_check cli-print-info "$BIN_ABS" --print-info
    fi
    expect_contains cli-print-info "$OUT/cli-print-info.log" "Pretty 0.2.0"
    expect_contains cli-print-info "$OUT/cli-print-info.log" "auto_order:"

    TMP=$(mktemp -d "${TMPDIR:-/tmp}/pretty-qa.XXXXXX") || exit 2
    mkdir -p "$TMP/inc"
    cat >"$TMP/inc/colours.conf" <<'EOF'
background #112233
color5 #aabbcc
map ctrl+shift+f find
unknown_kitty_option yes
EOF
    cat >"$TMP/kitty.conf" <<'EOF'
font_size 17.0
cursor_shape underline
include inc/colours.conf
EOF
    CONFIG="$TMP/config.toml"
    run_check cli-import-kitty-conf "$BIN_ABS" --import-kitty-conf "$TMP/kitty.conf" --config "$CONFIG"
    if [ -f "$CONFIG" ]; then
        expect_contains cli-import-kitty-conf-result "$CONFIG" "font_size = 17.0"
        expect_contains cli-import-kitty-conf-result "$CONFIG" 'cursor_shape = "underline"'
        expect_contains cli-import-kitty-conf-result "$CONFIG" '"#112233"'
        expect_contains cli-import-kitty-conf-result "$CONFIG" '"#aabbcc"'
        expect_contains cli-import-kitty-conf-result "$CONFIG" 'find = "Ctrl+Shift+F"'
        expect_contains cli-import-kitty-conf-warnings "$OUT/cli-import-kitty-conf.log" "skipped: unknown_kitty_option"
    else
        note "cli-import-kitty-conf-result: config file was not written -> FAIL"
        FAIL=$((FAIL + 1))
    fi

    # A missing include is reported as a warning and must not destroy the
    # existing target config (import is intentionally non-fatal).
    echo 'font_size = 12.0' >"$TMP/keep.toml"
    cat >"$TMP/broken.conf" <<'EOF'
include does-not-exist.conf
EOF
    run_check cli-import-missing-include "$BIN_ABS" --import-kitty-conf "$TMP/broken.conf" --config "$TMP/keep.toml"
    expect_contains cli-import-missing-include "$OUT/cli-import-missing-include.log" "cannot read"
    expect_contains cli-import-missing-include-kept "$TMP/keep.toml" "font_size = 12.0"

    # A missing *source* file is a hard error (non-zero exit).
    "$BIN_ABS" --import-kitty-conf "$TMP/no-such-file.conf" --config "$TMP/missing-source.toml" \
        >"$OUT/cli-import-missing-source.log" 2>&1
    MISSING_SOURCE_RC=$?
    expect_nonzero cli-import-missing-source "$MISSING_SOURCE_RC"
    expect_contains cli-import-missing-source "$OUT/cli-import-missing-source.log" "failed to import"

    # Malformed TOML: the loader must log the parse error, fall back to
    # defaults and leave the file untouched (exit 0 for --print-info).
    BAD="$TMP/bad.toml"
    printf 'font_size = "not a number"\n[broken\n' >"$BAD"
    BAD_BEFORE=$(cksum "$BAD")
    RUST_LOG=info run_check cli-malformed-toml timeout 180 "$BIN_ABS" --config "$BAD" --print-info
    expect_contains cli-malformed-toml "$OUT/cli-malformed-toml.log" "using defaults"
    expect_contains cli-malformed-toml "$OUT/cli-malformed-toml.log" "Pretty 0.2.0"
    BAD_AFTER=$(cksum "$BAD")
    if [ "$BAD_BEFORE" = "$BAD_AFTER" ]; then
        note "cli-malformed-toml: config file unchanged -> PASS"
        PASS=$((PASS + 1))
    else
        note "cli-malformed-toml: config file was modified -> FAIL"
        FAIL=$((FAIL + 1))
    fi

    "$BIN_ABS" --definitely-not-an-option >"$OUT/cli-unknown-arg.log" 2>&1
    UNKNOWN_RC=$?
    expect_nonzero cli-unknown-arg "$UNKNOWN_RC"
    expect_contains cli-unknown-arg "$OUT/cli-unknown-arg.log" "unknown argument"

    rm -rf "$TMP"
fi

note ""
note "PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
note "logs: $OUT"
if [ "$FAIL" -ne 0 ]; then
    exit 1
fi
exit 0
