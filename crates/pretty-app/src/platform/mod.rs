//! Platform-specific integration. All non-portable window-system code lives
//! here; the rest of the app is platform-neutral.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod wayland;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod x11;
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub mod other;

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
