#!/usr/bin/env bash
# Real-hardware acceptance tests for Pretty.
#
# Requirements: a running KDE Plasma/X11 session (XWayland is acceptable), a
# Vulkan-capable GPU, xdotool, ImageMagick, xclip, spectacle, vttest and python3.
# The script captures logs and screenshots, exercises both backends, kitty
# protocols, scrolling/resize/swapchain recreation, vttest and the 60fps baseline.
set -uo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
BIN=${PRETTY_BIN:-$ROOT/target/release/pretty}
STAMP=$(date '+%Y%m%d-%H%M%S')
ART="$ROOT/test-artifacts/hardware-$STAMP"
SHOTS="$ART/screenshots"
mkdir -p "$SHOTS"
SUMMARY="$ART/summary.txt"
: > "$SUMMARY"
FAIL=0
APP_PID=""
WID=""
PATTERN_PID=""
export DISPLAY=${DISPLAY:-:0}
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
# Keep the protocol probe deterministic. The host's running Fcitx XIM would
# otherwise consume synthesized key press/release events during the kitty
# keyboard CSI-u test; IME has a separate tests/evidence path.
export XMODIFIERS="@im=none"
unset WAYLAND_DISPLAY
export TERMINFO="$ROOT/build/test-terminfo"
mkdir -p "$TERMINFO"
tic -x -o "$TERMINFO" packaging/pretty.terminfo >/dev/null

pass() { echo "PASS: $*" | tee -a "$SUMMARY" | tee -a "$ART/results.log"; }
fail() { echo "FAIL: $*" | tee -a "$SUMMARY" | tee -a "$ART/results.log"; FAIL=$((FAIL+1)); }
info() { echo "INFO: $*" | tee -a "$ART/results.log"; }
have() { command -v "$1" >/dev/null 2>&1; }

