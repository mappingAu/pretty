# Pretty v0.2.5 — touch scrolling and settings overscroll

**Run:** 0.2.5 patch release
**Date:** 2026-11-21 (Asia/Shanghai)
**Host:** Fedora 45, KDE Plasma 6.8.2 (Wayland), NVIDIA GTX 1050
**Starting point:** `v0.2.4` (`0648a27`)

## 1. One-finger touch now scrolls the terminal

Before, a one-finger drag in the terminal started a text selection (and a left
mouse press in mouse-reporting applications), so the same swipe that users
expect to scroll selected text instead. Two-finger scrolling existed, but the
primary gesture was selection.

Now:

- A one-finger vertical drag scrolls the focused pane's scrollback by whole
  terminal rows, with the sub-cell remainder kept so slow drags accumulate.
- A drag never starts a selection and never emits mouse-drag events.
- A tap (no movement past the 6 px threshold) still focuses the pane; in a
  mouse-reporting TUI it emits one press+release click at the touched cell.
- Two-finger scrolling keeps the existing centroid behavior.
- Settings/menu/egui-consumed touches are not treated as terminal gestures.

The line conversion lives in `touch_scroll_lines(delta, cell_h)` and is unit
tested for zero, sub-row, exact-row, negative, remainder and invalid metrics.

## 2. Settings overscroll now springs back without moving the mouse

egui asks for a repaint while its kinetic/rubber-band scroll is animating
(`Context::request_repaint`). The app never forwarded that request to winit, so
a touch or touch-like left drag that ended past the bottom of the settings list
stayed visually held at the overscroll offset; the spring animation only became
visible when the next mouse move happened to generate a frame.

`PrettyApp::draw_ui` now checks `egui_ctx.has_requested_repaint()` after
`run_ui` and requests a winit frame. The rubber-band/coast animation therefore
runs immediately, including after a touch release, without requiring pointer
movement. The request is only forwarded while egui asks for it; an idle window
is not repainted continuously.

## Verification

```
cargo test --workspace --locked
# 56 app + 61 core + 9 audit + 2 PTY (+1 ignored) + 4 QA + 15 renderer
# = 147 passed, 1 ignored, 0 failed
```

New tests:

- `touch_drags_scroll_in_whole_rows_and_keep_the_remainder`

Manual checks on the release binary:

- Settings page: open, drag/scroll to the bottom and keep pulling; the list
  rubber-bands and springs back while the finger/left button is still held or
  immediately after release, without moving the mouse.
- Terminal: a one-finger swipe scrolls history; text is not selected.
- Tapping a mouse-reporting pane still delivers one click, not a drag.

## Limitations

- This host has no touchscreen and no virtual-pointer permission, so a physical
  one-finger swipe could not be automated. The gesture path and line conversion
  are unit tested; the two-finger path is unchanged and already shipped.
- Windows/macOS GUI runtime remains CI-built/tested but not run on physical
  hardware, unchanged from earlier releases.
