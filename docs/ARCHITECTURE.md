# Pretty — Architecture Design

Status: **baseline design, implemented by this repository**. This document describes the
architecture that the delivered source actually implements. Where a design element is a
reserved platform extension (Metal, DirectX) it is called out explicitly.

As-built versions: Rust 1.99.1, wgpu 30.0.1 (with a tiny vendored wgpu-hal GLES
selection patch), winit 0.30.13, alacritty_terminal 0.26.0, egui 0.37.0. The
renderer/terminal abstraction survived contact with real hardware: the Vulkan
and GLES paths share all higher-level code, and Metal/DirectX are explicit
unsupported stubs.

## 1. Goals and non-goals

Pretty is a GPU-accelerated terminal emulator whose compatibility target is the kitty
terminal protocol. The first-class rendering APIs are **Vulkan** and **OpenGL ES 3.0+**
through `wgpu`. Metal and Direct3D are reserved interfaces: selecting them produces an
explicit "not implemented" error and never silently falls back.

Non-goals for this release: a complete replacement for every kitty extension, a
multiplexer/server architecture, and shipping a software rasterizer as a normal rendering
path. The terminal must remain usable without a GPU only through explicit diagnostics, not
through a hidden CPU renderer.

## 2. Workspace layout

```
pretty/
├── Cargo.toml                    # workspace, shared release profile
├── crates/
│   ├── pretty-core/              # terminal state, PTY plumbing, kitty extensions
│   │   ├── src/lib.rs
│   │   ├── src/terminal.rs       # wrapper around alacritty_terminal::Term
│   │   ├── src/events.rs         # external terminal events (title, bell, clipboard)
│   │   ├── src/graphics.rs       # kitty graphics parser, image store, placements
│   │   ├── src/stream.rs         # streaming APC/OSC interceptor in front of VTE
│   │   ├── src/input.rs          # kitty keyboard protocol + mouse encoders
│   │   ├── src/notify.rs         # OSC 9 / OSC 777 notification handling
│   │   ├── src/pty.rs            # portable-pty session, resize, reader thread
│   │   └── src/layout.rs         # tiling layout tree (backend-independent)
│   ├── pretty-renderer/          # GPU abstraction layer and all wgpu code
│   │   ├── src/lib.rs
│   │   ├── src/backend.rs        # BackendDriver trait + Vulkan/GLES drivers,
│   │   │                         # cfg-gated Metal/DX reserved skeletons
│   │   ├── src/context.rs        # instance/adapter/device/surface/swapchain
│   │   ├── src/gpu.rs            # solid/image pipelines, quad batching, textures
│   │   ├── src/text.rs           # font selection, shaping, glyph atlases, glyph pipelines
│   │   ├── src/renderer.rs       # frame composition, egui pass, present
│   │   └── src/theme.rs          # render colours and theme
│   └── pretty-app/               # binary: winit event loop, windows, panes, UI
│       ├── src/main.rs           # CLI, logging, GLES/backend env setup
│       ├── src/app.rs            # winit app, PTYs, panes, key/mouse routing
│       ├── src/config.rs         # TOML configuration and paths
│       ├── src/platform/x11.rs   # KWin opacity/blur properties
│       ├── src/platform/other.rs # explicit no-op fallback for non-X11
│       └── src/settings.rs       # custom egui settings UI
├── assets/                       # bundled fonts, icon, demo scripts
├── packaging/                    # deb/, rpm spec, tar layout, terminfo, desktop file
├── scripts/                      # build, test, benchmark, package, cross-check helpers
├── tests/                        # integration checks driven by scripts
└── docs/                         # this design + compat matrix, test report, etc.
```

Dependency direction is strictly one-way:

```
pretty-app ──> pretty-renderer ──> (wgpu, font stack)
     │
     └────────> pretty-core ─────> alacritty_terminal, vte, portable-pty
```

`pretty-core` has no dependency on `wgpu`, `winit`, or font code. `pretty-renderer` has no
dependency on window management beyond raw surface creation; it accepts a surface and
pane drawing data. The app is the composition root.

## 3. Terminal core

### 3.1 Why reuse `alacritty_terminal` for the state machine

