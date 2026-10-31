//! Native egui settings overlay. The overlay theme itself is configurable.

use egui::Color32;
use pretty_renderer::{CellMetrics, GpuInfo};

use crate::config::Config;

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
) -> bool {
    apply_style(ctx, &state.draft);
    let mut changed = false;

    if !state.open {
        egui::Area::new(egui::Id::new("pretty-gear"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 10.0))
            .show(ctx, |ui| {
                let button = egui::Button::new(egui::RichText::new("Settings").size(13.0))
                    .fill(Color32::from_rgba_unmultiplied(20, 26, 36, 230))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 110, 140)));
                if ui.add(button).on_hover_text("Open Pretty settings (Ctrl+,)").clicked() {
                    state.open = true;
                    state.draft = state.draft.clone();
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
    egui::Window::new("Pretty Settings")
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .resizable(true)
        .default_size([620.0, 500.0])
        .default_pos([300.0, 85.0])
        .min_width(480.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut tab, Tab::General, "General");
                ui.selectable_value(&mut tab, Tab::Appearance, "Appearance");
                ui.selectable_value(&mut tab, Tab::Rendering, "Rendering");
                ui.selectable_value(&mut tab, Tab::Keyboard, "Keyboard");
                ui.selectable_value(&mut tab, Tab::About, "About");
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                match tab {
                    Tab::General => general(ui, &mut draft, info, &mut changed),
                    Tab::Appearance => appearance(ui, &mut draft, &mut changed),
                    Tab::Rendering => rendering(ui, &mut draft, info, &mut changed),
                    Tab::Keyboard => keyboard(ui, info),
                    Tab::About => about(ui, version, info),
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save now").clicked() {
                    changed = true;
                }
                ui.label(egui::RichText::new("Changes are applied live and saved automatically.").small());
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
    changed: &mut bool,
) {
    egui::Grid::new("general-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        ui.label("Font family");
        if ui.text_edit_singleline(&mut cfg.font_family).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Font size");
        if ui.add(egui::Slider::new(&mut cfg.font_size, 8.0..=32.0).suffix(" px")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Line height");
        if ui.add(egui::Slider::new(&mut cfg.line_height, 0.9..=1.6)).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Ligatures");
        if ui.checkbox(&mut cfg.ligatures, "Enable programming ligatures (liga/calt)").changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("OpenType features");
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

        ui.label("Scrollback");
        if ui.add(egui::Slider::new(&mut cfg.scrollback, 0..=200_000).suffix(" lines")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Shell");
        if ui.text_edit_singleline(&mut cfg.shell).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Cursor");
        if ui.checkbox(&mut cfg.cursor_blink, "Blinking cursor").changed() {
            *changed = true;
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(format!(
            "Cell grid: {:.0}×{:.0}px  ·  current pane {}×{} cells",
            info.metrics.cell_w,
            info.metrics.cell_h,
            (ui.available_width() / info.metrics.cell_w) as u32,
            24
        ))
        .small()
        .weak(),
    );
}

fn appearance(ui: &mut egui::Ui, cfg: &mut Config, changed: &mut bool) {
    egui::Grid::new("appearance-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        ui.label("Opacity");
        if ui.add(egui::Slider::new(&mut cfg.opacity, 0.25..=1.0).fixed_decimals(2)).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Compositor blur");
        if ui.checkbox(&mut cfg.blur, "Blur desktop behind the window (KWin/X11)").changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Padding");
        if ui.add(egui::Slider::new(&mut cfg.padding, 0.0..=30.0).suffix(" px")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Tiling gap");
        if ui.add(egui::Slider::new(&mut cfg.gap, 0.0..=16.0).suffix(" px")).changed() {
            *changed = true;
        }
        ui.end_row();

        ui.label("Background");
        *changed |= color_row(ui, &mut cfg.background);
        ui.end_row();
        ui.label("Foreground");
        *changed |= color_row(ui, &mut cfg.foreground);
        ui.end_row();
        ui.label("Cursor");
        *changed |= color_row(ui, &mut cfg.cursor);
        ui.end_row();
        ui.label("Accent");
        *changed |= color_row(ui, &mut cfg.accent);
        ui.end_row();
        ui.label("Selection background");
        *changed |= color_row(ui, &mut cfg.selection_bg);
        ui.end_row();
        ui.label("Selection foreground");
        *changed |= color_row(ui, &mut cfg.selection_fg);
        ui.end_row();
    });
    ui.separator();
    ui.heading("Configuration UI");
    ui.label("The settings overlay is themed separately and can be customised:");
    egui::Grid::new("ui-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        ui.label("UI accent");
        *changed |= color_row(ui, &mut cfg.ui_accent);
        ui.end_row();
        ui.label("UI panel opacity");
        if ui.add(egui::Slider::new(&mut cfg.ui_panel_opacity, 0.5..=1.0)).changed() {
            *changed = true;
        }
        ui.end_row();
    });
    ui.add_space(4.0);
    if ui.checkbox(&mut cfg.prefer_x11_for_blur, "Prefer X11/XWayland for compositor blur (restart required)").changed() {
        *changed = true;
    }
}

fn rendering(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    info: &RuntimeInfo<'_>,
    changed: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label("Backend");
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
        egui::RichText::new("Backend changes take effect after restarting Pretty.")
            .small()
            .weak(),
    );
    if ui.checkbox(&mut cfg.vsync, "Vertical sync (FIFO)").changed() {
        *changed = true;
    }
    ui.separator();
    egui::Grid::new("gpu-grid").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
        ui.label("GPU");
        ui.label(format!("{} ({:?})", info.gpu.adapter.name, info.gpu.adapter.device_type));
        ui.end_row();
        ui.label("Driver");
        ui.label(format!("{} {}", info.gpu.adapter.driver, info.gpu.adapter.driver_info));
        ui.end_row();
        ui.label("wgpu backend");
        ui.label(format!("{:?}  ·  {:?}", info.gpu.wgpu_backend, info.gpu.selected));
        ui.end_row();
        ui.label("Surface");
        ui.label(format!(
            "{:?}  ·  alpha {:?}  ·  {:?}",
            info.gpu.surface_format, info.gpu.alpha_mode, info.gpu.present_mode
        ));
        ui.end_row();
        ui.label("Max texture");
        ui.label(format!("{} px", info.gpu.max_texture_dimension));
        ui.end_row();
        ui.label("Kitty keyboard flags");
        ui.label(format!("{} (0b{:05b})", info.kitty_flags, info.kitty_flags));
        ui.end_row();
        ui.label("Terminal modes");
        ui.label(&info.mode);
        ui.end_row();
        ui.label("Font");
        ui.label(format!("{} @ {:.0}px cells", info.font_family, info.metrics.cell_w));
        ui.end_row();
    });
    if let Some(note) = &info.backend_note {
        ui.add_space(6.0);
        ui.colored_label(egui::Color32::from_rgb(240, 190, 90), note);
    }
    ui.add_space(8.0);
    ui.label("Vulkan and OpenGL ES 3.0+ are implemented. Metal and Direct3D are reserved interface stubs and are never silently selected.");
}

