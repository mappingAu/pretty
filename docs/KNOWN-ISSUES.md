# Known issues and limitations

This file is intentionally explicit. The project does not silently downgrade a
renderer or represent a compile-only stub as a supported platform.

## Resolved in 0.2.1

- Normal window close no longer segfaults: the egui/Smithay Wayland clipboard
  worker is now dropped before the window and its `wl_display` proxies. KDE no
  longer files a crash report for a normal close.
- The custom title bar follows the terminal frame: the tab bar and settings
  button share the terminal's top line, bottom line and right margin, and all
  chrome boxes use the settings button's 6-point corner radius.
- The Wayland `app_id` and window icon are set explicitly, and the initial
  pane/window title uses the shell/program name instead of repeating
  `Pretty`.
- The idle IME cursor-area feedback loop (an empty preedit event requesting a
  new frame, at compositor refresh rate) is gone; the fullscreen title bar no
  longer repaints continuously. See `docs/TEST-REPORT-V0.2.1.md`.

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
5. **Metal (macOS) and Direct3D 12 (Windows) are not GUI-tested on physical
   hardware in this release.** Both backends are implemented through wgpu and
   compiled into the native Windows/macOS release binaries
   (`.github/workflows/build.yml`, unit-tested there). Local evidence is
   `cargo check` for the cross targets only, because this Linux host has no
   MinGW/Apple linker. Selecting a backend that was not compiled in, or that
   belongs to another OS, fails with a clear typed error. An invalid
   `--renderer`/`PRETTY_RENDERER` value now exits non-zero with the valid list
   instead of silently running `auto`; a bad `renderer` key in the config file
   is still normalised to `auto` with a warning (a config typo must not make
   the terminal unstartable).
6. **No fallback rasterizer by default.** If no supported hardware adapter
   exists, Pretty exits with a diagnostic rather than starting a software
   renderer. `--allow-software` is an explicit testing-only escape hatch for
   packaging/CI smoke tests and headless containers (for example llvmpipe/Xvfb
   or the Windows WARP adapter); it logs a warning, is never enabled by
   default, and is not used for any GPU-rendering claim in this repository. A
   normal launch never uses it.
7. **Glyph atlas eviction is absent.** One 2048² R8 and one 1024² RGBA page are
   used. A glyph that does not fit (or cannot be rasterized) is skipped, the
   atlas-full condition is logged once, and the failed glyph is remembered so it
   is not re-rasterized every frame. Unusual sessions with tens of thousands of
   unique glyph/style combinations can still lose glyphs.
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
12. **Graphics placement is anchored to a content grid line at command time.**
    The renderer adds the live scroll offset when drawing, so `display_offset`
    is no longer baked into the anchor (fixed in 0.2.0). Scrolling and resizing
    still reposition images using cell coordinates; exact kitty semantics for
    images that scroll with historical lines are not fully implemented.
13. **Mouse mode 1015 (urxvt) is not tracked.** X10, 1000, 1002, 1003, 1005 and
    1006 are implemented.
14. **Kitty OSC 99 (rich notification) and pointer-shape protocol** are not
    implemented. OSC 9 and OSC 777 notifications are.
15. **Notifications are Linux/Windows desktop notifications via notify-rust.**
    macOS uses the logged fallback because its notification backend needs an
    Apple SDK; it is a compile target only. On a headless or D-Bus-less session
    delivery fails and is logged.
16. **OSC 52 is copy-only by default.** `allow_osc52_copy = true` lets
    applications write the clipboard; `allow_osc52_paste = false` denies
    clipboard reads (an exfiltration channel for untrusted terminal output)
    unless the user opts in through the settings panel or config file. The core
    defaults to copy-only as well and the stream layer never overrides the
    configured policy. Clipboard access depends on arboard and the desktop
    session; store (copy) was verified end-to-end, while the query/paste path is
    implemented but not separately scripted in the hardware suite.
17. **Android is compile-only.** `cargo check --features android` passes with
    winit's native-activity feature, but there is no NDK/Gradle APK build, no
    `android_main` activity wrapper, no soft keyboard and no Android clipboard
    backend (the clipboard path is a logged no-op there). Do not ship the
    Android target as a working APK.