Writing a correct VT parser and grid from scratch would add a large conformance risk without
adding product value. Pretty uses `alacritty_terminal 0.26` (Apache-2.0) as the **terminal
state engine**: grid, scrollback, wide cells, combining characters, selectable alt screen,
SGR, OSC 8 hyperlinks, OSC 52 clipboard events, DEC cursor styles, mouse mode tracking, and
the kitty keyboard mode stack (push/pop/query/replace/difference). This is a library, not a
process dependency: Pretty owns the window, PTY, GPU, input policy, and graphics protocol.

The parts required by this project that `alacritty_terminal` does **not** provide are
implemented here:

* kitty graphics protocol (APC `G`) parsing, image storage, placement, deletion and
  responses;
* a streaming interceptor that delivers APC and notification OSC data without corrupting
  the VTE stream and handles sequences split across read boundaries;
* kitty keyboard **encoding** (alacritty exposes the negotiated flag set, not the outgoing
  encoding policy);
* mouse report encoding and Pretty's focus/shortcut policy;
* tiling panes, scrollback interaction, selection presentation, and the GPU renderer.

### 3.2 Data flow

```
                  ┌──────────────────────────── winit ────────────────────────────┐
                  │ keyboard / mouse / resize / scale / focus / redraw requests     │
                  └───────────────┬───────────────────────────────┬───────────────┘
                                  │                               │
                     Shortcut / kitty encoder                resize events
                                  │                               │
┌───────────┐   bytes   ┌─────────▼─────────┐   VTE bytes  ┌──────▼──────┐
│ PTY child │ ────────> │  StreamInterceptor│ ───────────> │ Term (grid) │
│ (shell)   │ <──────── │  APC/OSC scanner  │              │ alacritty   │
└───────────┘  input    └──────┬────────────┘              └──────┬──────┘
                               │ graphics / notifications          │ renderable_content()
                               ▼                                   ▼
                      ┌─────────────────┐                 ┌──────────────────┐
                      │ GraphicsStore   │                 │ TilingLayout      │
                      │ Images + place  │                 │ pane viewports    │
                      └────────┬────────┘                 └────────┬─────────┘
                               │ image commands / textures          │ cells, cursor,
                               ▼                                    ▼ decorations
                      ┌──────────────────────────────────────────────────────┐
                      │              pretty-renderer                         │
                      │  FontManager → shape → GlyphAtlas → quad batches     │
                      │  ImageTextures · clear/alpha · optional blur layer   │
                      │  wgpu Device/Queue/Surface; Vulkan or GLES driver    │
                      └──────────────────────┬───────────────────────────────┘
                                             ▼
                                     X11 / Wayland / ...
```

Reader thread → bounded channel → winit proxy wakeup; input writes go directly to the PTY
master under a mutex. The UI thread never blocks on PTY reads. Terminal state is only
mutated on the UI thread, so no render locks are needed.

### 3.3 Kitty graphics protocol

