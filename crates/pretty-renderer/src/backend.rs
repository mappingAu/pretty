//! Backend drivers and runtime selection.

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

    pub fn is_reserved(self) -> bool {
        matches!(self, Self::Metal | Self::Dx)
    }

    /// Build the ordered candidate list for `auto`.
    pub fn candidates(self) -> Vec<Self> {
        match self {
            Self::Auto => vec![Self::Vulkan, Self::Gles],
            other => vec![other],
        }
    }

    pub fn compile_enabled(self) -> bool {
        match self {
            Self::Auto => true,
            Self::Vulkan => cfg!(feature = "vulkan"),
            Self::Gles => cfg!(feature = "gles"),
            Self::Metal => cfg!(feature = "metal"),
            Self::Dx => cfg!(feature = "dx"),
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

/// Construct the driver for a real (implemented) backend. Reserved backends are
/// rejected with an explicit error and never silently fall back.
pub fn driver_for(kind: RendererKind) -> Result<Box<dyn BackendDriver>, String> {
    match kind {
        RendererKind::Vulkan => {
            #[cfg(feature = "vulkan")]
            {
                Ok(Box::new(VulkanDriver))
            }
            #[cfg(not(feature = "vulkan"))]
            {
                Err("Pretty was built without the Vulkan feature".into())
            }
        }
        RendererKind::Gles => {
            #[cfg(feature = "gles")]
            {
                Ok(Box::new(GlesDriver))
            }
            #[cfg(not(feature = "gles"))]
            {
                Err("Pretty was built without the GLES feature".into())
            }
        }
        RendererKind::Metal => Err(
            "Metal backend is a reserved interface and is not implemented; \
             select vulkan or gles"
                .into(),
        ),
        RendererKind::Dx => Err(
            "DirectX backend is a reserved interface and is not implemented; \
             select vulkan or gles"
                .into(),
        ),
        RendererKind::Auto => Err("internal error: Auto has no direct driver".into()),
    }
}

pub const RESERVED_BACKENDS: &[(&str, &str)] = &[
    (
        "metal",
        "reserved interface skeleton; returns UnsupportedBackend, never selected",
    ),
    (
        "dx",
        "reserved interface skeleton; returns UnsupportedBackend, never selected",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_backends_are_explicit_errors() {
        assert!(matches!(driver_for(RendererKind::Metal), Err(ref e) if e.contains("reserved")));
        assert!(matches!(driver_for(RendererKind::Dx), Err(ref e) if e.contains("reserved")));
    }

    #[test]
    fn auto_prefers_vulkan_then_gles() {
        assert_eq!(
            RendererKind::Auto.candidates(),
            vec![RendererKind::Vulkan, RendererKind::Gles]
        );
    }
}