cleanup() {
  if [ -n "${PATTERN_PID:-}" ] && kill -0 "$PATTERN_PID" 2>/dev/null; then
    kill "$PATTERN_PID" 2>/dev/null || true
    PATTERN_PID=""
  fi
  if [ -n "${APP_PID:-}" ] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null || true
    for _ in $(seq 1 20); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
    kill -9 "$APP_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
  echo "missing release binary $BIN; run cargo build --release -p pretty-app" >&2
  exit 2
fi
for tool in xdotool import spectacle xclip python3; do
  have "$tool" || { echo "missing required test tool: $tool" >&2; exit 2; }
done

# A dedicated test config avoids mutating the user's configuration.
CONFIG="$ART/test-config.toml"
python3 - "$CONFIG" <<'PY'
import sys
from pathlib import Path
text = Path.home().joinpath('.config/pretty/config.toml')
base = text.read_text() if text.exists() else ''
import re
def setval(key, value, src):
    pattern = rf'(?m)^{re.escape(key)}\s*=.*$'
    if re.search(pattern, src):
        return re.sub(pattern, f'{key} = {value}', src)
    return src + f'\n{key} = {value}\n'
for key, value in {
    'renderer': '"auto"',
    'opacity': '0.55',
    'blur': 'true',
    'prefer_x11_for_blur': 'true',
    'font_size': '15.0',
    'font_family': '"JetBrains Mono"',
    'window_width': '1280',
    'window_height': '820',
    'ui_panel_opacity': '1.0',
    'cursor_blink': 'true',
    'shell': '"/bin/bash"',
    'vsync': 'true',
}.items():
    base = setval(key, value, base)
Path(sys.argv[1]).write_text(base)
PY

# ---------- environment / GPU evidence ----------
{
  echo "date=$(date -Is)"
  echo "kernel=$(uname -a)"
  echo "os=$(cat /etc/os-release | tr '\n' ' ')"
  echo "cpu=$(lscpu | grep 'Model name' | sed 's/^[^:]*: *//')"
  echo "gpu=$(lspci -nn | grep -Ei 'vga|3d|display' | head -2)"
  echo "nvidia=$(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>/dev/null || true)"
  echo "vulkan_driver=$(vulkaninfo --summary 2>/dev/null | grep -E 'driverName|driverInfo|deviceName' | head -6)"
  echo "egl_renderer=$(eglinfo 2>/dev/null | grep -m1 'OpenGL ES profile version')"
} > "$ART/hardware.txt"
info "artifacts: $ART"
"$BIN" --print-info > "$ART/print-info.txt" 2>&1
if grep -q 'adapter="NVIDIA' "$ART/print-info.txt"; then pass "Vulkan hardware adapter detected"; else fail "no hardware Vulkan adapter"; fi
if grep -q 'OpenGL ES 3\.' "$ART/print-info.txt"; then pass "GLES 3.x EGL adapter detected"; else fail "no GLES 3.x adapter"; fi

# ---------- helpers ----------
wait_window() {
  local pid="$1" i w
  for i in $(seq 1 150); do
    # Search by PID, not title: OSC 0/2 tests deliberately change the title
    # immediately after startup and title-based searches can match stale windows.
    WID=$(DISPLAY="$DISPLAY" timeout 5 xdotool search --onlyvisible --pid "$pid" 2>/dev/null | tail -1)
    if [ -n "$WID" ]; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}
window_geometry() {
  eval "$(DISPLAY="$DISPLAY" xdotool getwindowgeometry --shell "$WID")"
  WX=$X; WY=$Y; WW=$WIDTH; WH=$HEIGHT
}
focus_terminal() {
  window_geometry
  DISPLAY="$DISPLAY" xdotool mousemove $((WX + WW / 2)) $((WY + WH / 2)) click 1
  sleep 0.3
}
launch() {
  local renderer="$1" title="$2" cmd="$3" log="$4"
  cleanup
  APP_PID=""
  WID=""
  env DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" TERMINFO="$TERMINFO" \
    RUST_LOG=info "$BIN" --config "$CONFIG" --renderer "$renderer" --title "$title" \
    --command "$cmd" >"$log" 2>&1 &
  APP_PID=$!
  local i
  for i in $(seq 1 120); do
    grep -q 'pane 1 spawned' "$log" && break
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.1
  done
  if ! grep -q 'pane 1 spawned' "$log"; then
    fail "$renderer: app did not reach a spawned pane"
    sed -n '1,160p' "$log" >> "$ART/results.log"
    return 1
  fi
  if ! wait_window "$APP_PID"; then
    fail "$renderer: window '$title' did not appear"
    sed -n '1,160p' "$log" >> "$ART/results.log"
    return 1
  fi
  sleep 0.8
  DISPLAY="$DISPLAY" xdotool windowactivate --sync "$WID" >/dev/null 2>&1 || true
  focus_terminal
  return 0
}
capture_client() {
  local name="$1"
  rm -f "$SHOTS/$name-client.png"
  timeout 15 env DISPLAY="$DISPLAY" import -window "$WID" "$SHOTS/$name-client.png" >/dev/null 2>&1
  [ -s "$SHOTS/$name-client.png" ] && pass "captured $name client screenshot" || fail "capture $name failed"
}
capture_desktop() {
  local name="$1"
  window_geometry
  local full="$SHOTS/$name-full.png"
  if timeout 30 spectacle -b -n -f -o "$full" >/dev/null 2>&1 && [ -s "$full" ]; then
    magick "$full" -crop "$((WW + 80))x$((WH + 100))+$((WX > 40 ? WX - 40 : 0))+$((WY > 50 ? WY - 50 : 0))" +repage "$SHOTS/$name-desktop.png" 2>/dev/null || true
    pass "captured desktop composition for $name"
  else
    fail "spectacle desktop capture failed for $name"
  fi
}
kill_app() {
  cleanup
  APP_PID=""
  WID=""
  sleep 0.5
}

# ---------- high-contrast desktop pattern for blur evidence ----------
if have display && have magick; then
  magick -size 1600x1000 xc:'#f4f7fb' \
    -fill '#0b1220' -pointsize 90 -gravity northwest -annotate +80+90 'SHARP BACKGROUND PATTERN' \
    -fill '#d92d5b' -pointsize 54 -gravity northwest -annotate +80+220 'blur should soften this text' \
    -fill '#1466d8' -pointsize 42 -gravity northwest -annotate +80+310 '0123456789  ABCDEFGHIJKLMNOPQRSTUVWXYZ' \
    -stroke '#1c2c44' -strokewidth 2 -draw 'line 0,420 1600,420,line 0,520 1600,520,line 0,620 1600,620,line 0,720 1600,720,line 0,820 1600,820' \
    -fill none -stroke '#0b1220' -strokewidth 8 -draw 'rectangle 780,640 1500,960' \
    "$ART/blur-pattern.png"
fi

# ---------- Vulkan feature run ----------
info "== Vulkan feature run =="
VLOG="$ART/vulkan.log"
if launch vulkan Pretty-Test-Vulkan "python3 $ROOT/assets/demo/feature-demo.py" "$VLOG"; then
  if [ -f "$ART/blur-pattern.png" ] && have display; then
    DISPLAY="$DISPLAY" display -geometry 1600x1000+160+40 "$ART/blur-pattern.png" >/dev/null 2>&1 &
    PATTERN_PID=$!
    sleep 1
    DISPLAY="$DISPLAY" xdotool windowraise "$WID" >/dev/null 2>&1 || true
    DISPLAY="$DISPLAY" xdotool windowactivate --sync "$WID" >/dev/null 2>&1 || true
    focus_terminal
  fi
  sleep 2
  capture_client vulkan-features
  capture_desktop vulkan-features
  if grep -q 'renderer=vulkan backend=Vulkan adapter="NVIDIA' "$VLOG"; then pass "Vulkan backend selected and logged"; else fail "Vulkan backend not in log"; fi
  # settings UI
  focus_terminal
  DISPLAY="$DISPLAY" xdotool key --clearmodifiers ctrl+comma
  sleep 1
  capture_client vulkan-settings
  DISPLAY="$DISPLAY" xdotool key --clearmodifiers ctrl+comma
  sleep 0.5
  # tiling
  focus_terminal
  DISPLAY="$DISPLAY" xdotool key --clearmodifiers ctrl+shift+e
  sleep 1.2
  capture_client vulkan-tiling
  capture_desktop vulkan-tiling
  cp "$SHOTS/vulkan-tiling-full.png" "$SHOTS/blur-transparency-full.png" 2>/dev/null || true
  if [ -n "$PATTERN_PID" ] && kill -0 "$PATTERN_PID" 2>/dev/null; then
    kill "$PATTERN_PID" 2>/dev/null || true
    PATTERN_PID=""
    sleep 0.5
  fi
  if grep -q 'pane 2 split' "$VLOG"; then pass "tiling split created a second pane"; else fail "tiling split not logged"; fi
  if DISPLAY="$DISPLAY" xprop -id "$WID" _KDE_NET_WM_BLUR_BEHIND_REGION 2>/dev/null | grep -q 'CARDINAL'; then pass "KWin blur region property applied"; else fail "KWin blur region property missing"; fi
  if grep -q 'pixel_alpha=true' "$VLOG"; then pass "X11 per-pixel ARGB alpha path used"; else fail "X11 per-pixel alpha path not logged"; fi
  if DISPLAY="$DISPLAY" xprop -id "$WID" _NET_WM_WINDOW_OPACITY 2>/dev/null | grep -q 'CARDINAL'; then fail "legacy whole-window opacity still present"; else pass "legacy whole-window opacity removed"; fi
  # resize/swapchain recreation
  window_geometry
  DISPLAY="$DISPLAY" xdotool windowsize "$WID" 1040 700
  sleep 1
  DISPLAY="$DISPLAY" xdotool windowsize "$WID" 820 620
  sleep 1
  DISPLAY="$DISPLAY" xdotool windowsize "$WID" 1280 820
  sleep 1
  if kill -0 "$APP_PID" 2>/dev/null; then pass "process survived resize/swapchain recreation"; else fail "process died during resize"; fi
  if grep -q 'swapchain reconfigured' "$VLOG"; then pass "swapchain reconfiguration logged"; else fail "no swapchain reconfiguration log"; fi
  if grep -Eiq 'panicked|thread .main. .*panic' "$VLOG"; then fail "panic observed in Vulkan log"; else pass "no panic in Vulkan log"; fi
  kill_app
else
  fail "Vulkan feature run could not start"
fi

# ---------- GLES feature run ----------
info "== GLES feature run =="
GLOG="$ART/gles.log"
if launch gles Pretty-Test-GLES "python3 $ROOT/assets/demo/feature-demo.py" "$GLOG"; then
  sleep 3
  capture_client gles-features
  capture_desktop gles-features
  if grep -q 'renderer=gles backend=Gl .*OpenGL ES 3\.' "$GLOG"; then pass "real OpenGL ES 3.x context selected"; else fail "GLES context not identified"; fi
  if grep -Eiq 'panicked|thread .main. .*panic' "$GLOG"; then fail "panic observed in GLES log"; else pass "no panic in GLES log"; fi
  kill_app
else
  fail "GLES feature run could not start"
fi

# ---------- Kitty keyboard protocol ----------
info "== kitty keyboard protocol =="
KLOG="$ART/keyboard.log"
KEYFILE="$ART/kitty-keys.txt"
if launch vulkan Pretty-Test-Keyboard "python3 $ROOT/assets/demo/kitty-keyboard-probe.py $KEYFILE 8" "$KLOG"; then
  sleep 1.5
  focus_terminal
  for key in a shift+a ctrl+a shift+Return Up Escape; do DISPLAY="$DISPLAY" xdotool key --clearmodifiers "$key"; sleep 0.25; done
  sleep 8
  capture_client kitty-keyboard
  if [ -s "$KEYFILE" ] && grep -aq 'AFTER_PUSH=.*\[?31u' "$KEYFILE" && grep -aq '\[97;1:1;97u' "$KEYFILE" && grep -aq '\[57352;' "$KEYFILE"; then
    pass "kitty keyboard: flags push/query and CSI-u encodings captured"
  else
    fail "kitty keyboard probe did not capture expected CSI-u sequences"
  fi
  kill_app
else
  fail "keyboard probe could not start"
fi

# ---------- Mouse tracking ----------
info "== mouse tracking =="
MLOG="$ART/mouse.log"
MFILE="$ART/mouse-events.txt"
if launch vulkan Pretty-Test-Mouse "python3 $ROOT/assets/demo/mouse-probe.py $MFILE 6" "$MLOG"; then
  sleep 1.5
  focus_terminal
  window_geometry
  DISPLAY="$DISPLAY" xdotool mousemove $((WX + 120)) $((WY + 160)) mousedown 1
  DISPLAY="$DISPLAY" xdotool mousemove $((WX + 220)) $((WY + 200))
  DISPLAY="$DISPLAY" xdotool mouseup 1
  sleep 6
  capture_client mouse-tracking
  if [ -s "$MFILE" ] && grep -aq $'\x1b\[<0;' "$MFILE"; then pass "SGR mouse press/release received (motion encoder covered by unit tests)"; else fail "mouse probe did not receive SGR events"; fi
  kill_app
else
  fail "mouse probe could not start"
fi

# ---------- Kitty graphics protocol responses ----------
info "== kitty graphics protocol responses =="
GRLOG="$ART/graphics.log"
GRFILE="$ART/graphics-responses.txt"
if launch vulkan Pretty-Test-Graphics "python3 $ROOT/assets/demo/graphics-probe.py $GRFILE" "$GRLOG"; then
  sleep 3
  capture_client kitty-graphics-probe
  if [ -s "$GRFILE" ] && grep -aq 'TRANSMIT=.*OK' "$GRFILE" && grep -aq 'QUERY_OK=.*OK' "$GRFILE" && grep -aq 'QUERY_AFTER_DELETE=.*EINVAL' "$GRFILE"; then
    pass "kitty graphics: chunked PNG transmit/query/delete responses verified"
  else
    fail "kitty graphics response check failed"
  fi
  kill_app
else
  fail "kitty graphics probe could not start"
fi

# ---------- OSC 52, title and notifications ----------
info "== OSC 2 / OSC 52 / OSC 777 =="
OLOG="$ART/osc.log"
OSC_CMD="printf '\\033]2;Pretty OSC Title\\007'; printf '\\033]52;c;%s\\007' \"\$(printf pretty-clipboard-ok | base64 -w0)\"; printf '\\033]777;notify;Pretty;OSC notification ok\\007'; sleep 3600"
if launch vulkan Pretty-Test-OSC "$OSC_CMD" "$OLOG"; then
  sleep 1.5
  TITLE=$(DISPLAY="$DISPLAY" xdotool getwindowname "$WID" 2>/dev/null || true)
  case "$TITLE" in *"Pretty OSC Title"*) pass "OSC 2 changed the window title";; *) fail "OSC 2 title not applied ($TITLE)";; esac
  CLIP=$(DISPLAY="$DISPLAY" xclip -selection clipboard -o 2>/dev/null || true)
  if [ "$CLIP" = "pretty-clipboard-ok" ]; then pass "OSC 52 clipboard store verified with xclip"; else fail "OSC 52 clipboard content mismatch: '$CLIP'"; fi
  sleep 1
  if grep -q 'notification delivered: Pretty' "$OLOG"; then pass "OSC 777 notification delivered through D-Bus"; else fail "notification delivery not confirmed"; fi
  kill_app
