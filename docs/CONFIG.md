# Pretty configuration reference

Pretty is configured with a single TOML file. Every key below is also editable
in the graphical settings panel (`Ctrl+,` or the **Settings** button), which
applies changes live and saves them after a short debounce (~220 ms); quitting
immediately after a change still flushes the pending save.

## File location

| Method | Location |
|---|---|
| Default | `~/.config/pretty/config.toml` (`$XDG_CONFIG_HOME` aware) |
| `$PRETTY_CONFIG` | absolute path in the environment variable |
| `--config PATH` | command-line override |
| First run | Pretty writes a default config file automatically |

Missing keys always fall back to the built-in default, so an old or partially
hand-written file keeps working after an upgrade. Invalid values are clamped
(or replaced with the default) and logged; they never abort startup.

## Graphical settings

| Tab | Controls |
|---|---|
| General | language, font family/size/line height, ligatures, OpenType features, scrollback, shell, window width/height + **Apply window size**, **Import kitty.conf** |
| Terminal | cursor shape/colour/blink interval, bell mode + visual-bell duration, copy-on-select, confirm-close, OSC 52 copy/paste policy |
| Appearance | theme preset, 16-colour ANSI palette, opacity/blur/padding/tiling gap, terminal colours, UI accent/panel opacity/UI scale, tab-bar visibility/height |
| Rendering | backend selector, vsync, GPU/driver/surface/protocol diagnostics |
| Keyboard | kitty keyboard flags and every configurable action binding |
| About | version, config path, GPU, backend, documentation links |

The right-click context menu and the tab bar read the *current* key bindings, so
rebinding or clearing an action is reflected there immediately.

## Key reference

### Rendering and terminal

| Key | Default | Notes |
|---|---|---|
| `renderer` | `"auto"` | `auto`, `vulkan`, `gles`, `metal` (macOS builds), `dx` (Windows builds); restart required |
| `vsync` | `true` | FIFO present mode |
| `font_family` | `"JetBrains Mono"` | any installed system font family; falls back to the bundled JetBrains Mono with a warning if missing |
| `font_size` | `15.0` | 7–42 px |
| `line_height` | `1.18` | 0.85–1.8 |
| `ligatures` | `true` | programming ligatures (`liga`/`calt`) |
| `font_features` | `["liga", "calt"]` | OpenType features passed to the shaper |
| `scrollback` | `10000` | lines of history, capped at 200,000; changes apply to panes created afterwards |
| `shell` | `$SHELL` / `cmd.exe` | program started in new panes |
| `padding` | `10.0` | terminal padding in logical px, 0–40 |
| `gap` | `5.0` | tiling gap between split panes, 0–20 |

**Font fallback** is automatic: the bundled JetBrains Mono families are primary,
bundled Noto Emoji / Noto Color Emoji provide emoji, and missing characters are
resolved lazily against the installed system fonts (Noto Sans Mono CJK SC,
Noto Sans CJK SC, DejaVu Sans, Symbola, …). There is intentionally no
`font_fallbacks` key: an installed font that covers a glyph is used
automatically, and a named family in `font_family` is always the first choice.

### Colours and themes

| Key | Default | Notes |
|---|---|---|
| `theme` | `"pretty"` | preset name, or `"custom"` |
| `background` | `"#0d1117"` | terminal background |
| `foreground` | `"#d8dee9"` | terminal foreground |
| `cursor` | `"#88c0d0"` | cursor colour |
| `accent` | `"#7aa2f7"` | palette accent used by protocol colour queries |
| `selection_bg` / `selection_fg` | `"#334b67"` / `"#eceff4"` | selection colours |
| `palette` | `[]` | optional 16 entries, `color0` … `color15`; unset entries keep the built-in palette |
| `opacity` | `0.90` | window background alpha, 0.2–1.0 |
| `blur` | `true` | compositor blur-behind (KWin X11/Wayland) |
| `prefer_x11_for_blur` | `false` | retained for compatibility; the blur/XWayland decision is automatic |

Built-in theme presets (also in **Settings → Appearance → Theme preset**):

| `theme` value | Label |
|---|---|
| `pretty` | Pretty (default) |
| `catppuccin-mocha` | Catppuccin Mocha |
| `dracula` | Dracula |
| `nord` | Nord |
| `gruvbox-dark` | Gruvbox Dark |
| `tokyo-night` | Tokyo Night |
| `solarized-dark` | Solarized Dark |
| `one-dark` | One Dark |

A preset fills only colour keys that are **absent** from the config file. This
hand-written file gets the complete Dracula palette:

```toml
theme = "dracula"
font_size = 16.0
```

while this one keeps the explicit background and only inherits the remaining
Dracula colours:

```toml
theme = "dracula"
background = "#000000"
```

Selecting a preset in the GUI writes all colours into the draft immediately;
later per-colour edits are stored alongside `theme` and win on the next load.

### UI look and feel

| Key | Default | Notes |
|---|---|---|
| `ui_scale` | `1.0` | extra zoom for the tab bar, menus and settings panel; 0.6–2.5 (terminal cell size is unaffected) |
| `ui_accent` | `"#7aa2f7"` | accent colour of the settings/context UI |
| `ui_panel_opacity` | `1.0` | settings panel opacity, 0.5–1.0 |
| `tab_bar_visible` | `true` | show/hide the tab bar; hide it for a kitty-like chrome-free window (`Ctrl+Shift+T`, the context menu and tab shortcuts still work) |
| `tab_bar_height` | `30.0` | tab bar height in UI points, 18–64 |
| `language` | `"auto"` | `auto`, `en`, `zh-CN` |

