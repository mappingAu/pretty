# kitty compatibility matrix

Reference: <https://sw.kovidgoyal.net/kitty/>. Status is deliberately binary:

- **Implemented** means the feature is wired through to real terminal behaviour
  and has the listed verification (unit test, protocol probe, or screenshot).
- **Partial** means a useful subset is real and the missing pieces are named.
- **Not implemented** means it does not claim to work.

Verification artifacts live in `docs/screenshots/` and `test-artifacts/` (from
`scripts/test-hardware.sh`).

## Core terminal features

| Feature | Status | Verification / notes |
|---|---|---|
| 24-bit true colour (SGR 38/48;2) | Implemented | `vulkan-features` screenshot; SGR passed through alacritty_terminal; feature demo prints 24-bit foreground/background |
| 256-colour and ANSI palette | Implemented | feature demo; theme palette in `pretty-renderer/src/theme.rs` |
| Bold, dim, italic, inverse, hidden | Implemented | renderer attributes + feature screenshot |
| Underline, double underline, undercurl, dotted/dashed, strikeout | Implemented | renderer decorations; feature demo contains underline/undercurl/strikeout |
| Underline colour (SGR 58) | Implemented | alacritty_terminal cell underline colour is mapped by core |
| Wide characters and combining marks | Implemented | alacritty_terminal grid; CJK in feature screenshot |
| Alternate screen | Implemented | alacritty_terminal; used by vttest and full-screen programs |
| Scrolling regions, insert/delete, erase | Implemented | alacritty_terminal; vttest cursor test launched through Pretty |
| Scrollback | Implemented | `scripts/test-hardware.sh` compares bottom-vs-Shift+PageUp screenshots; default 10,000 lines |
| Resize / PTY resize / swapchain recreation | Implemented | hardware script resizes 1040×700→820×620→1280×820 and checks logs/process |
| Bracketed paste | Implemented | `encode_paste` tests incl. 200~/201~ wrapping; app routes Ctrl+Shift+V |
| Focus in/out reporting | Implemented | app writes CSI I/O when terminal mode is enabled |
| Mouse: X10, 1000, 1002, 1003, 1005, 1006 | Implemented | encoder unit tests; 1002+1006 end-to-end probe in hardware script |
| Mouse: 1015 (urxvt) | Not implemented | alacritty_terminal does not expose a 1015 mode; documented limitation |
| Cursor shape via DECSCUSR | Implemented | core tests `default_cursor_is_block`, `decscusr_changes_cursor_shape`; cursor-shape demo log/screenshots |
| Window title OSC 0/2 | Implemented | hardware script asserts `Pretty OSC Title` |
| Bell | Implemented | app requests user attention and logs |
| Selection / copy / paste | Implemented | simple selection, Ctrl+Shift+C/V, Shift+Mouse overrides mouse reporting |

## kitty graphics protocol

