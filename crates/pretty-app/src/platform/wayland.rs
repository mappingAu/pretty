//! KDE Wayland background blur integration.
//!
//! Plasma 6 exposes `ext_background_effect_manager_v1` (the successor of
//! `org_kde_kwin_blur`). Native Wayland also gives us text-input-v3, so Fcitx
//! uses its normal candidate popup instead of an XIM fallback. We attach a
//! full-surface blur effect to winit's wl_surface and let KWin composite the
//! per-pixel-alpha terminal background through it.

use std::sync::atomic::{AtomicBool, Ordering};

use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_registry, wl_region::WlRegion, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};

static BLUR_INSTALLED: AtomicBool = AtomicBool::new(false);

struct BlurState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for BlurState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for BlurState {
    fn event(
        _state: &mut Self,
        _proxy: &WlCompositor,
        _event: wayland_client::protocol::wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlRegion, ()> for BlurState {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegion,
        _event: wayland_client::protocol::wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for BlurState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtBackgroundEffectManagerV1,
        _event: wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for BlurState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtBackgroundEffectSurfaceV1,
        _event: wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

struct BlurHandle {
    _conn: Connection,
    _surface: WlSurface,
    _manager: ExtBackgroundEffectManagerV1,
    _effect: ExtBackgroundEffectSurfaceV1,
}

/// Attach a whole-surface background blur effect to a native Wayland window.
///
/// The handle is deliberately leaked for the lifetime of the process: the
/// compositor object must outlive the surface. Repeated calls are no-ops.
pub fn install_blur(window: &winit::window::Window) -> Result<(), String> {
    if BLUR_INSTALLED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let window_handle = window.window_handle().map_err(|e| e.to_string())?;
    let display_handle = window.display_handle().map_err(|e| e.to_string())?;
    let display = match display_handle.as_raw() {
        RawDisplayHandle::Wayland(h) => h.display.as_ptr(),
        other => return Err(format!("Wayland blur needs a Wayland display, got {other:?}")),
    };
    let surface = match window_handle.as_raw() {
        RawWindowHandle::Wayland(h) => h.surface.as_ptr(),
        other => return Err(format!("Wayland blur needs a Wayland surface, got {other:?}")),
    };

    // SAFETY: the raw handles come from winit's live Wayland window; the
    // resulting Connection is a "guest" backend that never closes the display.
    let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
    let conn = Connection::from_backend(backend);
    let (globals, queue) = registry_queue_init::<BlurState>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();

    // SAFETY: same live wl_surface as above. from_id validates the interface.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface as *mut _) }
        .map_err(|e| e.to_string())?;
    let surface = WlSurface::from_id(&conn, id).map_err(|e| e.to_string())?;
    let manager = globals
        .bind::<ExtBackgroundEffectManagerV1, BlurState, ()>(&qh, 1..=1, ())
        .map_err(|e| format!("ext_background_effect_manager_v1 unavailable: {e}"))?;
    let compositor = globals
        .bind::<WlCompositor, BlurState, ()>(&qh, 4..=4, ())
        .map_err(|e| format!("wl_compositor unavailable: {e}"))?;
    let region: WlRegion = compositor.create_region(&qh, ());
    // KWin clips the region to the surface. Using a generous fixed rectangle
    // avoids having to recreate the effect after every window resize.
    region.add(0, 0, 32768, 32768);
    let effect = manager.get_background_effect(&surface, &qh, ());
    effect.set_blur_region(Some(&region));
    region.destroy();
    conn.flush().map_err(|e| e.to_string())?;

    Box::leak(Box::new(BlurHandle {
        _conn: conn,
        _surface: surface,
        _manager: manager,
        _effect: effect,
    }));
    log::info!("KWin Wayland compositor blur enabled (ext_background_effect_v1)");
    Ok(())
}