### Cursor, bell and session behaviour

| Key | Default | Notes |
|---|---|---|
| `cursor_blink` | `true` | user setting wins over a terminal `DECSCUSR` steady style |
| `cursor_blink_interval_ms` | `530` | blink period, 100–2000 ms |
| `cursor_shape` | `"block"` | default shape: `block`, `beam`, `underline`; applications may override with DECSCUSR |
| `bell` | `"attention"` | `attention` (task-bar/window hint), `visual` (accent border flash), `none` |
| `visual_bell_duration_ms` | `120` | flash duration for `bell = "visual"`, 20–2000 ms |
| `copy_on_select` | `false` | copy the selection to the clipboard when a mouse/touch drag ends |
| `confirm_close` | `false` | ask before closing the window / quitting (including `Ctrl+Shift+Q` and closing the last tab) |
| `allow_osc52_copy` | `true` | may applications write the clipboard through OSC 52? |
| `allow_osc52_paste` | `false` | may applications **read** the clipboard through OSC 52? Opt-in: any process writing to the terminal could otherwise exfiltrate the clipboard |

The keyboard copy/paste shortcuts always work; the OSC 52 keys only gate
requests made by the shell or an application. Clipboard **reads** through
OSC 52 are disabled by default because a process could otherwise exfiltrate the
clipboard with a query escape; enable `allow_osc52_paste` only for trusted
programs. `bell = "attention"` is also what
the importer maps kitty's `enable_audio_bell` to, because Pretty has no audio
output; a red `bell = "none"` disables the behaviour entirely.

### Window

| Key | Default | Notes |
|---|---|---|
| `window_width` | `1280` | startup logical width, 320–7680 |
| `window_height` | `820` | startup logical height, 200–4320 |

The settings panel stores the values and has an **Apply window size** button; a
manual window resize is not written back to the config.

### Shortcuts

`[shortcuts]` maps a stable action name to a binding. Defaults:

```toml
[shortcuts]
copy = "Ctrl+Shift+C"
paste = "Ctrl+Shift+V"
select_all = "Ctrl+Shift+A"
new_tab = "Ctrl+Shift+T"
close_pane = "Ctrl+Shift+W"
split_right = "Ctrl+Shift+E"
split_down = "Ctrl+Shift+O"
find = "Ctrl+Shift+F"
settings = "Ctrl+,"
next_tab = "Ctrl+Tab"
prev_tab = "Ctrl+Shift+Tab"
quit = "Ctrl+Shift+Q"
zoom_in = "Ctrl+Shift+="
zoom_out = "Ctrl+Shift+-"
zoom_reset = "Ctrl+Shift+0"
```

Binding syntax: `Ctrl+`, `Shift+`, `Alt+`/`Option+`, `Super+`/`Cmd+` and a key
name (letter, digit, `F1`–`F12`, `Left`/`Right`/`Up`/`Down`, `Home`, `End`,
`PageUp`, `PageDown`, `Insert`, `Delete`, `Backspace`, `Enter`, `Tab`, `Escape`,
`Space`, punctuation such as `,` or `=`). Clearing a binding disables that
action. The settings toggle is no longer hard-coded: clearing `settings` really
disables `Ctrl+,`.

## Importing `kitty.conf`

```bash
pretty --import-kitty-conf ~/.config/kitty/kitty.conf        # merges into ~/.config/pretty/config.toml
pretty --import-kitty-conf kitty.conf --config /tmp/test.toml  # explicit target
```

The same merge is available in **Settings → General → Import kitty.conf**
(type the path, press **Import**). Importing prints a one-line summary and logs
every skipped or malformed option; an unknown option never aborts the import.

Supported options:

| kitty option | Pretty key |
|---|---|
| `font_family` | `font_family` |
| `font_size` | `font_size` |
| `foreground`, `background`, `cursor` | same names |
| `selection_foreground`, `selection_background` | `selection_fg`, `selection_bg` |
| `color0` … `color15` | `palette` entries (partial palettes keep the built-ins for unset slots) |
| `background_opacity` | `opacity` |
| `cursor_shape` | `cursor_shape` (`block`/`beam`/`underline`) |
| `cursor_blink_interval` | `cursor_blink` + `cursor_blink_interval_ms` (`0` disables blinking) |
| `scrollback_lines` | `scrollback` (`unlimited` maps to 200,000 with a warning) |
| `window_padding_width` | `padding` (first value; warning if four values are given) |
| `enable_audio_bell` | `bell = "attention"` / `"none"` + warning that there is no audio output |
| `copy_on_select` | `copy_on_select` |
| `confirm_os_window_close` | `confirm_close` (nonzero = confirm) |
| `disable_ligatures` | `ligatures` (`never` → on, `always` → off) |
| `shell` | `shell` |
| `map <keys> <action>` | `[shortcuts]` entries for new/close/next/previous tab, find, copy, paste, select-all, zoom, quit, settings |
| `include PATH` | recursive include, relative to the including file, `~` and `*` globs supported, cycle and depth checked |

Everything else (for example `tab_bar_style`, `cursor_opacity`,
`window_margin_width` or kitty-only actions such as `launch`) is listed as
unsupported in the import report; the rest of the file is still applied.

## Saving and reloading

- The settings panel edits a draft, applies it to the live renderer immediately
  and debounces the disk write; **Save now** forces the write.
- CLI overrides (`--font-size`, `--opacity`, `--no-blur`) are runtime-only and
  are not persisted.
- A malformed TOML file is reported in the log and replaced by defaults for
  that launch; the file itself is left untouched.
