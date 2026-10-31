# Pretty

Pretty is a hardware-accelerated terminal emulator written in Rust. It targets
the **kitty terminal protocol**, renders through **wgpu/Vulkan** or **OpenGL ES
3.0+**, and is designed to feel familiar to Konsole users: graphical settings,
split panes, scrollback, clipboard integration and desktop notifications.

![Pretty Vulkan feature demo](docs/screenshots/vulkan-features.png)

## Status

| Area | State |
|---|---|
| Vulkan backend | **working**, tested on an NVIDIA GTX 1050 / driver 580.178.04 |
| GLES backend | **working**, tested with a real OpenGL ES 3.2 NVIDIA context |
| Metal / DirectX | **reserved only** — explicit `UnsupportedBackend` error, never silently selected |
| kitty graphics | PNG/RGB/RGBA, chunked transfer, placement, delete, query, z-index; animation/file/SHM are not implemented |
| kitty keyboard | all five progressive-enhancement flags, query/push/pop, tested CSI-u output |
| Tiling, transparency, blur | working tiling panes; KWin/X11 window opacity + blur-behind |
| Packages | deb, rpm and relocatable tar.gz with desktop entry, icon, man page and terminfo |

See [docs/KITTY-COMPAT.md](docs/KITTY-COMPAT.md) for the item-by-item protocol
matrix, [docs/TEST-REPORT.md](docs/TEST-REPORT.md) for real-hardware evidence and
[docs/KNOWN-ISSUES.md](docs/KNOWN-ISSUES.md) for honest limitations.

## Features

- GPU-rendered terminal grid, true 24-bit colour, bold/italic/dim, underline
  styles, strikeout, cursor shapes and selection.
- Text shaping with **JetBrains Mono ligatures** (`=>`, `!=`, `->`, `===`, …),
  configurable OpenType features and system-font fallback for CJK.
- Bundled monochrome Noto Emoji fallback and colour-font support in the atlas.
- **Kitty graphics protocol**: PNG and raw RGB/RGBA transfers, base64 chunks,
  transmit/display, placements, z-index, delete and query, with protocol
  responses. Images are clipped to their pane and move with scrollback.
- **Kitty keyboard protocol**: disambiguate, event types, alternate keys, all
  keys as CSI-u, associated text; progressive push/pop/query tested.
- OSC 8 hyperlinks rendered with an underline; Ctrl+click opens the URL.
- OSC 52 clipboard copy/paste (configurable), OSC 0/2 window titles, OSC 9 and
  OSC 777 desktop notifications.
- Full mouse reporting: X10, 1000, 1002, 1003, 1005 and 1006.
- Scrollback (default 10,000 lines), scrollbar, Shift+PageUp/PageDown and
  wheel scrolling; scroll regions and alternate screen from the terminal core.
- **Tiling panes** with real PTYs, keyboard focus navigation and per-pane
  terminals.
- Translucent window with **KWin blur-behind** on KDE/X11; no fake CPU blur.
- A customisable egui settings panel (Ctrl+, or the Settings button) covering
  font, colours, opacity, blur, padding, scrollback, tiling gap, vsync and
  backend selection. Only a restart is required to change GPU backend.
- Startup logs include requested/resolved backend, GPU, driver, surface format,
  alpha mode, font and negotiated protocol level.

## Build

Prerequisites on Fedora (Rust 1.89+ is required; the repository pins a stable
toolchain):

```bash
sudo dnf install rust cargo gcc gcc-c++ pkgconf-pkg-config \
  fontconfig-devel freetype-devel libxkbcommon-devel wayland-devel \
  mesa-libEGL-devel libX11-devel libxcb-devel vulkan-loader-devel \
  openssl-devel
cargo build --release
```

Using rustup (recommended for portable packaging and cross checks):

```bash
rustup toolchain install stable
rustup target add x86_64-pc-windows-gnu x86_64-apple-darwin \
  aarch64-apple-darwin aarch64-linux-android
cargo build --release
```

The binary is `target/release/pretty`. The bundled terminfo is generated with:

```bash
tic -x -o ~/.terminfo packaging/pretty.terminfo
```

A relocatable tarball and native packages are produced with:

```bash
scripts/build-packages.sh
```

## Run

```bash
./target/release/pretty
PRETTY_RENDERER=vulkan ./target/release/pretty
PRETTY_RENDERER=gles   ./target/release/pretty
./target/release/pretty --print-info
./target/release/pretty --renderer metal   # explicit reserved-backend error
```

The selected backend can also be set in `~/.config/pretty/config.toml`.
`PRETTY_RENDERER` and `--renderer` take precedence. Pretty never falls back to a
CPU rasteriser. If no hardware-compatible adapter exists, startup fails with the
exact reason.

### GLES note

