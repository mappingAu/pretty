Pretty for Windows (portable)
=============================

Pretty         : hardware-accelerated, kitty-compatible terminal emulator
This build     : Windows x86_64, Direct3D 12 backend compiled in
Runtime needs  : Windows 10 1903+ (Direct3D 12 capable GPU and driver)

Quick start
-----------
1. Extract this folder anywhere (no installer, no registry changes).
2. Run `pretty.exe`.

The binary is portable: it does not write next to itself. Settings are stored
in the per-user roaming config directory:

    %APPDATA%\pretty\pretty\config\config.toml

Use `pretty.exe --config <path>` or the PRETTY_CONFIG environment variable to
override it.

Backends
--------
`auto` (the default) prefers Direct3D 12, then Vulkan, then OpenGL ES:

    pretty.exe --renderer auto
    pretty.exe --renderer dx
    pretty.exe --renderer vulkan
    pretty.exe --print-info

`--print-info` lists the backend features compiled into this binary, the
backend order for `auto` and every adapter that can be enumerated.
Selecting a backend that is not compiled in, or that cannot exist on Windows,
fails with an explicit error; Pretty never falls back to a CPU rasteriser.

Known limitations
-----------------
* The archive is unsigned. Windows SmartScreen may warn on first launch
  ("More info" -> "Run anyway"). Verify SHA256SUMS-windows.txt before running.
* Blur-behind and per-pixel-alpha compositing are KWin/X11 and Wayland
  features; on Windows, opacity uses the Win32 composition alpha mode that the
  GPU driver reports. See docs/KNOWN-ISSUES.md.
* This build is produced by the native GitHub Actions Windows runner
  (.github/workflows/build.yml). It is compile/build tested there; the GUI was
  not executed on a physical Windows machine as part of this release.

Docs and licence
----------------
docs/README.md, docs/RENDERER.md, docs/CROSS-COMPILE.md,
docs/CROSS-PLATFORM.md, docs/KNOWN-ISSUES.md
LICENSE: GPL-3.0-or-later. Bundled fonts remain under the SIL OFL 1.1.
