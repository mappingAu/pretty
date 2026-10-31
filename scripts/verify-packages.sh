#!/usr/bin/env bash
# Install and launch verification for all three distribution artifacts.
#
# Host GUI checks use the real NVIDIA GPU. The Debian package is installed and
# launched in a Debian 12 container under Xvfb/Mesa only to validate packaging;
# that is explicitly software compositing and is not used as GPU evidence.
set -uo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
STAMP=$(date '+%Y%m%d-%H%M%S')
ART="$ROOT/test-artifacts/packaging-$STAMP"
mkdir -p "$ART/screenshots"
SUMMARY="$ART/summary.txt"
: > "$SUMMARY"
FAIL=0
PASS() { echo "PASS: $*" | tee -a "$SUMMARY"; }
FAILMSG() { echo "FAIL: $*" | tee -a "$SUMMARY"; FAIL=$((FAIL+1)); }
INFO() { echo "INFO: $*" | tee -a "$SUMMARY"; }
have() { command -v "$1" >/dev/null 2>&1; }

export DISPLAY=${DISPLAY:-:0}
unset WAYLAND_DISPLAY
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export TERMINFO="$ROOT/build/test-terminfo"
mkdir -p "$TERMINFO"
tic -x -o "$TERMINFO" packaging/pretty.terminfo >/dev/null 2>&1 || true
# For installed packages, use the system-wide terminfo; unset the dev override.
unset TERMINFO

find_artifacts() {
  DEB=$(find "$ROOT/dist" -maxdepth 1 -name '*.deb' | head -1)
  RPM=$(find "$ROOT/dist" -maxdepth 1 -name '*.rpm' | head -1)
  TAR=$(find "$ROOT/dist" -maxdepth 1 -name '*.tar.gz' | head -1)
  for f in "$DEB" "$RPM" "$TAR"; do
    [ -s "$f" ] || { echo "missing package artifact under dist/ (run scripts/build-packages.sh)" >&2; exit 2; }
  done
  echo "deb=$DEB" > "$ART/artifacts.txt"
  echo "rpm=$RPM" >> "$ART/artifacts.txt"
  echo "tar=$TAR" >> "$ART/artifacts.txt"
  INFO "artifacts: $(basename "$DEB") $(basename "$RPM") $(basename "$TAR")"
}
find_artifacts
if (cd "$ROOT/dist" && sha256sum -c SHA256SUMS >"$ART/sha256-check.txt" 2>&1); then
  PASS "dist/SHA256SUMS verified"
else
  FAILMSG "dist/SHA256SUMS verification failed"
fi

launch_and_capture() {
  local label="$1"
  local binary="$2"
  local cfg="$3"
  local log="$ART/$label.log"
  local shot="$ART/screenshots/$label.png"
  env DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" RUST_LOG=info \
    "$binary" --config "$cfg" --renderer vulkan --title "Pretty-$label" \
    --command "printf '\\033[1;38;2;122;162;247m%s\\033[0m\\n\\nPackage launch OK\\nbackend: Vulkan\\n'; sleep 30" \
    >"$log" 2>&1 &
  local pid=$! wid="" i
  for i in $(seq 1 100); do
    wid=$(DISPLAY="$DISPLAY" timeout 5 xdotool search --onlyvisible --pid "$pid" 2>/dev/null | tail -1)
    [ -n "$wid" ] && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  local ready=0
  for i in $(seq 1 100); do
    if grep -q 'pane 1 spawned' "$log"; then ready=1; break; fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.2
  done
  sleep 2
  if [ "$ready" = 1 ] && kill -0 "$pid" 2>/dev/null && [ -n "$wid" ]; then
    timeout 15 env DISPLAY="$DISPLAY" import -window "$wid" "$shot" >/dev/null 2>&1
    colors=$(identify -format '%k' "$shot" 2>/dev/null || echo 0)
    if [ -s "$shot" ] && [ "${colors:-0}" -gt 8 ]; then
      PASS "$label: launched and rendered a non-blank window ($colors colours)"
    else
      FAILMSG "$label: screenshot is blank (colours=$colors)"
    fi
  else
    FAILMSG "$label: did not launch a ready window"
    tail -30 "$log" >> "$SUMMARY"
  fi
  kill "$pid" 2>/dev/null || true
  sleep 0.3
  kill -9 "$pid" 2>/dev/null || true
}

