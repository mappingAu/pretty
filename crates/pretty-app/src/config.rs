//! Pretty configuration model and persistence.
//!
//! This module owns every user-facing knob except the ones that describe the
//! running GPU (those live in [`crate::app::AppOptions`] / CLI overrides). It
//! also contains:
//!
//! * the built-in theme presets (`theme = "catppuccin-mocha"`, ...),
//! * a practical `kitty.conf` importer (`pretty --import-kitty-conf`,
//!   or *Settings → General → Import kitty.conf*),
//! * the validation layer that keeps hand-edited/nonsense values safe.
//!
//! The file format is TOML, so every scalar in [`Config`] is a config key with
//! the same name. See `docs/CONFIG.md` for the documented table.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use pretty_core::Rgb;
use pretty_renderer::{RendererKind, Theme};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    // ---- rendering / terminal ----
    pub renderer: String,
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub ligatures: bool,
    pub font_features: Vec<String>,
    pub opacity: f32,
    pub blur: bool,
    pub padding: f32,
    pub gap: f32,
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub accent: String,
    pub selection_bg: String,
    pub selection_fg: String,
    /// Optional ANSI palette override: up to 16 `#rrggbb` strings for
    /// `color0` … `color15`. Empty/missing entries keep the built-in palette.
    pub palette: Vec<String>,
    pub scrollback: usize,
    pub vsync: bool,
    pub shell: String,
    /// May an application read the clipboard through OSC 52?
    ///
    /// Opt-in: any process that can write to the terminal could otherwise
    /// exfiltrate the clipboard with `\e]52;c;?\a`.
    pub allow_osc52_paste: bool,
    /// May an application write the clipboard through OSC 52?
    pub allow_osc52_copy: bool,
    pub window_width: u32,
    pub window_height: u32,
    // ---- UI look and feel ----
    /// Additional egui zoom factor for the tab bar, menu and settings panel.
    pub ui_scale: f32,
    pub ui_accent: String,
    pub ui_panel_opacity: f32,
    pub theme: String,
    pub tab_bar_visible: bool,
    pub tab_bar_height: f32,
    pub language: String,
    // ---- cursor / bell / session behaviour ----
    pub cursor_blink: bool,
    pub cursor_blink_interval_ms: u32,
    /// Default cursor shape: `block`, `beam` or `underline` (DECSCUSR can still
    /// override it at runtime).
    pub cursor_shape: String,
    /// Bell behaviour: `attention` (window attention hint), `visual` (border
    /// flash) or `none`.
    pub bell: String,
    pub visual_bell_duration_ms: u32,
    /// Copy the primary selection to the clipboard when a mouse selection ends.
    pub copy_on_select: bool,
    /// Ask for confirmation before closing the window or quitting.
    pub confirm_close: bool,
    pub prefer_x11_for_blur: bool,
    /// Action name -> key binding, e.g. `new_tab = "Ctrl+Shift+T"`.
    pub shortcuts: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            renderer: "auto".into(),
            font_family: "JetBrains Mono".into(),
            font_size: 15.0,
            line_height: 1.18,
            ligatures: true,
            font_features: vec!["liga".into(), "calt".into()],
            opacity: 0.9,
            blur: true,
            padding: 10.0,
            gap: 5.0,
            background: "#0d1117".into(),
            foreground: "#d8dee9".into(),
            cursor: "#88c0d0".into(),
            accent: "#7aa2f7".into(),
            selection_bg: "#334b67".into(),
            selection_fg: "#eceff4".into(),
            palette: Vec::new(),
            scrollback: 10_000,
            vsync: true,
            shell: default_shell(),
            allow_osc52_paste: false,
            allow_osc52_copy: true,
            window_width: 1280,
            window_height: 820,
            ui_scale: 1.0,
            ui_accent: "#7aa2f7".into(),
            ui_panel_opacity: 1.0,
            theme: "pretty".into(),
            tab_bar_visible: true,
            tab_bar_height: 30.0,
            language: "auto".into(),
            cursor_blink: true,
            cursor_blink_interval_ms: 530,
            cursor_shape: "block".into(),
            bell: "attention".into(),
            visual_bell_duration_ms: 120,
            copy_on_select: false,
            confirm_close: false,
            prefer_x11_for_blur: false,
            shortcuts: default_shortcuts(),
        }
    }
}

pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

