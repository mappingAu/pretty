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
//! Explicitly selecting a backend that is not available produces a typed
//! [`BackendError`] instead of silently falling back to another API. The same
//! [`RendererKind`] to [`wgpu::Backend`] mapping is used by adapter selection
//! and by `--print-info`, so a driver cannot disagree with the instance it was
//! created from.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use wgpu::{AdapterInfo, Backend, Backends, CompositeAlphaMode, DeviceType, SurfaceCapabilities};

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
    pub const ALL: [Self; 5] = [
        Self::Auto,
        Self::Vulkan,
        Self::Gles,
        Self::Metal,
        Self::Dx,
    ];

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

    /// Cargo feature that compiles this backend into the binary.
    pub const fn required_feature(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Vulkan => Some("vulkan"),
            Self::Gles => Some("gles"),
            Self::Metal => Some("metal"),
            Self::Dx => Some("dx"),
        }
    }

    /// Operating system on which this backend can exist, if it is restricted.
    pub const fn required_os(self) -> Option<&'static str> {
        match self {
            Self::Metal => Some("macOS"),
            Self::Dx => Some("Windows"),
            _ => None,
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
            Self::Auto | Self::Vulkan | Self::Gles => true,
            Self::Metal => cfg!(target_os = "macos"),
            Self::Dx => cfg!(target_os = "windows"),
        }
    }

    /// The concrete `wgpu` backend for this kind (`None` for `auto`).
    pub const fn wgpu_backend(self) -> Option<Backend> {
        match self {
            Self::Auto => None,
            Self::Vulkan => Some(Backend::Vulkan),
            Self::Gles => Some(Backend::Gl),
            Self::Metal => Some(Backend::Metal),
            Self::Dx => Some(Backend::Dx12),
        }
    }

    /// The `wgpu::Backends` bit for this kind (`empty` for `auto`).
    pub const fn wgpu_backends(self) -> Backends {
        match self {
            Self::Auto => Backends::empty(),
            Self::Vulkan => Backends::VULKAN,
            Self::Gles => Backends::GL,
            Self::Metal => Backends::METAL,
            Self::Dx => Backends::DX12,
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

/// Why a concrete backend cannot be used by this build.
///
/// The two failure modes are deliberately distinct so callers (and users) can
/// tell a packaging problem from a platform mistake:
///
/// * [`BackendError::FeatureNotCompiled`] — the binary was built without e.g.
///   `--features metal`; rebuilding fixes it.
/// * [`BackendError::WrongOperatingSystem`] — the backend is compiled in but
///   cannot exist on this host (Metal on Linux, Direct3D 12 on macOS, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    FeatureNotCompiled {
        backend: RendererKind,
    },
    WrongOperatingSystem {
        backend: RendererKind,
        host: &'static str,
        required: &'static str,
    },
    /// `auto` is a selection policy, not a driver.
    NoConcreteBackend,
}

impl BackendError {
    /// Backend the error refers to (`None` for `auto`).
    pub fn backend(&self) -> Option<RendererKind> {
        match self {
            Self::FeatureNotCompiled { backend }
            | Self::WrongOperatingSystem { backend, .. } => Some(*backend),
            Self::NoConcreteBackend => None,
        }
    }

    /// True when rebuilding with another Cargo feature would help.
    pub fn is_feature_missing(&self) -> bool {
        matches!(self, Self::FeatureNotCompiled { .. })
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FeatureNotCompiled { backend } => {
                let feature = backend.required_feature().unwrap_or("unknown");
                write!(
                    f,
                    "Pretty was built without the {backend} backend \
                     (rebuild with --features {feature})"
                )
            }
            Self::WrongOperatingSystem {
                backend,
                host,
                required,
            } => write!(
                f,
                "the {backend} backend is only available on {required}; \
                 this binary targets {host} (use --renderer vulkan or --renderer gles)"
            ),
            Self::NoConcreteBackend => {
                f.write_str("internal error: 'auto' is a selection policy, not a backend driver")
            }
        }
    }
}

impl std::error::Error for BackendError {}

pub trait BackendDriver: Send + Sync {
    fn kind(&self) -> RendererKind;

    /// The single `wgpu` backend this driver selects adapters from.
    fn wgpu_backend(&self) -> Backend {
        self.kind().wgpu_backend().unwrap_or(Backend::Vulkan)
    }

    fn wgpu_backends(&self) -> Backends {
        self.kind().wgpu_backends()
    }

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
    ///
    /// Metal and Direct3D 12 use the same policy as Vulkan/GLES: prefer a
    /// premultiplied transparent mode when opacity is requested, and fall back
    /// to opaque with a warning when the surface cannot composite alpha.
    fn choose_alpha_mode(
        &self,
        caps: &SurfaceCapabilities,
        transparent: bool,
    ) -> CompositeAlphaMode {
        let mode = select_alpha_mode(&caps.alpha_modes, transparent);
        if transparent && mode == CompositeAlphaMode::Opaque {
            log::warn!(
                "{}: surface does not support transparent compositing (modes: {:?}); using opaque",
                self.kind(),
                caps.alpha_modes
            );
        }
        mode
    }
}

