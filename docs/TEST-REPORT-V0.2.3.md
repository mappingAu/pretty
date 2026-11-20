# Pretty v0.2.3 — native Wayland file drops

**Run:** 0.2.3 patch release
**Date:** 2026-11-20 (Asia/Shanghai)
**Host:** Fedora 45, KDE Plasma 6.8.2 (Wayland), NVIDIA GTX 1050 /
proprietary 580.196.07; Rust stable 1.100
**Starting point:** `v0.2.2` (`fa91e30`)

This release fixes the reported file-drop issue. It does not change the renderer
or the 0.2.1/0.2.2 fixes.

## 1. File drag-and-drop on native Wayland

### Root cause

`winit` 0.30 has no file drag-and-drop implementation for its Wayland backend.
It emits `HoveredFile` / `DroppedFile` on X11, Windows and macOS, but not on
Wayland, so an X11/Windows drop worked while a Dolphin-to-Pretty drag on a
native Wayland session produced no event at all. Pasting a file works because it
goes through the clipboard (`text/uri-list`), not the data-device drag path.

### Fix

- Added `guth-wayland-dnd = "=0.1.3"` (MIT) as a Linux-only dependency. It
  attaches a private event queue to the window's own `wl_display` — the same
  mechanism Smithay uses for the clipboard — owns a data device per seat, and
  surfaces incoming `Hovered`, `Left` and `Dropped { paths }` events.
- `PrettyApp` stores the `FileDnd` handle, drains its events on
  `UserEvent::DndWake`, shows the same drop hint/copy cursor as the X11 path,
  and feeds every dropped path through the existing `handle_file_drop` path.
- Dropped paths are shell-quoted and inserted with a trailing space, exactly
  like the paste/XDROP paths. The new `file_drop_text` helper is unit-tested
  (`dropped_file_text_is_shell_quoted_with_a_trailing_space`).
- The Wayland DnD worker is dropped and joined in `shutdown_clipboard`, before
  the window/display proxies, so closing does not repeat the old clipboard
  shutdown crash.
- X11, Windows and macOS keep winit's own file-drop events; the new code is
  `cfg(target_os = "linux")`.

### Verification

- Desktop launch with the new binary logs
  `Wayland file drag-and-drop bridge enabled`.
- Opening and closing a native-Wayland window with the bridge attached exits 0
  and leaves no `coredumpctl` entry.
- The drop text/quoting behavior is covered by the new unit test.
- **Limitation:** this host has no Wayland virtual-pointer/EIS input permission,
  so an automated pointer drag from Dolphin could not be scripted. The attach
  path, worker lifecycle, event loop wakeup and insertion helper are exercised;
  the physical drag itself was left for the reporter to confirm with the same
  Dolphin setup.

## Tests

```
cargo test --workspace --locked
46 app + 61 core unit + 9 audit + 2 PTY (+1 ignored) + 4 QA + 15 renderer
= 137 passed, 1 ignored, 0 failed
```

New/changed tests:

- `app::tests::dropped_file_text_is_shell_quoted_with_a_trailing_space`
- existing `shell_quote_path` tests cover the same quoting rules.

## Release note

**Issue #1 (Windows GLES crash) is not fixed in 0.2.3.** GLES remains disabled
on Windows; the investigation and fix are deferred to the next **minor**
release, as stated on the v0.2.3 release page.

## Limitations

- The Wayland DnD bridge relies on `winit` 0.30 lacking data-device support;
  when winit gains native Wayland DnD this dependency should be removed.
- Physical drag-and-drop could not be automated on this host (no virtual
  pointer device); see §1.
- Windows/macOS GUI behavior is CI-built/tested but not run on physical
  hardware, unchanged from earlier releases.
