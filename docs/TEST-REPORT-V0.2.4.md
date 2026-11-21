# Pretty v0.2.4 — Terminal=true desktop launchers

**Run:** 0.2.4 patch release
**Date:** 2026-11-20 (Asia/Shanghai)
**Host:** Fedora 45, KDE Plasma 6.8.2 (Wayland); Rust stable 1.100
**Starting point:** `v0.2.3` (`644d05d`)

This release fixes the `.desktop` launch problem from the report: an entry
with `Terminal=true`.

## Root cause

The reported shortcut is effectively:

```ini
[Desktop Entry]
Type=Application
Exec=...
Terminal=true
```

KDE resolves `Terminal=true` by launching the **default terminal**, not the
Exec program directly. On this host the default terminal is Pretty, so KDE
runs:

```text
/usr/bin/pretty -e <Exec...>
```

Pretty's CLI had a terminal-emulator-shaped `-e` option, but it consumed
**one** argument and then continued parsing the command's own arguments as
Pretty options. For example:

```text
pretty -e sh -c 'sleep 1'
```

became “run command `sh`”, then `-c` was rejected as an unknown Pretty option
(exit code 1). KDE also saw a startup delay while the outer terminal waited.
When `Exec` was Pretty itself, the old parser launched a second Pretty inside
the first (nested window).

## Fix

- `-e` and the historical `-x` now consume **everything to the end of the
  command line** as the command, exactly like terminal emulators. The command's
  flags are no longer parsed as Pretty options.
- Each argv element is shell-quoted when there is more than one; a single
  `--command` argument is kept as a shell string for compatibility.
- If the command is Pretty itself (same executable path, or the same file name
  for a PATH lookup), the nested self-launch is ignored: the outer window is
  the terminal the desktop requested. Remaining arguments such as `--title` or
  `--config` are still parsed as Pretty options, so the visible window keeps
  the intended settings.
- The same handling applies to `--command`; `--command` remains available for
  explicit shell strings.

## New tabs: no inherited `-e` command

`-e`/`-x` argv belongs to the window that the desktop launched. `new_tab` and
`split_pane` ask `PaneCommand::for_new_pane()`: `Argv` becomes `LoginShell`, so
a shortcut that starts `htop` still opens a normal shell in a new tab. A
`--command` shell string keeps its existing behavior. The initial pane uses the
full `-e` argv directly (through `pane.pty`), not a shell wrapper, so arguments
such as `-c` reach the program exactly once.

## Window title follows the running program

The title was only updated by OSC 0/2 or the spawn-time shell name, so a shell
that never emits a title stayed "bash" even while htop/vim was running. Pane
titles now sample the PTY foreground process group with `tcgetpgrp()` and read
`/proc/<pgid>/comm` on Linux (other Unix targets keep the fallback). Sampling
runs on the existing 16 ms blink tick at most every 500 ms; it does not add a
new timer or repaint an idle window. An explicit OSC title, and `--title`, win
over the sampled name; clearing the OSC title returns to it. New tabs start
from the login shell name, and `-e htop` starts from `htop`, not `bash`.

## GLES on Wayland is announced

GLES cannot present in a native Wayland window on this driver, so Pretty runs
the window through X11/XWayland. That fallback is no longer silent:

- **Startup:** when the configured backend is GLES on a Wayland session, the
  window shows a dismissible modal ("GLES runs through XWayland") with the
  session (`Wayland`) and the actual window (`X11 (XWayland)`), plus a note
  that IME, scaling and blur differ from native Wayland. `--renderer vulkan`
  and `auto` show nothing.
- **Settings → Rendering:** selecting GLES on a Wayland session opens the same
  explanation with "Use GLES anyway" / "Cancel". Cancelling rolls the draft
  back; confirming applies it (restart required, as before).
- **Backend that cannot start:** a backend that `Config::validate` rewrites to
  `auto` (e.g. `metal` on Linux, GLES on Windows) now produces a
  "Rendering backend unavailable" modal naming the requested backend, from a
  config file or from the settings dialog.
- `Config::requested_renderer` keeps the user's original choice (runtime-only,
  never persisted) so the notices can name what was requested.
- The winit dependency now declares its features explicitly and
  `wayland-client`/`wayland-protocols` are Linux-only, so other targets still
  build.
- Cancelling the settings dialog restores the backend that was in effect
  before the click (not `auto`).
- Everything else is written to be inert off Linux: `gles_needs_wayland_notice`
  returns false unless `target_os = "linux"` and the session is Wayland, the
  startup notice only appears for GLES on a Wayland session or for a backend
  that `validate()` rewrote, and the tests pass in the Linux and Windows-like
  (`vulkan,dx`) and macOS-like (`vulkan,metal`) feature sets.

## Verification

Direct terminal-launcher form (the exact shape KDE used for `Terminal=true`):

```text
# self-exec: one Pretty window, no nested process
./target/debug/pretty --title SelfNestedTest \
  -e ./target/debug/pretty --title Ignored --config /tmp/pretty-test.toml
# log: terminal launcher asked Pretty to run itself; ignoring nested launch

# command with arguments: the -c/-d flags reach the child, no exit 1
./target/debug/pretty --title ShTest -e sh -c 'sleep 4'
# child: sleep 4 (pid 18218)
```

New CLI tests (`crates/pretty-app/src/main.rs`):

- `terminal_e_consumes_the_whole_argv_verbatim`
- `terminal_x_is_an_alias_for_e`
- `one_command_argument_is_kept_as_a_shell_string`
- `terminal_e_requires_a_command`
- `self_launch_is_detected_and_skipped`
- `new_tabs_do_not_repeat_the_launcher_command` (app)
- `gles_request_shows_a_wayland_notice_instead_of_switching_silently` (app)
- `unavailable_backend_reports_the_requested_name` (app)
- `gles_request_is_tracked_for_the_deprecation_notice` (config)

```
cargo test --workspace --locked
# 55 app + 61 core + 9 audit + 2 PTY (+1 ignored) + 4 QA + 15 renderer
# = 146 passed, 1 ignored, 0 failed
```

The native-Wayland drag-and-drop bridge is unchanged.

## Release note

**Issue #1 (Windows GLES) is not fixed in 0.2.4.** GLES remains disabled on
Windows and the wgpu/ANGLE investigation is deferred to the next **minor**
release, as stated on the v0.2.4 release page.

## Limitations

- This was tested with the exact argv shape KDE produced
  (`pretty -e <path> [args...]`), plus self and `sh -c` cases. Changing the
  whole desktop default-terminal setting was not left modified on the host.
- Windows/macOS GUI runtime remains CI-built/tested but not run on physical
  hardware, unchanged from earlier releases.
- The 0.2.3 physical-drag automation limitation is unchanged (the bridge is
  attached and lifecycle-tested, but this host has no virtual pointer).
