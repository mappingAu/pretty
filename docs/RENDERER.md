# Pretty renderer abstraction

Pretty renders through `wgpu` and treats **Vulkan** and **OpenGL ES 3.0+** as
real backends. Metal and Direct3D are reserved interface skeletons: their
constructors return `UnsupportedBackend`, and the runtime refuses them before a
device is created.

## 1. Source layout

```
crates/pretty-renderer/src/
├── backend.rs   RendererKind, BackendDriver, VulkanDriver, GlesDriver,
│                MetalDriver/DxDriver reserved cfg-gated skeletons
├── context.rs   wgpu instance, display handle, adapter selection, device,
│                surface caps, surface configuration/reconfiguration
├── gpu.rs       global uniform, solid-quad pipeline, image pipeline/textures
├── text.rs      fontdb fallback discovery, rustybuzz shaping, swash
│                rasterization, R8/RGBA glyph atlases, glyph pipeline
├── renderer.rs  frame composition, pane backgrounds, decorations, cursor,
│                selection, scrollbar, image z-order, egui pass, present
└── theme.rs     colour resolution, palettes, opacity, font/atlas options
```

The crate depends only on `pretty-core` types (`Frame`, `Rect`, `FrameImage`,
colour specs). It does not know about PTYs, settings, or pane trees.

## 2. Backend selection

`RendererKind` is `Auto | Vulkan | Gles | Metal | Dx`. Resolution order is:

1. `--renderer`;
2. `PRETTY_RENDERER`;
3. `config.renderer`;
4. `auto`.

`auto` probes Vulkan then GLES. `GpuContext::new` creates a wgpu instance with
the union of compiled backends, supplies winit's owned display handle (required
by the GL/EGL path), creates the surface with the window handle, enumerates
adapters and chooses one that:

- matches the candidate backend;
- is not `DeviceType::Cpu`;
- passes the driver's `accept_adapter` policy;
- can present to this surface.

Discrete GPUs rank above integrated GPUs. The exact error from every failed
candidate is retained in the final `NoAdapter` diagnostic. No CPU renderer is
started if selection fails.

The driver trait is intentionally small:

```rust
pub trait BackendDriver {
    fn kind(&self) -> RendererKind;
    fn wgpu_backends(&self) -> wgpu::Backends;
    fn accept_adapter(&self, info: &AdapterInfo) -> Result<(), String>;
    fn choose_alpha_mode(&self, caps: &SurfaceCapabilities, transparent: bool)
        -> wgpu::CompositeAlphaMode;
}
```

`VulkanDriver` requires a Vulkan adapter. `GlesDriver` requires the adapter
driver string to identify **OpenGL ES** or ANGLE; a desktop OpenGL context is
rejected. This is what prevents the backend from silently being "GL 3.3" rather
than GLES.

### 2.1 Local wgpu-hal patch

Upstream wgpu's EGL code prefers desktop OpenGL whenever `EGL_CLIENT_APIS`
advertises `OpenGL`. That would make `PRETTY_RENDERER=gles` select a desktop GL
3.3 context. `vendor/wgpu-hal` is the upstream 30.0.1 crate with two minimal
GLES-selection hooks:

- `PRETTY_GL_FORCE_GLES` hides the desktop OpenGL client API during context
  selection, causing a real `EGL_OPENGL_ES_API` context to be created;
- `PRETTY_GL_ALLOW_NON_NATIVE` allows the presentation config tier when the EGL
  implementation does not expose `EGL_NATIVE_RENDERABLE` even for a window
  config.

The application sets both variables on Linux before creating the event loop.
The selected driver string is logged and validated. This patch is part of the
delivered source and is reproducible by `cargo build --release`.

## 3. Surface and swapchain

`GpuContext` owns the `Surface`, device, queue, adapter, selected
`SurfaceConfiguration` and the physical size. On creation it:

1. creates the instance (with display handle);
2. creates the surface;
3. chooses the adapter as above;
4. prefers `Bgra8UnormSrgb`/`Rgba8UnormSrgb`;
5. selects `Fifo` for vsync, otherwise Mailbox/Immediate when supported;
6. selects a non-opaque composite mode when requested and supported;
7. configures the surface and logs format/alpha/present mode.

`reconfigure(width, height)` is idempotent and called from winit's
`Resized`, `ScaleFactorChanged`, and on `CurrentSurfaceTexture::Lost` or
`Outdated`. The app never touches a raw swapchain texture. `Suboptimal` frames
are presented and the swapchain is reconfigured on the next resize; timeout or
occlusion skips the frame; validation errors are reported, not ignored.

On this NVIDIA X11 host the Vulkan Xlib surface advertises only `Opaque`
compositing, but the 32-bit ARGB X11 pixmap still carries the shader alpha.
Pretty therefore uses **per-pixel client alpha** on X11 when opacity < 1:
background alpha = configured opacity, text/image alpha = 1, transparent clear.
It removes `_NET_WM_WINDOW_OPACITY` and keeps
`_KDE_NET_WM_BLUR_BEHIND_REGION`, so KWin blurs the alpha background while
foreground text remains opaque. This path works for Vulkan and GLES. Native
Wayland surfaces use the native PreMultiplied alpha path instead.

