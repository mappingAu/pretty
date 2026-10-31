//! winit application: windows, panes, input routing, UI and rendering.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use pretty_core::{
    encode_key, encode_mouse, encode_paste, CursorStyle, Key, KeyEventKind, KeyInput, Layout,
    Modifiers as CoreModifiers, MouseEventKind, PaneId, PtySession, Rect, TermMode,
    TermSize, Terminal, TerminalEvent, VteRgb,
};
use pretty_renderer::{PaneDraw, RenderOutcome, Renderer, RendererKind, UiPaint};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey as WinitNamedKey};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{Fullscreen, Window, WindowId};

use crate::config::Config;
use crate::platform;
use crate::settings::{self, RuntimeInfo, SettingsState};

#[derive(Clone, Debug)]
pub enum UserEvent {
    PtyReady(PaneId),
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

pub struct PrettyApp {
    config: Config,
    options: AppOptions,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    settings: SettingsState,
    panes: HashMap<PaneId, Pane>,
    layout: Layout,
    focused: PaneId,
    next_pane: u64,
    #[cfg(not(target_os = "android"))]
    clipboard: Option<arboard::Clipboard>,
    modifiers: ModifiersState,
    mouse: MouseState,
    scale_factor: f64,
    blink_on: bool,
    last_blink: Instant,
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
        Self {
            config,
            options,
            proxy,
            window: None,
            renderer: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            settings,
            panes: HashMap::new(),
            layout: Layout::leaf(PaneId(0)),
            focused: PaneId(0),
            next_pane: 1,
            #[cfg(not(target_os = "android"))]
            clipboard: None,
            modifiers: ModifiersState::default(),
            mouse: MouseState::default(),
            scale_factor: 1.0,
            blink_on: true,
            last_blink: Instant::now(),
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
        self.egui_ctx.set_pixels_per_point(self.scale_factor as f32);
        let theme = self.config.theme();
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

        if self.config.blur {
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
        self.resize_all();
        self.update_title();
        self.window.as_ref().unwrap().request_redraw();
    }

    fn gap(&self) -> f32 {
        self.config.gap * self.scale_factor as f32
    }

    fn layout_area(&self) -> Rect {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or(PhysicalSize::new(1280, 800));
        let margin = (6.0 * self.scale_factor as f32).round();
        Rect::new(
            margin,
            margin,
            (size.width as f32 - margin * 2.0).max(1.0),
            (size.height as f32 - margin * 2.0).max(1.0),
        )
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
            osc52_copy: true,
            osc52_paste: self.config.allow_osc52_paste,
            cursor_style: CursorStyle {
                shape: pretty_core::CoreCursorShape::Block,
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
        self.blink_on = true;
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
        let mode = self.panes.get(&self.focused).map(|p| p.term.mode()).unwrap_or_default();
        let bytes = encode_paste(mode, &text);
        self.write_to_focused(&bytes);
    }

    fn select_all(&mut self) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.term.select_all();
        }
    }

