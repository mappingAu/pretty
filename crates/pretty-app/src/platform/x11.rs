//! KDE/X11 compositor blur integration.
//!
//! KWin recognizes `_KDE_NET_WM_BLUR_BEHIND_REGION` on X11 windows. An empty
//! CARDINAL array asks KWin to blur the entire window area. This is used only
//! when the window is translucent; no fake blur is rendered by Pretty itself.
//! Wayland windows cannot use X11 properties; `set_blur_behind` logs that the
//! compositor must provide blur through its own rules in that case.

use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as XprotoExt, PropMode};
use x11rb::wrapper::ConnectionExt;

const BLUR_ATOM: &[u8] = b"_KDE_NET_WM_BLUR_BEHIND_REGION";
const OPACITY_ATOM: &[u8] = b"_NET_WM_WINDOW_OPACITY";

pub fn apply_compositor_effects(window: &Window, blur: bool, opacity: f32) -> Result<(), String> {
    let handle = window.window_handle().map_err(|e| e.to_string())?;
    let xid = match handle.as_raw() {
        RawWindowHandle::Xlib(h) => h.window as u32,
        RawWindowHandle::Xcb(h) => h.window.get(),
        other => {
            return Err(format!(
                "KWin blur property needs an X11 window, got {other:?} (Wayland/XWayland may require WINIT_UNIX_BACKEND=x11)"
            ))
        }
    };
    let (conn, _) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let atom = conn
        .intern_atom(false, BLUR_ATOM)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom;
    let opacity_atom = conn
        .intern_atom(false, OPACITY_ATOM)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom;
    let size = window.inner_size();
    if blur {
        // An empty region is ignored by current KWin. Provide a rectangle that
        // covers the whole client area. Coordinates are client-relative.
        conn.change_property32(
            PropMode::REPLACE,
            xid,
            atom,
            AtomEnum::CARDINAL,
            &[0, 0, size.width, size.height],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.delete_property(xid, atom).map_err(|e| e.to_string())?;
    }
    let pixel_alpha = std::env::var_os("PRETTY_PIXEL_ALPHA").is_some();
    if opacity < 0.999 && !pixel_alpha {
        // The Vulkan Xlib swapchain only exposes an opaque composite mode on
        // this driver. The compositor applies whole-window alpha instead; KWin
        // also uses this alpha to blend the blur region.
        let value = (opacity.clamp(0.05, 1.0) * u32::MAX as f32) as u32;
        conn.change_property32(
            PropMode::REPLACE,
            xid,
            opacity_atom,
            AtomEnum::CARDINAL,
            &[value],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.delete_property(xid, opacity_atom).map_err(|e| e.to_string())?;
    }
    conn.flush().map_err(|e| e.to_string())?;
    let check = conn
        .get_property(false, xid, atom, AtomEnum::ANY, 0, 16)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    log::info!(
        "KDE compositor effects: xid=0x{xid:x} blur={} opacity={:.2} pixel_alpha={} blur_region={} values",
        blur,
        opacity,
        pixel_alpha,
        check.value_len
    );
    Ok(())
}
