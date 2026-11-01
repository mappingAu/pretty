//! Native egui settings overlay. The overlay theme itself is configurable.

use egui::Color32;
use pretty_renderer::{CellMetrics, GpuInfo};

use crate::config::Config;
use crate::i18n::{tr, Lang};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    General,
    Appearance,
    Rendering,
    Keyboard,
    About,
}

pub struct SettingsState {
    pub open: bool,
    tab: Tab,
    pub draft: Config,
}

impl SettingsState {
    pub fn new(config: &Config) -> Self {
        Self { open: false, tab: Tab::General, draft: config.clone() }
    }

    pub fn sync(&mut self, config: &Config) {
        if !self.open {
            self.draft = config.clone();
        }
    }
}

pub struct RuntimeInfo<'a> {
    pub gpu: &'a GpuInfo,
    pub metrics: CellMetrics,
    pub kitty_flags: u32,
    pub mode: String,
    pub backend_note: Option<String>,
    pub config_path: String,
    pub font_family: String,
}

/// Render the settings overlay. Returns true if the draft changed and should be
/// applied/saved by the application.
pub fn show(
    ctx: &egui::Context,
    state: &mut SettingsState,
    info: &RuntimeInfo<'_>,
    version: &str,
    lang: Lang,
) -> bool {
    apply_style(ctx, &state.draft);
    let mut changed = false;

    if !state.open {
        egui::Area::new(egui::Id::new("pretty-gear"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 10.0))
            .show(ctx, |ui| {
                let button = egui::Button::new(egui::RichText::new(tr(lang, "Settings")).size(13.0))
                    .min_size(egui::vec2(104.0, 30.0))
                    .fill(Color32::from_rgba_unmultiplied(20, 26, 36, 230))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 110, 140)));
                if ui.add(button).on_hover_text(tr(lang, "Open Pretty settings (Ctrl+,)")).clicked() {
                    state.open = true;
                }
            });
    }

    let mut open = state.open;
    let mut draft = state.draft.clone();
    let mut tab = state.tab;
    let panel_alpha = (state.draft.ui_panel_opacity.clamp(0.5, 1.0) * 255.0) as u8;
    let panel_fill = Color32::from_rgba_unmultiplied(24, 31, 43, panel_alpha);
    let frame = egui::Frame::new()
        .fill(panel_fill)
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(54, 70, 94)))
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(egui::Margin::same(12));
    egui::Window::new(tr(lang, "Pretty Settings"))
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .resizable(true)
        .default_size([620.0, 500.0])
        .default_pos([300.0, 85.0])
        .min_width(480.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut tab, Tab::General, tr(lang, "General"));
                ui.selectable_value(&mut tab, Tab::Appearance, tr(lang, "Appearance"));
                ui.selectable_value(&mut tab, Tab::Rendering, tr(lang, "Rendering"));
                ui.selectable_value(&mut tab, Tab::Keyboard, tr(lang, "Keyboard"));
                ui.selectable_value(&mut tab, Tab::About, tr(lang, "About"));
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                match tab {
                    Tab::General => general(ui, &mut draft, info, lang, &mut changed),
                    Tab::Appearance => appearance(ui, &mut draft, lang, &mut changed),
                    Tab::Rendering => rendering(ui, &mut draft, info, lang, &mut changed),
                    Tab::Keyboard => keyboard(ui, &mut draft, info, lang, &mut changed),
                    Tab::About => about(ui, version, info, lang),
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Save now")).clicked() {
                    changed = true;
                }
                ui.label(egui::RichText::new(tr(lang, "Changes are applied live and saved automatically.")).small());
            });
        });
    state.open = open;
    state.tab = tab;
    if changed {
        state.draft = draft;
    }
    changed
}