    fn handle_shortcut(&mut self, event: &winit::event::KeyEvent) -> bool {
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
        let alt = self.modifiers.alt_key();
        if event.state != ElementState::Pressed {
            return false;
        }
        let logical = event.logical_key.as_ref();
        if ctrl && shift {
            match logical {
                WinitKey::Character(s) => match s.to_ascii_lowercase().as_str() {
                    "q" => {
                        if let Some(w) = &self.window {
                            w.set_visible(false);
                        }
                        std::process::exit(0);
                    }
                    "c" => {
                        self.copy_selection();
                        return true;
                    }
                    "v" => {
                        self.paste_clipboard();
                        return true;
                    }
                    "e" => {
                        self.split_pane(pretty_core::Axis::Horizontal);
                        return true;
                    }
                    "o" => {
                        self.split_pane(pretty_core::Axis::Vertical);
                        return true;
                    }
                    "w" => {
                        self.close_pane(self.focused);
                        return true;
                    }
                    "+" | "=" => {
                        self.adjust_font_size(1.0);
                        return true;
                    }
                    "-" | "_" => {
                        self.adjust_font_size(-1.0);
                        return true;
                    }
                    "0" => {
                        self.config.font_size = 15.0;
                        self.config_changed();
                        return true;
                    }
                    "a" => {
                        self.select_all();
                        return true;
                    }
                    _ => {}
                },
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
        if ctrl && !shift {
            if matches!(logical, WinitKey::Character(s) if s == ",") {
                self.settings.open = !self.settings.open;
                if self.settings.open {
                    self.settings.sync(&self.config);
                    self.settings.draft = self.config.clone();
                }
                return true;
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
        self.config.font_size = (self.config.font_size + delta).clamp(8.0, 32.0);
        self.config_changed();
    }

    fn config_changed(&mut self) {
        self.config.validate();
        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(e) = renderer.set_theme(self.config.theme()) {
                log::error!("failed to apply theme: {e}");
            }
        }
        if let Some(window) = &self.window {
            if self.config.blur {
                if let Err(e) = platform::apply_compositor_effects(window, self.config.blur, self.config.opacity) {
                    log::warn!("blur unavailable: {e}");
                }
            } else {
                let _ = platform::apply_compositor_effects(window, false, self.config.opacity);
            }
        }
        self.resize_all();
        if let Err(e) = crate::config::save(&self.options.config_path, &self.config) {
            log::warn!("failed to save config: {e}");
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
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
            }
            Err(e) => log::error!("failed to create split pane: {e}"),
        }
    }

    fn close_pane(&mut self, id: PaneId) {
        if self.panes.len() <= 1 {
            if let Some(w) = &self.window {
                w.set_visible(false);
            }
            std::process::exit(0);
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
        }
    }

    fn focus_direction(&mut self, dx: i32, dy: i32) {
        let current = match self.panes.get(&self.focused) {
            Some(p) => p.rect,
            None => return,
        };
        let (cx, cy) = current.center();
        let mut best: Option<(PaneId, f32)> = None;
        for (id, pane) in &self.panes {
            if *id == self.focused {
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
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn pane_at(&self, x: f64, y: f64) -> Option<(PaneId, usize, usize)> {
        for (id, pane) in &self.panes {
            if pane.rect.contains(x as f32, y as f32) {
                let m = self.renderer.as_ref()?.cell_metrics();
                let px = x as f32 - pane.rect.x - m.padding;
                let py = y as f32 - pane.rect.y - m.padding;
                if px < 0.0 || py < 0.0 {
                    return Some((*id, 0, 0));
                }
                let col = (px / m.cell_w).floor() as usize;
                let row = (py / m.cell_h).floor() as usize;
                return Some((*id, col.min(pane.cols.saturating_sub(1)), row.min(pane.rows.saturating_sub(1))));
            }
        }
        None
    }

    fn handle_pty(&mut self, id: PaneId, event_loop: &ActiveEventLoop) {
        let mut chunks = Vec::new();
        {
            let Some(pane) = self.panes.get(&id) else { return };
            while let Ok(chunk) = pane.pty_rx.try_recv() {
                chunks.push(chunk);

            }
        }
        if chunks.is_empty() {
            return;
        }
        {
            let Some(pane) = self.panes.get_mut(&id) else { return };
            for chunk in chunks {
                pane.term.advance(&chunk);
            }
        }
        self.blink_on = true;
        self.handle_terminal_events(id);
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
                    if let Some(w) = &self.window {
                        w.request_user_attention(Some(winit::window::UserAttentionType::Informational));
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
                    self.blink_on = true;
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
        if self.handle_shortcut(event) {
            return;
        }
        if self.settings.open {
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
        let Some((pane_id, col, row)) = self.pane_at(pos.x, pos.y) else {
            if state == ElementState::Released {
                self.mouse.button = None;
                self.mouse.selecting = false;
            }
            return;
        };
        if egui_consumed || self.settings.open {
            return;
        }
        let previous_focus = self.focused;
        self.focused = pane_id;
        if previous_focus != pane_id {
            self.update_title();
        }
        let Some(button) = map_mouse_button(button) else { return };
        let shift = self.modifiers.shift_key();
        if state == ElementState::Pressed
            && button == pretty_core::MouseButton::Left
            && self.modifiers.control_key()
        {
            let uri = self
                .panes
                .get(&pane_id)
                .and_then(|pane| pane.term.hyperlink_at(row, col));
            if let Some(uri) = uri {
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
        if self.settings.open || egui_consumed {
            return;
        }
        let Some((pane_id, col, row)) = self.pane_at(pos.x, pos.y) else { return };
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
        if self.settings.open || egui_consumed {
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
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => (y * 3.0) as i32,
            MouseScrollDelta::PixelDelta(p) => {
                if p.y.abs() < 1.0 { 0 } else { (p.y / 18.0) as i32 }
            }
        };
        if lines != 0 {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.term.scroll(lines);
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
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
        self.egui_ctx.set_pixels_per_point(window.scale_factor() as f32);
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
        let (output, output_cursor) = {
            let state = self.egui_state.as_mut()?;
            let raw_input = state.take_egui_input(&window);
            let output = self.egui_ctx.run_ui(raw_input, |root| {
                changed = settings::show(
                    root.ctx(),
                    &mut self.settings,
                    &info,
                    env!("CARGO_PKG_VERSION"),
                );
            });
            let cursor = egui_cursor_icon(&output.platform_output);
            state.handle_platform_output(&window, output.platform_output.clone());
            (output, cursor)
        };
        if let Some(cursor) = output_cursor {
            window.set_cursor(cursor);
        }
        if changed {
            self.config = self.settings.draft.clone();
            self.config.validate();
            self.settings.draft = self.config.clone();
            self.config_changed();
            self.pending_ui_apply = true;
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
            } else if !self.settings.open && self.config.cursor_blink && !self.blink_on {
                if let Some(c) = frame.cursor.as_mut() {
                    c.visible = false;
                }
            }
            if self.settings.open {
                frame.cursor = None;
            }
            frames.push((rect, id, frame));
        }
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
            if last.elapsed() < Duration::from_millis(300) {
                return;
            }
        }
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
        match event {
            WindowEvent::CloseRequested => {
                if let Some(w) = &self.window {
                    w.set_visible(false);
                }
                std::process::exit(0);
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width.max(1), size.height.max(1));
                }
                self.resize_all();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = scale_factor;
                self.egui_ctx.set_pixels_per_point(scale_factor as f32);
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
                if event.state == ElementState::Pressed {
                    if self.modifiers.control_key() && matches!(&event.logical_key, WinitKey::Character(s) if s == ",") {
                        self.settings.open = !self.settings.open;
                        if self.settings.open {
                            self.settings.draft = self.config.clone();
                        }
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                    if self.settings.open && event.logical_key.as_ref() == WinitKey::Named(WinitNamedKey::Escape) {
                        self.settings.open = false;
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                }
                if self.settings.open {
                    return;
                }
                self.process_key(&event);
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                if !self.settings.open {
                    self.write_to_focused(text.as_bytes());
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
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
                self.process_wheel(delta, egui_consumed);
            }
            WindowEvent::Focused(focused) => {
                if focused {
                    self.blink_on = true;
                    self.last_blink = Instant::now();
                }
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
        let blinking = self
            .panes
            .get(&self.focused)
            .map(|p| p.term.cursor_style().blinking)
            .unwrap_or(false)
            && self.config.cursor_blink
            && !self.settings.open;
        if blinking {
            let now = Instant::now();
            if now >= self.last_blink {
                self.last_blink = now + Duration::from_millis(500);
                self.blink_on = !self.blink_on;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(self.last_blink));
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
        self.apply_ui_changes_debounced();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // CLI overrides (--opacity/--font-size/--no-blur) are runtime-only and
        // must not silently rewrite the user's persistent configuration.
    }
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
