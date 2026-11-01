# Known issues and limitations

This file is intentionally explicit. The project does not silently downgrade a
renderer or represent a compile-only stub as a supported platform.

## UX follow-up gaps

- **Touch was verified with synthetic XInput2/uinput injection on this host's
  `wch.cn TouchScreen` device, not by a person touching the panel.** The same
  event path is used by winit, but the physical gesture calibration (edge
  coordinates, palm rejection and two-finger momentum) should be checked
  interactively.
- **IME auto-configuration targets Fcitx5.** Native Wayland uses text-input-v3
  and Fcitx's normal candidate popup; the X11/XWayland fallback uses XIM. If the
  session uses IBus or a different XIM server, set `XMODIFIERS` explicitly. The
  candidate window is drawn by the input method; Pretty only supplies the cursor
  area/preedit overlay and commits.
- **Chinese UI needs a CJK system font.** Pretty inserts a system font into
  egui when found; a minimal container without Noto CJK/WenQuanYi will show
  missing-glyph boxes for Chinese labels. This does not affect terminal text
  fallback, which uses the renderer's font database.
- **Context menu vs. applications that capture the mouse:** plain right-click
  opens the menu only when mouse reporting is off; use Shift+right-click while
  a TUI has mouse reporting enabled.
- **File drag/drop requires an XDG drag source.** Paste converts `file://`
  clipboard URIs, but sources that expose only proprietary formats still need
  a text/plain fallback from the source application.
- **Very large mpv frames are still decoded synchronously on the UI thread.**
  The image table and GPU cache no longer grow without bound, but 4K/high-FPS
  video can still use significant CPU; a dedicated image worker is future work.
- **No plugin system.** The terminal has clear core/renderer/app boundaries,
  but no stable plugin ABI or manifest format is implemented.

## Rendering / platform

1. **GLES uses X11/XWayland on Linux.** With the current NVIDIA 580 driver, the
   Wayland EGL path does not expose a usable window presentation config
   (`EGL_NATIVE_RENDERABLE` absent; `eglCreateWindowSurface` fails). Pretty
   therefore clears `WAYLAND_DISPLAY` when the GLES backend is selected and uses
   an X11 surface. Native Wayland GLES is not usable. Vulkan works on native
   Wayland with KWin blur via `ext_background_effect_v1`.
2. **GLES requires the vendored wgpu-hal GLES patch.** Upstream wgpu would pick
   desktop OpenGL on this EGL implementation. See `docs/RENDERER.md` and
   `vendor/wgpu-hal`. This is a deliberate, documented local patch, not a silent
   fallback.
3. **Compositor blur requires KDE Plasma.** Native Wayland uses
   `ext_background_effect_manager_v1`; X11/XWayland sets
   `_KDE_NET_WM_BLUR_BEHIND_REGION` and uses a 32-bit ARGB client buffer with
   per-pixel alpha (removing `_NET_WM_WINDOW_OPACITY`). Other compositors may
   ignore the protocol/region or client alpha; transparency may then degrade to
   opaque. The situation is logged.
4. **Vulkan X11 surface still reports Opaque compositing on this driver.** Pretty
   works around that by rendering an ARGB client buffer with per-pixel alpha on
   X11 (`PRETTY_PIXEL_ALPHA` is enabled automatically when opacity < 1); the
   whole-window opacity property is then removed so KWin can blur the alpha
   background and keep text opaque. A compositor that ignores client alpha may
   fall back to an opaque appearance; KWin 6 / XWayland is verified.
5. **Metal and Direct3D are not implemented.** The `metal`/`dx` features compile
   the reserved driver skeleton; selecting them fails with a clear
   `UnsupportedBackend` error.
6. **No fallback rasterizer by default.** If no supported hardware adapter
   exists, Pretty exits with a diagnostic rather than starting a software
   renderer. `--allow-software` is an explicit testing-only escape hatch used
   solely for the Debian container smoke test on llvmpipe; it is not silent. A
   normal launch never uses it.
7. **Glyph atlas eviction is absent.** One 2048² R8 and one 1024² RGBA page are
   used. A glyph that does not fit is skipped with a warning. Unusual sessions
   with tens of thousands of unique glyph/style combinations can hit this.