The stream interceptor recognizes `ESC _ G ... ESC \` (APC), including chunked sequences.
It removes the APC from the VTE stream (the underlying parser does not implement APC), then
hands the payload to `GraphicsStore`. Supported attributes:

* `a=t` / `a=T` transmit and transmit+display, `a=p` place, `a=d` delete, `a=q` query;
* `f=100` PNG (decoded with `image`), `f=24` RGB, `f=32` RGBA;
* `m=1` chunked base64 payloads, with a bounded accumulator;
* `i` image id, `p` placement id, `s`/`v` dimensions;
* `c`/`r` cell span, `x`/`y` pixel offset, `z` z-index, `C=1` (do not move cursor);
* response codes `OK` / `EINVAL` / `ENOENT` / `ENOSPC` written to the PTY unless `q=2`.

Images are associated with the pane's terminal instance. Placements are resolved in cell
coordinates at render time, so they move correctly when the pane is resized or scrolled.
z-index `< 0` is drawn below text; `>= 0` is drawn above text. Out-of-order/malformed
payloads produce an error response, never a panic. Animation, shared-memory transmission and
file-path transmission are explicitly unsupported and are listed in `KITTY-COMPAT.md`.

### 3.4 Input encoding

Keyboard events are translated by `pretty-core::input` using the flags exposed by
`TermMode::KITTY_KEYBOARD_PROTOCOL`:

| flag | value | behavior in Pretty |
|---|---|---|
| disambiguate | 1 | modified/special keys use CSI-u where required |
| report event types | 2 | press/repeat/release encoded as `;mod:eventu` |
| alternate keys | 4 | shifted/base-layout key reported |
| all keys as escape | 8 | printable keys also use CSI-u |
| associated text | 16 | text codepoints appended after `;` |

When kitty flags are not active, UTF-8 text and traditional xterm sequences are emitted,
including application-cursor and application-keypad modes. Mouse reporting supports X10,
normal (1000), button-motion (1002) and any-motion (1003) tracking with UTF-8 (1005), SGR
(1006) and urxvt (1015) encodings. Shift overrides mouse reporting for local selection.

### 3.5 Tiling and panes

`pretty-core::layout` defines an immutable tree:

```rust
pub enum Layout {
    Leaf(PaneId),
    Split { axis: Axis, ratio: f32, first: Box<Layout>, second: Box<Layout> },
}
```

The app maps the tree to pixel rectangles (configured gap, 1 px focused border), then calls
`Term::resize`/PTY `resize` for every pane in the same frame. Each pane owns its own PTY,
terminal, graphics store, selection and scroll offset. Splitting/closing terminates the
corresponding child process. The terminal core does not know about egui, winit, or pixels.
Panes are in-window splits; there is no tab/session model yet.

### 3.6 Scrollback and viewport

Alacritty's history grid stores scrollback (`scrolling_history` configurable, default
10,000). Wheel and keyboard scroll call `Term::scroll_display`; any new output while the view
is scrolled follows the standard "stay in history" behavior. The renderer derives a scrollbar
thumb from `display_offset / history_size`. Resizing calls `Term::resize` and PTY resize with
the cell metrics; the renderer reconfigures the swapchain before using new dimensions.

## 4. Renderer abstraction layer

### 4.1 Selection and probing

`RendererPreference` is resolved in this order:

1. `--renderer <name>` command-line flag;
2. `PRETTY_RENDERER` environment variable (`vulkan`, `gles`, `metal`, `dx`, `auto`);
3. config file (`renderer = "..."`);
4. `auto`: Vulkan, then GLES.

Each candidate is probed by creating a `wgpu` instance restricted to that backend,
enumerating adapters and matching a surface-compatible adapter. A candidate that fails
validation is recorded with the precise error and the next candidate is tried. If all
candidates fail, startup fails with a diagnostic; there is no CPU raster fallback. The chosen
backend, adapter name, driver, type, backend API, swapchain format, alpha mode and downlevel
flags are logged at startup and exposed by `pretty --print-info`.

### 4.2 Backend driver trait

```rust
pub trait BackendDriver: Send + Sync {
    fn kind(&self) -> RendererKind;
    fn wgpu_backend(&self) -> wgpu::Backends;
    /// Can this adapter be used for this driver? (e.g. reject CPU adapters for auto)
    fn accept_adapter(&self, info: &AdapterInfo) -> Result<(), String>;
    /// Pick a composite alpha mode that matches the requested window opacity.
    fn choose_alpha_mode(&self, caps: &SurfaceCapabilities, transparent: bool)
        -> wgpu::CompositeAlphaMode;
}
```

Concrete drivers:

* `VulkanDriver` — `wgpu::Backends::VULKAN`, real Vulkan required;
* `GlesDriver` — `wgpu::Backends::GL`, requires an EGL/GLES adapter;
* `MetalDriver` — feature `metal` (and macOS); returns
  `RendererError::UnsupportedBackend("Metal backend is a reserved interface ...")`;
* `DxDriver` — feature `dx` (and Windows); same explicit unsupported error.

The driver abstraction is deliberately narrow: wgpu already provides the resource
abstraction. This keeps the codec (Vulkan/GLES) differences to instance creation and
adapter/alpha policy instead of duplicating device code. Adding a new backend means adding a
driver implementation, a feature flag, and a section in `docs/RENDERER.md`; the renderer
never contains `cfg` checks in the hot path.

### 4.3 Surface and swapchain

`GpuContext::new(window)`:

1. builds an instance for the driver;
2. creates the surface (wgpu safe window surface);
3. requests an adapter compatible with the surface;
4. requests a device with the adapter's default limits and empty extra features;
5. queries `SurfaceCapabilities`, selects sRGB format and a supported present mode
   (Mailbox if available, else Fifo), selects alpha mode for transparency;
6. configures the surface and logs the resulting configuration.

On `Resize`, `ScaleFactorChanged`, `SurfaceError::Lost`, `SurfaceError::Outdated` and
`SurfaceError::Timeout`, the app calls `reconfigure(width, height)` before drawing. Surface
configuration state is owned by `GpuContext`; the app never touches raw swapchain images.

### 4.4 Pipelines, batching, atlas

The renderer uses one instanced quad pipeline with two shaders (solid and glyph texture),
plus an image pipeline. Each frame builds CPU-side vertex/instance buffers:

* backgrounds and decorations: solid quads (cell backgrounds, cursor, underlines,
  hyperlinks, selection, pane borders, dividers);
* glyphs: textured quads backed by a single R8/RGBA glyph atlas; tint from cell foreground;
* images: RGBA textures, one bind group per image, drawn in z-order per pane;
* egui settings overlay: egui-wgpu renderer sharing the same device/queue.

Draw batches are merged by changing only the texture bind group; pane clipping uses a
scissor rectangle so no per-pane surfaces are required. The font atlas is a shelf packer
(2048×2048 initially, grows to 4096×4096 in one step if supported). Glyphs are rasterized at
physical pixel size and re-rasterized on DPI/font-size changes. Cell dimensions come from
font metrics, and the PTY is resized when the grid changes, not on every repaint.

### 4.5 Text, ligatures, fallback, emoji

The font stack is:

1. bundled **JetBrains Mono** (Regular/Bold/Italic/BoldItalic) as the primary monospace face;
2. bundled **Noto Emoji** for emoji fallback;
3. system fonts discovered through `fontdb` for CJK and other fallback.

Shaping uses `rustybuzz` with `liga`/`calt` enabled by default. Same-style runs of a line are
shaped together so programming ligatures such as `=>`, `!=`, `->` produce a single glyph.
Each glyph's cluster maps back to the cell column, preserving the fixed terminal grid even
when a ligature spans multiple cells. Fallback glyphs are shaped per cluster. Emoji and
symbol glyphs are selected by font cmap coverage and rasterized through `swash`, including
color bitmap/COLR data when the font provides it; grayscale masks are tinted.

Configurable OpenType features (`font_features`) are passed to rustybuzz. The renderer never
depends on a particular UI framework; the font system is owned by `pretty-renderer`.

As built, glyphs live in one 2048² R8 atlas plus one 1024² RGBA atlas with a shelf allocator
and per-`(face, glyph)` cache. There is no eviction yet; overflow is logged and the glyph is
skipped. This limitation is recorded in KNOWN-ISSUES.md.

### 4.6 Transparency and blur

The window is created with transparency support. The surface alpha mode is selected from
`SurfaceCapabilities` and the configured background alpha. All shader color output is
premultiplied when the surface uses a premultiplied mode. A configurable alpha (default
`0.88`) makes the desktop visible through the terminal.

On KDE Plasma under X11/XWayland, the app sets
`_KDE_NET_WM_BLUR_BEHIND_REGION` (a rectangle covering the client area) and
`_NET_WM_WINDOW_OPACITY` via x11rb (isolated in `pretty-app::platform::x11`).
KWin then blurs the wallpaper/windows behind the translucent window, which is
observable in the captured desktop composition. This NVIDIA X11 Vulkan surface
only exposes `Opaque` composite alpha, so transparency is supplied by the
compositor window property rather than per-pixel swapchain alpha. The window is
created with a 32-bit ARGB visual when opacity/blur are requested. Other
compositors may ignore the properties; Pretty logs the failure and never fakes
blur in its own renderer.

### 4.7 Configuration UI

A custom egui overlay provides General / Appearance / Keyboard / Rendering pages. It is
rendered into the same swapchain after terminal content. Changing font size, theme colors,
opacity, cursor shape, tiling gaps or scrollback applies live; renderer backend changes are
saved and require a restart (clearly displayed). The overlay theme (accent, panel opacity,
corner radius) is itself configurable, satisfying the custom configuration-UI requirement.
Settings persist to the platform config directory as TOML.

## 5. Platform abstraction and packaging

* All Linux/X11-specific window-property code is in `platform/x11.rs`, gated by
  `#[cfg(all(unix, not(target_os = "macos")))]`; non-X11 builds use a no-op implementation
  with logging.