/// Shared alpha-mode policy for every backend (including Metal and DX12).
pub fn select_alpha_mode(
    modes: &[CompositeAlphaMode],
    transparent: bool,
) -> CompositeAlphaMode {
    if transparent {
        for mode in [
            CompositeAlphaMode::PreMultiplied,
            CompositeAlphaMode::PostMultiplied,
            CompositeAlphaMode::Inherit,
        ] {
            if modes.contains(&mode) {
                return mode;
            }
        }
    }
    CompositeAlphaMode::Opaque
}

pub struct VulkanDriver;
impl BackendDriver for VulkanDriver {
    fn kind(&self) -> RendererKind {
        RendererKind::Vulkan
    }
}

pub struct GlesDriver;
impl BackendDriver for GlesDriver {
    fn kind(&self) -> RendererKind {
        RendererKind::Gles
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
}

/// Direct3D 12 backend driver (Windows). Enabled by the `dx` Cargo feature.
#[cfg(any(target_os = "windows", feature = "dx"))]
pub struct DxDriver;
#[cfg(any(target_os = "windows", feature = "dx"))]
impl BackendDriver for DxDriver {
    fn kind(&self) -> RendererKind {
        RendererKind::Dx
    }
}

#[cfg(any(feature = "metal", feature = "dx"))]
fn ensure_platform(kind: RendererKind) -> Result<(), BackendError> {
    if kind.platform_supported() {
        return Ok(());
    }
    Err(BackendError::WrongOperatingSystem {
        backend: kind,
        host: std::env::consts::OS,
        required: kind.required_os().unwrap_or("this operating system"),
    })
}

/// Construct the driver for a backend that is compiled in and supported by the
/// host platform. Unavailable backends are rejected with [`BackendError`] and
/// never silently fall back.
pub fn driver_for(kind: RendererKind) -> Result<Box<dyn BackendDriver>, BackendError> {
    match kind {
        RendererKind::Vulkan => {
            #[cfg(feature = "vulkan")]
            {
                Ok(Box::new(VulkanDriver))
            }
            #[cfg(not(feature = "vulkan"))]
            {
                Err(BackendError::FeatureNotCompiled {
                    backend: RendererKind::Vulkan,
                })
            }
        }
        RendererKind::Gles => {
            #[cfg(feature = "gles")]
            {
                Ok(Box::new(GlesDriver))
            }
            #[cfg(not(feature = "gles"))]
            {
                Err(BackendError::FeatureNotCompiled {
                    backend: RendererKind::Gles,
                })
            }
        }
        RendererKind::Metal => {
            #[cfg(feature = "metal")]
            {
                ensure_platform(RendererKind::Metal)?;
                Ok(Box::new(MetalDriver))
            }
            #[cfg(not(feature = "metal"))]
            {
                Err(BackendError::FeatureNotCompiled {
                    backend: RendererKind::Metal,
                })
            }
        }
        RendererKind::Dx => {
            #[cfg(feature = "dx")]
            {
                ensure_platform(RendererKind::Dx)?;
                Ok(Box::new(DxDriver))
            }
            #[cfg(not(feature = "dx"))]
            {
                Err(BackendError::FeatureNotCompiled {
                    backend: RendererKind::Dx,
                })
            }
        }
        RendererKind::Auto => Err(BackendError::NoConcreteBackend),
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
    fn required_feature_and_os_are_declared() {
        assert_eq!(RendererKind::Vulkan.required_feature(), Some("vulkan"));
        assert_eq!(RendererKind::Gles.required_feature(), Some("gles"));
        assert_eq!(RendererKind::Metal.required_feature(), Some("metal"));
        assert_eq!(RendererKind::Dx.required_feature(), Some("dx"));
        assert_eq!(RendererKind::Auto.required_feature(), None);
        assert_eq!(RendererKind::Metal.required_os(), Some("macOS"));
        assert_eq!(RendererKind::Dx.required_os(), Some("Windows"));
        assert_eq!(RendererKind::Vulkan.required_os(), None);
    }

    #[test]
    fn wgpu_backend_mapping_is_consistent() {
        assert_eq!(RendererKind::Auto.wgpu_backend(), None);
        assert_eq!(RendererKind::Auto.wgpu_backends(), Backends::empty());
        for (kind, backend, backends) in [
            (RendererKind::Vulkan, Backend::Vulkan, Backends::VULKAN),
            (RendererKind::Gles, Backend::Gl, Backends::GL),
            (RendererKind::Metal, Backend::Metal, Backends::METAL),
            (RendererKind::Dx, Backend::Dx12, Backends::DX12),
        ] {
            assert_eq!(kind.wgpu_backend(), Some(backend));
            assert_eq!(kind.wgpu_backends(), backends);
        }
    }

    #[test]
    fn auto_has_no_direct_driver() {
        let error = match driver_for(RendererKind::Auto) {
            Ok(_) => panic!("auto must not construct a driver"),
            Err(error) => error,
        };
        assert_eq!(error, BackendError::NoConcreteBackend);
        assert!(!BackendError::NoConcreteBackend.is_feature_missing());
    }

    #[cfg(feature = "vulkan")]
    #[test]
    fn vulkan_driver_uses_the_vulkan_wgpu_backend() {
        let driver = driver_for(RendererKind::Vulkan).expect("vulkan feature is compiled in");
        assert_eq!(driver.kind(), RendererKind::Vulkan);
        assert_eq!(driver.wgpu_backend(), Backend::Vulkan);
        assert_eq!(driver.wgpu_backends(), Backends::VULKAN);
    }

    #[cfg(feature = "gles")]
    #[test]
    fn gles_driver_uses_the_gl_wgpu_backend() {
        let driver = driver_for(RendererKind::Gles).expect("gles feature is compiled in");
        assert_eq!(driver.kind(), RendererKind::Gles);
        assert_eq!(driver.wgpu_backend(), Backend::Gl);
        assert_eq!(driver.wgpu_backends(), Backends::GL);
    }

    #[cfg(all(feature = "metal", not(target_os = "macos")))]
    #[test]
    fn metal_feature_on_a_non_macos_host_is_a_typed_wrong_os_error() {
        match driver_for(RendererKind::Metal) {
            Err(error) => {
                assert_eq!(
                    error,
                    BackendError::WrongOperatingSystem {
                        backend: RendererKind::Metal,
                        host: std::env::consts::OS,
                        required: "macOS",
                    }
                );
                assert!(!error.is_feature_missing());
                let message = error.to_string();
                assert!(message.contains("macOS"), "unexpected message: {message}");
                assert!(
                    message.contains("--renderer vulkan"),
                    "unexpected message: {message}"
                );
            }
            Ok(_) => panic!("Metal must never be constructed on a non-macOS host"),
        }
    }

    #[cfg(all(feature = "dx", not(target_os = "windows")))]
    #[test]
    fn dx_feature_on_a_non_windows_host_is_a_typed_wrong_os_error() {
        match driver_for(RendererKind::Dx) {
            Err(error) => {
                assert_eq!(
                    error,
                    BackendError::WrongOperatingSystem {
                        backend: RendererKind::Dx,
                        host: std::env::consts::OS,
                        required: "Windows",
                    }
                );
                let message = error.to_string();
                assert!(message.contains("Windows"), "unexpected message: {message}");
            }
            Ok(_) => panic!("Direct3D 12 must never be constructed on a non-Windows host"),
        }
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    fn metal_driver_is_available_on_macos() {
        let driver = driver_for(RendererKind::Metal).expect("metal feature on macOS");
        assert_eq!(driver.kind(), RendererKind::Metal);
        assert_eq!(driver.wgpu_backend(), Backend::Metal);
    }

    #[cfg(all(feature = "dx", target_os = "windows"))]
    #[test]
    fn dx_driver_is_available_on_windows() {
        let driver = driver_for(RendererKind::Dx).expect("dx feature on Windows");
        assert_eq!(driver.kind(), RendererKind::Dx);
        assert_eq!(driver.wgpu_backend(), Backend::Dx12);
    }

    #[cfg(not(feature = "vulkan"))]
    #[test]
    fn missing_vulkan_feature_is_a_typed_error() {
        let error = match driver_for(RendererKind::Vulkan) {
            Ok(_) => panic!("vulkan feature must not be compiled in for this test"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            BackendError::FeatureNotCompiled {
                backend: RendererKind::Vulkan
            }
        );
        assert!(error.is_feature_missing());
        assert!(error.to_string().contains("--features vulkan"));
    }

    #[cfg(not(feature = "metal"))]
    #[test]
    fn metal_without_the_feature_is_a_typed_error() {
        let error = match driver_for(RendererKind::Metal) {
            Ok(_) => panic!("metal feature must not be compiled in for this test"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            BackendError::FeatureNotCompiled {
                backend: RendererKind::Metal
            }
        );
        assert!(error.to_string().contains("--features metal"));
    }

    #[cfg(not(feature = "dx"))]
    #[test]
    fn dx_without_the_feature_is_a_typed_error() {
        let error = match driver_for(RendererKind::Dx) {
            Ok(_) => panic!("dx feature must not be compiled in for this test"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            BackendError::FeatureNotCompiled {
                backend: RendererKind::Dx
            }
        );
        assert!(error.to_string().contains("--features dx"));
    }

    #[test]
    fn alpha_mode_policy_is_shared_by_every_backend() {
        use CompositeAlphaMode::{Auto, Opaque, PostMultiplied, PreMultiplied};
        assert_eq!(select_alpha_mode(&[Opaque], false), Opaque);
        assert_eq!(
            select_alpha_mode(&[Opaque, PostMultiplied], true),
            PostMultiplied
        );
        assert_eq!(
            select_alpha_mode(&[Auto, PreMultiplied, PostMultiplied], true),
            PreMultiplied
        );
        assert_eq!(
            select_alpha_mode(&[Auto, PostMultiplied], true),
            PostMultiplied
        );
        assert_eq!(select_alpha_mode(&[Auto, Opaque], true), Opaque);
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
        for kind in RendererKind::ALL {
            assert_eq!(kind.as_str().parse::<RendererKind>(), Ok(kind));
        }
    }
}
