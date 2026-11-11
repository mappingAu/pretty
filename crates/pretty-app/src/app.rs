//! winit application: windows, panes, input routing, UI and rendering.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, TryRecvError};
use pretty_core::{
    encode_key, encode_mouse, encode_paste, CursorStyle, Key, KeyEventKind, KeyInput, Layout,
    Modifiers as CoreModifiers, MouseEventKind, PaneId, PtySession, Rect, TermMode,
    TermSize, Terminal, TerminalEvent, VteRgb,
};
use pretty_renderer::{PaneDraw, RenderOutcome, Renderer, RendererKind, UiPaint};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, Ime, MouseButton as WinitMouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey as WinitNamedKey};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{Fullscreen, Window, WindowId};

use crate::config::Config;
use crate::i18n::{tr, Lang};
use crate::menu::{self, ContextAction};
use crate::platform;
use crate::settings::{self, RuntimeInfo, SettingsState};

#[derive(Clone, Debug)]
pub enum UserEvent {
    PtyReady(PaneId),
    /// Wake-up from the blink timer thread. The event loop does not reach
    /// `about_to_wait` while input events keep arriving, so a control-flow
    /// deadline alone is not enough to keep the cursor blinking.
    BlinkTick,
}

/// Cursor blink phase driven by wall-clock deadlines.
///
/// The phase is deliberately independent of frame presentation: any event
/// (timer tick, redraw, input) may advance it once the deadline has passed, so
/// a busy event loop can delay a toggle by at most the tick period instead of
/// freezing the cursor in the on/off state.
struct BlinkPhase {
    on: bool,
    next: Instant,
}

impl BlinkPhase {
    /// Start visible and schedule the first toggle one full interval later.
    fn started(interval: Duration) -> Self {
        Self {
            on: true,
            next: Instant::now() + interval,
        }
    }

    /// Restart in the "on" phase. Called when the terminal content or input
    /// changes so the cursor does not disappear under the user's eyes.
    fn reset(&mut self, interval: Duration) {
        self.on = true;
        self.next = Instant::now() + interval;
    }

    /// Advance the phase if `now` has reached the deadline.
    ///
    /// The next deadline is derived from the previous one (not from `now`), so
    /// toggles stay at the configured cadence instead of drifting by the
    /// event-loop latency. A stall longer than one interval resynchronises
    /// instead of firing a burst of catch-up toggles.
    fn advance(&mut self, now: Instant, interval: Duration) -> bool {
        if now < self.next {
            return false;
        }
        self.on = !self.on;
        let next = self.next + interval;
        // Drift compensation keeps the cadence on its original grid, but a
        // toggle that happened just *before* the next deadline must not create
        // a tiny second phase (the old code could produce a ~35 ms off/on
        // blip, the visible "cursor flicker" under load). Resynchronise when
        // the corrected deadline is less than half an interval away.
        self.next = if next > now + interval / 2 {
            next
        } else {
            now + interval
        };
        log::trace!(
            "cursor blink phase -> {} (next in {:?})",
            self.on,
            self.next.saturating_duration_since(now)
        );
        true
    }
}

pub struct AppOptions {
    pub renderer: RendererKind,
    pub command: Option<String>,
    pub fullscreen: bool,
    pub title: Option<String>,
    pub config_path: PathBuf,
    pub benchmark_frames: Option<u32>,
    pub allow_software: bool,
}

struct Pane {
    term: Terminal,
    pty: PtySession,
    pty_rx: Receiver<Vec<u8>>,
    term_events: Receiver<TerminalEvent>,
    title: String,
    rect: Rect,
    cols: usize,
    rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Shortcut {
    ctrl: bool,
    shift: bool,
    alt: bool,
    super_key: bool,
    key: String,
}

impl Shortcut {
    fn parse(binding: &str) -> Option<Self> {
        let mut out = Self {
            ctrl: false,
            shift: false,
            alt: false,
            super_key: false,
            key: String::new(),
        };
        for part in binding.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => out.ctrl = true,
                "shift" => out.shift = true,
                "alt" | "option" => out.alt = true,
                "super" | "meta" | "win" | "cmd" => out.super_key = true,
                _ => out.key = normalize_shortcut_key(part),
            }
        }
        if out.key.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn from_event(
        event: &winit::event::KeyEvent,
        modifiers: ModifiersState,
    ) -> Option<Self> {
        let key = match event.logical_key.as_ref() {
            WinitKey::Character(s) => normalize_shortcut_key(s),
            WinitKey::Named(WinitNamedKey::Tab) => "tab".into(),
            WinitKey::Named(WinitNamedKey::ArrowUp) => "up".into(),
            WinitKey::Named(WinitNamedKey::ArrowDown) => "down".into(),
            WinitKey::Named(WinitNamedKey::ArrowLeft) => "left".into(),
            WinitKey::Named(WinitNamedKey::ArrowRight) => "right".into(),
            WinitKey::Named(WinitNamedKey::PageUp) => "pageup".into(),
            WinitKey::Named(WinitNamedKey::PageDown) => "pagedown".into(),
            WinitKey::Named(WinitNamedKey::Escape) => "escape".into(),
            WinitKey::Named(WinitNamedKey::Enter) => "enter".into(),
            WinitKey::Named(WinitNamedKey::Space) => "space".into(),
            WinitKey::Named(WinitNamedKey::Home) => "home".into(),
            WinitKey::Named(WinitNamedKey::End) => "end".into(),
            WinitKey::Named(WinitNamedKey::Insert) => "insert".into(),
            WinitKey::Named(WinitNamedKey::Delete) => "delete".into(),
            WinitKey::Named(WinitNamedKey::Backspace) => "backspace".into(),
            WinitKey::Named(WinitNamedKey::F1) => "f1".into(),
            WinitKey::Named(WinitNamedKey::F2) => "f2".into(),
            WinitKey::Named(WinitNamedKey::F3) => "f3".into(),
            WinitKey::Named(WinitNamedKey::F4) => "f4".into(),
            WinitKey::Named(WinitNamedKey::F5) => "f5".into(),
            WinitKey::Named(WinitNamedKey::F6) => "f6".into(),
            WinitKey::Named(WinitNamedKey::F7) => "f7".into(),
            WinitKey::Named(WinitNamedKey::F8) => "f8".into(),
            WinitKey::Named(WinitNamedKey::F9) => "f9".into(),
            WinitKey::Named(WinitNamedKey::F10) => "f10".into(),
            WinitKey::Named(WinitNamedKey::F11) => "f11".into(),
            WinitKey::Named(WinitNamedKey::F12) => "f12".into(),
            _ => return None,
        };
        Some(Self {
            ctrl: modifiers.control_key(),
            shift: modifiers.shift_key(),
            alt: modifiers.alt_key(),
            super_key: modifiers.super_key(),
            key,
        })
    }
}

fn normalize_shortcut_key(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "+" | "equal" => "=".into(),
        "_" | "minus" => "-".into(),
        " " | "space" => "space".into(),
        "esc" => "escape".into(),
        "return" => "enter".into(),
        other => other.trim().to_string(),
    }
}

const SHORTCUT_ACTIONS: &[(&str, &str)] = &[
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
];

#[derive(Clone)]
struct Tab {
    layout: Layout,
    focused: PaneId,
    title: String,
}

#[derive(Clone, Copy, Debug)]
enum TabAction {
    Activate(usize),
    New,
    Close(usize),
    Find,
}

#[derive(Default)]
struct SearchState {
    open: bool,
    query: String,
    matches: Vec<(usize, usize, usize)>, // row, start col, length
    current: usize,
}

struct MouseState {
    position: Option<PhysicalPosition<f64>>,
    button: Option<pretty_core::MouseButton>,
    selecting: bool,
    selection_pane: Option<PaneId>,
    last_cell: Option<(PaneId, usize, usize)>,
}

impl Default for MouseState {
    fn default() -> Self {
        Self {
            position: None,
            button: None,
            selecting: false,
            selection_pane: None,
            last_cell: None,
        }
    }
}

#[derive(Clone, Debug)]
struct TouchGesture {
    id: u64,
    start: PhysicalPosition<f64>,
    last: PhysicalPosition<f64>,
    pane: PaneId,
    row: usize,
    col: usize,
    moved: bool,
    mouse: bool,
    selecting: bool,
}

pub struct PrettyApp {
    config: Config,
    options: AppOptions,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    settings: SettingsState,
    settings_was_open: bool,
    settings_anim_until: Option<Instant>,
    /// Modal confirmation shown before quitting when `confirm_close` is set.
    confirm_close_dialog: bool,
    /// Deadline for the visual-bell border flash.
    bell_flash_until: Option<Instant>,
    panes: HashMap<PaneId, Pane>,
    layout: Layout,
    focused: PaneId,
    tabs: Vec<Tab>,
    active_tab: usize,
    next_pane: u64,
    #[cfg(not(target_os = "android"))]
    clipboard: Option<arboard::Clipboard>,
    modifiers: ModifiersState,
    mouse: MouseState,
    scale_factor: f64,
    blink: BlinkPhase,
    blink_thread_stop: Arc<AtomicBool>,
    frame_times: Vec<f32>,
    last_frame: Instant,
    pub failed: bool,
    blur_applied: bool,
    frame_count: u64,
    last_log_frame: Instant,
    last_ui_change: Option<Instant>,
    pending_ui_apply: bool,
    cursor_logged: bool,
    last_cursor_shape: Option<pretty_core::CursorShape>,
    benchmark: Option<Benchmark>,
    lang: Lang,
    context_menu: Option<egui::Pos2>,
    search: SearchState,
    exit_requested: bool,
    drop_hover: Option<PathBuf>,
    ime_preedit: String,
    ime_cursor: Option<egui::Pos2>,
    /// Last time the pointer moved/clicked, for `mouse_hide_wait`.
    last_pointer_activity: Instant,
    /// Whether the OS pointer is currently hidden by the idle timer.
    pointer_hidden: bool,
    /// Logical size captured by `remember_window_size`, persisted on exit.
    pending_window_size: Option<(u32, u32)>,
    touch_positions: HashMap<u64, PhysicalPosition<f64>>,
    touch_scroll_y: Option<f64>,
    touch_scroll_pane: Option<PaneId>,
    touch_gesture: Option<TouchGesture>,
}

struct Benchmark {
    target: u32,
    frames: u32,
    start: Instant,
    last: Instant,
    intervals: Vec<f32>,
}