else
  fail "OSC probe could not start"
fi

# ---------- Scrollback and swapchain under output ----------
info "== scrollback / resize =="
SLOG="$ART/scroll.log"
SCROLL_CMD="for i in \$(seq 1 220); do printf 'scrollback line %03d — lorem ipsum dolor sit amet\\n' \$i; done; sleep 3600"
if launch vulkan Pretty-Test-Scroll "$SCROLL_CMD" "$SLOG"; then
  sleep 2
  focus_terminal
  capture_client scrollback-bottom
  DISPLAY="$DISPLAY" xdotool key --clearmodifiers shift+Page_Up
  sleep 0.7
  capture_client scrollback-up
  if [ "$(sha256sum "$SHOTS/scrollback-bottom-client.png" | cut -d' ' -f1)" != "$(sha256sum "$SHOTS/scrollback-up-client.png" | cut -d' ' -f1)" ]; then
    pass "scrollback viewport changed after Shift+PageUp"
  else
    fail "scrollback screenshot unchanged"
  fi
  DISPLAY="$DISPLAY" xdotool windowsize "$WID" 1000 700; sleep 0.8
  DISPLAY="$DISPLAY" xdotool windowsize "$WID" 1280 820; sleep 0.8
  kill -0 "$APP_PID" 2>/dev/null && pass "process alive after resize with scrollback" || fail "process died after scrollback resize"
  # Font zoom path (Ctrl+Shift+Plus).
  focus_terminal
  DISPLAY="$DISPLAY" xdotool key --clearmodifiers ctrl+shift+plus
  sleep 1
  capture_client zoomed
  if [ "$(sha256sum "$SHOTS/scrollback-up-client.png" | cut -d' ' -f1)" != "$(sha256sum "$SHOTS/zoomed-client.png" | cut -d' ' -f1)" ]; then
    pass "font zoom reflowed the terminal"
  else
    fail "font zoom screenshot unchanged"
  fi
  kill_app