/// Built-in colour themes. A preset only fills colour keys that are *absent*
/// from a hand-written config file; explicit user colours always win.
pub struct ThemePreset {
    pub name: &'static str,
    pub label: &'static str,
    pub background: &'static str,
    pub foreground: &'static str,
    pub cursor: &'static str,
    pub accent: &'static str,
    pub selection_bg: &'static str,
    pub selection_fg: &'static str,
    pub palette: [&'static str; 16],
}

pub const THEME_PRESETS: &[ThemePreset] = &[
    ThemePreset {
        name: "pretty",
        label: "Pretty (default)",
        background: "#0d1117",
        foreground: "#d8dee9",
        cursor: "#88c0d0",
        accent: "#7aa2f7",
        selection_bg: "#334b67",
        selection_fg: "#eceff4",
        palette: [
            "#1d1f21", "#cc5555", "#4caf50", "#d7a843", "#4f9bd8", "#c678dd", "#34b6b6",
            "#d8dee9", "#606670", "#eb6f6f", "#7ed07e", "#f0c674", "#7aa2f7", "#d89fe0",
            "#6dd3d3", "#ffffff",
        ],
    },
    ThemePreset {
        name: "catppuccin-mocha",
        label: "Catppuccin Mocha",
        background: "#1e1e2e",
        foreground: "#cdd6f4",
        cursor: "#f5e0dc",
        accent: "#89b4fa",
        selection_bg: "#585b70",
        selection_fg: "#cdd6f4",
        palette: [
            "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5",
            "#bac2de", "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7",
            "#94e2d5", "#a6adc8",
        ],
    },
    ThemePreset {
        name: "dracula",
        label: "Dracula",
        background: "#282a36",
        foreground: "#f8f8f2",
        cursor: "#f8f8f2",
        accent: "#bd93f9",
        selection_bg: "#44475a",
        selection_fg: "#f8f8f2",
        palette: [
            "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd",
            "#f8f8f2", "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df",
            "#a4ffff", "#ffffff",
        ],
    },
    ThemePreset {
        name: "nord",
        label: "Nord",
        background: "#2e3440",
        foreground: "#d8dee9",
        cursor: "#d8dee9",
        accent: "#88c0d0",
        selection_bg: "#434c5e",
        selection_fg: "#eceff4",
        palette: [
            "#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0",
            "#e5e9f0", "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead",
            "#8fbcbb", "#eceff4",
        ],
    },
    ThemePreset {
        name: "gruvbox-dark",
        label: "Gruvbox Dark",
        background: "#282828",
        foreground: "#ebdbb2",
        cursor: "#ebdbb2",
        accent: "#d79921",
        selection_bg: "#504945",
        selection_fg: "#fbf1c7",
        palette: [
            "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a",
            "#a89984", "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b",
            "#8ec07c", "#ebdbb2",
        ],
    },
    ThemePreset {
        name: "tokyo-night",
        label: "Tokyo Night",
        background: "#1a1b26",
        foreground: "#c0caf5",
        cursor: "#c0caf5",
        accent: "#7aa2f7",
        selection_bg: "#33467c",
        selection_fg: "#c0caf5",
        palette: [
            "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff",
            "#a9b1d6", "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7",
            "#7dcfff", "#c0caf5",
        ],
    },
    ThemePreset {
        name: "solarized-dark",
        label: "Solarized Dark",
        background: "#002b36",
        foreground: "#839496",
        cursor: "#93a1a1",
        accent: "#268bd2",
        selection_bg: "#073642",
        selection_fg: "#93a1a1",
        palette: [
            "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198",
            "#eee8d5", "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4",
            "#93a1a1", "#fdf6e3",
        ],
    },
    ThemePreset {
        name: "one-dark",
        label: "One Dark",
        background: "#282c34",
        foreground: "#abb2bf",
        cursor: "#528bff",
        accent: "#61afef",
        selection_bg: "#3e4451",
        selection_fg: "#abb2bf",
        palette: [
            "#282c34", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2",
            "#abb2bf", "#5c6370", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd",
            "#56b6c2", "#ffffff",
        ],
    },
];

pub fn theme_preset(name: &str) -> Option<&'static ThemePreset> {
    THEME_PRESETS.iter().find(|preset| preset.name == name)
}

impl Config {
    /// Apply a preset to every colour key (used by the GUI preset selector and
    /// by tests). Returns `false` for an unknown preset name.
    pub fn apply_theme_preset(&mut self, name: &str) -> bool {
        let Some(preset) = theme_preset(name) else { return false };
        self.theme = preset.name.to_owned();
        self.background = preset.background.to_owned();
        self.foreground = preset.foreground.to_owned();
        self.cursor = preset.cursor.to_owned();
        self.accent = preset.accent.to_owned();
        self.selection_bg = preset.selection_bg.to_owned();
        self.selection_fg = preset.selection_fg.to_owned();
        self.ui_accent = preset.accent.to_owned();
        self.palette = preset.palette.iter().map(|c| (*c).to_owned()).collect();
        true
    }

    /// Preset colours are defaults: a hand-written config may name only
    /// `theme = "dracula"` and still get the complete palette, while explicit
    /// colour keys in the same file are never overwritten.
    fn apply_preset_defaults(&mut self, keys: &[String]) {
        if self.theme == "pretty" || self.theme == "custom" {
            return;
        }
        let Some(preset) = theme_preset(&self.theme) else { return };
        let present = |key: &str| keys.iter().any(|k| k == key);
        if !present("background") {
            self.background = preset.background.to_owned();
        }
        if !present("foreground") {
            self.foreground = preset.foreground.to_owned();
        }
        if !present("cursor") {
            self.cursor = preset.cursor.to_owned();
        }
        if !present("accent") {
            self.accent = preset.accent.to_owned();
        }
        if !present("selection_bg") {
            self.selection_bg = preset.selection_bg.to_owned();
        }
        if !present("selection_fg") {
            self.selection_fg = preset.selection_fg.to_owned();
        }
        if !present("ui_accent") {
            self.ui_accent = preset.accent.to_owned();
        }
        if !present("palette") {
            self.palette = preset.palette.iter().map(|c| (*c).to_owned()).collect();
        }
    }