8. **Kitty image caches are bounded, with an animation-specific policy.** The
   store caps the 256 newest images / 256 MiB of decoded pixels and removes old
   GPU textures every frame. Opaque RGB auto-id frames at the same anchor replace
   the previous placement (mpv's kitty VO pattern). `a=f`/`a=a` animation commands
   are still unsupported, and high-resolution video decode remains synchronous.
9. **No damage-based partial redraw.** Every presented frame rebuilds solid and
   text geometry for all panes. It is fast enough for the tested 129×37 grid
   (tab bar included) in release mode, but it is not the most efficient possible
   renderer.
10. **Color emoji font.** The bundled Noto Color Emoji is COLRv1; the swash-based
    raster path currently falls back to monochrome Noto Emoji for most emoji.
    Colour bitmap fonts/COLRv0 are supported when available.

## Terminal / kitty protocol

11. **Kitty graphics advanced features are not implemented**: zlib compression
    (`o=z`), file-path transmission, shared-memory transmission, animation
    frames, unicode placeholders and image hyperlinks. These return explicit
    protocol errors where applicable.
12. **Graphics placement is anchored to the viewport/cursor cell at command
    time.** Scrolling and resizing reposition images using cell coordinates;
    exact kitty semantics for images that scroll with historical lines are not
    fully implemented.
13. **Mouse mode 1015 (urxvt) is not tracked.** X10, 1000, 1002, 1003, 1005 and
    1006 are implemented.
14. **Kitty OSC 99 (rich notification) and pointer-shape protocol** are not
    implemented. OSC 9 and OSC 777 notifications are.
15. **Notifications are Linux/Windows desktop notifications via notify-rust.**
    macOS uses the logged fallback because its notification backend needs an
    Apple SDK; it is a reserved compile target. On a headless or D-Bus-less session delivery fails and is
    logged.
16. **OSC 52 paste is enabled by default.** `allow_osc52_paste = true` permits
    processes to request clipboard reads as well as writes. Set it to `false` if
    untrusted terminal output is a concern. Clipboard access depends on arboard
    and the desktop session; store (copy) was verified end-to-end, while the
    query/paste path is implemented but not separately scripted in the hardware
    suite.
17. **Android is compile-only.** `cargo check --features android` passes with
    winit's native-activity feature, but there is no NDK/Gradle APK build, no
    `android_main` activity wrapper, no soft keyboard and no Android clipboard
    backend (the clipboard path is a logged no-op there). Do not ship the
    Android target as a working APK.
18. **macOS/Windows are compile-checked only.** No runtime testing was possible
    on this host. The Metal/DirectX drivers are reserved interfaces and are not
    claimed to work.
19. **Tiling is in-window split panes**, not arbitrary compositor tiling. Each
    pane has its own PTY; there are no tabs or sessions yet.
20. **Selection is simple linear selection.** Semantic/block/line selection and
    double/triple-click word/line selection are not implemented.
21. **Terminal emulation delegates core VT state to `alacritty_terminal`.** A few
    very old VT edge cases and proprietary sequences may behave differently from
    kitty; run `vttest` for the tested subset.
22. **No config UI for all options.** Font family/size, colors, opacity, blur,
    padding, gap, scrollback, shell, vsync, UI accent/opacity and backend are
    graphical; some advanced fields remain file-only.
23. **The `pretty` executable name can collide with existing commands named
    `pretty`.** The packaged application is `pretty-terminal` with binary
    `/usr/bin/pretty`; check your distribution before installing.

## Test/host limitations

24. **vttest is interactive.** The hardware script records menu/cursor-test
    screenshots and survival, not a complete pass/fail score for every vttest
    subtest.
25. **esctest was not available on the test host.** The report says so rather
    than claiming it ran.
26. **The GLES screenshot was produced on the same NVIDIA GPU using an actual
    OpenGL ES 3.2 context**, not on a separate integrated GPU. The backend and
    driver are validated by the logged adapter string.
27. **Settings-open rendering is modal.** Terminal content is intentionally not
    drawn behind the settings dialog; this avoids an unreadable double exposure
    over the translucent window. Closing settings restores the terminal on the
    next frame.
28. **The settings UI is intentionally dark and compact.** It uses a custom
    egui theme (`ui_accent`, `ui_panel_opacity`); screenshots from the test
    harness may look understated on a black background.
