//! Platform-specific integration. All non-portable window-system code lives
//! here; the rest of the app is platform-neutral.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod x11;
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub mod other;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub use x11::apply_compositor_effects;
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub use other::apply_compositor_effects;
