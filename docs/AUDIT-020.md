# AUDIT-020 — core terminal / kitty protocol security & UX audit

Audit of `pretty-core` (terminal state, stream/APC/OSC protocol layer, kitty
graphics, input encoders) for the 0.2.0 hardening pass, plus a read-only review
of the application UX paths (tabs/focus, search, IME, file drop, settings,
hyperlinks). Terminal output is treated as untrusted input throughout: any byte
sequence a program can write must not be able to exhaust terminal memory/CPU,
escape its protocol framing, silently change parser state, or social-engineer
the user through notifications/hyperlinks.

- **Owner:** core-audit
- **Write scope:** `crates/pretty-core/src/`, `crates/pretty-core/tests/`,
  `docs/AUDIT-020.md`
- **No version bump / no commit** as requested.
- **Test command:** `cargo test -p pretty-core --release` (54 unit tests +
  9 integration tests in `crates/pretty-core/tests/audit_020.rs`).
- **Workspace compile checkpoint:** `cargo check --workspace --release` passed
  (exit 0) and `cargo check -p pretty-app --release` passes against the updated
  core API; re-run full CI after integration because other owners are editing
  concurrently.

Findings are ordered by severity. "Fixed" means patched in this pass with a
regression test; "Reported" means the fix belongs to another owner
(`backend-platform` / `ui-customization`) or was left as a documented risk.

---

## 1. Fixed in `pretty-core`

### CORE-1 (High) — Oversized OSC desynchronises the VTE parser
`crates/pretty-core/src/stream.rs` (old `Mode::Osc` oversize branch; fixed at
`stream.rs:251-281`, `stream.rs:334-372`)

When an OSC sequence exceeded `MAX_OSC` (4 MiB), the filter discarded the
payload and started discarding until a terminator — but the `ESC ]` introducer
had already been forwarded to the VTE parser. The parser therefore stayed inside
an unterminated OSC string: all following output was swallowed as OSC payload,
and the first later `BEL`/`ST` anywhere in the stream (for example a bell or
another title) terminated the pseudo-OSC. Reproduced with a >4 MiB `OSC 0`
title followed by `hi`: `hi` did not render.

**Fix:** when an oversized OSC is abandoned, a `BEL` is appended to the byte
stream handed to the VTE parser so the parser returns to ground state; the same
recovery is applied by `StreamFilter::finish()` for a half-forwarded OSC. The
payload of a discarded sequence is never retained (`Mode::Ignore` keeps only a
one-byte `ESC` lookahead), so a hostile stream cannot pin megabytes while the
terminator is awaited.

**Tests:** `stream::tests::oversized_osc_is_terminated_for_the_vte_parser`,
`stream::tests::oversized_osc_with_st_terminator_recovers`,
`stream::tests::discarded_oversized_osc_does_not_buffer_payload`,
`stream::tests::unterminated_osc_at_stream_end_is_closed`,
`tests/audit_020.rs::oversized_osc_does_not_swallow_following_output`,
`tests/audit_020.rs::stream_filter_recovers_after_oversized_osc`.

### CORE-2 (High) — APC scanning was quadratic (CPU DoS via kitty graphics stream)
`crates/pretty-core/src/stream.rs:167-249`