    /// Fill a partial `palette` with the built-in ANSI colours so the GUI can
    /// always edit 16 slots.
    pub fn ensure_palette(&mut self) {
        let defaults = Theme::default().palette;
        while self.palette.len() < 16 {
            let rgb = defaults[self.palette.len()];
            self.palette
                .push(format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b));
        }
        self.palette.truncate(16);
    }

    pub fn theme(&self) -> Theme {
        let mut t = Theme::default();
        t.background = parse_color(&self.background, t.background);
        t.foreground = parse_color(&self.foreground, t.foreground);
        t.cursor = parse_color(&self.cursor, t.cursor);
        t.accent = parse_color(&self.accent, t.accent);
        t.selection_bg = parse_color(&self.selection_bg, t.selection_bg);
        t.selection_fg = parse_color(&self.selection_fg, t.selection_fg);
        for (i, value) in self.palette.iter().take(16).enumerate() {
            if let Some(rgb) = Rgb::parse_hex(value) {
                t.palette[i] = rgb;
            }
        }
        t.opacity = self.opacity.clamp(0.2, 1.0);
        t.font_family = self.font_family.clone();
        t.font_size = self.font_size.clamp(7.0, 42.0);
        t.font_features = self.font_features.clone();
        t.ligatures = self.ligatures;
        t.line_height = self.line_height.clamp(0.85, 1.8);
        t.padding = self.padding.clamp(0.0, 40.0);
        t.gap = self.gap.clamp(0.0, 20.0);
        t.cursor_blink = self.cursor_blink;
        t
    }

    pub fn shortcut(&self, action: &str) -> Option<&str> {
        self.shortcuts.get(action).map(String::as_str)
    }

    pub fn validate(&mut self) {
        sane_f32(&mut self.font_size, 15.0);
        sane_f32(&mut self.line_height, 1.18);
        sane_f32(&mut self.opacity, 0.9);
        sane_f32(&mut self.padding, 10.0);
        sane_f32(&mut self.gap, 5.0);
        sane_f32(&mut self.ui_panel_opacity, 1.0);
        sane_f32(&mut self.ui_scale, 1.0);
        sane_f32(&mut self.tab_bar_height, 30.0);
        self.font_size = self.font_size.clamp(7.0, 42.0);
        self.line_height = self.line_height.clamp(0.85, 1.8);
        self.opacity = self.opacity.clamp(0.2, 1.0);
        self.padding = self.padding.clamp(0.0, 40.0);
        self.gap = self.gap.clamp(0.0, 20.0);
        // 1M lines of wide cells is a multi-GB hazard; the GUI slider already
        // caps at 200k, so apply the same cap to hand-edited configs.
        self.scrollback = self.scrollback.min(200_000);
        self.window_width = self.window_width.clamp(320, 7680);
        self.window_height = self.window_height.clamp(200, 4320);
        self.ui_panel_opacity = self.ui_panel_opacity.clamp(0.5, 1.0);
        self.ui_scale = self.ui_scale.clamp(0.6, 2.5);
        self.tab_bar_height = self.tab_bar_height.clamp(18.0, 64.0);
        self.cursor_blink_interval_ms = self.cursor_blink_interval_ms.clamp(100, 2_000);
        self.visual_bell_duration_ms = self.visual_bell_duration_ms.clamp(20, 2_000);
        match self.cursor_shape.as_str() {
            "block" | "beam" | "underline" => {}
            other => {
                log::warn!("unsupported cursor_shape '{other}'; using block");
                self.cursor_shape = "block".into();
            }
        }
        match self.bell.as_str() {
            "attention" | "visual" | "none" => {}
            other => {
                log::warn!("unsupported bell '{other}'; using attention");
                self.bell = "attention".into();
            }
        }
        if self.theme != "custom" && theme_preset(&self.theme).is_none() {
            log::warn!("unknown theme '{}'; using custom colours", self.theme);
            self.theme = "custom".into();
        }
        // Partial palettes are fine (unset entries keep the built-in colour);
        // reject only values that are actually malformed.
        for value in &self.palette {
            if !value.is_empty() && Rgb::parse_hex(value).is_none() {
                log::warn!("invalid palette colour '{value}'; entry ignored");
            }
        }
        if self.shell.trim().is_empty() {
            self.shell = default_shell();
        }
        match self.language.as_str() {
            "auto" | "en" | "en-US" | "zh" | "zh-CN" => {}
            other => {
                log::warn!("unsupported language '{other}'; using auto");
                self.language = "auto".into();
            }
        }
        if self.renderer.parse::<RendererKind>().is_err() {
            log::warn!("invalid renderer '{}'; falling back to auto", self.renderer);
            self.renderer = "auto".into();
        }
        for (action, binding) in default_shortcuts() {
            self.shortcuts.entry(action).or_insert(binding);
        }
        // A binding for an unknown action is harmless but likely a typo.
        let defaults = default_shortcuts();
        let known: HashSet<&str> = defaults.keys().map(String::as_str).collect();
        for action in self.shortcuts.keys() {
            if !known.contains(action.as_str()) {
                log::warn!("unknown shortcut action '{action}' (ignored; see docs/CONFIG.md)");
            }
        }
        for (name, value) in [
            ("background", &self.background),
            ("foreground", &self.foreground),
            ("cursor", &self.cursor),
            ("accent", &self.accent),
            ("selection_bg", &self.selection_bg),
            ("selection_fg", &self.selection_fg),
            ("ui_accent", &self.ui_accent),
        ] {
            if Rgb::parse_hex(value).is_none() {
                log::warn!("invalid {name} colour '{value}'; using default");
            }
        }
    }
}