18. **macOS/Windows GUI runtime is not tested in this release.** The Linux
    development host has no MinGW/Apple linker, so local Windows/macOS checks are
    `cargo check` only. Native MSVC/Apple release binaries are built and their
    unit tests are run by `.github/workflows/build.yml`; that CI does not launch
    the GUI. Metal and Direct3D 12 are implemented backends, but this release
    does not claim GUI verification on physical Windows/Mac hardware.
19. **Tiling is in-window split panes plus tabs**, not arbitrary compositor
    tiling or kitty sessions. Each pane has its own PTY; each tab has its own
    pane layout. Kitty session files (`session`/`save_as_session`) are not
    supported. The `scrollback` setting can be changed at runtime and applies to
    all open panes (shrinking trims retained history immediately).
20. **Selection is simple linear selection.** Semantic/block/line selection and
    double/triple-click word/line selection are not implemented.
21. **Terminal emulation delegates core VT state to `alacritty_terminal`.** A few
    very old VT edge cases and proprietary sequences may behave differently from
    kitty; run `vttest` for the tested subset.
22. **Some advanced fields remain file-only.** The settings UI now covers
    font, colours, cursor opacity/text colour, dim strength, pane borders,
    padding/gap, opacity/blur, scrollback (applied live), wheel/scrollbar,
    pointer hiding, focus-follows-mouse, tab bar (edge/style/align/min tabs/
    templates/colours), window decorations/remember-size, backend, vsync,
    language, UI scale and every key binding. `font_features`, the raw
    `[shortcuts]` table, `prefer_x11_for_blur` and a few legacy keys can still
    be edited only in the file.
23. **The `pretty` executable name can collide with existing commands named
    `pretty`.** The packaged application is `pretty-terminal` with binary
    `/usr/bin/pretty`; check your distribution before installing.

## Configuration freedom and UI edges (0.2.0)

29. **Tab title templates are deliberately simple.** `tab_title_template` /
    `active_tab_title_template` support `{index}` (1-based), `{title}` and
    `{count}`. Kitty's Python-style expressions, colour markup and
    `{layout_name}`/`{num_windows}` are not interpreted; unknown placeholders
    are kept verbatim. `tab_bar_style = "slant"` imports as `fade`.
30. **Not every kitty option exists.** Cursor trails (`cursor_trail*`),
    background images, `window_margin_width`, tab activity/bell markers and
    arbitrary kitty actions are not implemented. The full supported list is in
    `docs/CONFIG.md`.
31. **Windows paths containing `%` are refused on drop/paste.** cmd.exe expands
    `%VAR%` even inside double quotes and has no escape, so Pretty logs a
    warning and does not insert the path rather than injecting an expansion
    vector. Paths with spaces and quotes use CommandLineToArgvW-style quoting.
32. **DPI coverage.** The UI is verified on this host's X11/XWayland path at
    scale factors 1.0, 1.25, 1.5 and 2.0 and with `ui_scale` 1.0/1.5
    (screenshots in `test-artifacts/ui-v020/`): settings fits the window, the
    tab bar/content measure in logical points, and IME/context-menu positions
    convert between physical pixels and points. Native Wayland fractional
    scaling, per-monitor mixed DPI and Windows/macOS display scaling were not
    exercised on physical hardware.
33. **Cursor blink timing under extreme input floods.** The blink phase is
    advanced from a 16 ms timer thread and from every presented frame, so it no
    longer starves when pointer events keep the loop away from
    `about_to_wait` (verified with a 60 fps screen capture during a synthetic
    mouse flood). Resolution is still one 16 ms tick, not a hardware timer.
34. **`focus_follows_mouse` never moves focus while a button is held**, and it
    only applies to panes of the active tab. This is intentional (selection and
    drags must not be interrupted).
35. **`hide_window_decorations` is compositor-dependent**. X11/KWin honours it
    live; some Wayland compositors ignore decoration requests from clients.
36. **`mouse_hide_wait` hides the OS pointer after inactivity.** A compositor or
    accessibility tool may override pointer visibility; moving or clicking
    always restores it.

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