fn keyboard(ui: &mut egui::Ui, info: &RuntimeInfo<'_>) {
    ui.heading("Kitty keyboard protocol");
    ui.label(format!("Negotiated flags: 0b{:05b} ({})", info.kitty_flags, info.kitty_flags));
    ui.label("  bit 1: disambiguate escape codes");
    ui.label("  bit 2: report event types (repeat/release)");
    ui.label("  bit 3: report alternate keys");
    ui.label("  bit 4: report all keys as escape codes");
    ui.label("  bit 5: report associated text");
    ui.add_space(10.0);
    ui.heading("Pretty shortcuts");
    for (keys, action) in [
        ("Ctrl+,", "Open/close this settings panel"),
        ("Ctrl+Shift+E", "Split focused pane to the right"),
        ("Ctrl+Shift+O", "Split focused pane down"),
        ("Ctrl+Shift+W", "Close focused pane"),
        ("Alt+Arrow", "Focus neighbouring pane"),
        ("Ctrl+Shift+C / V", "Copy selection / paste"),
        ("Shift+PageUp / PageDown", "Scroll history by page"),
        ("Ctrl+Shift+Up / Down", "Scroll history by line"),
        ("Ctrl+Shift+Plus / Minus / 0", "Increase / decrease / reset font size"),
        ("Shift+Mouse", "Force local selection over mouse reporting"),
        ("Ctrl+Shift+Q", "Quit Pretty"),
    ] {
        ui.horizontal(|ui| {
            ui.monospace(format!("{keys:28}"));
            ui.label(action);
        });
    }
    ui.add_space(8.0);
    ui.label("The terminal pass-through uses kitty CSI-u encoding when an application enables the protocol; otherwise it emits xterm-compatible sequences.");
}

fn about(ui: &mut egui::Ui, version: &str, info: &RuntimeInfo<'_>) {
    ui.heading(format!("Pretty {version}"));
    ui.label("A hardware-accelerated terminal emulator with kitty protocol support, Vulkan and GLES backends, tiling panes and compositor blur.");
    ui.add_space(8.0);
    egui::Grid::new("about-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Config");
        ui.label(&info.config_path);
        ui.end_row();
        ui.label("GPU");
        ui.label(&info.gpu.adapter.name);
        ui.end_row();
        ui.label("Backend");
        ui.label(format!("{:?}", info.gpu.wgpu_backend));
        ui.end_row();
        ui.label("License");
        ui.label("MIT OR Apache-2.0; bundled fonts under OFL-1.1");
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.label("Documentation: docs/KITTY-COMPAT.md, docs/ARCHITECTURE.md, docs/RENDERER.md, docs/TEST-REPORT.md");
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
    visuals.hyperlink_color = a;
    let panel_alpha = (config.ui_panel_opacity.clamp(0.5, 1.0) * 255.0) as u8;
    visuals.window_fill = Color32::from_rgba_unmultiplied(24, 31, 43, panel_alpha);
    visuals.panel_fill = Color32::from_rgba_unmultiplied(24, 31, 43, panel_alpha);
    visuals.extreme_bg_color = Color32::from_rgb(10, 14, 21);
    visuals.faint_bg_color = Color32::from_rgba_unmultiplied(34, 44, 60, panel_alpha);
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.visuals.window_corner_radius = egui::CornerRadius::same(8);
    ctx.set_style_of(egui::Theme::Dark, style);
}