fn sane_f32(value: &mut f32, fallback: f32) {
    if !value.is_finite() || *value <= 0.0 {
        *value = fallback;
    }
}

pub fn default_shortcuts() -> HashMap<String, String> {
    [
        ("copy", "Ctrl+Shift+C"),
        ("paste", "Ctrl+Shift+V"),
        ("select_all", "Ctrl+Shift+A"),
        ("new_tab", "Ctrl+Shift+T"),
        ("close_pane", "Ctrl+Shift+W"),
        ("split_right", "Ctrl+Shift+E"),
        ("split_down", "Ctrl+Shift+O"),
        ("find", "Ctrl+Shift+F"),
        ("settings", "Ctrl+,"),
        ("next_tab", "Ctrl+Tab"),
        ("prev_tab", "Ctrl+Shift+Tab"),
        ("quit", "Ctrl+Shift+Q"),
        ("zoom_in", "Ctrl+Shift+="),
        ("zoom_out", "Ctrl+Shift+-"),
        ("zoom_reset", "Ctrl+Shift+0"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

pub fn parse_color(s: &str, fallback: Rgb) -> Rgb {
    Rgb::parse_hex(s).unwrap_or(fallback)
}

pub fn config_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Ok(path) = std::env::var("PRETTY_CONFIG") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Some(proj) = directories::ProjectDirs::from("org", "pretty", "pretty") {
        proj.config_dir().join("config.toml")
    } else {
        PathBuf::from("pretty.toml")
    }
}

pub fn load(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<toml::Value>(&text) {
            Ok(value) => {
                let keys: Vec<String> = value
                    .as_table()
                    .map(|table| table.keys().cloned().collect())
                    .unwrap_or_default();
                match value.try_into::<Config>() {
                    Ok(mut config) => {
                        config.apply_preset_defaults(&keys);
                        config.validate();
                        log::info!("configuration loaded from {}", path.display());
                        config
                    }
                    Err(e) => {
                        log::error!("failed to parse {}: {e}; using defaults", path.display());
                        Config::default()
                    }
                }
            }
            Err(e) => {
                log::error!("failed to parse {}: {e}; using defaults", path.display());
                Config::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let config = Config::default();
            if let Err(e) = save(path, &config) {
                log::warn!("could not write default config {}: {e}", path.display());
            }
            config
        }
        Err(e) => {
            log::error!("failed to read {}: {e}; using defaults", path.display());
            Config::default()
        }
    }
}

pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// kitty.conf importer
// ---------------------------------------------------------------------------

/// Result of importing a `kitty.conf`. `applied` counts recognised options;
/// unknown kitty options are recorded (never fatal) so the user can audit what
/// was skipped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KittyImportReport {
    pub applied: usize,
    pub warnings: Vec<String>,
    pub unsupported: Vec<String>,
}

impl KittyImportReport {
    pub fn summary(&self) -> String {
        format!(
            "{} option(s) imported, {} unsupported key(s), {} warning(s)",
            self.applied,
            self.unsupported.len(),
            self.warnings.len()
        )
    }

    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }
}

/// Parse `source` and apply every supported kitty option to `config`.
pub fn import_kitty_conf(source: &Path, config: &mut Config) -> Result<KittyImportReport, String> {
    let mut report = KittyImportReport::default();
    let mut seen = HashSet::new();
    parse_kitty_file(source, config, &mut report, &mut seen, 0)?;
    config.validate();
    Ok(report)
}

/// Import into the config stored at `target`, then write it back atomically.
pub fn import_kitty_conf_to_file(
    source: &Path,
    target: &Path,
) -> Result<KittyImportReport, String> {
    let mut config = load(target);
    let report = import_kitty_conf(source, &mut config)?;
    save(target, &config)?;
    Ok(report)
}

const MAX_INCLUDE_DEPTH: usize = 8;

fn parse_kitty_file(
    path: &Path,
    config: &mut Config,
    report: &mut KittyImportReport,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_INCLUDE_DEPTH {
        report.warn(format!(
            "include depth limit ({MAX_INCLUDE_DEPTH}) reached at {}",
            path.display()
        ));
        return Ok(());
    }
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canonical.clone()) {
        report.warn(format!("include cycle detected at {}; skipped", path.display()));
        return Ok(());
    }
    let text = std::fs::read_to_string(&canonical)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let base_dir = canonical
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(PathBuf::new);
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once(char::is_whitespace) else {
            report.warn(format!(
                "{}:{line_no}: missing value for '{line}'",
                path.display()
            ));
            continue;
        };
        let value = strip_quotes(rest.trim());
        if key.eq_ignore_ascii_case("include") {
            for include in expand_include(&base_dir, value) {
                if let Err(e) = parse_kitty_file(&include, config, report, seen, depth + 1) {
                    report.warn(format!("{}:{line_no}: {e}", path.display()));
                }
            }
            continue;
        }
        if !apply_kitty_option(key, value, config, report) {
            if !report.unsupported.iter().any(|k| k.as_str() == key) {
                report.unsupported.push(key.to_owned());
            }
            report.warn(format!(
                "{}:{line_no}: unsupported kitty option '{key}' (ignored)",
                path.display()
            ));
        }
    }
    Ok(())
}

fn strip_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn kitty_color(value: &str) -> Option<String> {
    Rgb::parse_hex(strip_quotes(value.trim()))
        .map(|rgb| format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b))
}