# ---------------- RPM on Fedora ----------------
INFO "== RPM install on Fedora =="
if have sudo && have rpm; then
  sudo dnf remove -y pretty-terminal >/dev/null 2>&1 || true
  if sudo dnf install -y "$RPM" >"$ART/rpm-install.log" 2>&1; then
    PASS "rpm installed by dnf"
    rpm -q pretty-terminal >"$ART/rpm-query.txt" && PASS "rpm -q reports package" || FAILMSG "rpm -q failed"
    rpm -ql pretty-terminal >"$ART/rpm-files.txt"
    grep -q '/usr/bin/pretty$' "$ART/rpm-files.txt" && PASS "rpm contains /usr/bin/pretty" || FAILMSG "binary missing from rpm"
    grep -q '/usr/share/terminfo/p/pretty$' "$ART/rpm-files.txt" && PASS "rpm contains terminfo" || FAILMSG "terminfo missing from rpm"
    grep -q '/usr/share/applications/pretty.desktop$' "$ART/rpm-files.txt" && PASS "rpm contains desktop entry" || FAILMSG "desktop entry missing from rpm"
    grep -q '/usr/share/icons/hicolor/256x256/apps/pretty.png$' "$ART/rpm-files.txt" && PASS "rpm contains icons" || FAILMSG "icons missing from rpm"
    grep -q '/usr/share/man/man1/pretty.1.gz$' "$ART/rpm-files.txt" && PASS "rpm contains man page" || FAILMSG "man page missing from rpm"
    if /usr/bin/pretty --version >"$ART/rpm-version.txt" 2>&1; then PASS "installed binary --version"; else FAILMSG "installed binary failed --version"; fi
    if infocmp -A /usr/share/terminfo pretty >"$ART/rpm-terminfo.txt" 2>&1; then PASS "system terminfo entry usable"; else FAILMSG "system terminfo entry unusable"; fi
    CFG="$ART/rpm-config.toml"
    cp "$ROOT/.ci/pretty-test-config.toml" "$CFG" 2>/dev/null || true
    if [ ! -s "$CFG" ]; then cp /tmp/pretty-test.toml "$CFG" 2>/dev/null || true; fi
    if [ -s "$CFG" ]; then python3 - "$CFG" <<'PY'
import sys, pathlib
p=pathlib.Path(sys.argv[1]); s=p.read_text()
s=s.replace('blur = true','blur = false').replace('opacity = 0.9','opacity = 1.0')
p.write_text(s)
PY
    else
      printf 'renderer = "vulkan"\nblur = false\nopacity = 1.0\n' > "$CFG"
    fi
    launch_and_capture rpm-installed /usr/bin/pretty "$CFG"
  else
    FAILMSG "rpm installation failed"; tail -30 "$ART/rpm-install.log" >> "$SUMMARY"
  fi
else
  FAILMSG "rpm/sudo not available; cannot install RPM"
fi

# ---------------- tar.gz direct ----------------
INFO "== relocatable tar.gz =="
TARROOT="$ART/tar-root"
mkdir -p "$TARROOT"
tar -xzf "$TAR" -C "$TARROOT"
TBIN=$(find "$TARROOT" -path '*/bin/pretty' -type f | head -1)
if [ -n "$TBIN" ] && [ -x "$TBIN" ]; then
  PASS "tar.gz extracted and bin/pretty is executable"
  if "$TBIN" --version >"$ART/tar-version.txt" 2>&1; then PASS "tarball binary --version"; else FAILMSG "tarball binary failed --version"; fi
  TCFG="$ART/tar-config.toml"
  cp "$ROOT/.ci/pretty-test-config.toml" "$TCFG" 2>/dev/null || true
  if [ ! -s "$TCFG" ]; then cp /tmp/pretty-test.toml "$TCFG" 2>/dev/null || true; fi
  if [ -s "$TCFG" ]; then python3 - "$TCFG" <<'PY'
import sys, pathlib
p=pathlib.Path(sys.argv[1]); s=p.read_text()
s=s.replace('blur = true','blur = false').replace('opacity = 0.9','opacity = 1.0')
p.write_text(s)
PY
  else
    printf 'renderer = "vulkan"\nblur = false\nopacity = 1.0\n' > "$TCFG"
  fi
  # Direct launch must find ../share/terminfo thanks to app auto-detection.
  launch_and_capture tar-installed "$TBIN" "$TCFG"
  if [ -f "$(dirname "$(dirname "$TBIN")")/share/terminfo/p/pretty" ]; then PASS "tarball includes share/terminfo/p/pretty"; else FAILMSG "tarball terminfo missing"; fi
  if [ -f "$(dirname "$(dirname "$TBIN")")/share/applications/pretty.desktop" ]; then PASS "tarball includes desktop file"; else FAILMSG "tarball desktop file missing"; fi
  if [ -f "$(dirname "$(dirname "$TBIN")")/share/icons/hicolor/256x256/apps/pretty.png" ]; then PASS "tarball includes icon"; else FAILMSG "tarball icon missing"; fi
