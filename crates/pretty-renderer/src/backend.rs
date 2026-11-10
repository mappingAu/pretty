//! Backend drivers and runtime selection.
//!
//! Vulkan, OpenGL ES 3.0+, Metal and Direct3D 12 are all implemented through
//! `wgpu`. Whether a backend is *available* depends on two independent things:
//!
//! 1. the Cargo feature (`vulkan`, `gles`, `metal`, `dx`) compiled into this
//!    binary, checked by [`RendererKind::compile_enabled`];
//! 2. the host operating system, which determines the `auto` preference order
//!    in [`RendererKind::candidates`] and rejects backends that cannot exist on
//!    that platform (Metal is macOS-only, Direct3D 12 is Windows-only).
//!
//! Explicitly selecting a backend that is not available produces a typed error
//! instead of silently falling back to another API.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use wgpu::{AdapterInfo, Backends, CompositeAlphaMode, DeviceType, SurfaceCapabilities};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RendererKind {
    #[default]
    Auto,
    Vulkan,
    Gles,
    Metal,
    Dx,
}

impl RendererKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Vulkan => "vulkan",
            Self::Gles => "gles",
            Self::Metal => "metal",
            Self::Dx => "dx",
        }
    }

    /// Build the ordered candidate list.
    ///
    /// `auto` is platform-aware: Windows prefers Direct3D 12, macOS prefers
    /// Metal, and every other target (Linux, FreeBSD, Android) prefers Vulkan
    /// and then OpenGL ES. Candidates whose Cargo feature is not compiled into
    /// this binary are skipped by [`crate::context::GpuContext`].
    pub fn candidates(self) -> Vec<Self> {
        match self {
            Self::Auto => {
                if cfg!(target_os = "windows") {
                    vec![Self::Dx, Self::Vulkan, Self::Gles]
                } else if cfg!(target_os = "macos") {
                    // wgpu's Vulkan backend can also run on macOS through
                    // MoltenVK when the feature is compiled in. Keep it as a
                    // fallback after the native Metal backend.
                    vec![Self::Metal, Self::Vulkan]
                } else {
                    vec![Self::Vulkan, Self::Gles]
                }
            }
            other => vec![other],
        }
    }

    /// Whether this backend was compiled into the binary (Cargo feature).
    pub const fn compile_enabled(self) -> bool {
        match self {
            Self::Auto => true,
            Self::Vulkan => cfg!(feature = "vulkan"),
            Self::Gles => cfg!(feature = "gles"),
            Self::Metal => cfg!(feature = "metal"),
            Self::Dx => cfg!(feature = "dx"),
        }
    }

    /// Whether this backend can exist on the host operating system at all.
    pub const fn platform_supported(self) -> bool {
        match self {
            Self::Auto => true,
            Self::Vulkan => true,
            Self::Gles => true,
            Self::Metal => cfg!(target_os = "macos"),
            Self::Dx => cfg!(target_os = "windows"),
        }
    }
}

impl fmt::Display for RendererKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RendererKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "vulkan" | "vk" => Ok(Self::Vulkan),
            "gles" | "gles3" | "gl" | "opengl" => Ok(Self::Gles),
            "metal" => Ok(Self::Metal),
            "dx" | "dx12" | "d3d12" | "directx" => Ok(Self::Dx),
            other => Err(format!(
                "unknown renderer '{other}' (expected auto, vulkan, gles, metal or dx)"
            )),
        }
    }
}

pub trait BackendDriver: Send + Sync {
    fn kind(&self) -> RendererKind;
    fn wgpu_backends(&self) -> Backends;
    /// Reject adapters that cannot be used by this driver.
    fn accept_adapter(&self, info: &AdapterInfo) -> Result<(), String> {
        if info.device_type == DeviceType::Cpu {
            return Err(format!(
                "{} is a CPU/software adapter; Pretty requires hardware acceleration",
                info.name
            ));
        }
        Ok(())
    }
    /// Select a compositing alpha mode for the requested opacity.
    fn choose_alpha_mode(
        &self,
        caps: &SurfaceCapabilities,
        transparent: bool,
    ) -> CompositeAlphaMode {
        if transparent {
            for mode in [
                CompositeAlphaMode::PreMultiplied,
                CompositeAlphaMode::PostMultiplied,
                CompositeAlphaMode::Inherit,
            ] {
                if caps.alpha_modes.contains(&mode) {
                    return mode;
                }
            }
            log::warn!(
                "{}: surface does not support transparent compositing (modes: {:?}); using opaque",
                self.kind(),
                caps.alpha_modes
            );
        }
        CompositeAlphaMode::Opaque
    }
}

pub struct VulkanDriver;
impl BackendDriver for VulkanDriver {
    fn kind(&self) -> RendererKind {
        RendererKind::Vulkan
    }
    fn wgpu_backends(&self) -> Backends {
        Backends::VULKAN
    }
}