fn kitty_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn set_palette_slot(config: &mut Config, index: usize, color: String) {
    if config.palette.len() < 16 {
        config.palette.resize_with(16, String::new);
    }
    config.palette[index] = color;
}

/// Apply one `key value` pair. Returns `false` when the key is not supported by
/// Pretty (the caller records it and keeps going).
fn apply_kitty_option(
    key: &str,
    value: &str,
    config: &mut Config,
    report: &mut KittyImportReport,
) -> bool {
    let value = strip_quotes(value.trim());
    match key {
        "font_family" => {
            config.font_family = value.to_owned();
            report.applied += 1;
        }
        "font_size" => match value.parse::<f32>() {
            Ok(size) => {
                config.font_size = size;
                report.applied += 1;
            }
            Err(_) => report.warn(format!("font_size: '{value}' is not a number")),
        },
        "foreground" => {
            set_kitty_color(config, report, "foreground", value, |c, v| c.foreground = v);
        }
        "background" => {
            set_kitty_color(config, report, "background", value, |c, v| c.background = v);
        }
        "cursor" => {
            set_kitty_color(config, report, "cursor", value, |c, v| c.cursor = v);
        }
        "selection_foreground" => {
            set_kitty_color(config, report, "selection_foreground", value, |c, v| c.selection_fg = v);
        }
        "selection_background" => {
            set_kitty_color(config, report, "selection_background", value, |c, v| c.selection_bg = v);
        }
        "background_opacity" => match value.parse::<f32>() {
            Ok(opacity) => {
                config.opacity = opacity;
                report.applied += 1;
            }
            Err(_) => report.warn(format!("background_opacity: '{value}' is not a number")),
        },
        "cursor_shape" => match value.to_ascii_lowercase().as_str() {
            "block" | "beam" | "underline" => {
                config.cursor_shape = value.to_ascii_lowercase();
                report.applied += 1;
            }
            other => report.warn(format!("cursor_shape: unsupported shape '{other}'")),
        },
        "cursor_blink_interval" => match value.parse::<f32>() {
            Ok(seconds) => {
                if seconds <= 0.0 {
                    config.cursor_blink = false;
                } else {
                    config.cursor_blink = true;
                    config.cursor_blink_interval_ms = (seconds * 1000.0).round() as u32;
                }
                report.applied += 1;
            }
            Err(_) => report.warn(format!("cursor_blink_interval: '{value}' is not a number")),
        },
        "scrollback_lines" => match value.parse::<usize>() {
            Ok(lines) => {
                config.scrollback = lines;
                report.applied += 1;
            }
            Err(_) => match value.to_ascii_lowercase().as_str() {
                "unlimited" => {
                    config.scrollback = 200_000;
                    report.applied += 1;
                    report.warn("scrollback_lines: 'unlimited' capped at 200000 lines");
                }
                _ => report.warn(format!("scrollback_lines: '{value}' is not an integer")),
            },
        },
        "window_padding_width" => {
            // kitty accepts one to four values; use the first (uniform padding).
            let first = value.split_whitespace().next().unwrap_or(value);
            match first.parse::<f32>() {
                Ok(padding) => {
                    config.padding = padding;
                    report.applied += 1;
                    if value.split_whitespace().count() > 1 {
                        report.warn(
                            "window_padding_width: only the first value is supported (uniform padding)",
                        );
                    }
                }
                Err(_) => report.warn(format!("window_padding_width: '{first}' is not a number")),
            }
        }
        "enable_audio_bell" => match kitty_bool(value) {
            Some(enabled) => {
                // Pretty has no audio output; map kitty's audio bell to the
                // closest available attention/flash behaviour.
                config.bell = if enabled { "attention".into() } else { "none".into() };
                report.applied += 1;
                if enabled {
                    report.warn(
                        "enable_audio_bell: Pretty has no audio bell; mapped to the 'attention' bell",
                    );
                }
            }
            None => report.warn(format!("enable_audio_bell: '{value}' is not a boolean")),
        },
        "copy_on_select" => match kitty_bool(value) {
            Some(enabled) => {
                config.copy_on_select = enabled;
                report.applied += 1;
            }
            None => report.warn(format!("copy_on_select: '{value}' is not a boolean")),
        },
        "confirm_os_window_close" => match value.parse::<i32>() {
            Ok(threshold) => {
                config.confirm_close = threshold != 0;
                report.applied += 1;
            }
            Err(_) => report.warn(format!(
                "confirm_os_window_close: '{value}' is not an integer"
            )),
        },
        "disable_ligatures" => match value.to_ascii_lowercase().as_str() {
            "never" => {
                config.ligatures = true;
                report.applied += 1;
            }
            "always" | "cursor" | "unfocused" => {
                config.ligatures = false;
                report.applied += 1;
                report.warn(
                    "disable_ligatures: only 'never'/'always' semantics are supported; ligatures disabled",
                );
            }
            _ => report.warn(format!("disable_ligatures: '{value}' is not a known mode")),
        },
        "shell" => {
            config.shell = value.to_owned();
            report.applied += 1;
        }
        "map" => {
            apply_kitty_map(value, config, report);
        }
        _ => {
            if let Some(index) = kitty_palette_index(key) {
                match kitty_color(value) {
                    Some(color) => {
                        set_palette_slot(config, index, color);
                        report.applied += 1;
                    }
                    None => report.warn(format!("{key}: invalid colour '{value}'")),
                }
                return true;
            }
            return false;
        }
    }
    true
}