fn general(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    info: &RuntimeInfo<'_>,
    lang: Lang,
    changed: &mut bool,
) {
    egui::Grid::new("general-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        ui.label(tr(lang, "Language"));
        egui::ComboBox::from_id_salt("language-choice")
            .selected_text(match cfg.language.as_str() {
                "en" => Lang::English.native_name(),
                "zh-CN" => Lang::Chinese.native_name(),
                _ => tr(lang, "Automatic (system)"),
            })
            .show_ui(ui, |ui| {
                if ui.selectable_value(&mut cfg.language, "auto".to_owned(), tr(lang, "Automatic (system)")).changed() {
                    *changed = true;
                }
                if ui.selectable_value(&mut cfg.language, "en".to_owned(), Lang::English.native_name()).changed() {
                    *changed = true;
                }
                if ui.selectable_value(&mut cfg.language, "zh-CN".to_owned(), Lang::Chinese.native_name()).changed() {
                    *changed = true;
                }
            });
        ui.end_row();

        ui.label(tr(lang, "Font family"));
        if ui.text_edit_singleline(&mut cfg.font_family).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Font size"));
        if ui.add(egui::Slider::new(&mut cfg.font_size, 8.0..=32.0).suffix(" px")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Line height"));
        if ui.add(egui::Slider::new(&mut cfg.line_height, 0.9..=1.6)).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Ligatures"));
        if ui.checkbox(&mut cfg.ligatures, tr(lang, "Enable programming ligatures (liga/calt)")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "OpenType features"));
        let mut features = cfg.font_features.join(", ");
        if ui.text_edit_singleline(&mut features).changed() {
            cfg.font_features = features
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Scrollback"));
        if ui.add(egui::Slider::new(&mut cfg.scrollback, 0..=200_000).suffix(" lines")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Shell"));
        if ui.text_edit_singleline(&mut cfg.shell).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Cursor"));
        if ui.checkbox(&mut cfg.cursor_blink, tr(lang, "Blinking cursor")).changed() {
            *changed = true;
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(format!(
            "{}: {:.0}×{:.0}px  ·  {}×{} cells",
            tr(lang, "Cell grid"),
            info.metrics.cell_w,
            info.metrics.cell_h,
            (ui.available_width() / info.metrics.cell_w) as u32,
            24
        ))
        .small()
        .weak(),
    );
}

fn appearance(ui: &mut egui::Ui, cfg: &mut Config, lang: Lang, changed: &mut bool) {
    egui::Grid::new("appearance-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        ui.label(tr(lang, "Opacity"));
        if ui.add(egui::Slider::new(&mut cfg.opacity, 0.25..=1.0).fixed_decimals(2)).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Compositor blur"));
        if ui.checkbox(&mut cfg.blur, tr(lang, "Blur desktop behind the window (KWin/X11)")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Padding"));
        if ui.add(egui::Slider::new(&mut cfg.padding, 0.0..=30.0).suffix(" px")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Tiling gap"));
        if ui.add(egui::Slider::new(&mut cfg.gap, 0.0..=16.0).suffix(" px")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label(tr(lang, "Background"));
        *changed |= color_row(ui, &mut cfg.background);
        ui.end_row();
        ui.label(tr(lang, "Foreground"));
        *changed |= color_row(ui, &mut cfg.foreground);
        ui.end_row();
        ui.label(tr(lang, "Cursor"));
        *changed |= color_row(ui, &mut cfg.cursor);
        ui.end_row();
        ui.label(tr(lang, "Accent"));
        *changed |= color_row(ui, &mut cfg.accent);
        ui.end_row();
        ui.label(tr(lang, "Selection background"));
        *changed |= color_row(ui, &mut cfg.selection_bg);
        ui.end_row();
        ui.label(tr(lang, "Selection foreground"));
        *changed |= color_row(ui, &mut cfg.selection_fg);
        ui.end_row();
    });
    ui.separator();
    ui.heading(tr(lang, "Configuration UI"));
    egui::Grid::new("ui-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        ui.label(tr(lang, "UI accent"));
        *changed |= color_row(ui, &mut cfg.ui_accent);
        ui.end_row();
        ui.label(tr(lang, "UI panel opacity"));
        if ui.add(egui::Slider::new(&mut cfg.ui_panel_opacity, 0.5..=1.0)).changed() {
            *changed = true;
        }
        ui.end_row();
    });
    ui.add_space(4.0);
}

fn rendering(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    info: &RuntimeInfo<'_>,
    lang: Lang,
    changed: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label(tr(lang, "Backend"));
        egui::ComboBox::from_id_salt("renderer-choice")
            .selected_text(cfg.renderer.clone())
            .show_ui(ui, |ui| {
                for value in ["auto", "vulkan", "gles"] {
                    if ui.selectable_value(&mut cfg.renderer, value.to_owned(), value).changed() {
                        *changed = true;
                    }
                }
                ui.add_enabled(false, egui::Button::new("metal (reserved)"));
                ui.add_enabled(false, egui::Button::new("dx (reserved)"));
            });
    });
    ui.label(
        egui::RichText::new(tr(lang, "Backend changes take effect after restarting Pretty."))
            .small()
            .weak(),
    );
    if ui.checkbox(&mut cfg.vsync, tr(lang, "Vertical sync (FIFO)")).changed() {
        *changed = true;
    }
    ui.separator();
    egui::Grid::new("gpu-grid").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
        ui.label(tr(lang, "GPU"));
        ui.label(format!("{} ({:?})", info.gpu.adapter.name, info.gpu.adapter.device_type));
        ui.end_row();
        ui.label(tr(lang, "Driver"));
        ui.label(format!("{} {}", info.gpu.adapter.driver, info.gpu.adapter.driver_info));
        ui.end_row();
        ui.label(tr(lang, "wgpu backend"));
        ui.label(format!("{:?}  ·  {:?}", info.gpu.wgpu_backend, info.gpu.selected));
        ui.end_row();
        ui.label(tr(lang, "Surface"));
        ui.label(format!(
            "{:?}  ·  alpha {:?}  ·  {:?}",
            info.gpu.surface_format, info.gpu.alpha_mode, info.gpu.present_mode
        ));
        ui.end_row();
        ui.label(tr(lang, "Max texture"));
        ui.label(format!("{} px", info.gpu.max_texture_dimension));
        ui.end_row();
        ui.label(tr(lang, "Kitty keyboard flags"));
        ui.label(format!("{} (0b{:05b})", info.kitty_flags, info.kitty_flags));
        ui.end_row();
        ui.label(tr(lang, "Terminal modes"));
        ui.label(&info.mode);
        ui.end_row();
        ui.label(tr(lang, "Font"));
        ui.label(format!("{} @ {:.0}px cells", info.font_family, info.metrics.cell_w));
        ui.end_row();
    });
    if let Some(note) = &info.backend_note {
        ui.add_space(6.0);
        ui.colored_label(egui::Color32::from_rgb(240, 190, 90), note);
    }
}