pub struct GlesDriver;
impl BackendDriver for GlesDriver {
    fn kind(&self) -> RendererKind {
        RendererKind::Gles
    }
    fn wgpu_backends(&self) -> Backends {
        Backends::GL
    }
    fn accept_adapter(&self, info: &AdapterInfo) -> Result<(), String> {
        if info.device_type == DeviceType::Cpu {
            return Err(format!(
                "{} is a CPU/software adapter; Pretty requires hardware acceleration",
                info.name
            ));
        }
        if info.driver.contains("OpenGL ES") || info.driver.contains("ANGLE") {
            return Ok(());
        }
        if std::env::var_os("PRETTY_GL_FORCE_GLES").is_some() {
            // The driver string can be empty when wgpu enumerates the adapter
            // through an EGL window platform. The local wgpu-hal patch has
            // already forced EGL_OPENGL_ES_API, and GpuContext logs the actual
            // window/presentation result. Do not reject a valid forced GLES
            // context solely because the optional string is absent.
            log::warn!(
                "GLES adapter driver string is empty; context was explicitly forced through EGL_OPENGL_ES_API"
            );
            return Ok(());
        }
        Err(format!(
            "GL backend selected a desktop OpenGL context ({}); Pretty's GLES backend requires a real OpenGL ES context",
            info.driver
        ))
    }
}

/// Metal backend driver (macOS). Enabled by the `metal` Cargo feature.
#[cfg(any(target_os = "macos", feature = "metal"))]
pub struct MetalDriver;
#[cfg(any(target_os = "macos", feature = "metal"))]
impl BackendDriver for MetalDriver {
    fn kind(&self) -> RendererKind {
        RendererKind::Metal
    }
    fn wgpu_backends(&self) -> Backends {
        Backends::METAL
    }
}

/// Direct3D 12 backend driver (Windows). Enabled by the `dx` Cargo feature.
#[cfg(any(target_os = "windows", feature = "dx"))]
pub struct DxDriver;
#[cfg(any(target_os = "windows", feature = "dx"))]
impl BackendDriver for DxDriver {
    fn kind(&self) -> RendererKind {
        RendererKind::Dx
    }
    fn wgpu_backends(&self) -> Backends {
        Backends::DX12
    }
}