fn set_kitty_color(
    config: &mut Config,
    report: &mut KittyImportReport,
    name: &str,
    value: &str,
    assign: impl FnOnce(&mut Config, String),
) {
    match kitty_color(value) {
        Some(color) => {
            assign(config, color);
            report.applied += 1;
        }
        None => report.warn(format!("{name}: invalid colour '{value}'")),
    }
}

fn kitty_palette_index(key: &str) -> Option<usize> {
    let number = key.strip_prefix("color")?;
    let index = number.parse::<usize>().ok()?;
    (index < 16).then_some(index)
}

fn kitty_key_name(token: &str) -> Option<String> {
    const NAMED: &[(&str, &str)] = &[
        ("left", "Left"),
        ("right", "Right"),
        ("up", "Up"),
        ("down", "Down"),
        ("pageup", "PageUp"),
        ("page_down", "PageDown"),
        ("pagedown", "PageDown"),
        ("home", "Home"),
        ("end", "End"),
        ("insert", "Insert"),
        ("delete", "Delete"),
        ("backspace", "Backspace"),
        ("enter", "Enter"),
        ("return", "Enter"),
        ("tab", "Tab"),
        ("escape", "Escape"),
        ("esc", "Escape"),
        ("space", "Space"),
        ("plus", "="),
        ("equal", "="),
        ("minus", "-"),
        ("comma", ","),
        ("period", "."),
        ("slash", "/"),
        ("backslash", "\\"),
        ("semicolon", ";"),
        ("apostrophe", "'"),
        ("grave", "`"),
    ];
    if let Some((_, name)) = NAMED.iter().find(|(kitty, _)| *kitty == token) {
        return Some((*name).to_owned());
    }
    if token.len() == 1 {
        return Some(match token.chars().next()? {
            ch if ch.is_ascii_alphabetic() => ch.to_ascii_uppercase().to_string(),
            ch => ch.to_string(),
        });
    }
    if let Some(number) = token.strip_prefix('f') {
        if number.parse::<u8>().is_ok() {
            return Some(format!("F{number}"));
        }
    }
    None
}

/// Translate a kitty key combo (`ctrl+shift+t`, `alt+left`, `super+enter`) into
/// Pretty's binding syntax (`Ctrl+Shift+T`, `Alt+Left`, ...).
pub fn kitty_key_binding(combo: &str) -> Option<String> {
    let mut modifiers: Vec<&'static str> = Vec::new();
    let mut key: Option<String> = None;
    for token in combo.split('+').map(str::trim).filter(|t| !t.is_empty()) {
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => push_mod(&mut modifiers, "Ctrl"),
            "shift" => push_mod(&mut modifiers, "Shift"),
            "alt" | "opt" | "option" => push_mod(&mut modifiers, "Alt"),
            "super" | "cmd" | "meta" | "win" => push_mod(&mut modifiers, "Super"),
            other => match kitty_key_name(other) {
                Some(name) => key = Some(name),
                None => return None,
            },
        }
    }
    let mut binding = modifiers.join("+");
    if !binding.is_empty() {
        binding.push('+');
    }
    binding.push_str(&key?);
    Some(binding)
}

fn push_mod(modifiers: &mut Vec<&'static str>, modifier: &'static str) {
    if !modifiers.contains(&modifier) {
        modifiers.push(modifier);
    }
}

fn kitty_action_for(name: &str) -> Option<&'static str> {
    Some(match name {
        "new_tab" => "new_tab",
        "close_tab" | "close_window" | "close_pane" => "close_pane",
        "next_tab" => "next_tab",
        "previous_tab" => "prev_tab",
        "start_search" | "find" => "find",
        "copy_to_clipboard" | "copy_and_clear_or_interrupt" => "copy",
        "paste_from_clipboard" | "paste_from_selection" => "paste",
        "select_all" => "select_all",
        "increase_font_size" => "zoom_in",
        "decrease_font_size" => "zoom_out",
        "reset_font_size" => "zoom_reset",
        "quit" => "quit",
        "open_settings" => "settings",
        _ => return None,
    })
}

fn apply_kitty_map(value: &str, config: &mut Config, report: &mut KittyImportReport) {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let Some(action_pos) = parts
        .iter()
        .position(|part| kitty_action_for(&part.to_ascii_lowercase()).is_some())
    else {
        report.warn(format!(
            "map: unsupported kitty action in '{value}' (ignored)"
        ));
        return;
    };
    let action = kitty_action_for(&parts[action_pos].to_ascii_lowercase()).unwrap();
    let keys = &parts[..action_pos];
    if keys.is_empty() {
        report.warn(format!("map: no key combo in '{value}'"));
        return;
    }
    for combo in keys {
        match kitty_key_binding(combo) {
            Some(binding) => {
                config.shortcuts.insert(action.to_owned(), binding);
                report.applied += 1;
            }
            None => report.warn(format!("map: unsupported key combo '{combo}'")),
        }
    }
}

fn expand_include(base_dir: &Path, value: &str) -> Vec<PathBuf> {
    let raw = strip_quotes(value.trim());
    let path = if let Some(rest) = raw.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(raw))
    } else if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        base_dir.join(raw)
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return vec![path];
    };
    if !name.contains('*') {
        return vec![path];
    }
    let dir = path.parent().unwrap_or(base_dir).to_path_buf();
    let mut matches = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if glob_match(name, &file_name) {
                matches.push(entry.path());
            }
        }
    }
    matches.sort();
    matches
}