Every PTY read re-scanned the whole accumulated APC payload from byte 0 for the
`ESC \` terminator, and complete APCs were re-processed recursively. A kitty
graphics payload of 64 MiB delivered in 64 KiB reads caused roughly
`n² / 2` byte comparisons (tens of GB of scanning), and a 64 KiB chunk full of
small APC sequences caused thousands of recursive frames.

**Fix:** the scan position is tracked incrementally (`apc_scanned`), only new
bytes plus the one-byte boundary are scanned, and `process_input` iterates
instead of recursing; the terminator position is converted back to "input bytes
consumed" so tails are handled without re-copying.

**Tests:** `stream::tests::apc_large_payload_scanned_incrementally` (256 KiB APC
in 1 KiB reads) and `stream::tests::many_complete_apcs_in_one_read_do_not_recurse`
(10 000 complete APCs in one read).

### CORE-3 (High) — PNG decompression bombs decoded before the pixel limit
`crates/pretty-core/src/graphics.rs:722-800`

`f=100` PNG payloads were passed to `image::load_from_memory_with_format`
without limits and the 32-megapixel check ran *after* `to_rgba8()`. A tiny
compressed PNG declaring e.g. 100 000 × 100 000 pixels could allocate gigabytes
(or hit the image crate's 512 MiB default allocation) before the terminal
rejected it.

**Fix:** the IHDR chunk is parsed and validated before decoding
(`png_ihdr_dimensions`), the `image::ImageReader` runs with explicit
width/height/`max_alloc` limits as defence in depth, and every decoded image is
re-checked against the pixel budget. Raw RGB/RGBA paths check `w*h` against the
same budget in `raw_dims`.

**Tests:** `graphics::tests::png_decompression_bomb_rejected_before_decode`
(malformed IDAT + 20 000² IHDR is rejected with "megapixel", proving the header
check runs first), `graphics::tests::png_limits_are_enforced_for_tiny_stores`,
`tests/audit_020.rs::png_decompression_bomb_is_rejected` (a real small PNG still
decodes).

### CORE-4 (Medium) — Graphics resident-byte budget bypassed by re-sending an image ID
`crates/pretty-core/src/graphics.rs:326-388`, `graphics.rs:479-500`

Eviction ran only when the image ID was new. A client could fill the store near
the 256 MiB budget with small images and then re-transmit an *existing* ID with
a much larger payload; the replacement was not budgeted and resident memory grew
past the cap. In an animation workload this is an unbounded memory path.

**Fix:** budget eviction now runs on every insert (including replacement), and
an image larger than the per-image byte budget is rejected. Bounded also on the
chunk-accumulation path and single-command path (`max_pending_bytes`).

**Tests:** `graphics::tests::retransmitting_a_known_id_cannot_exceed_the_byte_budget`,
`graphics::tests::resident_bytes_are_evicted_for_new_ids` (small test budgets so
the enforcement path is exercised cheaply).

### CORE-5 (Medium) — Auto-assigned kitty image IDs could overwrite explicit client images
`crates/pretty-core/src/graphics.rs:581-597`

Auto image IDs started at 1 and did not skip IDs already used by a client that
addressed images explicitly. A program that mixed explicit IDs and auto IDs could
silently have its image replaced; placements then pointed at the wrong pixels.

**Fix:** `alloc_image_id` skips IDs present in the store and is bounded by the
number of resident images.

**Test:** `graphics::tests::auto_ids_do_not_overwrite_explicit_ids`.

### CORE-6 (Medium) — Paste injection: bracketed-paste body could terminate the paste
`crates/pretty-core/src/input.rs:571-603`

`encode_paste` wrapped the clipboard text in `ESC[200~`/`ESC[201~` without
filtering it. Pasting text that contains `ESC[201~` closes the bracketed paste
early, so the rest of the clipboard text is interpreted as real keyboard input
(the classic "paste from a web page executes commands" attack); embedded
`ESC]52;…`/`ESC[2J` sequences also reach the application when it trusts the
paste body.

**Fix:** control characters are stripped from the paste body when bracketed
paste is active (tab/newline/CR preserved for layout), which removes every
escape sequence including the end marker. Non-bracketed paste keeps the
historical byte-for-byte behaviour (no framing contract exists there).

**Tests:** `input::tests::bracketed_paste_cannot_be_terminated_early`,
`input::tests::bracketed_paste_keeps_layout_controls`,
`input::tests::unbracketed_paste_is_unchanged_for_compatibility`,
`tests/audit_020.rs::bracketed_paste_cannot_inject_escape_sequences`.

### CORE-7 (Medium) — Notification injection and notification spam
`crates/pretty-core/src/stream.rs:371-440` (+ `notify.rs` delivery unchanged)

OSC 9 / OSC 777 payloads were forwarded verbatim to the desktop notification
daemon: embedded `ESC`/`BEL`/newlines allow UI/format spoofing, bidi override
characters allow visual spoofing of the notification source, and an unbounded
number of notifications could be emitted (a one-line shell loop can spawn
thousands of DBus notifications/threads).

**Fix:** notification title/body are sanitized (C0/C1 controls removed, tabs and
newlines become spaces, bidi/zero-width format characters removed) and capped
(256 title / 4096 body characters); a token-bucket limiter forwards at most
8 notifications per second per terminal. OSC 9;4 progress reports stay ignored.

**Tests:** `stream::tests::notification_text_is_sanitized`,
`stream::tests::notification_fields_are_capped`,
`stream::tests::notification_spam_is_rate_limited`,
`tests/audit_020.rs::notifications_are_sanitized_and_rate_limited`.

### CORE-8 (Medium) — Kitty image anchors baked in the current scroll offset
`crates/pretty-core/src/graphics.rs:228-240`, `crates/pretty-core/src/terminal.rs:144-156`

`GraphicsStore::handle` subtracted `display_offset` from the cursor row even
though the caller passed the cursor's grid line. The renderer adds the live
display offset when drawing (`pretty-renderer/src/gpu.rs:480`:
`row = anchor_line + display_offset`). Images written while the user had
scrolled up were therefore permanently displaced by the scroll offset (and the
anchor even depended on the scroll position rather than content).

**Fix:** the anchor is stored as the cursor's grid line with no offset baked in;
both sides now agree that the renderer applies the offset. Scroll drift of
existing images when *new output* scrolls the grid remains a documented kitty
limitation (`docs/KNOWN-ISSUES.md` item 12), not a regression.

**Tests:** `graphics::tests::anchor_does_not_depend_on_scroll_offset`,
`terminal::tests::graphics_anchor_ignores_scroll_position`,
`tests/audit_020.rs::kitty_image_anchor_is_scroll_independent`.

### CORE-9 (Low) — kitty `q=2` errors were still written to the PTY; `q=1` not honoured
`crates/pretty-core/src/graphics.rs:326-388`, `graphics.rs:562-580`

Per the kitty graphics protocol `q=0` = all responses, `q=1` = suppress OK
responses only, `q=2` = suppress everything including errors. The old code sent
OK for `q=1` and always sent errors, even for `q=2`. Animation clients (mpv's
kitty VO) use `q=2` precisely to keep the PTY response stream quiet.

**Fix:** success responses are emitted only for `q=0`; `error()` returns nothing
for `q=2` but still reports errors for `q=0`/`q=1`; `q` values >2 are treated as
2. Errors remain control-character sanitised.

**Tests:** `graphics::tests::quiet_two_suppresses_error_responses`,
`graphics::tests::quiet_one_suppresses_success_only`,
`tests/audit_020.rs::graphics_quiet_mode_suppresses_error_responses`.

### CORE-10 (Medium, defence in depth) — OSC 52 clipboard read enabled by default in core
`crates/pretty-core/src/terminal.rs:54-75`

`CoreConfig::default()` mapped `osc52_paste: true`, i.e. any program could ask
the terminal to read the user's clipboard and receive it through the PTY. This
is an exfiltration channel for untrusted terminal output and contradicts
alacritty's copy-only default.

**Fix:** the core default is now copy-only (`osc52_paste: false`); clipboard
*write* stays enabled by default. The application's own config defaults are
still insecure (see APP-2) and must be changed by their owner for the shipped
binary to benefit.

**Tests:** `terminal::tests::osc52_read_is_denied_by_default`,
`terminal::tests::osc52_read_allowed_only_when_configured`,
`terminal::tests::osc52_write_is_allowed_by_default`,
`tests/audit_020.rs::osc52_read_is_denied_by_default_config`.

### CORE-11 (Medium) — No runtime scrollback limit API (blocked APP-6 live apply)
`crates/pretty-core/src/terminal.rs:45-52,111-180`, `crates/pretty-core/src/lib.rs:30`

`scrollback` was only read when a `Terminal` was created; there was no way to
change the history limit on a live terminal, so the settings slider could not
take effect until a new pane/tab was opened. The app also depends on a bounded
value: the reported slider allows up to 200 000 lines and large history limits
are a memory hazard (one line ≈ `columns * cell_size` bytes).

**Fix / API:**
- `pub const MAX_SCROLLBACK_LINES: usize = 200_000` (re-exported at the crate
  root as `pretty_core::MAX_SCROLLBACK_LINES`).
- Initial `CoreConfig::scrollback` is clamped to the same value at construction.
- `pub fn set_scrollback(&mut self, lines: usize) -> usize` — clamps `lines`,
  preserves all other terminal options (kitty keyboard flags, OSC 52 policy,
  cursor style, escape chars) by keeping the last applied `alacritty_terminal`
  `Config` in the `Terminal`, and applies the change through
  `Term::set_options` → `Grid::update_history`. Shrinking trims already retained
  history immediately; growing applies to subsequent output. Returns the
  effective (clamped) limit.
- `pub fn scrollback(&self) -> usize` — effective limit for UI/diagnostics.

**Tests:** `terminal::tests::set_scrollback_updates_history_limit_and_keeps_terminal_usable`
(100 → cap 300 lines → shrink to 10 trims immediately → grow to 50 → terminal
still parses/render input with cursor), `set_scrollback_clamps_to_core_maximum`,
`initial_scrollback_is_clamped`,
`set_scrollback_preserves_other_terminal_options` (kitty flags and OSC 52 paste
policy survive the option update).

**Follow-up for the app owner:** call
`for pane in self.panes.values_mut() { pane.term.set_scrollback(self.config.scrollback); }`
from `config_changed` (e.g. in `apply_ui_changes_debounced`) and prefer
`pretty_core::MAX_SCROLLBACK_LINES` over a duplicated `200_000` literal. This
closes APP-6; no app file was edited by this audit.

### Verified-not-a-bug (regression guards added)

- **UTF-8 split across PTY reads** (`tests/audit_020.rs::utf8_split_across_pty_reads_is_reassembled`):
  the vte parser keeps partial-codepoint state across `Processor::advance` calls
  and the stream filter does not reorder bytes, so this already worked; the test
  now pins the behaviour.
- **Raw RGB/RGBA payload sizing**: `raw_dims` validates the exact byte count
  against `s=`/`v=` before conversion, so the raw paths cannot allocate more than
  the stream filter allows.

---

## 2. Reported — application / UX paths (read-only, owned by others)

These were found while auditing the app-facing UX contract. **No app files were
modified.** Line numbers refer to the working tree at audit time and may drift
while others edit; symbols are given so they can be relocated.

### APP-1 (High) — OSC 8 hyperlinks opened with no scheme allowlist — **FIXED by ui-customization, verified**
`app.rs:1600-1618` now refuses non-`http`/`https`/`mailto` targets via
`is_safe_hyperlink` (`app.rs:2884-2891`) before `open::that_detached`; unit tests
at `app.rs:3158+`.
`crates/pretty-app/src/app.rs:1550-1565` (`process_mouse_input`, Ctrl+Left click
→ `open::that_detached(&uri)`)

Any URI a program writes via OSC 8 (or a plain URL that gets detected) is passed
to `xdg-open`/`open` without an allowlist or confirmation. Malicious output can
register/abuse handler schemes, open local files (`file:///…`), hand
credentials-bearing URLs to arbitrary registered apps, or launch a browser to a
phishing page. Terminal programs (kitty, alacritty, foot, …) restrict this to
`http`/`https`/`mailto` (often with a confirmation option).

**Recommendation:** allowlist `http`/`https` (optionally `mailto`) before
`open::that_detached`; log+ignore other schemes; consider a configurable
`allow_hyperlinks`/`confirm_hyperlink_open` setting. The URI itself can stay
untrusted in `pretty_core::Terminal::hyperlink_at` (core returns raw data by
design); enforcement belongs where the OS action happens.

### APP-2 (High) — App defaults still enable OSC 52 clipboard *read* — **FIXED by ui-customization, verified**
`config.rs:108` now defaults `allow_osc52_paste: false` (copy remains enabled;
toggles in `settings.rs`).
`crates/pretty-app/src/config.rs:105-106` (`allow_osc52_paste: true`,
`allow_osc52_copy: true`), wired at `crates/pretty-app/src/app.rs:651-663`
(`CoreConfig { osc52_copy, osc52_paste }`), toggles at
`crates/pretty-app/src/settings.rs:532-544`

The core now defaults to copy-only (CORE-10), but the application passes
`allow_osc52_paste: true` from its own default config, so the shipped binary
still lets any process read the clipboard (`printf '\e]52;c;?\a'`). This is
documented as a known issue but is still an exfiltration channel for untrusted
output (e.g. `ssh` to an untrusted host, `cat` of a crafted file).

**Recommendation:** default `allow_osc52_paste = false` (keep copy default
configurable), or add a permission prompt / "read allowed" acknowledgement.
Settings text already explains the risk; the default should match the docs.

### APP-3 (High) — Hidden-tab panes keep stale rectangles and can steal focus/clicks — **FIXED by ui-customization, verified**
`pane_at` (`app.rs:1314-1331`) now skips panes not in `self.layout`, so hidden
tabs are non-interactive.
`crates/pretty-app/src/app.rs:1273-1289` (`pane_at` iterates **all** `self.panes`
without checking the active layout), `app.rs:686-706` (`resize_all` only updates
panes in the *current* layout), `app.rs:1086-1094` (`load_active_tab` restores
layout/focus but not pane rects), `app.rs:1528` (`self.focused = pane_id` from
hit-testing), also used by `process_cursor_moved` / touch.

Panes of inactive tabs stay in `self.panes` and keep the rectangle they had when
their tab was active. Since every tab's layout typically covers the same full
area, a click in the terminal can hit a hidden pane (HashMap iteration order),
focus it, and type/paste/scroll into an invisible shell — including sending
mouse reports to a program the user cannot see. Same class: `process_cursor_moved`
can forward motion/mouse-drag reports to hidden panes.

**Recommendation:** filter hit-testing by `self.layout.contains(id)` (or by
`self.panes` membership restricted to active layout) in `pane_at`, and only
allow focus changes for active panes.

### APP-4 (Medium) — Search highlights/selects wrong columns for non-ASCII text — **FIXED by ui-customization, verified**
`refresh_search` (`app.rs:817+`) now matches on `Vec<char>` with a
folded→original char map, so CJK/emoji matches highlight correctly.
`crates/pretty-app/src/app.rs:830-866` (`refresh_search`)

`lower = line.to_lowercase()` and `lower[from..].find(&query)` produce **byte**
offsets, but `start_char` is then used as an index into `char_cols`, a per-char
vector (`char_cols.get(start_char)`), and `end_char = start_char + query.len()`
treats the query's byte length as a character count. For any line with CJK/emoji
before or at a match the highlighted/selected cell is shifted (or the match is
dropped by the `.get()` break), and CJK searches select the wrong length. Some
case-folding can also change byte length, compounding the skew.

**Recommendation:** work in char indices (`line.char_indices()`/`Vec<(usize,
usize)>` byte→cell map) or build the search on a `Vec<char>` and convert; then
compute the match length in chars.

### APP-5 (Medium) — Exited pane in an inactive tab is never removed (zombie pane) — **FIXED by ui-customization, verified**
`close_pane` (`app.rs:1227+`) resolves the owning tab first and closes/updates
that tab instead of the active one.
`crates/pretty-app/src/app.rs:1211-1227` (`close_pane` removes from the **active**
`self.layout`), `app.rs:1419-1420` (`TerminalEvent::Exit`/`ChildExit` →
`close_pane(id)`), `app.rs:2454-2457` (`PtyReady` → `handle_pty` for every pane
regardless of tab)

When a program exits in a non-active tab, `close_pane` runs against the active
tab's layout: `layout.remove(id)` fails and `if leaves <= 1` closes the wrong tab
if the active layout has a single pane. The dead pane stays in `self.panes`
forever (kept `Pane`/`Terminal`/PTY handles, stale title), and the active tab can
even be closed instead. Switching back to that tab shows a frozen dead terminal.

**Recommendation:** locate the pane's owning tab first (`tabs.iter().position(|t|
t.layout.contains(id))`), operate on that tab's layout/panes, and only fall back
to `close_tab` when that tab itself becomes empty. Alternatively route
`Exit`/`ChildExit` through a per-pane close that knows its tab.

### APP-6 (Low/Medium) — `scrollback` setting has no live effect on existing panes — **CORE API ADDED (CORE-11); app wiring pending**
App config is now clamped to 200 000 lines (`config.rs:393`); remaining step is
calling `Terminal::set_scrollback` from `config_changed` (see CORE-11).
`crates/pretty-app/src/config.rs:388` (clamped to 1 000 000), settings slider
`crates/pretty-app/src/settings.rs:283` (0–200 000 lines), spawn-only wiring
`crates/pretty-app/src/app.rs:651-663`, `config_changed`
`crates/pretty-app/src/app.rs:1046-1072`

`scrollback` is read only when a `Terminal` is created; `config_changed` never
updates live terminals (`Terminal` has no setter). Moving the slider appears to
do nothing until a new tab/pane is opened. Also note the app clamp of 1 000 000
lines is itself a memory hazard (millions of cells), considerably above the
10 000 default and the 200 000 slider maximum.

**Recommendation:** either document "applies to new terminals", or add a
`Terminal::set_scrollback`/recreate path, and align the clamp with the slider
maximum (200 000) unless there is a reason not to.

### APP-7 (Low) — Windows file-drop quoting is not cmd.exe-safe
`crates/pretty-app/src/app.rs:2828-2838` (`shell_quote_path`), used by
`handle_file_drop` `app.rs:1912-1920`

The Windows branch only wraps the path in double quotes and escapes `"` with a
backslash (not a cmd.exe escape). cmd.exe performs `%VAR%` expansion inside
double quotes, so dropping a file named like `%COMSPEC%` injects expansion into
the shell command line; the dropped text is not executed until Enter, but it is
still a paste-injection foothold on Windows. POSIX quoting is correct and was
has unit tests (`app.rs:3036-3043`).

**Recommendation:** use Windows `CommandLineToArgvW`-style quoting (backslash
doubling before quotes) and/or escape `%`, or write the path via a
single-quote-free mechanism; alternatively only insert paths as bracketed paste
(now that core filters control characters) and let the user edit them.

### APP-8 (Low) — Minor UX inconsistencies — **FIXED by ui-customization, verified**
File drop now also respects `search.open` (`app.rs:1967-1970`); padding click
clamping is handled in the active-pane hit test.
- `handle_file_drop` (`app.rs:1912-1920`) ignores `self.search.open` while the
  find overlay is open: a drop writes the path to the terminal behind the
  overlay (settings is guarded, search is not). Compare `process_ime`
  (`app.rs:1705-1736`), which guards commit/preedit on both overlays.
- `pane_at` returns `(0, 0)` for clicks in the left/top padding of a pane
  (`app.rs:1273-1289`), so a click in the padding selects cell (0,0) instead of
  being ignored.
- Background-tab PTY output still calls `w.request_redraw()` for every chunk
  (`handle_pty`, `app.rs:1456+`); with several busy background tabs this burns
  frames for invisible panes.

---

## 3. Residual risks / follow-ups (not fixed here)

1. **Unbounded terminal event channel.** `Terminal::new` uses
   `crossbeam_channel::unbounded()` for `TerminalEvent`s. Output that produces
   many events (title changes, DA/colour queries, bell) can grow the queue if
   the app is slower than the PTY. The PTY read channel is bounded (1024
   chunks), which caps the burst, but a bounded event channel with
   coalescing/dropping for low-value events (Title/Bell) would be a cleaner
   design. Cross-crate/API change; left for `backend-platform`.
2. **Graphics memory ceiling.** The store now enforces 32 MP per image, 256
   images, 256 MiB resident and 1024 placements on *every* insert. Peak transient
   use is the chunk buffer plus the decoded buffer (≈ up to 2×128 MiB for a
   maximally-sized RGBA image); a smaller `max_pending_bytes` for compressed
   formats could tighten this further.
3. **Notification throttling is per terminal/pane.** 8/s per pane still allows
   many panes/tabs to aggregate; a global app-level limiter would be stronger.
   The app also spawns one thread per notification (`app.rs:1413-1422`); the
   core limiter bounds that to 8/s/pane now, but a small queue/worker would be
   cleaner.
4. **Image scroll drift with new output** remains as documented in
   `docs/KNOWN-ISSUES.md` item 12 (anchors are grid lines; a full scrollback
   anchor model is not implemented). This pass only removed the scroll-offset
   bug at placement time (CORE-8).
5. **Paste filtering is only active in bracketed-paste mode.** Unbracketed paste
   keeps raw bytes for compatibility (applications that enable bracketed paste
   get the safe path; shells/programs that do not enable it are responsible for
   their own paste handling, as with other terminals).
6. **`docs/KNOWN-ISSUES.md` updated:** item 16 now documents copy-only OSC 52
   defaults (APP-2 fixed) and item 19 now documents tabs and live scrollback
   instead of claiming there are no tabs/sessions. Both edits are documentation
   only; no code semantics changed by them.
7. **Catastrophic-but-bounded inputs:** `MAX_OSC` (4 MiB) and `MAX_APC`
   (64 MiB) permit a hostile program to make the filter buffer that much before
   recovery; this is intentional to keep real kitty image/video frames working
   and is now linear (no quadratic blowup). Notifications are capped at 4 KiB
   body.

---

## 4. Test evidence

```
$ cargo test -p pretty-core --release
...
running 54 tests
test result: ok. 54 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
     Running tests/audit_020.rs
running 9 tests
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

New/updated tests by area:

| Area | Unit tests | Integration tests |
| --- | --- | --- |
| OSC/APC parser recovery & bounds | `stream::tests::oversized_*`, `discarded_oversized_*`, `unterminated_osc_*`, `apc_large_payload_*`, `many_complete_apcs_*` | `oversized_osc_does_not_swallow_following_output`, `stream_filter_recovers_after_oversized_osc` |
| Notifications | `notification_text_is_sanitized`, `notification_fields_are_capped`, `notification_spam_is_rate_limited` | `notifications_are_sanitized_and_rate_limited` |
| Kitty graphics | `png_decompression_bomb_rejected_before_decode`, `png_limits_are_enforced_for_tiny_stores`, `retransmitting_a_known_id_cannot_exceed_the_byte_budget`, `resident_bytes_are_evicted_for_new_ids`, `auto_ids_do_not_overwrite_explicit_ids`, `anchor_does_not_depend_on_scroll_offset`, `quiet_*` | `png_decompression_bomb_is_rejected`, `kitty_image_anchor_is_scroll_independent`, `graphics_quiet_mode_suppresses_error_responses` |
| Paste injection | `bracketed_paste_cannot_be_terminated_early`, `bracketed_paste_keeps_layout_controls`, `unbracketed_paste_is_unchanged_for_compatibility` | `bracketed_paste_cannot_inject_escape_sequences` |
| Scrollback live limit | `set_scrollback_updates_history_limit_and_keeps_terminal_usable`, `set_scrollback_clamps_to_core_maximum`, `initial_scrollback_is_clamped`, `set_scrollback_preserves_other_terminal_options` | — |
| OSC 52 policy defaults | `osc52_read_is_denied_by_default`, `osc52_read_allowed_only_when_configured`, `osc52_write_is_allowed_by_default` | `osc52_read_is_denied_by_default_config` |
| UTF-8 chunk boundaries | — | `utf8_split_across_pty_reads_is_reassembled` |

No GUI/hardware tests were run (out of scope for this audit).

## 5. Files changed

- `crates/pretty-core/src/stream.rs` — CORE-1/2/7 (parser recovery, bounds, notification policy) + unit tests.
- `crates/pretty-core/src/graphics.rs` — CORE-3/4/5/8/9 (PNG limits, budgets, ID allocation, anchor contract, quiet responses) + unit tests.
- `crates/pretty-core/src/input.rs` — CORE-6 (bracketed-paste filtering) + unit tests.
- `crates/pretty-core/src/terminal.rs` — CORE-8/10/11 (anchor wiring, secure OSC 52 default, live scrollback API) + unit tests.
- `crates/pretty-core/src/lib.rs` — re-export `MAX_SCROLLBACK_LINES`.
- `crates/pretty-core/tests/audit_020.rs` — new integration suite (9 tests).
- `docs/AUDIT-020.md` — this report.
- `docs/KNOWN-ISSUES.md` — stale items 16 and 19 corrected (documentation only, requested by ui-customization).

Not touched: `crates/pretty-app/**`, `crates/pretty-renderer/**`, `crates/pretty-*/src/main.rs`,
CI/packaging and the version. `docs/KNOWN-ISSUES.md` only had the two stale
items above corrected.
