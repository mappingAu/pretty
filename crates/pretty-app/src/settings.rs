//! Native egui settings overlay. The overlay theme itself is configurable.
//!
//! Every control edits a *draft* copy of [`Config`]. The application applies the
//! draft live and debounces the disk write (see `PrettyApp::config_changed`).

use std::path::PathBuf;

use egui::Color32;
use pretty_renderer::{CellMetrics, GpuInfo};

use crate::config::{self, Config};
use crate::i18n::{tr, Lang};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    General,
    Terminal,
    Appearance,
    Rendering,
    Keyboard,
    About,
}

pub struct SettingsState {
    pub open: bool,
    tab: Tab,
    pub draft: Config,
    /// Path typed into the kitty.conf importer.
    pub import_path: String,
    /// Result of the most recent import: `(success, message)`.
    pub import_status: Option<(bool, String)>,
    /// Set by the import button; consumed by the application after the frame.
    pub pending_import: Option<PathBuf>,
}

impl SettingsState {
    pub fn new(config: &Config) -> Self {
        Self {
            open: false,
            tab: Tab::General,
            draft: config.clone(),
            import_path: String::new(),
            import_status: None,
            pending_import: None,
        }
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

/// What the application should do with a settings frame.
#[derive(Clone, Debug, Default)]
pub struct SettingsOutcome {
    /// The draft changed and should be applied/queued for saving.
    pub changed: bool,
    /// The user asked to import this `kitty.conf`.
    pub import: Option<PathBuf>,
    /// The user asked to resize the window to these logical pixels.
    pub resize: Option<(u32, u32)>,
}

fn screen_metrics(ctx: &egui::Context) -> egui::Rect {
    let screen = ctx.content_rect();
    if screen.width().is_finite() && screen.width() > 0.0 {
        screen
    } else {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))
    }
}

/// Render the settings overlay.
pub fn show(
    ctx: &egui::Context,
    state: &mut SettingsState,
    info: &RuntimeInfo<'_>,
    version: &str,
    lang: Lang,
) -> SettingsOutcome {
    apply_style(ctx, &state.draft);
    let mut outcome = SettingsOutcome::default();
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
                if ui
                    .add(button)
                    .on_hover_text(tr(lang, "Open Pretty settings (Ctrl+,)"))
                    .clicked()
                {
                    state.open = true;
                }
            });
    }

    let mut open = state.open;
    // Settings should never open larger than the window; the previous fixed
    // 620x500 default overhung a small (e.g. tiled 640x480) window and hid the
    // footer buttons.
    let screen = screen_metrics(ctx);
    let default_w = (screen.width() - 48.0).clamp(360.0, 680.0);
    let default_h = (screen.height() - 48.0).clamp(280.0, 540.0);
    let default_x = ((screen.width() - default_w) * 0.5).max(8.0);
    let default_y = ((screen.height() - default_h) * 0.5).max(8.0);
    let max_height = (screen.height() - 16.0).max(240.0);
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
        .default_size([default_w, default_h])
        .default_pos([default_x, default_y])
        .min_width(340.0)
        .max_height(max_height)
        .show(ctx, |ui| {
            // Wrapping keeps every tab reachable on a narrow window; the old
            // single row clipped "About" once Chinese labels were selected.
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut tab, Tab::General, tr(lang, "General"));
                ui.selectable_value(&mut tab, Tab::Terminal, tr(lang, "Terminal"));
                ui.selectable_value(&mut tab, Tab::Appearance, tr(lang, "Appearance"));
                ui.selectable_value(&mut tab, Tab::Rendering, tr(lang, "Rendering"));
                ui.selectable_value(&mut tab, Tab::Keyboard, tr(lang, "Keyboard"));
                ui.selectable_value(&mut tab, Tab::About, tr(lang, "About"));
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match tab {
                    Tab::General => {
                        general(ui, &mut draft, info, lang, &mut changed, &mut outcome.resize);
                        ui.separator();
                        kitty_import_section(ui, state, lang);
                    }
                    Tab::Terminal => terminal(ui, &mut draft, lang, &mut changed),
                    Tab::Appearance => appearance(ui, &mut draft, lang, &mut changed),
                    Tab::Rendering => rendering(ui, &mut draft, info, lang, &mut changed),
                    Tab::Keyboard => keyboard(ui, &mut draft, info, lang, &mut changed),
                    Tab::About => about(ui, version, info, lang),
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Save now")).clicked() {
                    changed = true;
                }
                ui.label(
                    egui::RichText::new(tr(lang, "Changes are applied live and saved automatically."))
                        .small(),
                );
            });
        });
    // The import button records a request the application fulfils after this
    // frame, so all file I/O (and error reporting) stays out of the UI layout.
    if let Some(path) = state.pending_import.take() {
        outcome.import = Some(path);
    }
    state.open = open;
    state.tab = tab;
    if changed {
        state.draft = draft;
    }
    outcome.changed = changed;
    outcome
}

