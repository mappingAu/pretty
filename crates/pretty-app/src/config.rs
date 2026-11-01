//! Pretty configuration model and persistence.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pretty_core::Rgb;
use pretty_renderer::{RendererKind, Theme};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
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
    pub scrollback: usize,
    pub vsync: bool,
    pub shell: String,
    pub allow_osc52_paste: bool,
    pub window_width: u32,
    pub window_height: u32,
    pub ui_accent: String,
    pub ui_panel_opacity: f32,
    pub prefer_x11_for_blur: bool,
    pub cursor_blink: bool,
    /// Action name -> key binding, e.g. `new_tab = "Ctrl+Shift+T"`.
    pub shortcuts: HashMap<String, String>,
    /// UI language: `auto`, `en`, or `zh-CN`.
    pub language: String,
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
            scrollback: 10_000,
            vsync: true,
            shell: default_shell(),
            allow_osc52_paste: true,
            window_width: 1280,
            window_height: 820,
            ui_accent: "#7aa2f7".into(),
            ui_panel_opacity: 1.0,
            prefer_x11_for_blur: false,
            cursor_blink: true,
            shortcuts: default_shortcuts(),
            language: "auto".into(),
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

impl Config {
    pub fn theme(&self) -> Theme {
        let mut t = Theme::default();
        t.background = parse_color(&self.background, t.background);
        t.foreground = parse_color(&self.foreground, t.foreground);
        t.cursor = parse_color(&self.cursor, t.cursor);
        t.accent = parse_color(&self.accent, t.accent);
        t.selection_bg = parse_color(&self.selection_bg, t.selection_bg);
        t.selection_fg = parse_color(&self.selection_fg, t.selection_fg);
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
        self.font_size = self.font_size.clamp(7.0, 42.0);
        self.line_height = self.line_height.clamp(0.85, 1.8);
        self.opacity = self.opacity.clamp(0.2, 1.0);
        self.padding = self.padding.clamp(0.0, 40.0);
        self.gap = self.gap.clamp(0.0, 20.0);
        self.scrollback = self.scrollback.min(1_000_000);
        self.window_width = self.window_width.clamp(320, 7680);
        self.window_height = self.window_height.clamp(200, 4320);
        self.ui_panel_opacity = self.ui_panel_opacity.clamp(0.5, 1.0);
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
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(mut config) => {
                config.validate();
                log::info!("configuration loaded from {}", path.display());
                config
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