fn keyboard(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    info: &RuntimeInfo<'_>,
    lang: Lang,
    changed: &mut bool,
) {
    ui.heading(tr(lang, "Kitty keyboard protocol"));
    ui.label(format!("{}: 0b{:05b} ({})", tr(lang, "Negotiated flags"), info.kitty_flags, info.kitty_flags));
    ui.label(format!("  {}", tr(lang, "bit 1: disambiguate escape codes")));
    ui.label(format!("  {}", tr(lang, "bit 2: report event types (repeat/release)")));
    ui.label(format!("  {}", tr(lang, "bit 3: report alternate keys")));
    ui.label(format!("  {}", tr(lang, "bit 4: report all keys as escape codes")));
    ui.label(format!("  {}", tr(lang, "bit 5: report associated text")));
    ui.add_space(10.0);
    ui.heading(tr(lang, "Shortcuts"));
    ui.label(tr(lang, "Edit a binding as Ctrl+Shift+T, Ctrl+, or Alt+Arrow. Changes apply immediately."));
    egui::Grid::new("shortcuts-grid")
        .num_columns(2)
        .spacing([14.0, 6.0])
        .show(ui, |ui| {
            for (action, label) in [
                ("new_tab", "New tab"),
                ("close_pane", "Close pane"),
                ("split_right", "Split right"),
                ("split_down", "Split down"),
                ("next_tab", "Next tab"),
                ("prev_tab", "Previous tab"),
                ("find", "Find"),
                ("copy", "Copy"),
                ("paste", "Paste"),
                ("select_all", "Select all"),
                ("settings", "Settings"),
                ("zoom_in", "Zoom in"),
                ("zoom_out", "Zoom out"),
                ("zoom_reset", "Reset zoom"),
                ("quit", "Quit"),
            ] {
                ui.label(tr(lang, label));
                let value = cfg.shortcuts.entry(action.to_owned()).or_default();
                if ui.text_edit_singleline(value).changed() {
                    *changed = true;
                }
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    ui.heading(tr(lang, "Fixed navigation"));
    for (keys, action) in [
        ("Alt+Arrow", "Focus neighbouring pane"),
        ("Shift+PageUp / PageDown", "Scroll history by page"),
        ("Ctrl+Shift+Up / Down", "Scroll history by line"),
        ("Shift+Mouse", "Force local selection over mouse reporting"),
    ] {
        ui.horizontal(|ui| {
            ui.monospace(format!("{keys:28}"));
            ui.label(tr(lang, action));
        });
    }
    ui.add_space(8.0);
    ui.label(tr(lang, "The terminal pass-through uses kitty CSI-u encoding when an application enables the protocol; otherwise it emits xterm-compatible sequences."));
}

fn about(ui: &mut egui::Ui, version: &str, info: &RuntimeInfo<'_>, lang: Lang) {
    ui.heading(format!("Pretty {version}"));
    ui.label(tr(lang, "A hardware-accelerated terminal emulator with kitty protocol support, Vulkan and GLES backends, tiling panes and compositor blur."));
    ui.add_space(8.0);
    egui::Grid::new("about-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label(tr(lang, "Config"));
        ui.label(&info.config_path);
        ui.end_row();
        ui.label(tr(lang, "GPU"));
        ui.label(&info.gpu.adapter.name);
        ui.end_row();
        ui.label(tr(lang, "Backend"));
        ui.label(format!("{:?}", info.gpu.wgpu_backend));
        ui.end_row();
        ui.label(tr(lang, "License"));
        ui.label("GPL-3.0-or-later; bundled fonts under OFL-1.1");
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.label(format!("{}: docs/KITTY-COMPAT.md, docs/ARCHITECTURE.md, docs/RENDERER.md, docs/TEST-REPORT.md", tr(lang, "Documentation")));
    ui.label("kitty protocol documentation: https://sw.kovidgoyal.net/kitty/");
}

fn color_row(ui: &mut egui::Ui, value: &mut String) -> bool {
    let current = pretty_core::Rgb::parse_hex(value).unwrap_or(pretty_core::Rgb::new(0, 0, 0));
    let mut color = Color32::from_rgb(current.r, current.g, current.b);
    let response = ui.color_edit_button_srgba(&mut color);
    if response.changed() {
        *value = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
        true
    } else {
        false
    }
}

fn apply_style(ctx: &egui::Context, config: &Config) {
    let accent = pretty_core::Rgb::parse_hex(&config.ui_accent)
        .unwrap_or(pretty_core::Rgb::new(0x7a, 0xa2, 0xf7));
    let a = Color32::from_rgb(accent.r, accent.g, accent.b);
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(Color32::from_rgb(224, 232, 245));
    visuals.selection.bg_fill = a.gamma_multiply(0.65);
    visuals.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.bg_fill = a;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, a);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(32, 43, 60);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(24, 32, 45);
    // egui's dark defaults use a faint grey for button text. That made the
    // context menu and tab-bar labels look blurred next to a terminal; use the
    // same bright foreground for every interactive state.
    let bright = Color32::from_rgb(232, 238, 248);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, bright);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, bright);
    visuals.hyperlink_color = a;
    let panel_alpha = (config.ui_panel_opacity.clamp(0.5, 1.0) * 255.0) as u8;
    visuals.window_fill = Color32::from_rgba_unmultiplied(24, 31, 43, panel_alpha);
    visuals.panel_fill = Color32::from_rgba_unmultiplied(24, 31, 43, panel_alpha);
    visuals.extreme_bg_color = Color32::from_rgb(10, 14, 21);
    visuals.faint_bg_color = Color32::from_rgba_unmultiplied(34, 44, 60, panel_alpha);
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = visuals;
    // Settings toggles should be instantaneous. The window fade animation was
    // the visible stutter (and, without a repaint source, could leave the panel
    // stuck dim); keep all UI transitions at zero.
    style.animation_time = 0.0;
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.visuals.window_corner_radius = egui::CornerRadius::same(8);
    ctx.set_style_of(egui::Theme::Dark, style);
}