/// Construct the driver for a backend that is compiled in and supported by the
/// host platform. Unavailable backends are rejected with an explicit error and
/// never silently fall back.
pub fn driver_for(kind: RendererKind) -> Result<Box<dyn BackendDriver>, String> {
    match kind {
        RendererKind::Vulkan => {
            #[cfg(feature = "vulkan")]
            {
                Ok(Box::new(VulkanDriver))
            }
            #[cfg(not(feature = "vulkan"))]
            {
                Err("Pretty was built without the Vulkan feature (enable the `vulkan` Cargo feature)".into())
            }
        }
        RendererKind::Gles => {
            #[cfg(feature = "gles")]
            {
                Ok(Box::new(GlesDriver))
            }
            #[cfg(not(feature = "gles"))]
            {
                Err("Pretty was built without the GLES feature (enable the `gles` Cargo feature)".into())
            }
        }
        RendererKind::Metal => {
            #[cfg(feature = "metal")]
            {
                if cfg!(target_os = "macos") {
                    Ok(Box::new(MetalDriver))
                } else {
                    Err(format!(
                        "the Metal backend is only available on macOS; this binary targets {} \
                         (use --renderer vulkan or --renderer gles)",
                        std::env::consts::OS
                    ))
                }
            }
            #[cfg(not(feature = "metal"))]
            {
                Err("Pretty was built without the Metal feature (Windows/Linux builds: \
                     use --renderer vulkan or --renderer gles; macOS: rebuild with --features metal)"
                    .into())
            }
        }
        RendererKind::Dx => {
            #[cfg(feature = "dx")]
            {
                if cfg!(target_os = "windows") {
                    Ok(Box::new(DxDriver))
                } else {
                    Err(format!(
                        "the Direct3D 12 backend is only available on Windows; this binary targets {} \
                         (use --renderer vulkan or --renderer gles)",
                        std::env::consts::OS
                    ))
                }
            }
            #[cfg(not(feature = "dx"))]
            {
                Err("Pretty was built without the Direct3D 12 feature (Linux/macOS builds: \
                     use --renderer vulkan or --renderer gles; Windows: rebuild with --features dx)"
                    .into())
            }
        }
        RendererKind::Auto => Err("internal error: Auto has no direct driver".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_auto_order() -> Vec<RendererKind> {
        if cfg!(target_os = "windows") {
            vec![RendererKind::Dx, RendererKind::Vulkan, RendererKind::Gles]
        } else if cfg!(target_os = "macos") {
            vec![RendererKind::Metal, RendererKind::Vulkan]
        } else {
            vec![RendererKind::Vulkan, RendererKind::Gles]
        }
    }

    #[test]
    fn auto_order_is_platform_aware() {
        assert_eq!(RendererKind::Auto.candidates(), expected_auto_order());
    }

    #[test]
    fn explicit_kinds_have_a_single_candidate() {
        for kind in [
            RendererKind::Vulkan,
            RendererKind::Gles,
            RendererKind::Metal,
            RendererKind::Dx,
        ] {
            assert_eq!(kind.candidates(), vec![kind]);
        }
    }

    #[test]
    fn compile_enabled_matches_cargo_features() {
        assert!(RendererKind::Auto.compile_enabled());
        assert_eq!(RendererKind::Vulkan.compile_enabled(), cfg!(feature = "vulkan"));
        assert_eq!(RendererKind::Gles.compile_enabled(), cfg!(feature = "gles"));
        assert_eq!(RendererKind::Metal.compile_enabled(), cfg!(feature = "metal"));
        assert_eq!(RendererKind::Dx.compile_enabled(), cfg!(feature = "dx"));
    }

    #[test]
    fn platform_support_is_os_specific() {
        assert!(RendererKind::Vulkan.platform_supported());
        assert!(RendererKind::Gles.platform_supported());
        assert_eq!(RendererKind::Metal.platform_supported(), cfg!(target_os = "macos"));
        assert_eq!(RendererKind::Dx.platform_supported(), cfg!(target_os = "windows"));
    }

    #[test]
    fn auto_has_no_direct_driver() {
        assert!(driver_for(RendererKind::Auto).is_err());
    }

    #[cfg(feature = "vulkan")]
    #[test]
    fn vulkan_driver_uses_the_vulkan_wgpu_backend() {
        let driver = driver_for(RendererKind::Vulkan).expect("vulkan feature is compiled in");
        assert_eq!(driver.kind(), RendererKind::Vulkan);
        assert_eq!(driver.wgpu_backends(), Backends::VULKAN);
    }

    #[cfg(feature = "gles")]
    #[test]
    fn gles_driver_uses_the_gl_wgpu_backend() {
        let driver = driver_for(RendererKind::Gles).expect("gles feature is compiled in");
        assert_eq!(driver.kind(), RendererKind::Gles);
        assert_eq!(driver.wgpu_backends(), Backends::GL);
    }

    #[cfg(feature = "metal")]
    #[test]
    fn metal_driver_is_available_on_macos_only() {
        match driver_for(RendererKind::Metal) {
            Ok(driver) => {
                assert!(cfg!(target_os = "macos"), "Metal driver must not be usable on non-macOS");
                assert_eq!(driver.kind(), RendererKind::Metal);
                assert_eq!(driver.wgpu_backends(), Backends::METAL);
            }
            Err(e) => {
                assert!(!cfg!(target_os = "macos"));
                assert!(e.contains("macOS"), "unexpected error: {e}");
            }
        }
    }

    #[cfg(feature = "dx")]
    #[test]
    fn dx_driver_is_available_on_windows_only() {
        match driver_for(RendererKind::Dx) {
            Ok(driver) => {
                assert!(cfg!(target_os = "windows"), "Dx12 driver must not be usable on non-Windows");
                assert_eq!(driver.kind(), RendererKind::Dx);
                assert_eq!(driver.wgpu_backends(), Backends::DX12);
            }
            Err(e) => {
                assert!(!cfg!(target_os = "windows"));
                assert!(e.contains("Windows"), "unexpected error: {e}");
            }
        }
    }

    #[cfg(not(feature = "metal"))]
    #[test]
    fn metal_without_the_feature_is_an_explicit_error() {
        match driver_for(RendererKind::Metal) {
            Err(err) => assert!(
                err.contains("without the Metal feature"),
                "unexpected error: {err}"
            ),
            Ok(_) => panic!("metal driver must not be available without the feature"),
        }
    }

    #[cfg(not(feature = "dx"))]
    #[test]
    fn dx_without_the_feature_is_an_explicit_error() {
        match driver_for(RendererKind::Dx) {
            Err(err) => assert!(
                err.contains("without the Direct3D 12 feature"),
                "unexpected error: {err}"
            ),
            Ok(_) => panic!("dx driver must not be available without the feature"),
        }
    }

    #[test]
    fn renderer_names_round_trip() {
        for (name, kind) in [
            ("auto", RendererKind::Auto),
            ("vulkan", RendererKind::Vulkan),
            ("gles", RendererKind::Gles),
            ("metal", RendererKind::Metal),
            ("dx", RendererKind::Dx),
            ("dx12", RendererKind::Dx),
            ("d3d12", RendererKind::Dx),
            ("directx", RendererKind::Dx),
            ("vk", RendererKind::Vulkan),
        ] {
            assert_eq!(name.parse::<RendererKind>(), Ok(kind), "parsing {name}");
        }
        for kind in [
            RendererKind::Auto,
            RendererKind::Vulkan,
            RendererKind::Gles,
            RendererKind::Metal,
            RendererKind::Dx,
        ] {
            assert_eq!(kind.as_str().parse::<RendererKind>(), Ok(kind));
        }
    }
}