## 4. Pipelines and rendering order

All geometry is built in physical pixels, then converted to NDC in WGSL using a
16-byte global uniform. The renderer uses:

| Pipeline | Input | Use |
|---|---|---|
| solid | rect + premultiplied colour | pane backgrounds, cell backgrounds, selection, underline, cursor block, borders, scrollbar |
| glyph R8 | rect + tint + atlas UV | monochrome glyphs |
| glyph RGBA | rect + tint + atlas UV | colour glyph/emoji bitmap atlases |
| image | rect + uv + tint | kitty graphics textures |
| egui | egui-wgpu | settings overlay |

One frame is submitted in one command buffer:

```
clear (transparent or theme background)
  → pane/cell backgrounds and decorations (solid)
  → kitty images with z-index < 0
  → text
  → kitty images with z-index >= 0
  → post-text decorations (cursor bar/underline, borders, scrollbar)
  → egui settings pass (LoadOp::Load)
  → present
```

Draw batches are merged: solid quads share one instance buffer; glyphs are split
into R8/RGBA instance buffers; images are grouped by texture bind group. Pane
clipping for images is done in CPU geometry (UV intersection), so images never
spill into a neighbour pane. Text and backgrounds are already within pane
rectangles; no scissor state is leaked to egui.

The renderer draws all PTY output and all panes every frame; it does not use
damage-based partial redraw yet. The frame benchmark measures this full path.

## 5. Text and glyph atlas

- Primary faces: bundled JetBrains Mono Regular/Bold/Italic/BoldItalic. A
  different configured family is resolved through `fontdb` with regular/bold/
  italic/bold-italic queries and falls back to JetBrains Mono with a logged
  warning.
- Emoji: bundled Noto Emoji and Noto Color Emoji. Noto Emoji is used as the
  monochrome fallback; the atlas supports RGBA color bitmaps/COLR glyphs when
  the font/driver provides them.
- Fallback: preloaded CJK/system families plus lazy `fontdb` cmap scanning.
- Shaping: rustybuzz per same-font/same-style run; `liga`/`calt` are enabled by
  default and every OpenType feature in `font_features` is parsed and passed.
  `ligatures = false` explicitly disables `liga` and `calt`.
- Layout: each glyph's cluster maps back to its terminal cell, so ligature
  glyphs span the correct grid cells while the grid remains fixed.
- Raster: swash scales outlines, colour outlines, alpha/colour bitmaps. Masks go
  to an R8 atlas; RGBA bitmaps go to an RGBA atlas. Atlases are shelf-packed,
  uploaded through a full dirty-page upload (`queue.write_texture`), and cached
  by `(face, glyph id)`.
- Metrics: font metrics determine cell width/height/baseline; the PTY is
  resized when the resulting column/row count changes, not on every repaint.
- Colour emoji are scaled into the cell box; monochrome glyphs use font
  placement and baseline.

Known atlas limits: one 2048² R8 page and one 1024² RGBA page. If an atlas is
full, a warning is logged and that glyph is skipped; there is no eviction yet.
See `KNOWN-ISSUES.md`.

## 6. Settings UI and theming

`pretty-app` owns an `egui_wgpu::Renderer` pass. It is drawn every frame after
the terminal; when the settings dialog is closed only the small **Settings**
button is visible. The dialog has General/Appearance/Rendering/Keyboard/About
tabs, live font/colour/opacity/padding/gap/scrollback controls, renderer choice
with a restart notice, and diagnostics.

The settings overlay style is configurable independently: `ui_accent` and
`ui_panel_opacity` are persisted in the same TOML file and applied through
`egui::Visuals`. The terminal theme is separate.

## 7. Adding another backend

1. Add a `RendererKind` variant and a `BackendDriver` implementation. For a
   reserved platform, keep it `#[cfg(any(target_os = "...", feature = "..."))]`
   and return a typed unsupported error from `driver_for`.
2. Add a Cargo feature and gate the wgpu backend feature.
3. Extend `RendererKind::candidates()` and the CLI/env parser.
4. Add surface/alpha policy to the driver. Reuse `GpuContext`, the quad
   pipelines and the text/image renderers; do not duplicate device code.
5. Add `--print-info` output, startup logging, and a hardware test entry.
6. Document the support level in `KITTY-COMPAT.md`/`KNOWN-ISSUES.md`; never mark
   a compile-only stub as working.

## 8. Diagnostics

`pretty --print-info` prints compiled backends and adapters. Startup logs:

- requested and selected renderer;
- wgpu backend, adapter name, type, vendor/device id, driver and driver info;
- surface format, alpha mode and present mode;
- font family/cell metrics;
- kitty keyboard flags and terminal modes on the first frame;
- swapchain reconfiguration;
- `BENCHMARK RESULT ...` in benchmark mode.

All errors are explicit; the only fallback is `auto` trying the next **hardware**
backend. GLES desktop-GL detection is a hard failure.