else
  fail "scrollback run could not start"
fi

# ---------- Cursor shapes ----------
info "== cursor shapes =="
CLOG="$ART/cursor.log"
if launch vulkan Pretty-Test-Cursor "bash $ROOT/assets/demo/cursor-shapes.sh" "$CLOG"; then
  sleep 1
  capture_client cursor-block
  if grep -q 'cursor shape=Some(Block)' "$CLOG"; then pass "block cursor rendered"; else fail "no block cursor log"; fi
  sleep 2.5
  focus_terminal
  capture_client cursor-underline
  if grep -q 'cursor shape=Some(Underline)' "$CLOG"; then pass "underline cursor rendered"; else fail "no underline cursor log"; fi
  sleep 2
  capture_client cursor-bar
  if grep -q 'cursor shape=Some(Beam)' "$CLOG"; then pass "bar cursor rendered"; else fail "no bar cursor log"; fi
  kill_app
else
  fail "cursor shape run could not start"
fi

# ---------- vttest ----------
info "== vttest =="
TLOG="$ART/vttest.log"
if have vttest && launch vulkan Pretty-Test-vttest "vttest" "$TLOG"; then
  sleep 2
  capture_client vttest-menu
  focus_terminal
  DISPLAY="$DISPLAY" xdotool key --clearmodifiers 1 Return
  sleep 1.5
  capture_client vttest-cursor-test
  kill -0 "$APP_PID" 2>/dev/null && pass "vttest executed inside Pretty without crash" || fail "vttest crashed Pretty"
  kill_app