impl PrettyApp {
    pub fn new(config: Config, options: AppOptions, proxy: EventLoopProxy<UserEvent>) -> Self {
        let settings = SettingsState::new(&config);
        let benchmark = options.benchmark_frames.map(|target| Benchmark {
            target,
            frames: 0,
            start: Instant::now(),
            last: Instant::now(),
            intervals: Vec::new(),
        });
        let lang = Lang::from_setting(&config.language);
        let egui_ctx = egui::Context::default();
        install_ui_font(&egui_ctx);
        egui_ctx.set_zoom_factor(config.ui_scale.clamp(0.5, 2.5));
        // A 16 ms tick bounds blink jitter to ~one frame and, unlike
        // `ControlFlow::WaitUntil`, still fires while a flood of pointer events
        // keeps the loop from reaching `about_to_wait`.
        let initial_blink_interval =
            Duration::from_millis(config.cursor_blink_interval_ms.clamp(50, 5_000) as u64);
        let blink_thread_stop = Arc::new(AtomicBool::new(false));
        {
            let proxy = proxy.clone();
            let stop = Arc::clone(&blink_thread_stop);
            if let Err(e) = std::thread::Builder::new()
                .name("pretty-blink".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(16));
                        if stop.load(Ordering::Relaxed) || proxy.send_event(UserEvent::BlinkTick).is_err() {
                            break;
                        }
                    }
                })
            {
                log::warn!("cursor blink timer thread unavailable: {e}");
            }
        }
        Self {
            config,
            options,
            proxy,
            window: None,
            renderer: None,
            egui_ctx,
            egui_state: None,
            settings,
            settings_was_open: false,
            settings_anim_until: None,
            confirm_close_dialog: false,
            bell_flash_until: None,
            panes: HashMap::new(),
            layout: Layout::leaf(PaneId(0)),
            focused: PaneId(0),
            tabs: Vec::new(),
            active_tab: 0,
            next_pane: 1,
            #[cfg(not(target_os = "android"))]
            clipboard: None,
            modifiers: ModifiersState::default(),
            mouse: MouseState::default(),
            scale_factor: 1.0,
            blink: BlinkPhase::started(initial_blink_interval),
            blink_thread_stop,
            frame_times: Vec::new(),
            last_frame: Instant::now(),
            failed: false,
            blur_applied: false,
            frame_count: 0,
            last_log_frame: Instant::now(),
            last_ui_change: None,
            pending_ui_apply: false,
            cursor_logged: false,
            last_cursor_shape: None,
            benchmark,
            lang,
            context_menu: None,
            search: SearchState::default(),
            exit_requested: false,
            drop_hover: None,
            ime_preedit: String::new(),
            ime_cursor: None,
            last_pointer_activity: Instant::now(),
            pointer_hidden: false,
            pending_window_size: None,
            touch_positions: HashMap::new(),
            touch_scroll_y: None,
            touch_scroll_pane: None,
            touch_gesture: None,
        }
    }

    /// Effective pixels-per-point for the UI layer: HiDPI scale factor times
    /// the user-configured UI scale. Terminal cell metrics are intentionally not
    /// affected, so the UI can be enlarged without reflowing the terminal.
    fn ui_ppp(&self) -> f32 {
        (self.scale_factor as f32 * self.config.ui_scale.clamp(0.5, 2.5)).max(0.25)
    }

    fn apply_ui_scale(&self) {
        self.egui_ctx.set_zoom_factor(self.config.ui_scale.clamp(0.5, 2.5));
    }

    fn blink_interval(&self) -> Duration {
        Duration::from_millis(self.config.cursor_blink_interval_ms.clamp(50, 5_000) as u64)
    }

    fn reset_blink(&mut self) {
        self.blink.reset(self.blink_interval());
    }

    /// True when the cursor should be blinking right now.
    fn blink_active(&self) -> bool {
        self.config.cursor_blink && !self.settings.open && !self.confirm_close_dialog
    }

    /// Advance the blink phase if the deadline passed. Returns true when the
    /// visible phase changed and a redraw is needed.
    fn advance_blink(&mut self) -> bool {
        if !self.blink_active() {
            return false;
        }
        let interval = self.blink_interval();
        self.blink.advance(Instant::now(), interval)
    }

    /// Record pointer activity and reveal an idle-hidden pointer.
    fn note_pointer_activity(&mut self) {
        self.last_pointer_activity = Instant::now();
        if self.pointer_hidden {
            if let Some(window) = &self.window {
                window.set_cursor_visible(true);
            }
            self.pointer_hidden = false;
        }
    }

    /// Apply the kitty `mouse_hide_wait` policy. Called from the blink tick so
    /// it works even while no frames are being produced.
    fn update_pointer_visibility(&mut self) {
        let Some(window) = &self.window else { return };
        if self.config.mouse_hide_wait <= 0.0 {
            if self.pointer_hidden {
                window.set_cursor_visible(true);
                self.pointer_hidden = false;
            }
            return;
        }
        let timeout = Duration::from_secs_f32(self.config.mouse_hide_wait.clamp(0.0, 60.0));
        let idle = self.last_pointer_activity.elapsed();
        let hide = idle >= timeout && !self.settings.open && !self.confirm_close_dialog;
        if hide && !self.pointer_hidden {
            window.set_cursor_visible(false);
            self.pointer_hidden = true;
        } else if !hide && self.pointer_hidden {
            window.set_cursor_visible(true);
            self.pointer_hidden = false;
        }
    }

    /// Persist a pending debounced UI edit immediately. `std::process::exit`
    /// skips `ApplicationHandler::exiting`, so quit paths must call this first;
    /// otherwise a setting changed <220ms before quitting was silently lost.
    fn flush_config_save(&mut self) {
        if !self.pending_ui_apply {
            return;
        }
        self.pending_ui_apply = false;
        if let Err(e) = crate::config::save(&self.options.config_path, &self.config) {
            log::warn!("failed to save config: {e}");
        }
    }

    fn request_exit(&mut self) {
        if self.config.confirm_close {
            self.confirm_close_dialog = true;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return;
        }
        self.flush_config_save();
        self.exit_requested = true;
    }

    /// Shortcut text for menus/tooltips; empty when the action is unbound.
    fn shortcut_hint(&self, action: &str) -> String {
        self.config
            .shortcut(action)
            .filter(|binding| !binding.trim().is_empty())
            .unwrap_or("")
            .to_owned()
    }

    /// Import a kitty.conf into the live config, keep the settings draft in
    /// sync and report the result in the settings panel.
    fn perform_kitty_import(&mut self, path: &std::path::Path) {
        match crate::config::import_kitty_conf(path, &mut self.config) {
            Ok(report) => {
                self.config.validate();
                self.settings.draft = self.config.clone();
                let mut message = format!("Imported {}: {}", path.display(), report.summary());
                if !report.unsupported.is_empty() {
                    message.push_str(&format!(" Unsupported: {}.", report.unsupported.join(", ")));
                }
                if !report.warnings.is_empty() {
                    message.push_str(&format!(" Warnings: {}", report.warnings.join("; ")));
                }
                log::info!("kitty.conf import: {message}");
                self.settings.import_status = Some((true, message));
                self.config_changed();
            }
            Err(error) => {
                log::warn!("kitty.conf import failed: {error}");
                self.settings.import_status = Some((false, format!("Import failed: {error}")));
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let transparent_window = self.options.renderer == RendererKind::Gles
            || self.config.opacity < 0.999
            || self.config.blur;
        let attrs = Window::default_attributes()
            .with_title(self.options.title.clone().unwrap_or_else(|| "Pretty".into()))
            .with_inner_size(LogicalSize::new(
                self.config.window_width as f64,
                self.config.window_height as f64,
            ))
            .with_transparent(transparent_window)
            .with_decorations(!self.config.hide_window_decorations)
            .with_visible(true);
        let attrs = if self.options.fullscreen {
            attrs.with_fullscreen(Some(Fullscreen::Borderless(None)))
        } else {
            attrs
        };
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("failed to create window: {e}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        self.scale_factor = window.scale_factor();
        self.apply_ui_scale();
        let theme = self.config.theme();
        // On X11 the compositor blur effect only blurs a client buffer that
        // actually has alpha. Using _NET_WM_WINDOW_OPACITY alone makes the
        // whole window translucent but leaves the desktop unblurred. Prefer
        // per-pixel alpha when the X11 path is in use and transparency is
        // requested; KWin then blurs behind the background while text stays
        // opaque. Wayland keeps its native alpha path.
        #[cfg(target_os = "linux")]
        {
            let x11 = std::env::var_os("DISPLAY").is_some()
                && std::env::var_os("WAYLAND_DISPLAY").is_none();
            if x11 && self.config.opacity < 0.999 {
                std::env::set_var("PRETTY_PIXEL_ALPHA", "1");
                log::info!(
                    "X11 compositor: per-pixel alpha enabled (opacity {:.2})",
                    self.config.opacity
                );
            }
        }

        let renderer = match Renderer::new(
            window.clone(),
            event_loop.owned_display_handle(),
            self.options.renderer,
            theme,
            transparent_window,
            self.config.vsync,
            self.scale_factor as f32,
            self.options.allow_software,
        ) {
            Ok(r) => r,
            Err(e) => {
                log::error!("failed to initialize renderer: {e}");
                log::error!(
                    "no silent software fallback is performed; fix the GPU/driver or set PRETTY_RENDERER"
                );
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        log::info!(
            "renderer ready: selected={:?} adapter={:?} driver={}",
            renderer.gpu.info.selected,
            renderer.gpu.info.adapter.name,
            renderer.gpu.info.adapter.driver
        );
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(self.scale_factor as f32),
            window.theme(),
            None,
        );
        self.egui_state = Some(egui_state);
        self.renderer = Some(renderer);
        self.window = Some(window.clone());
        window.set_ime_allowed(true);

        // X11 blur properties must be set before the window is mapped. The
        // Wayland ext_background_effect object is created after the first
        // frame, once winit has committed the surface (see draw()).
        if self.config.blur && !platform::window_is_wayland(&window) {
            match platform::apply_compositor_effects(&window, self.config.blur, self.config.opacity) {
                Ok(()) => log::debug!("compositor effects applied before map; will reapply after first frame"),
                Err(e) => log::warn!("blur-behind unavailable: {e}"),
            }
        }

        // Initial pane.
        let id = PaneId(self.next_pane);
        self.next_pane += 1;
        self.layout = Layout::leaf(id);
        self.focused = id;
        let area = self.layout_area();
        let rect = self.layout.rects(area, self.gap()).first().map(|(_, r)| *r).unwrap_or(area);
        match self.spawn_pane(id, rect) {
            Ok(pane) => {
                self.panes.insert(id, pane);
            }
            Err(e) => {
                log::error!("failed to spawn shell: {e}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        }
        self.tabs.push(Tab {
            layout: self.layout.clone(),
            focused: self.focused,
            title: "Terminal 1".into(),
        });
        self.active_tab = 0;
        self.resize_all();
        self.update_title();
        self.window.as_ref().unwrap().request_redraw();
    }

    fn gap(&self) -> f32 {
        self.config.gap * self.scale_factor as f32
    }

    fn layout_area(&self) -> Rect {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or(PhysicalSize::new(1280, 800));
        let margin = self.ui_margin();
        let tab_h = self.tab_bar_height();
        let bottom_edge = self.config.tab_bar_edge == "bottom";
        let top = if bottom_edge { margin } else { margin + tab_h };
        let bottom = margin + if bottom_edge { tab_h } else { 0.0 };
        Rect::new(
            margin,
            top,
            (size.width as f32 - margin * 2.0).max(1.0),
            (size.height as f32 - top - bottom).max(1.0),
        )
    }

    /// Margin around the terminal panes in physical pixels (matches the egui
    /// space left around the tab bar, including UI scale).
    fn ui_margin(&self) -> f32 {
        (6.0 * self.ui_ppp()).round()
    }

    /// Whether the tab bar is currently shown, honouring `tab_bar_min_tabs`.
    fn tab_bar_shown(&self) -> bool {
        if !self.config.tab_bar_visible {
            return false;
        }
        let minimum = self.config.tab_bar_min_tabs.max(1);
        self.tabs.len() >= minimum
    }

    fn tab_bar_height(&self) -> f32 {
        if !self.tab_bar_shown() {
            return 0.0;
        }
        (self.config.tab_bar_height.clamp(18.0, 64.0) * self.ui_ppp()).round()
    }

    fn pane_grid_size(&self, rect: Rect) -> (usize, usize) {
        let metrics = self
            .renderer
            .as_ref()
            .map(|r| r.cell_metrics())
            .unwrap_or_default();
        let padding = metrics.padding;
        let cols = ((rect.w - padding * 2.0) / metrics.cell_w.max(1.0)).floor() as usize;
        let rows = ((rect.h - padding * 2.0) / metrics.cell_h.max(1.0)).floor() as usize;
        (cols.clamp(2, 1000), rows.clamp(1, 1000))
    }

    fn spawn_pane(&mut self, id: PaneId, rect: Rect) -> Result<Pane, String> {
        let (cols, rows) = self.pane_grid_size(rect);
        let metrics = self.renderer.as_ref().map(|r| r.cell_metrics()).unwrap_or_default();
        let cell_w = metrics.cell_w.round().max(1.0) as u16;
        let cell_h = metrics.cell_h.round().max(1.0) as u16;
        let shell = if self.config.shell.trim().is_empty() {
            crate::config::default_shell()
        } else {
            self.config.shell.clone()
        };
        let argv = match &self.options.command {
            Some(command) => shell_command(&shell, command),
            None => vec![shell],
        };
        let (backend, gpu_name) = self
            .renderer
            .as_ref()
            .map(|r| {
                (
                    r.gpu.info.selected.as_str().to_owned(),
                    r.gpu.info.adapter.name.clone(),
                )
            })
            .unwrap_or_else(|| ("unknown".into(), "unknown".into()));
        let env = vec![
            ("TERM".to_owned(), "pretty".to_owned()),
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
            ("TERM_PROGRAM".to_owned(), "Pretty".to_owned()),
            ("TERM_PROGRAM_VERSION".to_owned(), env!("CARGO_PKG_VERSION").to_owned()),
            ("PRETTY_BACKEND".to_owned(), backend),
            ("PRETTY_GPU".to_owned(), gpu_name),
            ("PRETTY_PANE_ID".to_owned(), id.0.to_string()),
        ];
        let proxy = self.proxy.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = proxy.send_event(UserEvent::PtyReady(id));
        });
        let mut pty = PtySession::spawn(
            argv,
            env,
            std::env::current_dir().ok(),
            cols as u16,
            rows as u16,
            cell_w,
            cell_h,
            wake,
        )
        .map_err(|e| e.to_string())?;
        let pty_rx = pty.take_receiver().ok_or("PTY reader channel missing")?;
        let core_config = pretty_core::CoreConfig {
            scrollback: self.config.scrollback,
            kitty_keyboard: true,
            osc52_copy: self.config.allow_osc52_copy,
            osc52_paste: self.config.allow_osc52_paste,
            cursor_style: CursorStyle {
                shape: cursor_shape_from_config(&self.config.cursor_shape),
                blinking: self.config.cursor_blink,
            },
        };
        let (term, term_events) = Terminal::new(TermSize::new(cols, rows), &core_config);
        log::info!(
            "pane {} spawned: pid={:?} {}x{} cells at {}x{}px",
            id.0,
            pty.child_pid(),
            cols,
            rows,
            rect.x,
            rect.y
        );
        Ok(Pane {
            term,
            pty,
            pty_rx,
            term_events,
            title: self
                .options
                .title
                .clone()
                .unwrap_or_else(|| "Pretty".into()),
            rect,
            cols,
            rows,
        })
    }

    fn resize_all(&mut self) {
        if self.renderer.is_none() {
            return;
        }
        let area = self.layout_area();
        let rects = self.layout.rects(area, self.gap());
        let metrics = self.renderer.as_ref().unwrap().cell_metrics();
        let cell_w = metrics.cell_w.round().max(1.0) as u16;
        let cell_h = metrics.cell_h.round().max(1.0) as u16;
        for (id, rect) in rects {
            let (cols, rows) = self.pane_grid_size(rect);
            let Some(pane) = self.panes.get_mut(&id) else { continue };
            pane.rect = rect;
            if (cols, rows) != (pane.cols, pane.rows) {
                pane.cols = cols;
                pane.rows = rows;
                pane.term.resize(cols, rows);
                if let Err(e) = pane.pty.resize(cols as u16, rows as u16, cell_w, cell_h) {
                    log::warn!("pane {} PTY resize failed: {e}", id.0);
                }
                log::debug!("pane {} resized to {}x{}", id.0, cols, rows);
            }
        }
    }

    fn update_title(&self) {
        let Some(window) = &self.window else { return };
        let title = self
            .panes
            .get(&self.focused)
            .map(|p| p.title.as_str())
            .filter(|t| !t.is_empty())
            .unwrap_or("Pretty");
        window.set_title(&format!("{title} — Pretty"));
    }

    fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.focused)
    }

    fn write_to_focused(&mut self, bytes: &[u8]) {
        let Some(pane) = self.focused_pane_mut() else { return };
        if let Err(e) = pane.pty.write(bytes) {
            log::warn!("PTY write failed: {e}");
        }
        if pane.term.display_offset() > 0 {
            pane.term.scroll_to_bottom();
        }
        self.reset_blink();
    }

    #[cfg(not(target_os = "android"))]
    fn clipboard_set(&mut self, text: String) -> Result<(), String> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.clipboard = Some(c),
                Err(e) => return Err(format!("clipboard unavailable: {e}")),
            }
        }
        self.clipboard
            .as_mut()
            .ok_or_else(|| "clipboard unavailable".to_string())?
            .set_text(text)
            .map_err(|e| e.to_string())
    }

    #[cfg(target_os = "android")]
    fn clipboard_set(&mut self, _text: String) -> Result<(), String> {
        Err("clipboard is not supported on Android".into())
    }

    #[cfg(not(target_os = "android"))]
    fn clipboard_get(&mut self) -> Result<String, String> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.clipboard = Some(c),
                Err(e) => return Err(format!("clipboard unavailable: {e}")),
            }
        }
        self.clipboard
            .as_mut()
            .ok_or_else(|| "clipboard unavailable".to_string())?
            .get_text()
            .map_err(|e| e.to_string())
    }

    #[cfg(target_os = "android")]
    fn clipboard_get(&mut self) -> Result<String, String> {
        Err("clipboard is not supported on Android".into())
    }

    fn copy_selection(&mut self) {
        let Some(text) = self.panes.get(&self.focused).and_then(|p| p.term.selection_text()) else {
            return;
        };
        let len = text.len();
        match self.clipboard_set(text) {
            Ok(()) => log::info!("copied {len} bytes to clipboard"),
            Err(e) => log::warn!("clipboard copy failed: {e}"),
        }
    }

    fn paste_clipboard(&mut self) {
        let Ok(text) = self.clipboard_get() else { return };
        // File managers put `file:///...` URIs on the clipboard rather than a
        // shell-ready path. Convert those to quoted local paths so pasting a
        // file behaves like dropping one.
        let text = file_uris_to_shell_paths(&text).unwrap_or(text);
        let mode = self.panes.get(&self.focused).map(|p| p.term.mode()).unwrap_or_default();
        let bytes = encode_paste(mode, &text);
        self.write_to_focused(&bytes);
    }

    fn select_all(&mut self) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.term.select_all();
        }
    }

    fn open_search(&mut self) {
        log::info!("find overlay opened");
        self.search.open = true;
        self.search.current = 0;
        self.egui_ctx
            .memory_mut(|m| m.request_focus(egui::Id::new("pretty-search-input")));
        self.refresh_search();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn refresh_search(&mut self) {
        self.search.matches.clear();
        self.search.current = 0;
        let query = self.search.query.to_lowercase();
        if query.is_empty() {
            if let Some(pane) = self.focused_pane_mut() {
                pane.term.clear_selection();
            }
            return;
        }
        // Match on characters, never byte offsets: the previous code mixed
        // `lower[from..].find()` byte positions with per-char column indices,
        // so CJK/emoji matches highlighted the wrong range or ran past the row.
        let query_chars: Vec<char> = query.chars().collect();
        let Some(pane) = self.focused_pane_mut() else { return };
        let mut found: Vec<(usize, usize, usize)> = Vec::new();
        let saved_offset = pane.term.display_offset();
        let history = pane.term.history_size();
        let rows = pane.rows.max(1);
        let cols = pane.cols;
        pane.term.scroll_to_top();
        loop {
            let frame = pane.term.snapshot();
            let base = history.saturating_sub(frame.display_offset);
            for row in 0..frame.rows {
                let mut chars: Vec<char> = Vec::new();
                let mut char_cols = Vec::new();
                for col in 0..cols {
                    let Some(cell) = frame.cell(row, col) else { break };
                    if cell.wide_spacer {
                        continue;
                    }
                    for ch in std::iter::once(cell.c).chain(cell.zero_width.iter().copied()) {
                        chars.push(ch);
                        char_cols.push(col);
                    }
                }
                // Fold per character and remember which original character each
                // folded scalar came from (case mapping may expand a char).
                let mut lower: Vec<char> = Vec::new();
                let mut lower_to_char: Vec<usize> = Vec::new();
                for (index, ch) in chars.iter().enumerate() {
                    for folded in ch.to_lowercase() {
                        lower.push(folded);
                        lower_to_char.push(index);
                    }
                }
                let mut start = 0;
                while start + query_chars.len() <= lower.len() {
                    if lower[start..start + query_chars.len()] == query_chars[..] {
                        let first = lower_to_char[start];
                        let last = lower_to_char[start + query_chars.len() - 1];
                        if let Some(&col) = char_cols.get(first) {
                            found.push((base + row, col, (last + 1 - first).max(1)));
                        }
                        start += query_chars.len().max(1);
                    } else {
                        start += 1;
                    }
                }
            }
            if frame.display_offset == 0 {
                break;
            }
            pane.term.scroll_page_down();
            if pane.term.display_offset() == frame.display_offset {
                break;
            }
            if pane.term.display_offset() > history + rows {
                break;
            }
        }
        pane.term.scroll_to_bottom();
        if saved_offset > 0 {
            pane.term.scroll(saved_offset as i32);
        }
        self.search.matches = found;
        if !self.search.matches.is_empty() {
            self.select_search_match();
        } else if let Some(pane) = self.focused_pane_mut() {
            pane.term.clear_selection();
        }
    }

    fn select_search_match(&mut self) {
        let Some(&(global_line, col, len)) = self.search.matches.get(self.search.current) else { return };
        let Some(pane) = self.focused_pane_mut() else { return };
        let history = pane.term.history_size();
        let delta = history.saturating_sub(global_line).min(history) as i32;
        pane.term.scroll_to_bottom();
        if delta > 0 {
            pane.term.scroll(delta);
        }
        let row = global_line.saturating_sub(history.saturating_sub(pane.term.display_offset()));
        pane.term.start_selection(row, col);
        pane.term.update_selection(row, col.saturating_add(len.saturating_sub(1)));
    }

    fn next_search_match(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        self.search.current = (self.search.current + 1) % self.search.matches.len();
        self.select_search_match();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn configured_shortcut(&self, event: &winit::event::KeyEvent) -> Option<&'static str> {
        let pressed = Shortcut::from_event(event, self.modifiers)?;
        for (action, _) in SHORTCUT_ACTIONS {
            let Some(binding) = self.config.shortcut(action) else { continue };
            if let Some(want) = Shortcut::parse(binding) {
                if want == pressed {
                    return Some(action);
                }
            }
        }
        None
    }

    fn run_shortcut(&mut self, action: &str) -> bool {
        match action {
            "copy" => self.copy_selection(),
            "paste" => self.paste_clipboard(),
            "select_all" => self.select_all(),
            "new_tab" => self.new_tab(),
            "close_pane" => self.close_pane(self.focused),
            "split_right" => self.split_pane(pretty_core::Axis::Horizontal),
            "split_down" => self.split_pane(pretty_core::Axis::Vertical),
            "find" => self.open_search(),
            "settings" => {
                self.settings.open = !self.settings.open;
                if self.settings.open {
                    self.settings.sync(&self.config);
                    self.settings.draft = self.config.clone();
                }
            }
            "next_tab" => self.next_tab(1),
            "prev_tab" => self.next_tab(-1),
            "quit" => self.request_exit(),
            "zoom_in" => self.adjust_font_size(1.0),
            "zoom_out" => self.adjust_font_size(-1.0),
            "zoom_reset" => {
                self.config.font_size = crate::config::Config::default().font_size;
                self.config_changed();
            }
            _ => return false,
        }
        true
    }

    fn handle_shortcut(&mut self, event: &winit::event::KeyEvent) -> bool {
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
        let alt = self.modifiers.alt_key();
        if event.state != ElementState::Pressed {
            return false;
        }
        let logical = event.logical_key.as_ref();
        if self.search.open && matches!(logical, WinitKey::Named(WinitNamedKey::Escape)) {
            self.search.open = false;
            if let Some(pane) = self.focused_pane_mut() {
                pane.term.clear_selection();
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return true;
        }
        // While an overlay text field owns the keyboard, only modified
        // shortcuts are global so a single-key binding cannot eat typed text.
        let overlay_open = self.settings.open || self.search.open || self.context_menu.is_some();
        if overlay_open && !ctrl && !alt && !self.modifiers.super_key() {
            return false;
        }
        if let Some(action) = self.configured_shortcut(event) {
            if self.run_shortcut(action) {
                return true;
            }
        }
        if ctrl && shift {
            match logical {
                WinitKey::Named(WinitNamedKey::ArrowUp) => {
                    if let Some(p) = self.focused_pane_mut() {
                        p.term.scroll(1);
                    }
                    return true;
                }
                WinitKey::Named(WinitNamedKey::ArrowDown) => {
                    if let Some(p) = self.focused_pane_mut() {
                        p.term.scroll(-1);
                    }
                    return true;
                }
                _ => {}
            }
        }
        if shift && !ctrl {
            match logical {
                WinitKey::Named(WinitNamedKey::PageUp) => {
                    if let Some(p) = self.focused_pane_mut() {
                        p.term.scroll_page_up();
                    }
                    return true;
                }
                WinitKey::Named(WinitNamedKey::PageDown) => {
                    if let Some(p) = self.focused_pane_mut() {
                        p.term.scroll_page_down();
                    }
                    return true;
                }
                _ => {}
            }
        }
        if alt && !ctrl {
            match logical {
                WinitKey::Named(WinitNamedKey::ArrowUp) => {
                    self.focus_direction(0, -1);
                    return true;
                }
                WinitKey::Named(WinitNamedKey::ArrowDown) => {
                    self.focus_direction(0, 1);
                    return true;
                }
                WinitKey::Named(WinitNamedKey::ArrowLeft) => {
                    self.focus_direction(-1, 0);
                    return true;
                }
                WinitKey::Named(WinitNamedKey::ArrowRight) => {
                    self.focus_direction(1, 0);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    fn adjust_font_size(&mut self, delta: f32) {
        self.config.font_size = (self.config.font_size + delta).clamp(7.0, 42.0);
        self.config_changed();
    }

    fn config_changed(&mut self) {
        self.config.validate();
        self.lang = Lang::from_setting(&self.config.language);
        self.apply_ui_scale();
        // Theme/compositor changes are cheap and should be visible while a
        // slider moves. Expensive PTY resizing and disk writes are debounced:
        // saving on every egui frame made dragging a toggle visibly stutter.
        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(e) = renderer.set_theme(self.config.theme()) {
                log::error!("failed to apply theme: {e}");
            }
        }
        if let Some(window) = &self.window {
            window.set_decorations(!self.config.hide_window_decorations);
            if self.config.blur {
                if let Err(e) = platform::apply_compositor_effects(window, self.config.blur, self.config.opacity) {
                    log::warn!("blur unavailable: {e}");
                }
            } else {
                let _ = platform::apply_compositor_effects(window, false, self.config.opacity);
            }
        }
        // The tab bar edge/height/min-tabs affect how many cells fit.
        self.resize_all();
        // Apply the scrollback limit to live terminals, not only to panes
        // created afterwards (the settings slider used to need a new tab).
        let scrollback = self.config.scrollback;
        for pane in self.panes.values_mut() {
            pane.term.set_scrollback(scrollback);
        }
        self.last_ui_change = Some(Instant::now());
        self.pending_ui_apply = true;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn save_active_tab(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.layout = self.layout.clone();
            tab.focused = self.focused;
            if let Some(pane) = self.panes.get(&self.focused) {
                if !pane.title.is_empty() {
                    tab.title = pane.title.clone();
                }
            }
        }
    }

    fn load_active_tab(&mut self) {
        if let Some(tab) = self.tabs.get(self.active_tab) {
            self.layout = tab.layout.clone();
            self.focused = tab.focused;
            if !self.layout.contains(self.focused) {
                self.focused = self.layout.leaves().first().copied().unwrap_or(PaneId(0));
            }
        }
    }

    fn new_tab(&mut self) {
        self.save_active_tab();
        let area = self.layout_area();
        let id = PaneId(self.next_pane);
        let rect = area;
        match self.spawn_pane(id, rect) {
            Ok(pane) => {
                self.panes.insert(id, pane);
                self.next_pane += 1;
                self.layout = Layout::leaf(id);
                self.focused = id;
                let n = self.tabs.len() + 1;
                self.tabs.push(Tab {
                    layout: self.layout.clone(),
                    focused: id,
                    title: format!("Terminal {n}"),
                });
                self.active_tab = self.tabs.len() - 1;
                self.resize_all();
                self.update_title();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                log::info!("tab {n} opened");
            }
            Err(e) => log::error!("failed to open tab: {e}"),
        }
    }

    fn activate_tab(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.active_tab {
            return;
        }
        self.save_active_tab();
        self.active_tab = index;
        self.load_active_tab();
        self.resize_all();
        self.update_title();
        self.reset_blink();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn next_tab(&mut self, delta: isize) {
        if self.tabs.len() < 2 {
            return;
        }
        let n = self.tabs.len() as isize;
        let next = (self.active_tab as isize + delta).rem_euclid(n) as usize;
        self.activate_tab(next);
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.request_exit();
            return;
        }
        if index != self.active_tab {
            // Closing a *background* tab used to reload the active tab from its
            // last saved snapshot, silently reverting a pane focus change made
            // since the last tab switch. Persist the live state first.
            self.save_active_tab();
        }
        let tab = self.tabs.remove(index);
        for id in tab.layout.leaves() {
            self.panes.remove(&id);
        }
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if index < self.active_tab {
            self.active_tab -= 1;
        }
        self.load_active_tab();
        self.resize_all();
        self.update_title();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        log::info!("tab closed");
    }

    fn split_pane(&mut self, axis: pretty_core::Axis) {
        let id = PaneId(self.next_pane);
        let target = self.focused;
        let Some(base_rect) = self.panes.get(&target).map(|p| p.rect) else { return };
        let approx = match axis {
            pretty_core::Axis::Horizontal => Rect::new(base_rect.x, base_rect.y, base_rect.w / 2.0, base_rect.h),
            pretty_core::Axis::Vertical => Rect::new(base_rect.x, base_rect.y, base_rect.w, base_rect.h / 2.0),
        };
        match self.spawn_pane(id, approx) {
            Ok(pane) => {
                self.panes.insert(id, pane);
                if !self.layout.split(target, id, axis) {
                    log::warn!("could not split layout for pane {target:?}");
                    self.panes.remove(&id);
                    return;
                }
                self.next_pane += 1;
                self.focused = id;
                self.resize_all();
                self.update_title();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                log::info!("pane {} split ({axis:?})", id.0);
                self.save_active_tab();
            }
            Err(e) => log::error!("failed to create split pane: {e}"),
        }
    }

    fn close_pane(&mut self, id: PaneId) {
        // Exit/ChildExit can arrive for a pane belonging to a *background* tab.
        // The old code only edited the active layout, which either leaked the
        // dead pane or closed the wrong (active) tab. Resolve the owner first.
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.layout.contains(id)) else {
            // Already gone (e.g. duplicate exit event): clean up defensively.
            self.panes.remove(&id);
            return;
        };
        if tab_index != self.active_tab {
            if self.tabs[tab_index].layout.leaves().len() <= 1 {
                self.panes.remove(&id);
                self.close_tab(tab_index);
                return;
            }
            if let Some(tab) = self.tabs.get_mut(tab_index) {
                if tab.layout.remove(id) && !tab.layout.contains(tab.focused) {
                    tab.focused = tab.layout.leaves().first().copied().unwrap_or(PaneId(0));
                }
            }
            self.panes.remove(&id);
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return;
        }
        if self.layout.leaves().len() <= 1 {
            self.close_tab(self.active_tab);
            return;
        }
        if self.layout.remove(id) {
            self.panes.remove(&id);
            let remaining = self.layout.leaves();
            self.focused = remaining.first().copied().unwrap_or(PaneId(0));
            self.resize_all();
            self.update_title();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            self.save_active_tab();
        }
    }

    fn focus_direction(&mut self, dx: i32, dy: i32) {
        let current = match self.panes.get(&self.focused) {
            Some(p) => p.rect,
            None => return,
        };
        let (cx, cy) = current.center();
        let mut best: Option<(PaneId, f32)> = None;
        let active: std::collections::HashSet<PaneId> =
            self.layout.leaves().into_iter().collect();
        for (id, pane) in &self.panes {
            if *id == self.focused || !active.contains(id) {
                continue;
            }
            let (px, py) = pane.rect.center();
            let vx = px - cx;
            let vy = py - cy;
            let wrong = (dx < 0 && vx >= -1.0)
                || (dx > 0 && vx <= 1.0)
                || (dy < 0 && vy >= -1.0)
                || (dy > 0 && vy <= 1.0);
            if dx != 0 && vx.signum() as i32 != dx {
                continue;
            }
            if dy != 0 && vy.signum() as i32 != dy {
                continue;
            }
            if wrong {
                continue;
            }
            let dist = vx.abs() + vy.abs();
            if best.map(|(_, d)| dist < d).unwrap_or(true) {
                best = Some((*id, dist));
            }
        }
        if let Some((id, _)) = best {
            self.focused = id;
            self.update_title();
            self.save_active_tab();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn pane_at(&self, x: f64, y: f64) -> Option<(PaneId, usize, usize)> {
        let m = self.renderer.as_ref()?.cell_metrics();
        for (id, pane) in &self.panes {
            // Only panes of the active tab are visible and interactive. Panes
            // of inactive tabs keep stale rectangles; the old loop let a click
            // in a hidden area focus an invisible shell and route keys to it.
            if !self.layout.contains(*id) || !pane.rect.contains(x as f32, y as f32) {
                continue;
            }
            // Clicks in the padding clamp to the nearest cell instead of
            // always reporting cell (0, 0).
            let px = (x as f32 - pane.rect.x - m.padding).max(0.0);
            let py = (y as f32 - pane.rect.y - m.padding).max(0.0);
            let col = (px / m.cell_w.max(1.0)).floor() as usize;
            let row = (py / m.cell_h.max(1.0)).floor() as usize;
            return Some((
                *id,
                col.min(pane.cols.saturating_sub(1)),
                row.min(pane.rows.saturating_sub(1)),
            ));
        }
        None
    }

    fn handle_pty(&mut self, id: PaneId, event_loop: &ActiveEventLoop) {
        let mut chunks = Vec::new();
        let mut disconnected = false;
        {
            let Some(pane) = self.panes.get(&id) else { return };
            loop {
                match pane.pty_rx.try_recv() {
                    Ok(chunk) => chunks.push(chunk),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if chunks.is_empty() && !disconnected {
            return;
        }
        if !chunks.is_empty() {
            let Some(pane) = self.panes.get_mut(&id) else { return };
            for chunk in chunks {
                pane.term.advance(&chunk);
            }
        }
        self.reset_blink();
        self.handle_terminal_events(id);
        if disconnected {
            // The PTY reader thread ends on EOF: the child exited and all of
            // its output has been drained. The old code ignored this and kept
            // a frozen, dead terminal (and its PTY handles) forever.
            log::info!("pane {:?} child exited (PTY closed); removing the pane", id);
            self.close_pane(id);
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        let _ = event_loop;
    }

    fn handle_terminal_events(&mut self, id: PaneId) {
        let events: Vec<TerminalEvent> = {
            let Some(pane) = self.panes.get(&id) else { return };
            let mut events = Vec::new();
            while let Ok(ev) = pane.term_events.try_recv() {
                events.push(ev);

            }
            events
        };
        for event in events {
            match event {
                TerminalEvent::Title(title) => {
                    if let Some(pane) = self.panes.get_mut(&id) {
                        pane.title = title;
                    }
                    if self.focused == id {
                        self.update_title();
                    }
                }
                TerminalEvent::ResetTitle => {
                    if let Some(pane) = self.panes.get_mut(&id) {
                        pane.title = "Pretty".into();
                    }
                    if self.focused == id {
                        self.update_title();
                    }
                }
                TerminalEvent::Bell => {
                    log::info!("terminal bell (pane {})", id.0);
                    match self.config.bell.as_str() {
                        "none" => {}
                        "visual" => {
                            let duration = Duration::from_millis(
                                self.config.visual_bell_duration_ms.clamp(20, 2_000) as u64,
                            );
                            self.bell_flash_until = Some(Instant::now() + duration);
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                        }
                        _ => {
                            if let Some(w) = &self.window {
                                w.request_user_attention(Some(
                                    winit::window::UserAttentionType::Informational,
                                ));
                            }
                        }
                    }
                }
                TerminalEvent::ClipboardStore(_kind, text) => {
                    match self.clipboard_set(text) {
                        Ok(()) => log::info!("OSC 52 clipboard store accepted"),
                        Err(e) => log::warn!("OSC 52 clipboard store failed: {e}"),
                    }
                }
                TerminalEvent::ClipboardLoad(_kind, formatter) => {
                    let text = self.clipboard_get().unwrap_or_default();
                    let response = formatter(&text);
                    if let Some(pane) = self.panes.get_mut(&id) {
                        let _ = pane.pty.write(response.as_bytes());
                    }
                }
                TerminalEvent::ColorRequest(index, formatter) => {
                    let theme = self.config.theme();
                    let rgb = match index {
                        256 => theme.foreground,
                        257 => theme.background,
                        258 => theme.cursor,
                        n if n < 16 => theme.palette[n],
                        n if n < 256 => pretty_core::color::palette_rgb(n as u8),
                        _ => theme.foreground,
                    };
                    let response = formatter(VteRgb { r: rgb.r, g: rgb.g, b: rgb.b });
                    if let Some(pane) = self.panes.get_mut(&id) {
                        let _ = pane.pty.write(response.as_bytes());
                    }
                }
                TerminalEvent::PtyWrite(text) => {
                    if let Some(pane) = self.panes.get_mut(&id) {
                        if let Err(e) = pane.pty.write(text.as_bytes()) {
                            log::warn!("terminal response write failed: {e}");
                        }
                    }
                }
                TerminalEvent::TextAreaSizeRequest(formatter) => {
                    if let Some(pane) = self.panes.get(&id) {
                        let m = self.renderer.as_ref().map(|r| r.cell_metrics()).unwrap_or_default();
                        let response = formatter(
                            pane.cols as u16,
                            pane.rows as u16,
                            m.cell_w.round() as u16,
                            m.cell_h.round() as u16,
                        );
                        if let Some(pane) = self.panes.get_mut(&id) {
                            let _ = pane.pty.write(response.as_bytes());
                        }
                    }
                }
                TerminalEvent::CursorBlinkingChange => {
                    self.reset_blink();
                }
                TerminalEvent::Wakeup => {}
                TerminalEvent::Exit => self.close_pane(id),
                TerminalEvent::ChildExit(_) => self.close_pane(id),
                TerminalEvent::Notification { title, body } => {
                    log::info!("desktop notification: {title}: {body}");
                    let title2 = title.clone();
                    let body2 = body.clone();
                    std::thread::spawn(move || {
                        match pretty_core::notify::send_notification(&title2, &body2) {
                            Ok(()) => log::info!("notification delivered: {title2}"),
                            Err(e) => log::warn!("notification delivery failed: {e}"),
                        }
                    });
                }
            }
        }
    }

    fn translate_key(&self, event: &winit::event::KeyEvent) -> Option<KeyInput> {
        let key = match &event.logical_key {
            WinitKey::Character(s) => Key::Char(s.chars().next()?),
            WinitKey::Named(WinitNamedKey::Space) => Key::Char(' '),
            WinitKey::Named(named) => Key::Named(map_named(*named)?),
            _ => return None,
        };
        let text = event.text.as_ref().map(|s| s.to_string());
        let kind = if event.state == ElementState::Released {
            KeyEventKind::Release
        } else if event.repeat {
            KeyEventKind::Repeat
        } else {
            KeyEventKind::Press
        };
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        let base_layout = match event.key_without_modifiers().as_ref() {
            WinitKey::Character(s) => s.chars().next(),
            _ => None,
        };
        #[cfg(target_os = "android")]
        let base_layout = match &event.logical_key {
            WinitKey::Character(s) => s.chars().next(),
            _ => None,
        };
        Some(KeyInput {
            key,
            text,
            base_layout,
            modifiers: CoreModifiers {
                shift: self.modifiers.shift_key(),
                ctrl: self.modifiers.control_key(),
                alt: self.modifiers.alt_key(),
                super_key: self.modifiers.super_key(),
            },
            kind,
        })
    }

    fn process_key(&mut self, event: &winit::event::KeyEvent) {
        if self.confirm_close_dialog {
            return;
        }
        if self.handle_shortcut(event) {
            return;
        }
        if self.settings.open || self.search.open || self.context_menu.is_some() {
            return;
        }
        if event.state == ElementState::Released {
            let flags = self
                .panes
                .get(&self.focused)
                .map(|p| p.term.kitty_keyboard_flags())
                .unwrap_or(0);
            if flags & 2 == 0 {
                return;
            }
        }
        if let Some(input) = self.translate_key(event) {
            let mode = self.panes.get(&self.focused).map(|p| p.term.mode()).unwrap_or_default();
            let bytes = encode_key(mode, &input);
            if !bytes.is_empty() {
                self.write_to_focused(&bytes);
            }
        }
    }

    fn process_mouse_input(&mut self, event_loop: &ActiveEventLoop, state: ElementState, button: WinitMouseButton, egui_consumed: bool) {
        let Some(pos) = self.mouse.position else { return };
        let hit = self.pane_at(pos.x, pos.y);
        let (pane_id, col, row) = match hit {
            Some(cell) => cell,
            None => {
                if state != ElementState::Released {
                    return;
                }
                // The pointer left the pane (or the terminal area) while a
                // button was held. The release must still reach the pane that
                // received the press, otherwise TUIs stay stuck in drag/select
                // state until another click.
                let Some(id) = self
                    .mouse
                    .selection_pane
                    .or_else(|| self.mouse.last_cell.map(|(id, _, _)| id))
                else {
                    self.mouse.button = None;
                    self.mouse.selecting = false;
                    return;
                };
                let (col, row) = self
                    .mouse
                    .last_cell
                    .filter(|(last_id, _, _)| *last_id == id)
                    .map(|(_, c, r)| (c, r))
                    .unwrap_or((0, 0));
                (id, col, row)
            }
        };
        log::debug!(
            "mouse input state={:?} button={:?} at {:?} consumed={} menu={:?}",
            state, button, pos, egui_consumed, self.context_menu
        );
        if egui_consumed || self.settings.open {
            return;
        }
        if self.context_menu.is_some() && state == ElementState::Pressed {
            self.context_menu = None;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return;
        }
        let previous_focus = self.focused;
        self.focused = pane_id;
        if previous_focus != pane_id {
            self.update_title();
        }
        let Some(button) = map_mouse_button(button) else { return };
        let shift = self.modifiers.shift_key();
        let mode = self.panes.get(&pane_id).map(|p| p.term.mode()).unwrap_or_default();
        if state == ElementState::Pressed
            && button == pretty_core::MouseButton::Right
            && (shift || !mode.intersects(TermMode::MOUSE_MODE))
        {
            let ppp = self.ui_ppp().max(0.25) as f64;
            self.context_menu = Some(egui::pos2(
                (pos.x / ppp) as f32,
                (pos.y / ppp) as f32,
            ));
            log::info!("context menu requested at {:?}", self.context_menu);
            self.mouse.button = None;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return;
        }
        if state == ElementState::Pressed
            && button == pretty_core::MouseButton::Left
            && self.modifiers.control_key()
        {
            let uri = self
                .panes
                .get(&pane_id)
                .and_then(|pane| pane.term.hyperlink_at(row, col));
            if let Some(uri) = uri {
                // OSC 8 targets are untrusted terminal output: any process can
                // emit one. Never hand file://, custom handler schemes or
                // credential-bearing URLs to the desktop opener.
                if !is_safe_hyperlink(&uri) {
                    log::warn!("refusing to open OSC 8 hyperlink with unsupported scheme: {uri}");
                    return;
                }
                log::info!("opening OSC 8 hyperlink: {uri}");
                if let Err(e) = open::that_detached(&uri) {
                    log::warn!("could not open hyperlink: {e}");
                }
                return;
            }
        }
        if state == ElementState::Pressed {
            self.mouse.button = Some(button);
            let mode = self.panes.get(&pane_id).map(|p| p.term.mode()).unwrap_or_default();
            if mode.intersects(TermMode::MOUSE_MODE) && !shift {
                let bytes = encode_mouse(mode, button, MouseEventKind::Press, col, row, self.core_modifiers());
                if let Some(bytes) = bytes {
                    if let Some(pane) = self.panes.get_mut(&pane_id) {
                        let _ = pane.pty.write(&bytes);
                    }
                }
            } else if button == pretty_core::MouseButton::Left {
                if let Some(pane) = self.panes.get_mut(&pane_id) {
                    pane.term.start_selection(row, col);
                }
                self.mouse.selecting = true;
                self.mouse.selection_pane = Some(pane_id);
            }
        } else {
            self.mouse.button = None;
            if self.mouse.selecting {
                self.mouse.selecting = false;
                self.mouse.selection_pane = None;
                if self.config.copy_on_select
                    && self
                        .panes
                        .get(&pane_id)
                        .map(|pane| pane.term.selection_present())
                        .unwrap_or(false)
                {
                    self.copy_selection();
                }
            } else {
                let mode = self.panes.get(&pane_id).map(|p| p.term.mode()).unwrap_or_default();
                if mode.intersects(TermMode::MOUSE_MODE) && !shift {
                    let bytes = encode_mouse(mode, button, MouseEventKind::Release, col, row, self.core_modifiers());
                    if let Some(bytes) = bytes {
                        if let Some(pane) = self.panes.get_mut(&pane_id) {
                            let _ = pane.pty.write(&bytes);
                        }
                    }
                }
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        let _ = event_loop;
    }

    fn process_cursor_moved(&mut self, pos: PhysicalPosition<f64>, egui_consumed: bool) {
        self.mouse.position = Some(pos);
        if self.settings.open || self.context_menu.is_some() || egui_consumed {
            return;
        }
        let Some((pane_id, col, row)) = self.pane_at(pos.x, pos.y) else { return };
        // kitty `focus_follows_mouse`: hovering a pane moves keyboard focus
        // there, but never while a button is held (selection/drag in progress).
        if self.config.focus_follows_mouse
            && pane_id != self.focused
            && self.mouse.button.is_none()
            && !self.mouse.selecting
        {
            self.focused = pane_id;
            self.update_title();
            self.save_active_tab();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if self.mouse.selecting && self.mouse.selection_pane == Some(pane_id) {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.term.update_selection(row, col);
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return;
        }
        let mode = self.panes.get(&pane_id).map(|p| p.term.mode()).unwrap_or_default();
        if mode.intersects(TermMode::MOUSE_MODE) && !self.modifiers.shift_key() {
            let Some(button) = self.mouse.button else {
                if mode.contains(TermMode::MOUSE_MOTION) {
                    let bytes = encode_mouse(mode, pretty_core::MouseButton::None, MouseEventKind::Motion, col, row, self.core_modifiers());
                    if let Some(bytes) = bytes {
                        if let Some(pane) = self.panes.get_mut(&pane_id) {
                            let _ = pane.pty.write(&bytes);
                        }
                    }
                }
                return;
            };
            let last = self.mouse.last_cell;
            if last == Some((pane_id, col, row)) {
                return;
            }
            self.mouse.last_cell = Some((pane_id, col, row));
            if mode.contains(TermMode::MOUSE_DRAG) || mode.contains(TermMode::MOUSE_MOTION) {
                let bytes = encode_mouse(mode, button, MouseEventKind::Motion, col, row, self.core_modifiers());
                if let Some(bytes) = bytes {
                    if let Some(pane) = self.panes.get_mut(&pane_id) {
                        let _ = pane.pty.write(&bytes);
                    }
                }
            }
        }
    }

    fn process_wheel(&mut self, delta: MouseScrollDelta, egui_consumed: bool) {
        if self.settings.open || self.context_menu.is_some() || egui_consumed {
            return;
        }
        let Some(pos) = self.mouse.position else { return };
        let Some((pane_id, col, row)) = self.pane_at(pos.x, pos.y) else { return };
        let mode = self.panes.get(&pane_id).map(|p| p.term.mode()).unwrap_or_default();
        let shift = self.modifiers.shift_key();
        if mode.intersects(TermMode::MOUSE_MODE) && !shift {
            let (dy, dx) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (y, x),
                MouseScrollDelta::PixelDelta(p) => ((p.y / 40.0) as f32, (p.x / 40.0) as f32),
            };
            let button = if dy > 0.0 {
                pretty_core::MouseButton::WheelUp
            } else if dy < 0.0 {
                pretty_core::MouseButton::WheelDown
            } else if dx > 0.0 {
                pretty_core::MouseButton::WheelRight
            } else {
                pretty_core::MouseButton::WheelLeft
            };
            if let Some(bytes) = encode_mouse(mode, button, MouseEventKind::Press, col, row, self.core_modifiers()) {
                if let Some(pane) = self.panes.get_mut(&pane_id) {
                    let _ = pane.pty.write(&bytes);
                }
            }
            return;
        }
        let lines = wheel_scroll_lines(
            delta,
            self.config.wheel_scroll_multiplier,
            self.config.wheel_scroll_min_lines,
        );
        if lines != 0 {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.term.scroll(lines);
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn process_ime(&mut self, ime: Ime, egui_consumed: bool) {
        match ime {
            Ime::Commit(text) => {
                log::debug!("IME commit: {} bytes", text.len());
                self.ime_preedit.clear();
                if !self.settings.open
                    && !self.search.open
                    && self.context_menu.is_none()
                    && !egui_consumed
                {
                    self.write_to_focused(text.as_bytes());
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            Ime::Preedit(text, _range) => {
                log::debug!("IME preedit: {} bytes", text.len());
                if !self.settings.open && !self.search.open && !egui_consumed {
                    self.ime_preedit = text;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            Ime::Enabled => log::info!("IME enabled"),
            Ime::Disabled => {
                self.ime_preedit.clear();
                log::info!("IME disabled");
            }
        }
    }

    fn process_touch(&mut self, touch: winit::event::Touch, egui_consumed: bool) {
        let id = touch.id;
        let loc = touch.location;
        match touch.phase {
            TouchPhase::Started => {
                log::debug!("touch start id={id} at {loc:?}");
                self.touch_positions.insert(id, loc);
                if self.touch_positions.len() >= 2 {
                    self.cancel_touch_gesture();
                    let (ax, ay) = self.touch_centroid();
                    if let Some((pane, ..)) = self.pane_at(ax, ay) {
                        self.touch_scroll_pane = Some(pane);
                        self.touch_scroll_y = Some(ay);
                    }
                    return;
                }
                if self.settings.open || self.context_menu.is_some() || egui_consumed {
                    return;
                }
                let Some((pane_id, col, row)) = self.pane_at(loc.x, loc.y) else { return };
                self.focused = pane_id;
                self.mouse.position = Some(loc);
                let mode = self.panes.get(&pane_id).map(|p| p.term.mode()).unwrap_or_default();
                let mouse = mode.intersects(TermMode::MOUSE_MODE);
                if mouse {
                    if let Some(bytes) = encode_mouse(mode, pretty_core::MouseButton::Left, MouseEventKind::Press, col, row, self.core_modifiers()) {
                        if let Some(pane) = self.panes.get_mut(&pane_id) {
                            let _ = pane.pty.write(&bytes);
                        }
                    }
                    self.mouse.button = Some(pretty_core::MouseButton::Left);
                    self.mouse.last_cell = Some((pane_id, col, row));
                } else if let Some(pane) = self.panes.get_mut(&pane_id) {
                    pane.term.start_selection(row, col);
                }
                self.touch_gesture = Some(TouchGesture {
                    id,
                    start: loc,
                    last: loc,
                    pane: pane_id,
                    row,
                    col,
                    moved: false,
                    mouse,
                    selecting: !mouse,
                });
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            TouchPhase::Moved => {
                self.touch_positions.insert(id, loc);
                if self.touch_positions.len() >= 2 {
                    let (_, ay) = self.touch_centroid();
                    if let Some(last) = self.touch_scroll_y {
                        let dy = ay - last;
                        let cell_h = self.renderer.as_ref().map(|r| r.cell_metrics().cell_h as f64).unwrap_or(18.0);
                        let lines = (dy / cell_h).trunc() as i32;
                        if lines != 0 {
                            if let Some(pane) = self.touch_scroll_pane.and_then(|p| self.panes.get_mut(&p)) {
                                pane.term.scroll(lines);
                            }
                            self.touch_scroll_y = Some(ay - (dy % cell_h));
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                        } else {
                            self.touch_scroll_y = Some(ay);
                        }
                    } else {
                        self.touch_scroll_y = Some(ay);
                    }
                    return;
                }
                let Some(mut g) = self.touch_gesture.take() else { return };
                if g.id != id {
                    self.touch_gesture = Some(g);
                    return;
                }
                let dx = loc.x - g.start.x;
                let dy = loc.y - g.start.y;
                if !g.moved && dx * dx + dy * dy > 36.0 {
                    g.moved = true;
                }
                g.last = loc;
                if g.moved {
                    if let Some((_, col, row)) = self.pane_at(loc.x, loc.y) {
                        g.col = col;
                        g.row = row;
                        let mode = self.panes.get(&g.pane).map(|p| p.term.mode()).unwrap_or_default();
                        if g.mouse {
                            if mode.contains(TermMode::MOUSE_DRAG) || mode.contains(TermMode::MOUSE_MOTION) {
                                if self.mouse.last_cell != Some((g.pane, col, row)) {
                                    self.mouse.last_cell = Some((g.pane, col, row));
                                    if let Some(bytes) = encode_mouse(mode, pretty_core::MouseButton::Left, MouseEventKind::Motion, col, row, self.core_modifiers()) {
                                        if let Some(pane) = self.panes.get_mut(&g.pane) {
                                            let _ = pane.pty.write(&bytes);
                                        }
                                    }
                                }
                            }
                        } else if let Some(pane) = self.panes.get_mut(&g.pane) {
                            pane.term.update_selection(row, col);
                        }
                    }
                }
                self.touch_gesture = Some(g);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                log::debug!("touch end id={id} at {loc:?}");
                self.touch_positions.remove(&id);
                if let Some(g) = self.touch_gesture.take() {
                    if g.id == id {
                        if g.mouse {
                            let mode = self.panes.get(&g.pane).map(|p| p.term.mode()).unwrap_or_default();
                            if let Some(bytes) = encode_mouse(mode, pretty_core::MouseButton::Left, MouseEventKind::Release, g.col, g.row, self.core_modifiers()) {
                                if let Some(pane) = self.panes.get_mut(&g.pane) {
                                    let _ = pane.pty.write(&bytes);
                                }
                            }
                            self.mouse.button = None;
                        } else if g.selecting && !g.moved {
                            if let Some(pane) = self.panes.get_mut(&g.pane) {
                                pane.term.clear_selection();
                            }
                        } else if g.selecting && g.moved && self.config.copy_on_select {
                            self.copy_selection();
                        }
                    }
                }
                if self.touch_positions.len() < 2 {
                    self.touch_scroll_y = None;
                    self.touch_scroll_pane = None;
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
        }
    }

    fn touch_centroid(&self) -> (f64, f64) {
        if self.touch_positions.is_empty() {
            return (0.0, 0.0);
        }
        let n = self.touch_positions.len() as f64;
        let (x, y) = self
            .touch_positions
            .values()
            .fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
        (x / n, y / n)
    }

    fn cancel_touch_gesture(&mut self) {
        if let Some(g) = self.touch_gesture.take() {
            if g.mouse {
                let mode = self.panes.get(&g.pane).map(|p| p.term.mode()).unwrap_or_default();
                if let Some(bytes) = encode_mouse(mode, pretty_core::MouseButton::Left, MouseEventKind::Release, g.col, g.row, self.core_modifiers()) {
                    if let Some(pane) = self.panes.get_mut(&g.pane) {
                        let _ = pane.pty.write(&bytes);
                    }
                }
                self.mouse.button = None;
            } else if g.selecting {
                if let Some(pane) = self.panes.get_mut(&g.pane) {
                    pane.term.clear_selection();
                }
            }
        }
    }

    fn handle_file_drop(&mut self, path: PathBuf) {
        if self.settings.open || self.search.open {
            return;
        }
        let path_text = path.to_string_lossy();
        let Some(quoted) = shell_quote_path(&path_text) else {
            // Windows cmd.exe expands %VAR% even inside double quotes and has
            // no escape for it; inserting such a path could inject expansion.
            log::warn!("refusing to insert a path containing '%' (cmd.exe expansion): {path_text}");
            return;
        };
        let text = format!("{quoted} ");
        let mode = self.panes.get(&self.focused).map(|p| p.term.mode()).unwrap_or_default();
        self.write_to_focused(&encode_paste(mode, &text));
        log::info!("file drop inserted path: {}", path.display());
        if let Some(w) = &self.window {
            w.set_cursor(winit::window::CursorIcon::Text);
            w.request_redraw();
        }
    }

    fn handle_context_action(&mut self, action: ContextAction) {
        match action {
            ContextAction::Copy => self.copy_selection(),
            ContextAction::Paste => self.paste_clipboard(),
            ContextAction::SelectAll => self.select_all(),
            ContextAction::Clear => self.write_to_focused(b"\x0c"),
            ContextAction::Find => self.open_search(),
            ContextAction::NewTab => self.new_tab(),
            ContextAction::SplitRight => self.split_pane(pretty_core::Axis::Horizontal),
            ContextAction::SplitDown => self.split_pane(pretty_core::Axis::Vertical),
            ContextAction::ClosePane => self.close_pane(self.focused),
            ContextAction::Settings => {
                self.settings.open = true;
                self.settings.draft = self.config.clone();
            }
            ContextAction::Quit => self.request_exit(),
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn update_ime_area(&mut self, frames: &[(Rect, PaneId, pretty_core::Frame)]) {
        // While the settings panel or the find box owns keyboard input, the
        // candidate window must follow *their* text cursor. egui-winit sets
        // that area from the platform output; overriding it with the terminal
        // cursor put the IME popup in the wrong corner of the window.
        if self.settings.open || self.search.open {
            self.ime_cursor = None;
            return;
        }
        let Some(window) = &self.window else { return };
        let Some(renderer) = &self.renderer else { return };
        let Some((rect, _, frame)) = frames.iter().find(|(_, id, _)| *id == self.focused) else {
            self.ime_cursor = None;
            return;
        };
        let Some(cursor) = frame.cursor.filter(|c| c.visible) else {
            self.ime_cursor = None;
            return;
        };
        let metrics = renderer.cell_metrics();
        // `metrics.padding` is already in physical pixels (config padding times
        // the DPI scale); the old code added the logical value on top, which
        // misplaced the candidate window by (scale-1)*padding on HiDPI.
        let padding = metrics.padding;
        let x = rect.x + padding + cursor.col as f32 * metrics.cell_w;
        let y = rect.y + padding + cursor.row as f32 * metrics.cell_h;
        let ppp = self.ui_ppp();
        self.ime_cursor = Some(egui::pos2(x / ppp, y / ppp));
        window.set_ime_cursor_area(
            PhysicalPosition::new(x as f64, y as f64),
            PhysicalSize::new(metrics.cell_w.max(1.0) as u32, metrics.cell_h.max(1.0) as u32),
        );
    }

    fn core_modifiers(&self) -> CoreModifiers {
        CoreModifiers {
            shift: self.modifiers.shift_key(),
            ctrl: self.modifiers.control_key(),
            alt: self.modifiers.alt_key(),
            super_key: self.modifiers.super_key(),
        }
    }

    fn build_ui_paint(&mut self) -> Option<UiPaint> {
        let window = self.window.clone()?;
        self.apply_ui_scale();
        // Keep the draft fresh while the panel is closed: opening it from the
        // gear button used to resurrect values from before a zoom shortcut or a
        // context-menu change, silently reverting them on the next edit.
        self.settings.sync(&self.config);
        let ppp = self.ui_ppp();
        let (gpu, metrics, font_family) = {
            let r = self.renderer.as_ref()?;
            (r.gpu.info.clone(), r.cell_metrics(), r.font_family().to_owned())
        };
        let (mode, kitty_flags) = self
            .panes
            .get(&self.focused)
            .map(|p| (format!("{:?}", p.term.mode()), p.term.kitty_keyboard_flags()))
            .unwrap_or_default();
        let info = RuntimeInfo {
            gpu: &gpu,
            metrics,
            kitty_flags,
            mode,
            backend_note: if self.options.renderer != RendererKind::Auto
                && self.options.renderer != gpu.selected
            {
                Some(format!(
                    "Requested {:?}, running {:?}. Backend changes require a restart.",
                    self.options.renderer, gpu.selected
                ))
            } else {
                None
            },
            config_path: self.options.config_path.display().to_string(),
            font_family,
        };
        let mut changed = false;
        let mut reset_request = false;
        let mut import_path: Option<PathBuf> = None;
        let mut resize_request: Option<(u32, u32)> = None;
        let lang = self.lang;
        let has_selection = self
            .panes
            .get(&self.focused)
            .map(|p| p.term.selection_present())
            .unwrap_or(false);
        let mut menu_open = self.context_menu;
        let mut menu_action = None;
        let mut tab_action = None;
        let tab_titles: Vec<String> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                self.panes
                    .get(&tab.focused)
                    .map(|p| p.title.clone())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| format!("Terminal {}", i + 1))
            })
            .collect();
        let tab_count = tab_titles.len();
        let tab_labels: Vec<String> = tab_titles
            .iter()
            .enumerate()
            .map(|(i, title)| {
                let template = if i == self.active_tab
                    && !self.config.active_tab_title_template.is_empty()
                {
                    self.config.active_tab_title_template.as_str()
                } else if self.config.tab_title_template.is_empty() {
                    "{index}: {title}"
                } else {
                    self.config.tab_title_template.as_str()
                };
                expand_tab_title(template, i, title, tab_count)
            })
            .collect();
        let active_tab = self.active_tab;
        let tab_bar_style = self.config.tab_bar_style.clone();
        let tab_bar_colors = TabBarColors::from_config(
            &self.config,
            &self.config.theme(),
            &tab_bar_style,
        );
        let tab_bar_align = self.config.tab_bar_align.clone();
        let mut search_query = self.search.query.clone();
        let mut search_next = false;
        let mut search_close = false;
        let search_open = self.search.open;
        let search_matches = self.search.matches.len();
        let search_current = self.search.current;
        // Tab bar geometry is expressed in egui points so it follows both the
        // HiDPI scale factor and the user's UI scale exactly like the layout
        // margin. The old code passed physical pixels as points, which doubled
        // the margin on 2x displays.
        let tab_bar_visible = self.tab_bar_shown();
        let tab_bar_height_points = self.config.tab_bar_height.clamp(18.0, 64.0);
        let margin_points = 6.0_f32;
        let window_size = window.inner_size();
        let window_width_points = window_size.width as f32 / ppp;
        let window_height_points = window_size.height as f32 / ppp;
        let tab_bar_width = (window_width_points - 132.0).max(240.0);
        let tab_bar_bottom = self.config.tab_bar_edge == "bottom";
        let tab_bar_top = if tab_bar_bottom {
            (window_height_points - margin_points - tab_bar_height_points).max(margin_points)
        } else {
            margin_points
        };
        let search_top = if tab_bar_visible {
            if tab_bar_bottom {
                margin_points + 8.0
            } else {
                margin_points + tab_bar_height_points + 8.0
            }
        } else {
            margin_points + 8.0
        };
        let new_tab_hint = self.shortcut_hint("new_tab");
        let find_hint = self.shortcut_hint("find");
        let shortcuts = self.config.shortcuts.clone();
        let accent = pretty_core::Rgb::parse_hex(&self.config.ui_accent)
            .unwrap_or(pretty_core::Rgb::new(0x7a, 0xa2, 0xf7));
        let bell_color = egui::Color32::from_rgb(accent.r, accent.g, accent.b);
        let drop_hover = self.drop_hover.clone();
        let preedit = self.ime_preedit.clone();
        let ime_cursor = self.ime_cursor;
        let confirm_dialog_open = self.confirm_close_dialog;
        let bell_flash = self
            .bell_flash_until
            .filter(|until| Instant::now() < *until)
            .is_some();
        let mut confirm_quit = false;
        let mut confirm_cancel = false;
        let (output, output_cursor) = {
            let state = self.egui_state.as_mut()?;
            let raw_input = state.take_egui_input(&window);
            let output = self.egui_ctx.run_ui(raw_input, |root| {
                if tab_bar_visible {
                    tab_action = show_tab_bar(
                        root.ctx(),
                        &tab_labels,
                        active_tab,
                        margin_points,
                        tab_bar_top,
                        tab_bar_height_points,
                        tab_bar_width,
                        &tab_bar_style,
                        &tab_bar_align,
                        &tab_bar_colors,
                        &new_tab_hint,
                        &find_hint,
                        lang,
                    );
                }
                if search_open {
                    egui::Window::new(tr(lang, "Find"))
                        .id(egui::Id::new("pretty-search"))
                        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, search_top))
                        .resizable(false)
                        .collapsible(false)
                        .show(root.ctx(), |ui| {
                            ui.horizontal(|ui| {
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut search_query)
                                        .id(egui::Id::new("pretty-search-input"))
                                        .desired_width(220.0),
                                );
                                if !response.has_focus() {
                                    response.request_focus();
                                }
                                if ui.button(tr(lang, "Next")).clicked() {
                                    search_next = true;
                                }
                                if ui.button(tr(lang, "Close")).clicked() {
                                    search_close = true;
                                }
                                if search_matches == 0 && !search_query.is_empty() {
                                    ui.label(tr(lang, "No matches"));
                                } else if search_matches > 0 {
                                    ui.label(format!("{}/{}", search_current + 1, search_matches));
                                }
                            });
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                search_next = true;
                            }
                        });
                }
                let outcome = settings::show(
                    root.ctx(),
                    &mut self.settings,
                    &info,
                    env!("CARGO_PKG_VERSION"),
                    lang,
                );
                changed = outcome.changed;
                reset_request = outcome.reset;
                import_path = outcome.import;
                resize_request = outcome.resize;
                if !self.settings.open {
                    menu_action =
                        menu::show(root.ctx(), &mut menu_open, lang, has_selection, &shortcuts);
                }
                if let Some(path) = &drop_hover {
                    if !self.settings.open && !confirm_dialog_open {
                        egui::Area::new(egui::Id::new("pretty-drop-hint"))
                            .order(egui::Order::Foreground)
                            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -20.0))
                            .show(root.ctx(), |ui| {
                                egui::Frame::popup(ui.style())
                                    .fill(egui::Color32::from_rgb(24, 31, 43))
                                    .show(ui, |ui| {
                                        ui.label(format!(
                                            "{}: {}",
                                            tr(lang, "Drop file to insert its path"),
                                            path.display()
                                        ));
                                    });
                            });
                    }
                }
                if !preedit.is_empty() && !self.settings.open && !search_open {
                    let p = ime_cursor.unwrap_or(egui::pos2(24.0, 48.0));
                    egui::Area::new(egui::Id::new("pretty-ime-preedit"))
                        .order(egui::Order::Tooltip)
                        .fixed_pos(p + egui::vec2(0.0, 18.0))
                        .show(root.ctx(), |ui| {
                            egui::Frame::popup(ui.style())
                                .fill(egui::Color32::from_rgb(24, 31, 43))
                                .show(ui, |ui| {
                                    ui.label(&preedit);
                                });
                        });
                }
                if confirm_dialog_open {
                    let modal = egui::Modal::new(egui::Id::new("pretty-confirm-quit")).show(
                        root.ctx(),
                        |ui| {
                            ui.set_min_width(320.0);
                            ui.heading(tr(lang, "Close Pretty?"));
                            ui.label(tr(
                                lang,
                                "Running terminal sessions will be terminated.",
                            ));
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button(tr(lang, "Quit")).clicked() {
                                    confirm_quit = true;
                                }
                                if ui.button(tr(lang, "Cancel")).clicked() {
                                    confirm_cancel = true;
                                }
                            });
                        },
                    );
                    let _ = modal;
                }
                if bell_flash {
                    // Visual bell: a short accent-coloured border flash. The
                    // repaint timer is driven from `about_to_wait`.
                    let screen = root.ctx().content_rect();
                    let painter = root.ctx().layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new("pretty-visual-bell"),
                    ));
                    painter.rect_stroke(
                        screen.shrink(3.0),
                        egui::CornerRadius::same(0),
                        egui::Stroke::new(3.0, bell_color),
                        egui::StrokeKind::Inside,
                    );
                }
            });
            let cursor = egui_cursor_icon(&output.platform_output);
            state.handle_platform_output(&window, output.platform_output.clone());
            (output, cursor)
        };
        self.context_menu = menu_open;
        if let Some(cursor) = output_cursor {
            window.set_cursor(cursor);
        }
        if let Some(action) = menu_action {
            self.handle_context_action(action);
        }
        match tab_action {
            Some(TabAction::Activate(i)) => self.activate_tab(i),
            Some(TabAction::New) => self.new_tab(),
            Some(TabAction::Close(i)) => self.close_tab(i),
            Some(TabAction::Find) => self.open_search(),
            None => {}
        }
        if search_open {
            if search_query != self.search.query {
                self.search.query = search_query;
                self.refresh_search();
            }
            if search_next {
                self.next_search_match();
            }
            if search_close {
                self.search.open = false;
                if let Some(pane) = self.focused_pane_mut() {
                    pane.term.clear_selection();
                }
            }
        }
        if reset_request {
            // A confirmed "Reset all settings" discards the draft and restores
            // every documented default, including the language and shortcuts.
            self.config = Config::default();
            self.config.validate();
            self.settings.draft = self.config.clone();
            self.settings.sync(&self.config);
            self.config_changed();
        } else if changed {
            self.config = self.settings.draft.clone();
            self.config.validate();
            self.settings.draft = self.config.clone();
            self.config_changed();
        }
        if let Some(path) = import_path {
            self.perform_kitty_import(&path);
        }
        if let Some((width, height)) = resize_request {
            let width = width.clamp(320, 7680);
            let height = height.clamp(200, 4320);
            self.config.window_width = width;
            self.config.window_height = height;
            let _ = window.request_inner_size(LogicalSize::new(width as f64, height as f64));
            self.config_changed();
        }
        if confirm_quit {
            self.confirm_close_dialog = false;
            self.flush_config_save();
            self.exit_requested = true;
        } else if confirm_cancel {
            self.confirm_close_dialog = false;
        }
        let pixels_per_point = output.pixels_per_point;
        let primitives = self.egui_ctx.tessellate(output.shapes, pixels_per_point);
        Some(UiPaint {
            primitives,
            textures_delta: output.textures_delta,
            pixels_per_point,
        })
    }

    fn draw(&mut self) {
        if self.window.is_none() || self.renderer.is_none() {
            return;
        }
        // A frame drawn at (or after) the blink deadline flips the phase, so a
        // coalesced timer tick can never leave the cursor stuck in the wrong
        // state; no extra redraw is needed because this frame already paints it.
        self.advance_blink();
        // Egui builds the settings/gear each frame. This also applies changes.
        let ui = self.build_ui_paint();
        if let Some(window) = &self.window {
            if !self.settings.open {
                window.set_cursor(winit::window::CursorIcon::Text);
            }
        }
        let mut frames = Vec::new();
        let rects = self.layout.rects(self.layout_area(), self.gap());
        for (id, rect) in rects {
            let Some(pane) = self.panes.get_mut(&id) else { continue };
            let mut frame = pane.term.snapshot();
            if id != self.focused {
                frame.cursor = None;
            } else if !self.settings.open && self.config.cursor_blink && !self.blink.on {
                if let Some(c) = frame.cursor.as_mut() {
                    c.off = true;
                }
            }
            if self.settings.open {
                frame.cursor = None;
            }
            frames.push((rect, id, frame));
        }
        self.update_ime_area(&frames);
        if let Some((_, _, frame)) = frames.first() {
            let shape = frame.cursor.map(|c| c.shape);
            if !self.cursor_logged || shape != self.last_cursor_shape {
                log::info!(
                    "terminal cursor shape={:?} at {:?}; kitty flags={} modes={:?}",
                    shape,
                    frame.cursor.map(|c| (c.row, c.col)),
                    self.panes.get(&self.focused).map(|p| p.term.kitty_keyboard_flags()).unwrap_or(0),
                    self.panes.get(&self.focused).map(|p| p.term.mode())
                );
                self.cursor_logged = true;
                self.last_cursor_shape = shape;
            }
        }
        // Settings is modal in appearance: do not draw terminal text through
        // the translucent egui window. The terminal is restored immediately on
        // close. Images/cursor are suppressed with the same empty pane list.
        let pane_draws: Vec<PaneDraw<'_>> = if self.settings.open {
            Vec::new()
        } else {
            frames
                .iter()
                .map(|(rect, id, frame)| PaneDraw {
                    rect: *rect,
                    frame,
                    focused: *id == self.focused,
                })
                .collect()
        };
        let Some(renderer) = self.renderer.as_mut() else { return };
        match renderer.render(&pane_draws, ui) {
            Ok(RenderOutcome::Presented) => {}
            Ok(RenderOutcome::Skipped) => {}
            Ok(RenderOutcome::Retry) => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            Err(e) => log::error!("render failed: {e}"),
        }
        if !self.blur_applied {
            // The X11 window is only managed/mapped by KWin after the first
            // frame. Re-apply the region now so the compositor sees it.
            if let Some(window) = &self.window {
                match platform::apply_compositor_effects(
                    window,
                    self.config.blur,
                    self.config.opacity,
                ) {
                    Ok(()) => self.blur_applied = true,
                    Err(e) => {
                        self.blur_applied = true;
                        log::warn!("compositor effects unavailable: {e}");
                    }
                }
            }
        }
        if let Some(bench) = self.benchmark.as_mut() {
            if bench.frames == 0 && bench.start.elapsed() < Duration::from_millis(1200) {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            } else {
            bench.frames += 1;
            let now = Instant::now();
            if bench.frames > 1 {
                bench.intervals.push(now.duration_since(bench.last).as_secs_f32());
            }
            bench.last = now;
            if bench.frames >= bench.target {
                let mut sorted = bench.intervals.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let avg = if sorted.is_empty() { 0.0 } else { sorted.iter().sum::<f32>() / sorted.len() as f32 };
                let p95 = sorted.get((sorted.len() as f32 * 0.95) as usize).copied().unwrap_or(avg);
                let fps = if avg > 0.0 { 1.0 / avg } else { 0.0 };
                log::info!(
                    "BENCHMARK RESULT backend={:?} adapter={:?} frames={} avg_frame_ms={:.2} p95_frame_ms={:.2} avg_fps={:.1} grid={}x{}",
                    renderer.gpu.info.selected,
                    renderer.gpu.info.adapter.name,
                    bench.target,
                    avg * 1000.0,
                    p95 * 1000.0,
                    fps,
                    self.panes.get(&self.focused).map(|p| p.cols).unwrap_or(0),
                    self.panes.get(&self.focused).map(|p| p.rows).unwrap_or(0)
                );
                std::process::exit(0);
            }
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            }
        }
        self.frame_count += 1;
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        if dt > 0.0 && dt < 1.0 {
            self.frame_times.push(dt);
            if self.frame_times.len() > 240 {
                self.frame_times.remove(0);
            }
        }
        self.last_frame = now;
        if now.duration_since(self.last_log_frame) > Duration::from_secs(10) && !self.frame_times.is_empty() {
            let avg = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
            log::debug!(
                "idle redraw interval: {} frames, avg {:.2} ms, backend={:?}",
                self.frame_count,
                avg * 1000.0,
                renderer.gpu.info.selected
            );
            self.frame_times.clear();
            self.last_log_frame = now;
        }
    }

    fn apply_ui_changes_debounced(&mut self) {
        if !self.pending_ui_apply {
            return;
        }
        if let Some(last) = self.last_ui_change {
            if last.elapsed() < Duration::from_millis(220) {
                return;
            }
        }
        self.resize_all();
        if let Err(e) = crate::config::save(&self.options.config_path, &self.config) {
            log::warn!("failed to save config: {e}");
        }
        self.pending_ui_apply = false;
    }
}

impl ApplicationHandler<UserEvent> for PrettyApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.create_window(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::PtyReady(id) => self.handle_pty(id, event_loop),
            UserEvent::BlinkTick => {
                if self.advance_blink() {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                self.update_pointer_visibility();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let (egui_consumed, egui_repaint) = if let (Some(state), Some(window)) = (self.egui_state.as_mut(), self.window.as_ref()) {
            let response = state.on_window_event(window, &event);
            (response.consumed, response.repaint)
        } else {
            (false, false)
        };
        if egui_repaint && !matches!(event, WindowEvent::RedrawRequested) {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if matches!(
            &event,
            WindowEvent::CursorMoved { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::Touch(_)
        ) {
            self.note_pointer_activity();
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.confirm_close_dialog {
                    return;
                }
                if self.config.confirm_close {
                    self.confirm_close_dialog = true;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                self.flush_config_save();
                self.exit_requested = true;
            }
            WindowEvent::Resized(size) => {
                self.context_menu = None;
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width.max(1), size.height.max(1));
                }
                if self.config.remember_window_size {
                    if let Some(window) = &self.window {
                        if !window.is_maximized() && window.fullscreen().is_none() {
                            let logical = size.to_logical::<f64>(self.scale_factor);
                            let width = (logical.width.round() as u32).clamp(320, 7680);
                            let height = (logical.height.round() as u32).clamp(200, 4320);
                            self.pending_window_size = Some((width, height));
                        }
                    }
                }
                self.resize_all();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = scale_factor;
                self.apply_ui_scale();
                if let Some(r) = self.renderer.as_mut() {
                    if let Err(e) = r.set_scale_factor(scale_factor as f32) {
                        log::error!("failed to apply scale factor: {e}");
                    }
                }
                self.resize_all();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && event.logical_key.as_ref() == WinitKey::Named(WinitNamedKey::Escape)
                {
                    if self.confirm_close_dialog {
                        self.confirm_close_dialog = false;
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                    if self.settings.open || self.context_menu.is_some() {
                        self.settings.open = false;
                        self.context_menu = None;
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                }
                // The hard-coded Ctrl+, handler is gone: the "settings" action
                // is configurable now, so rebinding or clearing it in
                // Settings -> Keyboard actually applies. process_key handles
                // shortcuts while an overlay is open and stops before text is
                // forwarded to the terminal.
                self.process_key(&event);
            }
            WindowEvent::Ime(ime) => {
                self.process_ime(ime, egui_consumed);
            }
            WindowEvent::Touch(touch) => {
                self.process_touch(touch, egui_consumed);
            }
            WindowEvent::HoveredFile(path) => {
                // Do not show the drop hint / copy cursor while the settings
                // panel is modal on top of the terminal.
                if !self.settings.open {
                    self.drop_hover = Some(path);
                    if let Some(w) = &self.window {
                        w.set_cursor(winit::window::CursorIcon::Copy);
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::HoveredFileCancelled => {
                self.drop_hover = None;
                if let Some(w) = &self.window {
                    w.set_cursor(winit::window::CursorIcon::Text);
                    w.request_redraw();
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.drop_hover = None;
                self.handle_file_drop(path);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.process_cursor_moved(position, egui_consumed);
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse.position = None;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.process_mouse_input(event_loop, state, button, egui_consumed);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.last_ui_change = Some(Instant::now());
                log::trace!("wheel delta={delta:?} egui_consumed={egui_consumed}");
                self.process_wheel(delta, egui_consumed);
            }
            WindowEvent::Focused(focused) => {
                if focused {
                    self.reset_blink();
                }
                // Do not toggle IME enabled state on focus. Re-enabling a XIM
                // context makes Fcitx reset the active input method to its
                // default (usually Chinese Pinyin) every time the window is
                // refocused. Winit already switches input focus for us.
                let bytes = if focused { b"\x1b[I".as_slice() } else { b"\x1b[O".as_slice() };
                let reports = self
                    .panes
                    .get(&self.focused)
                    .map(|p| p.term.mode().contains(TermMode::FOCUS_IN_OUT))
                    .unwrap_or(false);
                if reports {
                    if let Some(pane) = self.panes.get_mut(&self.focused) {
                        let _ = pane.pty.write(bytes);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.draw();
                self.apply_ui_changes_debounced();
            }
            WindowEvent::Occluded(occluded) => {
                if !occluded {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit_requested {
            event_loop.exit();
            return;
        }
        // The user-facing cursor_blink setting must win over a terminal
        // DECSCUSR "steady" style: shells and full-screen programs often emit
        // `CSI 2 q`, which previously left Blink permanently off. The interval
        // is configurable (cursor_blink_interval_ms) and independent of the
        // terminal's cursor style.
        if self.advance_blink() {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if self.blink_active() {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(self.blink.next));
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
        self.apply_ui_changes_debounced();
        if self.pending_ui_apply {
            if let Some(last) = self.last_ui_change {
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                    last + Duration::from_millis(220),
                ));
            }
        }
        // Visual bell: repaint while the border flash is fading out.
        if let Some(until) = self.bell_flash_until {
            let now = Instant::now();
            if now < until {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(until));
            } else {
                self.bell_flash_until = None;
            }
        }
        // egui's window open/close animation needs frames to advance. The
        // terminal's blink timer used to be the accidental repaint source; with
        // the cursor hidden while settings are open that timer is disabled, so
        // drive the short animation explicitly instead of leaving a dim/blank
        // settings panel on screen.
        let now = Instant::now();
        if self.settings.open != self.settings_was_open {
            self.settings_was_open = self.settings.open;
            self.settings_anim_until = Some(now + Duration::from_millis(320));
        }
        if let Some(until) = self.settings_anim_until {
            if now < until {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                    now + Duration::from_millis(16),
                ));
            } else {
                self.settings_anim_until = None;
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.blink_thread_stop.store(true, Ordering::Relaxed);
        if self.config.remember_window_size {
            if let Some((width, height)) = self.pending_window_size.take() {
                if width != self.config.window_width || height != self.config.window_height {
                    self.config.window_width = width;
                    self.config.window_height = height;
                    self.pending_ui_apply = true;
                }
            }
        }
        // CLI overrides (--opacity/--font-size/--no-blur) are runtime-only and
        // must not silently rewrite the user's persistent configuration. A UI
        // edit made within the 220ms debounce window is still persisted.
        if self.pending_ui_apply {
            if let Err(e) = crate::config::save(&self.options.config_path, &self.config) {
                log::warn!("failed to save config on exit: {e}");
            }
        }
    }
}

/// Translate a wheel event into a line count using the kitty
/// `wheel_scroll_multiplier` and `wheel_scroll_min_lines` settings.
fn wheel_scroll_lines(delta: MouseScrollDelta, multiplier: f32, min_lines: i32) -> i32 {
    let multiplier = if multiplier.is_finite() {
        multiplier.clamp(0.1, 10.0)
    } else {
        3.0
    };
    let min_lines = min_lines.clamp(1, 20);
    let raw = match delta {
        MouseScrollDelta::LineDelta(_, y) => y * multiplier,
        MouseScrollDelta::PixelDelta(p) => {
            if p.y.abs() < 0.5 {
                0.0
            } else {
                // 18 physical pixels per text line before the multiplier.
                ((p.y / 18.0) as f32) * multiplier
            }
        }
    };
    if raw == 0.0 {
        return 0;
    }
    let lines = raw.round() as i32;
    let lines = if lines.abs() < min_lines {
        min_lines * if raw > 0.0 { 1 } else { -1 }
    } else {
        lines
    };
    lines
}

fn shell_command(shell: &str, command: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        vec![shell.to_owned(), "/C".to_owned(), command.to_owned()]
    }
    #[cfg(not(windows))]
    {
        vec![shell.to_owned(), "-c".to_owned(), command.to_owned()]
    }
}

fn map_named(named: WinitNamedKey) -> Option<pretty_core::input::NamedKey> {
    use pretty_core::input::NamedKey as N;
    Some(match named {
        WinitNamedKey::Enter => N::Enter,
        WinitNamedKey::Tab => N::Tab,
        WinitNamedKey::Backspace => N::Backspace,
        WinitNamedKey::Escape => N::Escape,
        WinitNamedKey::Insert => N::Insert,
        WinitNamedKey::Delete => N::Delete,
        WinitNamedKey::ArrowLeft => N::Left,
        WinitNamedKey::ArrowRight => N::Right,
        WinitNamedKey::ArrowUp => N::Up,
        WinitNamedKey::ArrowDown => N::Down,
        WinitNamedKey::PageUp => N::PageUp,
        WinitNamedKey::PageDown => N::PageDown,
        WinitNamedKey::Home => N::Home,
        WinitNamedKey::End => N::End,
        WinitNamedKey::CapsLock => N::CapsLock,
        WinitNamedKey::ScrollLock => N::ScrollLock,
        WinitNamedKey::NumLock => N::NumLock,
        WinitNamedKey::PrintScreen => N::PrintScreen,
        WinitNamedKey::Pause => N::Pause,
        WinitNamedKey::ContextMenu => N::Menu,
        WinitNamedKey::F1 => N::F(1),
        WinitNamedKey::F2 => N::F(2),
        WinitNamedKey::F3 => N::F(3),
        WinitNamedKey::F4 => N::F(4),
        WinitNamedKey::F5 => N::F(5),
        WinitNamedKey::F6 => N::F(6),
        WinitNamedKey::F7 => N::F(7),
        WinitNamedKey::F8 => N::F(8),
        WinitNamedKey::F9 => N::F(9),
        WinitNamedKey::F10 => N::F(10),
        WinitNamedKey::F11 => N::F(11),
        WinitNamedKey::F12 => N::F(12),
        _ => return None,
    })
}

fn map_mouse_button(button: WinitMouseButton) -> Option<pretty_core::MouseButton> {
    Some(match button {
        WinitMouseButton::Left => pretty_core::MouseButton::Left,
        WinitMouseButton::Middle => pretty_core::MouseButton::Middle,
        WinitMouseButton::Right => pretty_core::MouseButton::Right,
        WinitMouseButton::Back => pretty_core::MouseButton::Back,
        WinitMouseButton::Forward => pretty_core::MouseButton::Forward,
        WinitMouseButton::Other(_) => return None,
    })
}

fn egui_cursor_icon(output: &egui::PlatformOutput) -> Option<winit::window::CursorIcon> {
    Some(match output.cursor_icon {
        egui::CursorIcon::Default => winit::window::CursorIcon::Default,
        egui::CursorIcon::PointingHand => winit::window::CursorIcon::Pointer,
        egui::CursorIcon::Text => winit::window::CursorIcon::Text,
        egui::CursorIcon::Crosshair => winit::window::CursorIcon::Crosshair,
        egui::CursorIcon::Grab => winit::window::CursorIcon::Grab,
        egui::CursorIcon::Grabbing => winit::window::CursorIcon::Grabbing,
        egui::CursorIcon::NoDrop | egui::CursorIcon::NotAllowed => winit::window::CursorIcon::NotAllowed,
        egui::CursorIcon::ResizeHorizontal => winit::window::CursorIcon::EwResize,
        egui::CursorIcon::ResizeVertical => winit::window::CursorIcon::NsResize,
        _ => winit::window::CursorIcon::Default,
    })
}


fn percent_decode_path(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| -> Option<u8> {
                match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Turn one or more `file://` URIs into shell-quoted local paths. Returns
/// `None` for ordinary clipboard text so normal paste is untouched.
fn file_uris_to_shell_paths(text: &str) -> Option<String> {
    let mut paths = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with('#') {
            continue;
        }
        let rest = line.strip_prefix("file://")?;
        let path = rest.strip_prefix("localhost").unwrap_or(rest);
        if !path.starts_with('/') {
            return None;
        }
        if let Some(quoted) = shell_quote_path(&percent_decode_path(path)) {
            paths.push(quoted);
        }
    }
    if paths.is_empty() {
        None
    } else {
        Some(format!("{} ", paths.join(" ")))
    }
}

/// Scheme policy for Ctrl+clicked OSC 8 hyperlinks. Untrusted terminal output
/// must not be able to launch local files or custom protocol handlers through
/// the desktop opener, so only normal web and mail links are allowed.
fn is_safe_hyperlink(uri: &str) -> bool {
    let uri = uri.trim();
    let lower = uri.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

/// Quote a dropped/pasted path for the configured shell.
///
/// Returns `None` on Windows when the path contains `%`: cmd.exe expands
/// environment variables inside double quotes and offers no escape, so the
/// only safe behaviour is to refuse the insertion instead of injecting it.
fn shell_quote_path(path: &str) -> Option<String> {
    #[cfg(windows)]
    {
        if path.contains('%') {
            return None;
        }
        // CommandLineToArgvW quoting: double backslashes before an embedded
        // quote (and before the closing quote), then escape the quote itself.
        let mut out = String::with_capacity(path.len() + 2);
        out.push('"');
        let mut backslashes = 0usize;
        for ch in path.chars() {
            match ch {
                '\\' => backslashes += 1,
                '"' => {
                    for _ in 0..(backslashes * 2 + 1) {
                        out.push('\\');
                    }
                    out.push('"');
                    backslashes = 0;
                }
                _ => {
                    for _ in 0..backslashes {
                        out.push('\\');
                    }
                    backslashes = 0;
                    out.push(ch);
                }
            }
        }
        for _ in 0..(backslashes * 2) {
            out.push('\\');
        }
        out.push('"');
        Some(out)
    }
    #[cfg(not(windows))]
    {
        Some(format!("'{}'", path.replace('\'', "'\\''")))
    }
}

/// Load one CJK-capable system font into egui's fallback chain. egui ships
/// Latin fonts only; without this the Chinese UI would render as tofu.
#[allow(clippy::too_many_arguments)]
struct TabBarColors {
    background: egui::Color32,
    active_bg: egui::Color32,
    active_fg: egui::Color32,
    inactive_bg: egui::Color32,
    inactive_fg: egui::Color32,
}

impl TabBarColors {
    fn from_config(config: &Config, theme: &pretty_renderer::Theme, style: &str) -> Self {
        let parse = |value: &str| {
            pretty_core::Rgb::parse_hex(value).map(|rgb| egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b))
        };
        let panel = egui::Color32::from_rgb(theme.panel.r, theme.panel.g, theme.panel.b);
        let accent = egui::Color32::from_rgb(theme.accent.r, theme.accent.g, theme.accent.b);
        let foreground =
            egui::Color32::from_rgb(theme.foreground.r, theme.foreground.g, theme.foreground.b);
        let active_bg_default = if style == "powerline" {
            accent
        } else {
            egui::Color32::from_rgba_unmultiplied(
                theme.border.r,
                theme.border.g,
                theme.border.b,
                220,
            )
        };
        let active_fg_default = if style == "powerline" {
            egui::Color32::from_rgb(12, 16, 24)
        } else {
            foreground
        };
        Self {
            background: parse(&config.tab_bar_background).unwrap_or(panel),
            active_bg: parse(&config.active_tab_background).unwrap_or(active_bg_default),
            active_fg: parse(&config.active_tab_foreground).unwrap_or(active_fg_default),
            inactive_bg: parse(&config.inactive_tab_background).unwrap_or(panel),
            inactive_fg: parse(&config.inactive_tab_foreground)
                .unwrap_or_else(|| foreground.gamma_multiply(0.72)),
        }
    }
}

/// Build a tab label from a kitty-style template.
///
/// Supported placeholders are `{index}` (1-based, matching the tab bar
/// numbering), `{title}` (the focused pane title) and `{count}` (the number of
/// open tabs). Unknown placeholders are kept verbatim so a typo is visible
/// instead of silently disappearing.
fn expand_tab_title(template: &str, index: usize, title: &str, count: usize) -> String {
    let mut out = String::with_capacity(template.len() + title.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let Some(close) = after.find('}') else {
            out.push_str(after);
            return out;
        };
        let key = &after[1..close];
        match key {
            "index" => out.push_str(&(index + 1).to_string()),
            "title" => out.push_str(title),
            "count" => out.push_str(&count.to_string()),
            _ => out.push_str(&after[..=close]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[allow(clippy::too_many_arguments)]
fn show_tab_bar(
    ctx: &egui::Context,
    labels: &[String],
    active: usize,
    margin: f32,
    top: f32,
    height: f32,
    width: f32,
    style: &str,
    align: &str,
    colors: &TabBarColors,
    new_tab_hint: &str,
    find_hint: &str,
    lang: Lang,
) -> Option<TabAction> {
    let mut action = None;
    egui::Area::new(egui::Id::new("pretty-tabs"))
        .order(egui::Order::Middle)
        .fixed_pos(egui::pos2(margin, top))
        .show(ctx, |ui| {
            let mut frame = egui::Frame::new()
                .fill(colors.background)
                .inner_margin(egui::Margin::symmetric(4, 2));
            match style {
                "fade" => {
                    frame = frame.fill(colors.background.gamma_multiply(0.55));
                }
                "powerline" => {
                    frame = frame.fill(colors.background);
                }
                _ => {
                    frame = frame.stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(45, 55, 72)));
                }
            }
            frame.show(ui, |ui| {
                ui.set_width(width);
                ui.set_min_height((height - 6.0).max(16.0));
                let mut draw_tabs = |ui: &mut egui::Ui| {
                    for (i, label) in labels.iter().enumerate() {
                        let is_active = i == active;
                        let text = egui::RichText::new(truncate_label(label, 48)).color(
                            if is_active { colors.active_fg } else { colors.inactive_fg },
                        );
                        let mut button = egui::Button::new(text)
                            .min_size(egui::vec2(0.0, (height - 12.0).max(16.0)))
                            .fill(if is_active {
                                colors.active_bg
                            } else {
                                colors.inactive_bg
                            });
                        if style == "separator" {
                            button = button.stroke(egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgb(45, 55, 72),
                            ));
                        }
                        let response = ui.add(button).on_hover_text(format!(
                            "{label}\n{}",
                            tr(lang, "Click to activate")
                        ));
                        if response.clicked() {
                            action = Some(TabAction::Activate(i));
                        }
                        if labels.len() > 1 {
                            let close = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("×")
                                            .color(colors.inactive_fg)
                                            .small(),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(tr(lang, "Close"));
                            if close.clicked() {
                                action = Some(TabAction::Close(i));
                            }
                        }
                        ui.add_space(2.0);
                    }
                    let new_tab_text = if new_tab_hint.is_empty() {
                        tr(lang, "New tab").to_owned()
                    } else {
                        format!("{} ({new_tab_hint})", tr(lang, "New tab"))
                    };
                    if ui
                        .add(egui::Button::new(
                            egui::RichText::new("+").color(colors.inactive_fg),
                        ))
                        .on_hover_text(new_tab_text)
                        .clicked()
                    {
                        action = Some(TabAction::New);
                    }
                    let find_text = if find_hint.is_empty() {
                        tr(lang, "Find").to_owned()
                    } else {
                        format!("{} ({find_hint})", tr(lang, "Find"))
                    };
                    if ui
                        .add(egui::Button::new(
                            egui::RichText::new(tr(lang, "Find")).color(colors.inactive_fg),
                        ))
                        .on_hover_text(find_text)
                        .clicked()
                    {
                        action = Some(TabAction::Find);
                    }
                };
                // Estimate the strip width from the labels. When everything
                // fits, the configured alignment applies; when it does not, a
                // horizontal scroll area keeps every tab (and the close
                // buttons) reachable instead of clipping them off-screen.
                let estimated: f32 = labels
                    .iter()
                    .map(|label| truncate_label(label, 48).chars().count() as f32 * 8.0 + 40.0)
                    .sum::<f32>()
                    + 140.0;
                if estimated > (width - 24.0).max(80.0) {
                    egui::ScrollArea::horizontal()
                        .id_salt("pretty-tabs-scroll")
                        .auto_shrink([false, true])
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                        )
                        .show(ui, |ui| {
                            ui.horizontal(|ui| draw_tabs(ui));
                        });
                } else {
                    // Centre/right align by offsetting the row; a manual offset
                    // is more reliable than Layout::with_main_align inside the
                    // tab-bar frame because the content is only as wide as its
                    // widgets.
                    let offset = match align {
                        "center" => ((width - estimated) * 0.5).max(0.0),
                        "right" => (width - estimated - 8.0).max(0.0),
                        _ => 0.0,
                    };
                    ui.horizontal(|ui| {
                        if offset > 0.0 {
                            ui.add_space(offset);
                        }
                        draw_tabs(ui);
                    });
                }
            });
        });
    action
}

/// Truncate a tab label to `max_chars` characters (the full title is still
/// available as a tooltip). Very long shell titles used to push the close
/// buttons and `+`/Find controls off the tab bar.
fn truncate_label(label: &str, max_chars: usize) -> String {
    if label.chars().count() <= max_chars {
        return label.to_owned();
    }
    let mut short: String = label.chars().take(max_chars.saturating_sub(1)).collect();
    short.push('…');
    short
}

fn cursor_shape_from_config(shape: &str) -> pretty_core::CoreCursorShape {
    match shape {
        "beam" => pretty_core::CoreCursorShape::Beam,
        "underline" => pretty_core::CoreCursorShape::Underline,
        _ => pretty_core::CoreCursorShape::Block,
    }
}

fn cjk_face_index(path: &std::path::Path) -> u32 {
    // TTC collections contain several regional faces; prefer the Simplified
    // Chinese face when fontdb can identify it.
    let mut db = fontdb::Database::new();
    if db.load_font_file(path).is_err() {
        return 0;
    }
    for name in ["Noto Sans CJK SC", "Noto Sans CJK", "Noto Sans Mono CJK SC"] {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(name)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        if let Some(id) = db.query(&query) {
            if let Some(face) = db.face(id) {
                return face.index;
            }
        }
    }
    0
}

fn install_ui_font(ctx: &egui::Context) {
    let started = Instant::now();
    // Fast path: loading every installed font through fontdb on every startup
    // costs noticeable time. The per-platform candidate list (Linux XDG font
    // directories, macOS /System/Library/Fonts, Windows %WINDIR%\Fonts and
    // Android /system/fonts) lives in platform/paths.rs; only a full system
    // scan if none of them exists.
    let paths = crate::platform::paths::Paths::host();
    let mut loaded: Option<(String, Vec<u8>, u32)> = None;
    for path in paths.cjk_font_candidates() {
        if !path.is_file() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(path) {
            loaded = Some((path.display().to_string(), bytes, cjk_face_index(path)));
            break;
        }
    }
    if loaded.is_none() {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        for name in [
            "Noto Sans CJK SC",
            "Noto Sans CJK",
            "Droid Sans Fallback",
            "WenQuanYi Micro Hei",
            "Microsoft YaHei",
            "PingFang SC",
        ] {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(name)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            };
            let Some(id) = db.query(&query) else { continue };
            let Some(face) = db.face(id) else { continue };
            let fontdb::Source::File(path) = &face.source else { continue };
            let Ok(bytes) = std::fs::read(path) else { continue };
            loaded = Some((path.to_string_lossy().into_owned(), bytes, face.index));
            break;
        }
    }
    if let Some((name, bytes, index)) = loaded {
        let data = egui::FontData::from_blob(std::sync::Arc::new(bytes), index);
        let families = vec![
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: egui::epaint::text::FontPriority::Lowest,
            },
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Monospace,
                priority: egui::epaint::text::FontPriority::Lowest,
            },
        ];
        ctx.add_font(egui::epaint::text::FontInsert::new("pretty-cjk", data, families));
        log::info!(
            "UI CJK fallback font: {} ({} ms)",
            name,
            started.elapsed().as_millis()
        );
        return;
    }
    log::warn!("no CJK system font found; Chinese UI text may show placeholder boxes");
}


#[cfg(test)]
mod tests {
    use super::{shell_quote_path, Shortcut};

    #[test]
    fn file_drop_quotes_spaces_and_quotes() {
        assert_eq!(shell_quote_path("/tmp/a b.txt").as_deref(), Some("'/tmp/a b.txt'"));
        assert_eq!(
            shell_quote_path("/tmp/it's.txt").as_deref(),
            Some("'/tmp/it'\\''s.txt'")
        );
        assert_eq!(shell_quote_path("/tmp/plain").as_deref(), Some("'/tmp/plain'"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_quoting_is_command_line_to_argv_safe() {
        assert_eq!(shell_quote_path(r"C:\a b.txt").as_deref(), Some(r#""C:\a b.txt""#));
        // A trailing backslash must be doubled before the closing quote.
        assert_eq!(shell_quote_path(r"C:\dir\").as_deref(), Some(r#""C:\dir\\""#));
        // Embedded quotes are backslash-doubled.
        assert_eq!(shell_quote_path(r#"C:\a"b.txt"#).as_deref(), Some(r#""C:\a\"b.txt""#));
        // cmd.exe variable expansion has no escape: refuse rather than inject.
        assert_eq!(shell_quote_path(r"C:\%COMSPEC%.txt"), None);
    }

    #[test]
    fn shortcut_parser_normalizes_modifiers_and_keys() {
        let s = Shortcut::parse("Ctrl+Shift+T").unwrap();
        assert!(s.ctrl && s.shift && !s.alt && s.key == "t");
        let s = Shortcut::parse("ctrl+,").unwrap();
        assert!(s.ctrl && !s.shift && s.key == ",");
        let s = Shortcut::parse("Ctrl+Shift+=").unwrap();
        assert_eq!(s.key, "=");
        assert!(Shortcut::parse("").is_none());
        let defaults = crate::config::default_shortcuts();
        assert_eq!(defaults.get("find").map(String::as_str), Some("Ctrl+Shift+F"));
        assert_eq!(defaults.get("new_tab").map(String::as_str), Some("Ctrl+Shift+T"));
    }

    #[test]
    fn tab_labels_are_truncated_with_ellipsis() {
        assert_eq!(super::truncate_label("short", 10), "short");
        assert_eq!(super::truncate_label("0123456789", 5), "0123…");
        // Multi-byte titles must not be cut mid-character.
        let long = "终端标题终端标题终端标题终端标题";
        let truncated = super::truncate_label(long, 6);
        assert_eq!(truncated.chars().count(), 6);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn cursor_shape_config_maps_to_core_shapes() {
        assert!(matches!(
            super::cursor_shape_from_config("beam"),
            pretty_core::CoreCursorShape::Beam
        ));
        assert!(matches!(
            super::cursor_shape_from_config("underline"),
            pretty_core::CoreCursorShape::Underline
        ));
        assert!(matches!(
            super::cursor_shape_from_config("block"),
            pretty_core::CoreCursorShape::Block
        ));
        // Unknown values have already been normalised by Config::validate;
        // the fallback stays safe.
        assert!(matches!(
            super::cursor_shape_from_config("nonsense"),
            pretty_core::CoreCursorShape::Block
        ));
    }

    #[test]
    fn osc8_hyperlink_scheme_allowlist() {
        assert!(super::is_safe_hyperlink("https://example.com/a?b=c"));
        assert!(super::is_safe_hyperlink("HTTP://EXAMPLE.COM"));
        assert!(super::is_safe_hyperlink("http://localhost:8080"));
        assert!(super::is_safe_hyperlink("mailto:dev@example.com"));
        // Dangerous or handler-invoking schemes must be refused.
        assert!(!super::is_safe_hyperlink("file:///etc/passwd"));
        assert!(!super::is_safe_hyperlink("javascript:alert(1)"));
        assert!(!super::is_safe_hyperlink("ms-settings:privacy"));
        assert!(!super::is_safe_hyperlink("ssh://host"));
        assert!(!super::is_safe_hyperlink(""));
    }

    #[test]
    fn clipboard_file_uris_become_shell_paths() {
        assert_eq!(
            super::file_uris_to_shell_paths("file:///tmp/a%20b.txt").as_deref(),
            Some("'/tmp/a b.txt' ")
        );
        assert_eq!(
            super::file_uris_to_shell_paths("file://localhost/tmp/a.txt\nfile:///tmp/b.txt").as_deref(),
            Some("'/tmp/a.txt' '/tmp/b.txt' ")
        );
        assert_eq!(super::file_uris_to_shell_paths("ordinary text"), None);
    }

    #[test]
    fn tab_title_templates_expand_placeholders() {
        assert_eq!(super::expand_tab_title("{index}: {title}", 0, "bash", 3), "1: bash");
        assert_eq!(
            super::expand_tab_title("{title} ({index}/{count})", 2, "vim", 4),
            "vim (3/4)"
        );
        // Unknown placeholders and unterminated braces stay visible instead of
        // silently disappearing, which makes template typos debuggable.
        assert_eq!(super::expand_tab_title("{unknown} {title}", 0, "x", 1), "{unknown} x");
        assert_eq!(super::expand_tab_title("plain", 0, "x", 1), "plain");
        assert_eq!(super::expand_tab_title("{title", 0, "x", 1), "{title");
        assert_eq!(super::expand_tab_title("{title}}", 0, "x", 1), "x}");
    }

    #[test]
    fn wheel_scrolling_respects_multiplier_and_minimum() {
        use winit::event::MouseScrollDelta;
        assert_eq!(
            super::wheel_scroll_lines(MouseScrollDelta::LineDelta(0.0, 1.0), 3.0, 1),
            3
        );
        assert_eq!(
            super::wheel_scroll_lines(MouseScrollDelta::LineDelta(0.0, -1.0), 3.0, 2),
            -3
        );
        // A tiny pixel delta still scrolls the configured minimum.
        assert_eq!(
            super::wheel_scroll_lines(
                MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 5.0)),
                1.0,
                3
            ),
            3
        );
        assert_eq!(
            super::wheel_scroll_lines(
                MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 90.0)),
                1.0,
                1
            ),
            5
        );
        assert_eq!(
            super::wheel_scroll_lines(MouseScrollDelta::LineDelta(0.0, 0.0), 3.0, 1),
            0
        );
        // Nonsense multipliers fall back to the default instead of panicking.
        assert_eq!(
            super::wheel_scroll_lines(
                MouseScrollDelta::LineDelta(0.0, 1.0),
                f32::NAN,
                1
            ),
            3
        );
    }

    #[test]
    fn blink_phase_starts_visible_and_toggles_at_the_deadline() {
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(530);
        let mut phase = super::BlinkPhase::started(interval);
        let start = phase.next - interval;
        assert!(phase.on, "the cursor must be visible immediately after startup");
        assert!(!phase.advance(start + Duration::from_millis(100), interval));
        assert!(phase.on);
        assert!(phase.advance(start + interval, interval));
        assert!(!phase.on);
        assert!(phase.advance(start + interval * 2, interval));
        assert!(phase.on);
    }

    #[test]
    fn blink_cadence_does_not_drift_when_ticks_arrive_late() {
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(530);
        let mut phase = super::BlinkPhase::started(interval);
        let start = phase.next - interval;
        // A tick arrives 20 ms late: the next deadline stays on the original
        // 530 ms grid instead of sliding to now + 530 ms.
        assert!(phase.advance(start + interval + Duration::from_millis(20), interval));
        assert_eq!(phase.next, start + interval * 2);
    }

    #[test]
    fn late_toggle_cannot_create_a_tiny_phase() {
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(530);
        let mut phase = super::BlinkPhase::started(interval);
        let start = phase.next - interval;
        // A frame arrives 300 ms after the deadline: the grid-corrected next
        // deadline would be only 230 ms away, which used to cause a rapid
        // second toggle (the visible flicker).
        let now = start + interval + Duration::from_millis(300);
        assert!(phase.advance(now, interval));
        assert!(
            phase.next >= now + interval / 2,
            "phase must stay visible for at least half an interval, next={:?}",
            phase.next - now
        );
    }

    #[test]
    fn blink_resynchronises_after_a_long_stall_without_bursting() {
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(530);
        let mut phase = super::BlinkPhase::started(interval);
        let start = phase.next - interval;
        let late = start + Duration::from_secs(5);
        assert!(phase.advance(late, interval));
        // Exactly one toggle happened and the next one is a full interval out.
        assert_eq!(phase.next, late + interval);
        assert!(!phase.advance(late + Duration::from_millis(1), interval));
    }

    #[test]
    fn blink_reset_shows_the_cursor_again() {
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(530);
        let mut phase = super::BlinkPhase::started(interval);
        let start = phase.next - interval;
        assert!(phase.advance(start + interval, interval));
        assert!(!phase.on);
        phase.reset(interval);
        assert!(phase.on);
        assert!(phase.next > Instant::now());
    }
}