fn general(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    info: &RuntimeInfo<'_>,
    lang: Lang,
    changed: &mut bool,
    resize: &mut Option<(u32, u32)>,
) {
    egui::Grid::new("general-grid")
        .num_columns(2)
        .spacing([14.0, 10.0])
        .show(ui, |ui| {
            ui.label(tr(lang, "Language"));
            egui::ComboBox::from_id_salt("language-choice")
                .selected_text(match cfg.language.as_str() {
                    "en" => Lang::English.native_name(),
                    "zh-CN" => Lang::Chinese.native_name(),
                    _ => tr(lang, "Automatic (system)"),
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut cfg.language, "auto".to_owned(), tr(lang, "Automatic (system)"))
                        .changed()
                    {
                        *changed = true;
                    }
                    if ui
                        .selectable_value(&mut cfg.language, "en".to_owned(), Lang::English.native_name())
                        .changed()
                    {
                        *changed = true;
                    }
                    if ui
                        .selectable_value(&mut cfg.language, "zh-CN".to_owned(), Lang::Chinese.native_name())
                        .changed()
                    {
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
            if ui
                .add(egui::Slider::new(&mut cfg.font_size, 7.0..=42.0).suffix(" px"))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();

            ui.label(tr(lang, "Line height"));
            if ui
                .add(egui::Slider::new(&mut cfg.line_height, 0.85..=1.8))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();

            ui.label(tr(lang, "Ligatures"));
            if ui
                .checkbox(&mut cfg.ligatures, tr(lang, "Enable programming ligatures (liga/calt)"))
                .changed()
            {
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
            if ui
                .add(egui::Slider::new(&mut cfg.scrollback, 0..=200_000).suffix(" lines"))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();

            ui.label(tr(lang, "Shell"));
            if ui.text_edit_singleline(&mut cfg.shell).changed() {
                *changed = true;
            }
            ui.end_row();
        });

    ui.add_space(6.0);
    ui.heading(tr(lang, "Window"));
    egui::Grid::new("window-grid")
        .num_columns(2)
        .spacing([14.0, 10.0])
        .show(ui, |ui| {
            ui.label(tr(lang, "Window width"));
            if ui
                .add(egui::DragValue::new(&mut cfg.window_width).range(320..=7680).speed(2.0))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();
            ui.label(tr(lang, "Window height"));
            if ui
                .add(egui::DragValue::new(&mut cfg.window_height).range(200..=4320).speed(2.0))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();
        });
    ui.horizontal(|ui| {
        if ui.button(tr(lang, "Apply window size")).clicked() {
            *resize = Some((cfg.window_width, cfg.window_height));
        }
        ui.label(
            egui::RichText::new(tr(lang, "Saved as the startup size; Apply resizes now."))
                .small()
                .weak(),
        );
    });

    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(format!(
            "{}: {:.0}×{:.0}px  ·  {:.0}px padding  ·  {} fallback fonts",
            tr(lang, "Cell grid"),
            info.metrics.cell_w,
            info.metrics.cell_h,
            cfg.padding,
            tr(lang, "system")
        ))
        .small()
        .weak(),
    );
}

fn kitty_import_section(ui: &mut egui::Ui, state: &mut SettingsState, lang: Lang) {
    ui.heading(tr(lang, "Kitty configuration import"));
    ui.label(
        egui::RichText::new(tr(
            lang,
            "Import font, colours, cursor, scrollback, padding, opacity, bell, copy-on-select and key maps from a kitty.conf. Unsupported options are reported, never fatal.",
        ))
        .small()
        .weak(),
    );
    ui.horizontal(|ui| {
        ui.label(tr(lang, "Path to kitty.conf"));
        ui.add(
            egui::TextEdit::singleline(&mut state.import_path)
                .desired_width(320.0)
                .hint_text("~/.config/kitty/kitty.conf"),
        );
    });
    ui.horizontal(|ui| {
        let enabled = !state.import_path.trim().is_empty();
        if ui
            .add_enabled(enabled, egui::Button::new(tr(lang, "Import")))
            .on_hover_text(tr(lang, "Merge the kitty.conf into this configuration"))
            .clicked()
        {
            state.pending_import = Some(PathBuf::from(state.import_path.trim()));
        }
        ui.label(
            egui::RichText::new(tr(
                lang,
                "Values present in kitty.conf overwrite this configuration; unsupported options are listed below.",
            ))
            .small()
            .weak(),
        );
    });
    if let Some((ok, message)) = &state.import_status {
        let color = if *ok {
            Color32::from_rgb(126, 208, 126)
        } else {
            Color32::from_rgb(235, 111, 111)
        };
        ui.colored_label(color, message);
    }
}

fn terminal(ui: &mut egui::Ui, cfg: &mut Config, lang: Lang, changed: &mut bool) {
    ui.heading(tr(lang, "Cursor"));
    egui::Grid::new("cursor-grid")
        .num_columns(2)
        .spacing([14.0, 10.0])
        .show(ui, |ui| {
            ui.label(tr(lang, "Cursor shape"));
            egui::ComboBox::from_id_salt("cursor-shape")
                .selected_text(match cfg.cursor_shape.as_str() {
                    "beam" => tr(lang, "Beam"),
                    "underline" => tr(lang, "Underline"),
                    _ => tr(lang, "Block"),
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut cfg.cursor_shape, "block".to_owned(), tr(lang, "Block"))
                        .changed()
                    {
                        *changed = true;
                    }
                    if ui
                        .selectable_value(&mut cfg.cursor_shape, "beam".to_owned(), tr(lang, "Beam"))
                        .changed()
                    {
                        *changed = true;
                    }
                    if ui
                        .selectable_value(
                            &mut cfg.cursor_shape,
                            "underline".to_owned(),
                            tr(lang, "Underline"),
                        )
                        .changed()
                    {
                        *changed = true;
                    }
                });
            ui.end_row();
            ui.label(tr(lang, "Cursor colour"));
            *changed |= color_row(ui, &mut cfg.cursor);
            ui.end_row();
            ui.label(tr(lang, "Blinking cursor"));
            if ui.checkbox(&mut cfg.cursor_blink, "").changed() {
                *changed = true;
            }
            ui.end_row();
            ui.label(tr(lang, "Blink interval"));
            if ui
                .add_enabled(
                    cfg.cursor_blink,
                    egui::Slider::new(&mut cfg.cursor_blink_interval_ms, 100..=2_000)
                        .suffix(" ms"),
                )
                .changed()
            {
                *changed = true;
            }
            ui.end_row();
        });

    ui.separator();
    ui.heading(tr(lang, "Bell"));
    egui::Grid::new("bell-grid")
        .num_columns(2)
        .spacing([14.0, 10.0])
        .show(ui, |ui| {
            ui.label(tr(lang, "Bell"));
            egui::ComboBox::from_id_salt("bell-mode")
                .selected_text(match cfg.bell.as_str() {
                    "visual" => tr(lang, "Visual flash"),
                    "none" => tr(lang, "Off"),
                    _ => tr(lang, "Attention (window hint)"),
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(
                            &mut cfg.bell,
                            "attention".to_owned(),
                            tr(lang, "Attention (window hint)"),
                        )
                        .changed()
                    {
                        *changed = true;
                    }
                    if ui
                        .selectable_value(
                            &mut cfg.bell,
                            "visual".to_owned(),
                            tr(lang, "Visual flash"),
                        )
                        .changed()
                    {
                        *changed = true;
                    }
                    if ui
                        .selectable_value(&mut cfg.bell, "none".to_owned(), tr(lang, "Off"))
                        .changed()
                    {
                        *changed = true;
                    }
                });
            ui.end_row();
            ui.label(tr(lang, "Visual bell duration"));
            if ui
                .add_enabled(
                    cfg.bell == "visual",
                    egui::Slider::new(&mut cfg.visual_bell_duration_ms, 20..=2_000)
                        .suffix(" ms"),
                )
                .changed()
            {
                *changed = true;
            }
            ui.end_row();
        });
    ui.label(
        egui::RichText::new(tr(
            lang,
            "Pretty has no audio output: the kitty audio bell is mapped to a window-attention hint.",
        ))
        .small()
        .weak(),
    );

    ui.separator();
    ui.heading(tr(lang, "Session behaviour"));
    if ui
        .checkbox(&mut cfg.copy_on_select, tr(lang, "Copy selection to clipboard when releasing the mouse"))
        .changed()
    {
        *changed = true;
    }
    if ui
        .checkbox(&mut cfg.confirm_close, tr(lang, "Confirm before closing the window or quitting"))
        .changed()
    {
        *changed = true;
    }
    if ui
        .checkbox(
            &mut cfg.allow_osc52_copy,
            tr(lang, "Allow applications to write the clipboard (OSC 52 copy)"),
        )
        .changed()
    {
        *changed = true;
    }
    if ui
        .checkbox(
            &mut cfg.allow_osc52_paste,
            tr(lang, "Allow applications to read the clipboard (OSC 52 paste)"),
        )
        .changed()
    {
        *changed = true;
    }
    ui.label(
        egui::RichText::new(tr(
            lang,
            "The user-initiated copy/paste shortcuts always work; OSC 52 only affects requests from the shell or applications.",
        ))
        .small()
        .weak(),
    );
}

fn appearance(ui: &mut egui::Ui, cfg: &mut Config, lang: Lang, changed: &mut bool) {
    ui.heading(tr(lang, "Theme"));
    egui::Grid::new("theme-grid")
        .num_columns(2)
        .spacing([14.0, 10.0])
        .show(ui, |ui| {
            ui.label(tr(lang, "Theme preset"));
            let current_label: &'static str = config::theme_preset(&cfg.theme)
                .map(|preset| preset.label)
                .unwrap_or("Custom colours");
            egui::ComboBox::from_id_salt("theme-preset")
                .selected_text(tr(lang, current_label))
                .show_ui(ui, |ui| {
                    for preset in config::THEME_PRESETS {
                        let selected = cfg.theme == preset.name;
                        if ui
                            .selectable_label(selected, tr(lang, preset.label))
                            .clicked()
                            && cfg.apply_theme_preset(preset.name)
                        {
                            *changed = true;
                        }
                    }
                    if ui
                        .selectable_label(cfg.theme == "custom", tr(lang, "Custom colours"))
                        .clicked()
                    {
                        cfg.theme = "custom".into();
                        *changed = true;
                    }
                });
            ui.end_row();
        });
    ui.label(
        egui::RichText::new(tr(
            lang,
            "Choosing a preset overwrites the colours below; explicit colours always win on load.",
        ))
        .small()
        .weak(),
    );

    ui.add_space(6.0);
    egui::CollapsingHeader::new(tr(lang, "ANSI palette"))
        .default_open(false)
        .show(ui, |ui| {
            cfg.ensure_palette();
            egui::Grid::new("palette-grid")
                .num_columns(4)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    for index in 0..16 {
                        ui.label(format!("{:02}", index));
                        *changed |= color_row(ui, &mut cfg.palette[index]);
                        if index % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
        });
    ui.separator();

    egui::Grid::new("appearance-grid")
        .num_columns(2)
        .spacing([14.0, 10.0])
        .show(ui, |ui| {
            ui.label(tr(lang, "Opacity"));
            if ui
                .add(egui::Slider::new(&mut cfg.opacity, 0.25..=1.0).fixed_decimals(2))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();

            ui.label(tr(lang, "Compositor blur"));
            if ui
                .checkbox(&mut cfg.blur, tr(lang, "Blur desktop behind the window (KWin/X11)"))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();

            ui.label(tr(lang, "Padding"));
            if ui
                .add(egui::Slider::new(&mut cfg.padding, 0.0..=40.0).suffix(" px"))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();

            ui.label(tr(lang, "Tiling gap"));
            if ui
                .add(egui::Slider::new(&mut cfg.gap, 0.0..=20.0).suffix(" px"))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();

            ui.label(tr(lang, "Background"));
            *changed |= color_row(ui, &mut cfg.background);
            ui.end_row();
            ui.label(tr(lang, "Foreground"));
            *changed |= color_row(ui, &mut cfg.foreground);
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
    egui::Grid::new("ui-grid")
        .num_columns(2)
        .spacing([14.0, 10.0])
        .show(ui, |ui| {
            ui.label(tr(lang, "UI accent"));
            *changed |= color_row(ui, &mut cfg.ui_accent);
            ui.end_row();
            ui.label(tr(lang, "UI panel opacity"));
            if ui
                .add(egui::Slider::new(&mut cfg.ui_panel_opacity, 0.5..=1.0))
                .changed()
            {
                *changed = true;
            }
            ui.end_row();
            ui.label(tr(lang, "UI scale"));
            if ui
                .add(
                    egui::Slider::new(&mut cfg.ui_scale, 0.75..=2.0)
                        .fixed_decimals(2)
                        .suffix("×"),
                )
                .changed()
            {
                *changed = true;
            }
            ui.end_row();
            ui.label(tr(lang, "Tab bar"));
            if ui.checkbox(&mut cfg.tab_bar_visible, tr(lang, "Show tab bar")).changed() {
                *changed = true;
            }
            ui.end_row();
            ui.label(tr(lang, "Tab bar height"));
            if ui
                .add_enabled(
                    cfg.tab_bar_visible,
                    egui::Slider::new(&mut cfg.tab_bar_height, 20.0..=48.0).suffix(" pt"),
                )
                .changed()
            {
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
                // Metal is only built into macOS binaries and Direct3D 12 into
                // Windows binaries; the platform-native API is picked
                // automatically via `auto`, so only advertise what can work
                // here. (Previously these were mislabelled "reserved".)
                let mut options: Vec<&str> = vec!["auto", "vulkan", "gles"];
                if cfg!(target_os = "macos") {
                    options.push("metal");
                }
                if cfg!(target_os = "windows") {
                    options.push("dx");
                }
                for value in options {
                    if ui
                        .selectable_value(&mut cfg.renderer, value.to_owned(), value)
                        .changed()
                    {
                        *changed = true;
                    }
                }
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
    egui::Grid::new("gpu-grid")
        .num_columns(2)
        .spacing([14.0, 6.0])
        .show(ui, |ui| {
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
    ui.label(format!(
        "{}: 0b{:05b} ({})",
        tr(lang, "Negotiated flags"),
        info.kitty_flags,
        info.kitty_flags
    ));
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
    egui::Grid::new("about-grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
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
    ui.label(format!(
        "{}: docs/CONFIG.md, docs/KITTY-COMPAT.md, docs/ARCHITECTURE.md, docs/RENDERER.md, docs/TEST-REPORT.md",
        tr(lang, "Documentation")
    ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_outcome_defaults() {
        let outcome = SettingsOutcome::default();
        assert!(!outcome.changed);
        assert!(outcome.import.is_none());
        assert!(outcome.resize.is_none());
    }

    #[test]
    fn state_sync_only_when_closed() {
        let mut config = Config::default();
        let mut state = SettingsState::new(&config);
        config.font_size = 20.0;
        state.sync(&config);
        assert_eq!(state.draft.font_size, 20.0);
        state.open = true;
        config.font_size = 11.0;
        state.sync(&config);
        assert_eq!(state.draft.font_size, 20.0);
    }
}