* Windows uses ConPTY through `portable-pty`; no `/dev`, `/proc` or POSIX path is used in
  cross-platform modules.
* macOS uses the Unix PTY path and would use the Metal driver when implemented. The Metal
  driver is explicitly a reserved interface and is not claimed as working.
* Android has a compile-only target check today. The cross-check reports the
  real result; there is no `android-activity` event-loop entry, NDK/SDK build, storage
  integration or soft-keyboard bridge, so no APK is claimed. A production Android port
  needs all of those (documented in KNOWN-ISSUES).
* Packages: deb/rpm/tar.gz contain the binary, desktop file, icon, man page, terminfo,
  bundled OFL fonts and licenses. Build scripts generate all three from one staged tree.

## 6. Observability and failure policy

* `env_logger` to stderr (test harnesses capture it); startup and benchmark results use
  `log::info`, protocol internals use debug/trace.
* Startup log includes version, backend requested/resolved, wgpu backend, adapter,
  driver, device type, surface format, alpha mode, present mode, font family, kitty feature
  level, TERM and PTY size.
* `--print-info` prints the same machine-readable report and exits; test scripts use it.
* Unsupported features fail loudly: reserved backends, unavailable blur, missing font,
  unsupported graphics command. They are not silently ignored.
* `docs/KNOWN-ISSUES.md` records every known limitation, including vttest items and
  compositor-specific behavior.