else
  FAILMSG "tar.gz has no executable bin/pretty"
fi

# ---------------- Debian package in a clean container ----------------
INFO "== deb install in Debian 12 container =="
if have podman; then
  cat > "$ART/deb-verify.sh" <<'INNER'
#!/bin/bash
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update >/tmp/apt.log 2>&1
if ! apt-get install -y --no-install-recommends /out/pkg.deb xvfb x11-utils imagemagick xdotool \
      libgl1-mesa-dri libegl-mesa0 dbus-x11 >/tmp/install.log 2>&1; then
  echo "APT_INSTALL_FAILED"
  tail -60 /tmp/install.log
  exit 3
fi
echo "DPKG_STATUS=$(dpkg-query -W -f='${Status} ${Version}' pretty-terminal 2>&1)"
pretty --version || exit 4
infocmp pretty >/dev/null || exit 5
test -f /usr/share/applications/pretty.desktop || exit 6
test -f /usr/share/icons/hicolor/256x256/apps/pretty.png || exit 7
Xvfb :99 -screen 0 1280x820x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
XVFB=$!
sleep 1
export DISPLAY=:99
printf 'renderer = "gles"\nblur = false\nopacity = 0.9\nfont_size = 14.0\nwindow_width = 1000\nwindow_height = 640\nshell = "/bin/sh"\n' > /tmp/pkg-config.toml
pretty --config /tmp/pkg-config.toml --renderer gles --allow-software --no-blur --opacity 0.9 \
  --command "printf 'Debian package launch smoke test\\n'; sleep 30" >/tmp/gui.log 2>&1 &
APP=$!
for i in $(seq 1 100); do
  WID=$(xdotool search --onlyvisible --pid "$APP" 2>/dev/null | tail -1)
  [ -n "$WID" ] && break
  kill -0 "$APP" 2>/dev/null || break
  sleep 0.2
done
for i in $(seq 1 100); do
  grep -q 'pane 1 spawned' /tmp/gui.log && break
  kill -0 "$APP" 2>/dev/null || break
  sleep 0.2
done
sleep 2
if [ -n "${WID:-}" ] && kill -0 "$APP" 2>/dev/null && grep -q 'pane 1 spawned' /tmp/gui.log; then
  import -window "$WID" /out/deb-installed.png 2>/dev/null || true
  cp /tmp/gui.log /out/deb-gui.log 2>/dev/null || true
  echo "GUI_LAUNCHED=1 WID=$WID"
else
  echo "GUI_LAUNCH_FAILED=1"
  tail -40 /tmp/gui.log
  kill "$XVFB" 2>/dev/null || true
  exit 8
fi
kill "$APP" 2>/dev/null || true
kill "$XVFB" 2>/dev/null || true
exit 0
INNER
  chmod +x "$ART/deb-verify.sh"
  cp "$DEB" "$ART/pkg.deb"
  if podman run --rm --security-opt label=disable \
      -v "$ROOT/dist:/dist:Z" -v "$ART:/out:Z" \
      docker.io/library/debian:bookworm-slim /out/deb-verify.sh >"$ART/deb-verify.log" 2>&1; then
    PASS "deb installed in clean Debian 12 container"
    grep -q 'DPKG_STATUS=install ok installed' "$ART/deb-verify.log" && PASS "dpkg reports installed state" || FAILMSG "dpkg installed state missing"
    grep -q 'GUI_LAUNCHED=1' "$ART/deb-verify.log" && PASS "deb package GUI launched under Xvfb/Mesa (explicit software smoke only)" || FAILMSG "deb package GUI failed to launch"
    grep -q 'ALLOWING SOFTWARE ADAPTER' "$ART/deb-gui.log" && PASS "deb smoke log explicitly reports software adapter" || FAILMSG "software-adapter warning absent"
    if [ -s "$ART/deb-installed.png" ] && [ "$(identify -format '%k' "$ART/deb-installed.png" 2>/dev/null || echo 0)" -gt 8 ]; then
      PASS "deb GUI screenshot captured (non-blank)"
    else
      FAILMSG "deb GUI screenshot missing or blank"
    fi
  else
    FAILMSG "deb container verification failed"
    tail -60 "$ART/deb-verify.log" >> "$SUMMARY"
  fi
else
  FAILMSG "podman unavailable; cannot verify deb install"
fi

{
  echo "== SUMMARY =="
  echo "failures=$FAIL"
  echo "artifacts=$ART"
} | tee -a "$SUMMARY"
[ "$FAIL" -eq 0 ]