wgpu's GL backend prefers desktop OpenGL when EGL advertises it. The repository
vendors a small `wgpu-hal` patch (`vendor/wgpu-hal`) that makes the backend create
an **actual `EGL_OPENGL_ES_API` context** when Pretty requests GLES. This is why
the driver string in the logs is `OpenGL ES 3.2 ...`. Pretty validates that the
driver is a GLES/ANGLE context and refuses desktop GL. On Linux, GLES currently
uses an X11/XWayland window surface; native Wayland EGL presentation is not
usable with this NVIDIA driver. See `docs/RENDERER.md` and
`docs/KNOWN-ISSUES.md`.

## Hardware-only policy

Pretty refuses CPU/software adapters on normal startup. If no Vulkan/GLES
hardware adapter is available it prints the reason, exits non-zero and does not
silently fall back to a CPU rasterizer. `--allow-software` is a testing-only
escape hatch for packaging CI on machines without a GPU (for example Xvfb and
llvmpipe). It is never enabled by default, logs a prominent warning, and is not
used for any GPU rendering claim in this repository.

## Configuration

Configuration is TOML at `~/.config/pretty/config.toml` (or `$PRETTY_CONFIG`,
or `--config`). It can also be edited graphically. Only CLI overrides such as
`--opacity` or `--no-blur` are runtime-only and are not persisted.

```toml
renderer = "auto"          # auto | vulkan | gles
font_family = "JetBrains Mono"
font_size = 15.0
line_height = 1.18
ligatures = true
font_features = ["liga", "calt"]
opacity = 0.90
blur = true                # KWin/X11 blur-behind
padding = 10.0
gap = 5.0
background = "#0d1117"
foreground = "#d8dee9"
cursor = "#88c0d0"
accent = "#7aa2f7"
selection_bg = "#334b67"
selection_fg = "#eceff4"
scrollback = 10000
vsync = true
shell = "/bin/bash"
allow_osc52_paste = true
window_width = 1280
window_height = 820
ui_accent = "#7aa2f7"
ui_panel_opacity = 1.0
prefer_x11_for_blur = true
cursor_blink = true
```

Because KWin exposes blur-behind only to X11 windows, Pretty clears
`WAYLAND_DISPLAY` when `blur = true` and XWayland is available. Set
`prefer_x11_for_blur = false` (and optionally `blur = false`) to use native
Wayland for Vulkan.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| Ctrl+, | Toggle graphical settings |
| Ctrl+Shift+E | Split focused pane to the right |
| Ctrl+Shift+O | Split focused pane down |
| Ctrl+Shift+W | Close focused pane |
| Alt+Arrow | Focus a neighbouring pane |
| Ctrl+Shift+C / Ctrl+Shift+V | Copy selection / paste |
| Shift+PageUp / Shift+PageDown | Scroll history by page |
| Ctrl+Shift+Up / Ctrl+Shift+Down | Scroll history by line |
| Ctrl+Shift+Plus / Minus / 0 | Increase / decrease / reset font size |
| Shift+Mouse | Local selection even when mouse reporting is active |
| Ctrl+Click on hyperlink | Open URL |
| Ctrl+Shift+Q | Quit |

## Testing

```bash
cargo test --workspace
scripts/test-hardware.sh      # needs a real KDE/X11 display, GPU, xdotool, xclip, vttest
scripts/cross-check.sh        # Windows/macOS/Android compile checks + feature matrix
```

The hardware script captures screenshots and logs under `test-artifacts/` and
copies the evidence used by `docs/TEST-REPORT.md` into `docs/screenshots/`.
`scripts/test-hardware.sh` is the canonical reproducible hardware test entry
point; it is not a mock.

## Packaging

`scripts/build-packages.sh` produces:

- `dist/pretty-terminal_0.1.0_amd64.deb`
- `dist/pretty-terminal-0.1.0-1.fc44.x86_64.rpm`
- `dist/pretty-0.1.0-linux-x86_64.tar.gz`

All contain the binary, desktop entry, icons, man page and `TERM=pretty` terminfo
entry. The deb/rpm packages install under `/usr`; the tarball is relocatable and
ships `install.sh`. Installation verification is documented in
[docs/PACKAGING.md](docs/PACKAGING.md).

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Renderer abstraction](docs/RENDERER.md)
- [Kitty compatibility matrix](docs/KITTY-COMPAT.md)
- [Real-hardware test report](docs/TEST-REPORT.md)
- [Packaging and install verification](docs/PACKAGING.md)
- [Known issues](docs/KNOWN-ISSUES.md)
- [Cross-compile report](docs/CROSS-COMPILE.md)

## License

Pretty is licensed under MIT OR Apache-2.0. Bundled JetBrains Mono and Noto fonts
are licensed under the SIL Open Font License 1.1; their license texts are shipped
in `assets/licenses/` and in installed packages.
