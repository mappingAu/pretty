# Pretty v0.2.4 — Terminal=true desktop launchers

**Run:** 0.2.4 patch release
**Date:** 2026-11-20 (Asia/Shanghai)
**Host:** Fedora 45, KDE Plasma 6.8.2 (Wayland); Rust stable 1.100
**Starting point:** `v0.2.3` (`644d05d`)

This release fixes the remaining `.desktop` launch case from the report. The
0.2.3 activation-token fix is still present, but it did not address a desktop
entry with `Terminal=true`.

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

winit 0.30's activation token was not the failing path here. Pretty's CLI had
a terminal-emulator-shaped `-e` option, but it consumed **one** argument and
then continued parsing the command's own arguments as Pretty options. For
example:

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

- `terminal_e_consumes_the_whole_command_line`
- `terminal_x_is_an_alias_for_e`
- `one_command_argument_is_kept_as_a_shell_string`
- `terminal_e_requires_a_command`
- `self_launch_is_detected_and_skipped`

```
cargo test --workspace --locked
# 51 app + 61 core + 9 audit + 2 PTY (+1 ignored) + 4 QA + 15 renderer
# = 142 passed, 1 ignored, 0 failed
```

The 0.2.3 native-Wayland drag-and-drop bridge and activation-token handling
are unchanged.

## Release note

**Issue #1 (Windows GLES) is not fixed in 0.2.4.** GLES remains disabled on
Windows and the wgpu/ANGLE investigation is deferred to the next **minor**
release (0.3.0), as on the v0.2.4 release page.

## Limitations

- This was tested with the exact argv shape KDE produced
  (`pretty -e <path> [args...]`), plus self and `sh -c` cases. Changing the
  whole desktop default-terminal setting was not left modified on the host.
- Windows/macOS GUI runtime remains CI-built/tested but not run on physical
  hardware, unchanged from earlier releases.
- The 0.2.3 physical-drag automation limitation is unchanged (the bridge is
  attached and lifecycle-tested, but this host has no virtual pointer).
