# Pretty v0.2.3 — file drops and desktop launch

**Run:** 0.2.3 patch release
**Date:** 2026-11-20 (Asia/Shanghai)
**Host:** Fedora 45, KDE Plasma 6.8.2 (Wayland), NVIDIA GTX 1050 /
proprietary 580.196.07; Rust stable 1.100
**Starting point:** `v0.2.2` (`fa91e30`)

This release fixes the two reported issues. It does not change the renderer or
the 0.2.1/0.2.2 fixes.

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

## 2. `.desktop` launch delay / silence / exit 1

### Root cause

Desktop launchers (KDE, GNOME, etc.) pass `XDG_ACTIVATION_TOKEN` (Wayland) or
`DESKTOP_STARTUP_ID` (X11) so the compositor can focus and raise the new
window. Winit only applies such a token when it is explicitly passed to
`WindowAttributes::with_activation_token`; Pretty never read the environment,
so the launcher's startup feedback waited for a window that was not activated
(and could report a failed launch). KDE's systemd application launcher also
sometimes starts the app without a token; in that case the window can open
without focus.

### Fix

- Read the launcher token with
  `EventLoopExtStartupNotify::read_token_from_env` and apply it with
  `with_activation_token` on Linux.
- Remove the token from the environment with `reset_activation_token_env` before
  spawning the shell, so the shell and its children do not inherit a stale
  activation token.
- If no token is present (observed with KDE's systemd application launcher),
  call `Window::request_user_attention(Informational)` after mapping. On
  Wayland this uses `xdg_activation` to request activation instead of silently
  opening behind the current window.

### Verification

- `gtk-launch` (token present): log
  `startup activation token: present`; window opens and is active.
- KDE `kstart --application` / systemd app service (token absent): log
  `startup activation token: absent` and
  `no launcher activation token; requested xdg_activation attention`; the
  window opens and becomes active.
- Five launcher runs (gtk-launch and kstart, token present/absent) completed
  without exit code 1 or a visible startup stall. The app continues to run
  normally from a terminal, where no activation token exists.

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
release (0.3.0), as stated on the v0.2.3 release page.

## Limitations

- The Wayland DnD bridge relies on `winit` 0.30 lacking data-device support;
  when winit gains native Wayland DnD this dependency should be removed.
- Physical drag-and-drop could not be automated on this host (no virtual
  pointer device); see §1.
- Windows/macOS GUI behavior is CI-built/tested but not run on physical
  hardware, unchanged from earlier releases.