Transport is APC `ESC _ G ... ESC \`, stripped from the VTE stream by a streaming
interceptor that preserves boundaries across PTY reads.

| Feature | Status | Verification / notes |
|---|---|---|
| PNG transmission `f=100` | Implemented | feature screenshot and `graphics-probe.py`; `image` crate decode with error responses |
| Raw RGB `f=24` / RGBA `f=32` | Implemented | core unit tests for chunked RGBA and bad-size rejection |
| Base64 chunks `m=1` | Implemented | feature demo sends a two-chunk PNG; graphics probe queries the result |
| Transmit `a=t`, transmit+display `a=T` | Implemented | probes/demos; display anchors at the cursor cell |
| Place `a=p` with `p`, `c`, `r`, `x`, `y`, `z` | Implemented | graphics probe places with `p=3,c=20,r=10,z=-1`; screenshot feature demo uses negative z-index and text overlay |
| Query `a=q`, image id `i` | Implemented | graphics probe checks OK for stored id, EINVAL after delete |
| Delete `a=d` by image/placement/z/all | Implemented | graphics probe verifies delete; core unit test `delete_all` |
| z-index < 0 below text, >= 0 above text | Implemented | render pass order in `renderer.rs`; feature demo sends `z=-1` and draws text over it |
| Images clipped to their pane | Implemented | CPU UV-rect clipping in `gpu.rs`; no spill with tiling |
| Images move with scrollback | Partial | placement anchors are stored as absolute lines and offset at draw; exact kitty semantics around history are not exhaustively tested |
| Response protocol `OK`/`EINVAL` with q suppression | Implemented | graphics probe captures responses |
| Max resource bounds | Implemented | 256 images, 1024 placements, 64 MiB APC, 32 megapixel decoded image |
| zlib compression `o=z` | Not implemented | returns explicit EINVAL |
| File path transmission, shared memory | Not implemented | not supported; listed in KNOWN-ISSUES |
| Animation / frames (`a=f`, `a=a`) | Not implemented | explicit EINVAL |
| Unicode placeholders / image hyperlinks | Not implemented | not in matrix scope |

## kitty keyboard protocol

| Feature / flag | Status | Verification |
|---|---|---|
| Negotiation query `CSI ? u` | Implemented | keyboard probe captures `CSI ? 0 u` before push and `CSI ? 31 u` after |
| Push `CSI > flags u`, set `CSI = flags u`, pop `CSI < n u` | Implemented | push/query verified end-to-end; push/pop core test in `terminal.rs` |
| Disambiguate escape codes (flag 1) | Implemented | encoder CSI-u for modified/special keys; probe shows `CSI 13;2:1;13u` for Shift+Enter |
| Report event types (flag 2) | Implemented | probe sees press (`:1`) and release (`:3`) records |
| Report alternate keys (flag 4) | Implemented | Shift+A encodes `65:65:97`; probe captures it |
| All keys as escape codes (flag 8) | Implemented | probe 'a' with all flags is `CSI 97;1:1;97u`, not raw text |
| Associated text (flag 16) | Implemented | probe captures `;97` / `;65` text parameters |
| Legacy fallback (no kitty flags) | Implemented | encoder unit tests for Ctrl+A/arrow/app-cursor; earlier interactive `helloworld` test |

## Other kitty / terminal extensions

| Feature | Status | Verification / notes |
|---|---|---|
| OSC 8 hyperlinks | Implemented | underlined in the feature screenshot; `Terminal::hyperlink_at` and Ctrl+click opening |
| OSC 52 clipboard copy/paste | Implemented | hardware script asserts `pretty-clipboard-ok` via X clipboard; copy-only/paste configurable |
| OSC 9 desktop notification | Implemented | app maps to notify-rust; hardware script sends OSC 777 and checks delivery |
| OSC 777 notify;title;body | Implemented | desktop notification delivery verified in hardware script |
| OSC 4/10/11 colour queries | Implemented | alacritty_terminal events answered by the app from the active theme |
| Kitty OSC 99 rich notifications | Not implemented | only OSC 9/777; documented |
| Kitty pointer shape protocol | Not implemented | not implemented; OS cursor is text/I-beam |
| Kitty file transfer / shell integration / remote control | Not implemented | not in scope |
| Synchronized updates (DEC 2026) | Not claimed | pass-through terminal core may accept; renderer does not guarantee atomic presentation |
| Cursor blink control | Implemented | config + DECSCUSR blinking shapes; timer in app |
| Font features / ligatures | Implemented | Rustybuzz, configurable `font_features`; feature screenshot shows `=> != -> === <= >=` |
| Emoji | Implemented (monochrome fallback) | bundled Noto Emoji rendered in feature screenshot; COLRv1 colour emoji path is a known limitation |
| Tiling panes | Implemented | hardware screenshot after Ctrl+Shift+E; separate PTYs |
| Background transparency | Implemented (compositor) | `_NET_WM_WINDOW_OPACITY`; full desktop screenshot |
| Background blur | Implemented (KWin/X11) | `_KDE_NET_WM_BLUR_BEHIND_REGION`; hardware screenshot and xprop checks |
| Window scaling / DPI | Implemented | font/atlas rebuild on scale factor; swapchain reconfigure |
| Notifications on all platforms | Partial | notify-rust supports Linux/Windows; macOS/Android/other fall back with a log |

## How to reproduce

```bash
# Protocol-level (no screenshots needed)
cargo test --workspace
python3 assets/demo/kitty-keyboard-probe.py /tmp/keys 8
python3 assets/demo/graphics-probe.py /tmp/graphics

# Full visual + hardware run
scripts/test-hardware.sh
```

Every **Implemented** row above has either a unit test in the repository, an
end-to-end probe in `scripts/test-hardware.sh`, or a captured screenshot linked
from `docs/TEST-REPORT.md`. The **Not implemented** rows are intentional and are
also listed in `docs/KNOWN-ISSUES.md`.