* No `todo!()` / `unimplemented!()` on the default path. Reserved backend skeletons are
  compiled under `#[cfg(any(target_os = "macos", feature = "metal"))]` /
  `#[cfg(any(target_os = "windows", feature = "dx"))]` and every attempt to select them
  returns a typed `UnsupportedBackend` error before a GPU device is created.

## 7. Testing strategy

| Level | Tool | What it proves |
|---|---|---|
| unit | `cargo test` | graphics parser, keyboard encoder, layout math, config merge |
| integration | shell + Python probes | real PTY, kitty keyboard handshake, OSC 52, title, notifications |
| rendering | host screenshots | Vulkan/GLES text, emoji, ligatures, graphics, transparency/blur, tiling |
| stress | `scripts/test-hardware.sh` | resize/scroll loop, swapchain recreate, 80×24 benchmark |
| conformance | vttest / esctest if present | recorded per-test results and visual evidence |
| packaging | rpm/dpkg/podman | install, terminfo, desktop entry, launch and screenshot |
| cross | `scripts/cross-check.sh` | `cargo check` for Windows, macOS, Android targets with explicit report |

## 8. Milestones

1. **M1 — core**: workspace, config, CLI/logging, PTY, alacritty state, key/mouse input,
   stream interceptor, kitty graphics store, settings persistence.
2. **M2 — renderer**: wgpu context/backends, font atlas/ligatures, terminal quads, images,
   resize/swapchain, egui layer, transparency.
3. **M3 — UX**: tiling panes, selection/clipboard, scrollbar, shortcuts, KDE blur, config UI.
4. **M4 — packaging**: deb/rpm/tar.gz, desktop/icon/man/terminfo, install validation.
5. **M5 — hardware evidence**: Vulkan + GLES screenshots, resize/scroll stress, benchmark,
   vttest, test report and compatibility matrix.
6. **M6 — cross checks and hardening**: Windows/macOS/Android checks, known issues, final
   acceptance.

Each milestone is validated by real commands whose output is kept in `docs/`.
The delivered state (M1–M6 except runtime testing of reserved platforms) is
evidenced by `docs/TEST-REPORT.md`, `docs/KITTY-COMPAT.md`, the captured
screenshots and `scripts/test-hardware.sh`. The GLES backend required a
deliberate local wgpu-hal patch to obtain a true GLES context; that is part of
the delivered architecture, not a test-only workaround.