else
  info "vttest not installed; skipped terminal conformance UI run"
fi

# ---------- Benchmarks ----------
info "== rendering benchmark (release) =="
for backend in vulkan gles; do
  BLOG="$ART/bench-$backend.log"
  env DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" TERMINFO="$TERMINFO" RUST_LOG=info \
    "$BIN" --config "$CONFIG" --renderer "$backend" --benchmark 180 \
    --command "bash $ROOT/assets/demo/bench.sh" >"$BLOG" 2>&1
  RESULT=$(grep 'BENCHMARK RESULT' "$BLOG" | tail -1)
  if [ -n "$RESULT" ]; then
    FPS=$(echo "$RESULT" | sed -n 's/.*avg_fps=\([0-9.]*\).*/\1/p')
    if awk -v fps="$FPS" 'BEGIN { exit !(fps >= 55) }'; then
      pass "$RESULT"
    else
      fail "$backend benchmark below 55 fps: $RESULT"
    fi
  else
    fail "$backend benchmark produced no result"
  fi
done

# ---------- publish evidence ----------
cp "$SHOTS/vulkan-features-client.png" "$ROOT/docs/screenshots/vulkan-features.png" 2>/dev/null || true
cp "$SHOTS/vulkan-settings-client.png" "$ROOT/docs/screenshots/vulkan-settings.png" 2>/dev/null || true
cp "$SHOTS/vulkan-tiling-client.png" "$ROOT/docs/screenshots/vulkan-tiling.png" 2>/dev/null || true
cp "$SHOTS/blur-transparency-full.png" "$ROOT/docs/screenshots/vulkan-blur-transparency.png" 2>/dev/null || true
cp "$SHOTS/cursor-block-client.png" "$ROOT/docs/screenshots/cursor-block.png" 2>/dev/null || true
cp "$SHOTS/cursor-underline-client.png" "$ROOT/docs/screenshots/cursor-underline.png" 2>/dev/null || true
cp "$SHOTS/gles-features-client.png" "$ROOT/docs/screenshots/gles-features.png" 2>/dev/null || true
cp "$SHOTS/kitty-keyboard-client.png" "$ROOT/docs/screenshots/kitty-keyboard.png" 2>/dev/null || true
cp "$SHOTS/kitty-graphics-probe-client.png" "$ROOT/docs/screenshots/kitty-graphics-probe.png" 2>/dev/null || true
cp "$SHOTS/mouse-tracking-client.png" "$ROOT/docs/screenshots/mouse-tracking.png" 2>/dev/null || true
cp "$SHOTS/scrollback-up-client.png" "$ROOT/docs/screenshots/scrollback.png" 2>/dev/null || true
cp "$SHOTS/zoomed-client.png" "$ROOT/docs/screenshots/zoom.png" 2>/dev/null || true
cp "$SHOTS/cursor-bar-client.png" "$ROOT/docs/screenshots/cursor-bar.png" 2>/dev/null || true
cp "$SHOTS/vttest-menu-client.png" "$ROOT/docs/screenshots/vttest.png" 2>/dev/null || true

{
  echo
  echo "== SUMMARY =="
  echo "failures=$FAIL"
  echo "artifacts=$ART"
  echo "backend logs: $VLOG $GLOG"
} | tee -a "$SUMMARY"
[ "$FAIL" -eq 0 ]
