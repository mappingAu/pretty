//! Platform-specific integration. All non-portable window-system code lives
//! here; the rest of the app is platform-neutral.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod wayland;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod x11;
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub mod other;

/// Cross-platform XDG/AppData/Application Support path resolution.
pub mod paths;

/// How the running process is actually connected to the display server.
///
/// The GLES backend runs on X11/XWayland on Linux, so "the user is on a
/// Wayland session" is not the same as "this window is a Wayland window".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    Wayland,
    X11,
    Other,
}

impl DisplayKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Wayland => "Wayland",
            Self::X11 => "X11",
            Self::Other => "unknown",
        }
    }
}

pub fn current_display_kind() -> DisplayKind {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        DisplayKind::Wayland
    } else if std::env::var_os("DISPLAY").is_some() {
        DisplayKind::X11
    } else {
        DisplayKind::Other
    }
}

/// True when this is a native Wayland session (even if Pretty itself fell
/// back to X11/XWayland for the GLES backend).
pub fn is_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var_os("XDG_SESSION_TYPE")
            .map(|value| value.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}

/// Probe the compositor advertised globals on a short-lived connection.
///
/// Used before falling back to X11 for GLES: if `wl_compositor` is advertised
/// and Pretty still cannot use native Wayland, that is a driver limitation the
/// user has to be told about. `None` means no Wayland connection is available.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn window_is_wayland(window: &winit::window::Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .window_handle()
        .map(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)))
        .unwrap_or(false)
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn apply_compositor_effects(
    window: &winit::window::Window,
    blur: bool,
    opacity: f32,
) -> Result<(), String> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    if let Ok(handle) = window.window_handle() {
        if matches!(handle.as_raw(), RawWindowHandle::Wayland(_)) {
            if blur {
                return wayland::install_blur(window);
            }
            // Wayland has no whole-window opacity property. The surface alpha
            // is configured by the renderer; no extra action is needed.
            return Ok(());
        }
    }
    x11::apply_compositor_effects(window, blur, opacity)
}
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub use other::apply_compositor_effects;

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub fn window_is_wayland(_window: &winit::window::Window) -> bool {
    false
}