fn glob_match(pattern: &str, name: &str) -> bool {
    // Only `*` is meaningful in kitty include patterns; everything else is
    // matched literally.
    fn helper(pattern: &[char], name: &[char]) -> bool {
        match pattern.split_first() {
            None => name.is_empty(),
            Some((&'*', rest)) => {
                if rest.is_empty() {
                    true
                } else {
                    (0..=name.len()).any(|offset| helper(rest, &name[offset..]))
                }
            }
            Some((&expected, rest)) => match name.split_first() {
                Some((&actual, tail)) => actual == expected && helper(rest, tail),
                None => false,
            },
        }
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    helper(&pattern, &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp(name: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "pretty-config-test-{}-{count}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_round_trip_through_toml() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(config, back);
        let theme = config.theme();
        assert_eq!(theme.font_size, 15.0);
        assert!(theme.cursor_blink);
    }

    #[test]
    fn validate_clamps_new_fields_and_rejects_bad_enums() {
        let mut config = Config::default();
        config.ui_scale = f32::NAN;
        config.tab_bar_height = 1000.0;
        config.cursor_blink_interval_ms = 0;
        config.visual_bell_duration_ms = 1;
        config.cursor_shape = "triangle".into();
        config.bell = "loud".into();
        config.theme = "not-a-theme".into();
        config.validate();
        assert_eq!(config.ui_scale, 1.0);
        assert_eq!(config.tab_bar_height, 64.0);
        assert_eq!(config.cursor_blink_interval_ms, 100);
        assert_eq!(config.visual_bell_duration_ms, 20);
        assert_eq!(config.cursor_shape, "block");
        assert_eq!(config.bell, "attention");
        assert_eq!(config.theme, "custom");
    }

    #[test]
    fn every_theme_preset_has_valid_colours() {
        for preset in THEME_PRESETS {
            for color in [
                preset.background,
                preset.foreground,
                preset.cursor,
                preset.accent,
                preset.selection_bg,
                preset.selection_fg,
            ]
            .into_iter()
            .chain(preset.palette.iter().copied())
            {
                assert!(
                    Rgb::parse_hex(color).is_some(),
                    "preset {} has invalid colour {color}",
                    preset.name
                );
            }
        }
        // The default theme must reproduce the built-in default colours.
        let mut config = Config::default();
        assert!(config.apply_theme_preset("pretty"));
        assert_eq!(config.background, Config::default().background);
        assert_eq!(config.palette.len(), 16);
    }

    #[test]
    fn preset_fills_absent_colours_but_keeps_explicit_ones() {
        let dir = unique_temp("preset");
        let path = dir.join("config.toml");
        std::fs::write(&path, "theme = \"dracula\"\nfont_size = 13.0\n").unwrap();
        let config = load(&path);
        assert_eq!(config.theme, "dracula");
        assert_eq!(config.background, "#282a36");
        assert_eq!(config.foreground, "#f8f8f2");
        assert_eq!(config.palette.len(), 16);
        assert_eq!(config.font_size, 13.0);

        std::fs::write(
            &path,
            "theme = \"dracula\"\nbackground = \"#000000\"\n",
        )
        .unwrap();
        let config = load(&path);
        assert_eq!(config.background, "#000000");
        assert_eq!(config.foreground, "#f8f8f2");
    }

    #[test]
    fn palette_override_reaches_renderer_theme() {
        let mut config = Config::default();
        let mut palette = vec![String::new(); 16];
        palette[1] = "#112233".into();
        config.palette = palette;
        let theme = config.theme();
        assert_eq!(theme.palette[1], Rgb::new(0x11, 0x22, 0x33));
        // Unset slots keep the built-in palette.
        assert_eq!(theme.palette[2], Theme::default().palette[2]);
    }

    #[test]
    fn kitty_import_common_options_and_unknown_keys() {
        let dir = unique_temp("kitty");
        let source = dir.join("kitty.conf");
        std::fs::write(
            &source,
            "# comment line\n\
             font_family   \"Hack Nerd Font\"\n\
             font_size 13.5\n\
             foreground #c0caf5\n\
             background #1a1b26\n\
             cursor #f7768e\n\
             selection_foreground #c0caf5\n\
             selection_background #33467c\n\
             cursor_shape beam\n\
             cursor_blink_interval 0\n\
             scrollback_lines 50000\n\
             window_padding_width 6\n\
             background_opacity 0.85\n\
             enable_audio_bell no\n\
             copy_on_select yes\n\
             confirm_os_window_close 0\n\
             color0 #15161e\n\
             color12 #7aa2f7\n\
             map ctrl+shift+t new_tab\n\
             map ctrl+shift+c copy_to_clipboard\n\
             map alt+left previous_tab\n\
             tab_bar_style powerline\n\
             totally_unknown_option yes\n",
        )
        .unwrap();
        let mut config = Config::default();
        let report = import_kitty_conf(&source, &mut config).unwrap();
        assert_eq!(config.font_family, "Hack Nerd Font");
        assert_eq!(config.font_size, 13.5);
        assert_eq!(config.foreground, "#c0caf5");
        assert_eq!(config.background, "#1a1b26");
        assert_eq!(config.selection_bg, "#33467c");
        assert_eq!(config.cursor_shape, "beam");
        assert!(!config.cursor_blink);
        assert_eq!(config.scrollback, 50_000);
        assert_eq!(config.padding, 6.0);
        assert_eq!(config.opacity, 0.85);
        assert_eq!(config.bell, "none");
        assert!(config.copy_on_select);
        assert!(!config.confirm_close);
        assert_eq!(config.palette[0], "#15161e");
        assert_eq!(config.palette[12], "#7aa2f7");
        assert_eq!(config.palette.len(), 16);
        assert_eq!(config.shortcut("new_tab"), Some("Ctrl+Shift+T"));
        assert_eq!(config.shortcut("copy"), Some("Ctrl+Shift+C"));
        assert_eq!(config.shortcut("prev_tab"), Some("Alt+Left"));
        assert!(report.unsupported.contains(&"tab_bar_style".to_owned()));
        assert!(report
            .unsupported
            .contains(&"totally_unknown_option".to_owned()));
        assert!(report.warnings.iter().any(|w| w.contains("tab_bar_style")));
        assert!(report.applied >= 18);
    }

    #[test]
    fn kitty_import_handles_bad_values_without_panicking() {
        let dir = unique_temp("kitty-bad");
        let source = dir.join("kitty.conf");
        std::fs::write(
            &source,
            "font_size many\ncursor_shape triangle\nforeground not-a-colour\n\
             background_opacity abc\ncopy_on_select maybe\nscrollback_lines lots\n",
        )
        .unwrap();
        let config_before = Config::default();
        let mut config = config_before.clone();
        let report = import_kitty_conf(&source, &mut config).unwrap();
        assert!(report.warnings.len() >= 6);
        assert_eq!(config.font_size, config_before.font_size);
        assert_eq!(config.cursor_shape, "block");
        assert_eq!(config.background, config_before.background);
        assert_eq!(config.opacity, config_before.opacity);
    }

    #[test]
    fn kitty_import_include_glob_and_cycle() {
        let dir = unique_temp("kitty-include");
        let main = dir.join("kitty.conf");
        let colors = dir.join("colors.conf");
        let extra = dir.join("extra-1.conf");
        std::fs::write(&main, "font_size 12\ninclude colors.conf\ninclude extra-*.conf\n").unwrap();
        std::fs::write(&colors, "foreground #abcdef\n").unwrap();
        std::fs::write(&extra, "background #123456\ninclude kitty.conf\n").unwrap();
        let mut config = Config::default();
        let report = import_kitty_conf(&main, &mut config).unwrap();
        assert_eq!(config.font_size, 12.0);
        assert_eq!(config.foreground, "#abcdef");
        assert_eq!(config.background, "#123456");
        // The back-reference to the main file must be reported, not recursed.
        assert!(report.warnings.iter().any(|w| w.contains("cycle")));
    }

    #[test]
    fn kitty_import_missing_file_is_an_error() {
        let mut config = Config::default();
        let error = import_kitty_conf(Path::new("/definitely/not/a/kitty.conf"), &mut config);
        assert!(error.is_err());
    }

    #[test]
    fn kitty_key_combo_translation() {
        assert_eq!(kitty_key_binding("ctrl+shift+t").as_deref(), Some("Ctrl+Shift+T"));
        assert_eq!(kitty_key_binding("alt+left").as_deref(), Some("Alt+Left"));
        assert_eq!(kitty_key_binding("super+enter").as_deref(), Some("Super+Enter"));
        assert_eq!(kitty_key_binding("ctrl+,").as_deref(), Some("Ctrl+,"));
        assert_eq!(kitty_key_binding("ctrl+shift+plus").as_deref(), Some("Ctrl+Shift+="));
        assert_eq!(kitty_key_binding("ctrl+f5").as_deref(), Some("Ctrl+F5"));
        assert_eq!(kitty_key_binding("bogus+key"), None);
    }

    #[test]
    fn importing_to_file_persists_the_result() {
        let dir = unique_temp("kitty-persist");
        let source = dir.join("kitty.conf");
        let target = dir.join("config.toml");
        std::fs::write(&source, "font_size 17\ncursor_shape underline\n").unwrap();
        let report = import_kitty_conf_to_file(&source, &target).unwrap();
        assert!(report.applied >= 2);
        let loaded = load(&target);
        assert_eq!(loaded.font_size, 17.0);
        assert_eq!(loaded.cursor_shape, "underline");
    }

    #[test]
    fn kitty_palette_and_bool_helpers() {
        assert_eq!(kitty_color("#FF0000").as_deref(), Some("#ff0000"));
        assert_eq!(kitty_color("not-a-colour"), None);
        assert_eq!(kitty_bool("yes"), Some(true));
        assert_eq!(kitty_bool("off"), Some(false));
        assert_eq!(kitty_bool("perhaps"), None);
        assert_eq!(kitty_palette_index("color5"), Some(5));
        assert_eq!(kitty_palette_index("color16"), None);
        assert!(glob_match("*.conf", "base.conf"));
        assert!(!glob_match("*.conf", "base.toml"));
    }

    #[test]
    fn ensure_palette_fills_sixteen_slots() {
        let mut config = Config::default();
        config.palette = vec!["#112233".into()];
        config.ensure_palette();
        assert_eq!(config.palette.len(), 16);
        assert_eq!(config.palette[0], "#112233");
        assert!(Rgb::parse_hex(&config.palette[15]).is_some());
    }
}
