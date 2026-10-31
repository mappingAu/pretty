//! Non-X11 platforms.

use winit::window::Window;

pub fn apply_compositor_effects(_window: &Window, enabled: bool, _opacity: f32) -> Result<(), String> {
    if enabled {
        log::warn!("compositor blur-behind is not implemented on this platform; \
                    transparency still works through the window surface");
    }
    Err("compositor blur is not supported on this platform".into())
}
