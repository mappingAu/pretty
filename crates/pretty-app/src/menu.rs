//! Native terminal right-click menu (egui overlay).

use std::collections::HashMap;

use crate::i18n::{tr, Lang};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextAction {
    Copy,
    Paste,
    SelectAll,
    Clear,
    Find,
    NewTab,
    SplitRight,
    SplitDown,
    ClosePane,
    Settings,
    Quit,
}

/// Shortcut label for a menu entry. Respects user edits in
/// `[shortcuts]`: a cleared binding renders no shortcut text.
fn shortcut_text(shortcuts: &HashMap<String, String>, action: &str, fallback: &str) -> String {
    match shortcuts.get(action) {
        Some(binding) => binding.clone(),
        None => fallback.to_owned(),
    }
}

/// Draw the context menu at `open` (logical points). Returns a chosen action and
/// closes the menu on selection, Escape, or an outside click.
pub fn show(
    ctx: &egui::Context,
    open: &mut Option<egui::Pos2>,
    lang: Lang,
    has_selection: bool,
    shortcuts: &HashMap<String, String>,
) -> Option<ContextAction> {
    let mut pos = (*open)?;
    let screen = ctx.content_rect();
    pos.x = pos.x.clamp(screen.left() + 4.0, (screen.right() - 210.0).max(screen.left() + 4.0));
    pos.y = pos.y.clamp(screen.top() + 4.0, (screen.bottom() - 330.0).max(screen.top() + 4.0));
    let mut action = None;
    let mut close = false;

    let area = egui::Area::new(egui::Id::new("pretty-context-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(egui::Color32::from_rgb(24, 31, 43))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 102, 132)))
                .inner_margin(egui::Margin::same(5))
                .show(ui, |ui| {
                    ui.set_min_width(200.0);
                    let item = |ui: &mut egui::Ui,
                                    enabled: bool,
                                    text: &'static str,
                                    action_name: &str,
                                    fallback: &'static str|
                     -> bool {
                        let shortcut = shortcut_text(shortcuts, action_name, fallback);
                        let button = egui::Button::new(tr(lang, text))
                            .shortcut_text(shortcut)
                            .min_size(egui::vec2(190.0, 25.0));
                        ui.add_enabled(enabled, button).clicked()
                    };
                    if item(ui, has_selection, "Copy", "copy", "Ctrl+Shift+C") {
                        action = Some(ContextAction::Copy);
                    }
                    if item(ui, true, "Paste", "paste", "Ctrl+Shift+V") {
                        action = Some(ContextAction::Paste);
                    }
                    if item(ui, true, "Select all", "select_all", "Ctrl+Shift+A") {
                        action = Some(ContextAction::SelectAll);
                    }
                    if item(ui, true, "Clear screen", "clear", "") {
                        action = Some(ContextAction::Clear);
                    }
                    if item(ui, true, "Find", "find", "Ctrl+Shift+F") {
                        action = Some(ContextAction::Find);
                    }
                    ui.separator();
                    if item(ui, true, "New tab", "new_tab", "Ctrl+Shift+T") {
                        action = Some(ContextAction::NewTab);
                    }
                    if item(ui, true, "Split right", "split_right", "Ctrl+Shift+E") {
                        action = Some(ContextAction::SplitRight);
                    }
                    if item(ui, true, "Split down", "split_down", "Ctrl+Shift+O") {
                        action = Some(ContextAction::SplitDown);
                    }
                    if item(ui, true, "Close pane", "close_pane", "Ctrl+Shift+W") {
                        action = Some(ContextAction::ClosePane);
                    }
                    ui.separator();
                    if item(ui, true, "Open settings", "settings", "Ctrl+,") {
                        action = Some(ContextAction::Settings);
                    }
                    if item(ui, true, "Quit", "quit", "Ctrl+Shift+Q") {
                        action = Some(ContextAction::Quit);
                    }
                });
        });

    // Only a primary click closes the menu. The menu is opened by a right
    // click; treating that same click as an outside click would dismiss it
    // before the user can see it.
    let clicked = ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    log::debug!(
        "context menu shown at {:?} rect={:?} primary_click={} pointer={:?} action={:?}",
        pos,
        area.response.rect,
        clicked,
        pointer,
        action
    );
    if clicked && pointer.is_some_and(|p| !area.response.rect.contains(p)) {
        close = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if action.is_some() {
        close = true;
    }
    if close {
        *open = None;
    }
    action
}
