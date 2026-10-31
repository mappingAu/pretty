//! wgpu instance, adapter, device, surface and swapchain management.

use std::fmt;
use std::sync::Arc;

use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;
use wgpu::{
    Adapter, AdapterInfo, Backend, Backends, Device, DeviceType, Instance, InstanceDescriptor,
    Queue, Surface, SurfaceConfiguration, TextureFormat,
};

use crate::backend::{driver_for, BackendDriver, RendererKind};

#[derive(Debug, thiserror::Error)]
pub enum RendererError {
    #[error("no usable GPU adapter: {0}")]
    NoAdapter(String),
    #[error("failed to create rendering surface: {0}")]
    Surface(String),
    #[error("failed to request GPU device: {0}")]
    Device(String),
    #[error("unsupported backend: {0}")]
    UnsupportedBackend(String),
    #[error("surface error: {0}")]
    SurfaceLost(String),
    #[error("font error: {0}")]
    Font(String),
    #[error("render error: {0}")]
    Render(String),
}

#[derive(Clone, Debug)]
pub struct GpuInfo {
    pub requested: RendererKind,
    pub selected: RendererKind,
    pub wgpu_backend: Backend,
    pub adapter: AdapterInfo,
    pub surface_format: TextureFormat,
    pub alpha_mode: wgpu::CompositeAlphaMode,
    pub present_mode: wgpu::PresentMode,
    pub transparent: bool,
    pub max_texture_dimension: u32,
}

impl fmt::Display for GpuInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "renderer={} backend={:?} adapter={:?} driver={} {} surface={:?} alpha={:?} present={:?} transparent={} max_texture={}",
            self.selected,
            self.wgpu_backend,
            self.adapter.name,
            self.adapter.driver,
            self.adapter.driver_info,
            self.surface_format,
            self.alpha_mode,
            self.present_mode,
            self.transparent,
            self.max_texture_dimension
        )
    }
}

pub struct GpuContext {
    pub instance: Instance,
    pub surface: Surface<'static>,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
    pub config: SurfaceConfiguration,
    pub info: GpuInfo,
    pub size: (u32, u32),
}

impl GpuContext {
    pub fn new(
        window: Arc<Window>,
        display_handle: OwnedDisplayHandle,
        requested: RendererKind,
        transparent_requested: bool,
        vsync: bool,
        allow_software: bool,
    ) -> Result<Self, RendererError> {
        if requested.is_reserved() {
            return Err(RendererError::UnsupportedBackend(format!(
                "{} backend is a reserved interface and is not implemented",
                requested
            )));
        }
        let candidates = requested.candidates();
        let mut enabled = Backends::empty();
        for candidate in &candidates {
            if !candidate.compile_enabled() {
                if requested != RendererKind::Auto {
                    return Err(RendererError::UnsupportedBackend(format!(
                        "Pretty was built without the {candidate} feature"
                    )));
                }
                continue;
            }
            match candidate {
                RendererKind::Vulkan => enabled |= Backends::VULKAN,
                RendererKind::Gles => enabled |= Backends::GL,
                _ => {}
            }
        }
        if enabled.is_empty() {
            return Err(RendererError::NoAdapter(
                "no rendering backend features are compiled in (enable vulkan or gles)".into(),
            ));
        }
        // GL/EGL needs the platform display to be supplied up front; Vulkan
        // ignores it. Supplying it also makes the GL surface compatible.
        let mut descriptor = InstanceDescriptor::new_with_display_handle(Box::new(display_handle));
        descriptor.backends = enabled;
        let instance = Instance::new(descriptor);
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::from_window_without_display(window.clone()))
            .map_err(|e| RendererError::Surface(e.to_string()))?;

        let adapters = pollster::block_on(instance.enumerate_adapters(enabled));
        if adapters.is_empty() {
            return Err(RendererError::NoAdapter(format!(
                "no wgpu adapters for {:?}",
                enabled
            )));
        }
        let mut attempts = Vec::new();
        let mut chosen: Option<(Adapter, RendererKind, Box<dyn BackendDriver>)> = None;
        for candidate in &candidates {
            if !candidate.compile_enabled() {
                attempts.push(format!("{candidate}: feature not compiled"));
                continue;
            }
            let driver = match driver_for(*candidate) {
                Ok(d) => d,
                Err(e) => {
                    attempts.push(format!("{candidate}: {e}"));
                    continue;
                }
            };
            let mut matching: Vec<&Adapter> = adapters
                .iter()
                .filter(|a| {
                    let info = a.get_info();
                    info.backend == wgpu_backend(*candidate)
                })
                .collect();
            matching.sort_by_key(|a| adapter_rank(&a.get_info()));
            let Some(adapter) = matching.into_iter().next() else {
                attempts.push(format!("{candidate}: no {:?} adapter", wgpu_backend(*candidate)));
                continue;
            };
            let info = adapter.get_info();
            if let Err(e) = driver.accept_adapter(&info) {
                if allow_software && info.device_type == wgpu::DeviceType::Cpu {
                    log::warn!(
                        "ALLOWING SOFTWARE ADAPTER (testing flag --allow-software): {} ({})",
                        info.name,
                        e
                    );
                } else {
                    attempts.push(format!("{candidate}: {e}"));
                    continue;
                }
            }
            if !adapter.is_surface_supported(&surface) {
                attempts.push(format!("{candidate}: adapter cannot present to this surface"));
                continue;
            }
            let caps = surface.get_capabilities(adapter);
            if caps.formats.is_empty() {
                attempts.push(format!("{candidate}: surface has no formats"));
                continue;
            }
            chosen = Some((adapter.clone(), *candidate, driver));
            break;
        }
        let Some((adapter, selected, driver)) = chosen else {
            return Err(RendererError::NoAdapter(attempts.join("; ")));
        };

        let adapter_info = adapter.get_info();
        let caps = surface.get_capabilities(&adapter);
        let alpha_mode = driver.choose_alpha_mode(&caps, transparent_requested);
        let transparent = transparent_requested
            && alpha_mode != wgpu::CompositeAlphaMode::Opaque
            && alpha_mode != wgpu::CompositeAlphaMode::Auto;
        if transparent_requested && !transparent {
            log::warn!("transparency was requested but the surface only supports opaque compositing");
        }
        let format = choose_format(&caps.formats);
        let present_mode = if vsync {
            wgpu::PresentMode::Fifo
        } else if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| RendererError::Surface("surface is not supported by adapter".into()))?;
        config.format = format;
        config.present_mode = present_mode;
        config.alpha_mode = alpha_mode;
        config.desired_maximum_frame_latency = 2;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pretty-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .map_err(|e| RendererError::Device(e.to_string()))?;
        surface.configure(&device, &config);

        let info = GpuInfo {
            requested,
            selected,
            wgpu_backend: adapter_info.backend,
            adapter: adapter_info,
            surface_format: config.format,
            alpha_mode,
            present_mode,
            transparent,
            max_texture_dimension: adapter.limits().max_texture_dimension_2d,
        };
        log::info!("GPU: {info}");
        Ok(Self {
            instance,
            surface,
            adapter,
            device,
            queue,
            config,
            info,
            size: (width, height),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn reconfigure(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.size == (width, height) {
            return;
        }
        self.size = (width, height);
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        log::info!(
            "swapchain reconfigured: {}x{} format={:?} present={:?}",
            width,
            height,
            self.config.format,
            self.config.present_mode
        );
    }

    pub fn current_texture(&mut self) -> wgpu::CurrentSurfaceTexture {
        self.surface.get_current_texture()
    }
}

fn wgpu_backend(kind: RendererKind) -> Backend {
    match kind {
        RendererKind::Vulkan => Backend::Vulkan,
        RendererKind::Gles => Backend::Gl,
        RendererKind::Metal => Backend::Metal,
        RendererKind::Dx => Backend::Dx12,
        RendererKind::Auto => Backend::Vulkan,
    }
}

fn adapter_rank(info: &AdapterInfo) -> u8 {
    match info.device_type {
        DeviceType::DiscreteGpu => 0,
        DeviceType::IntegratedGpu => 1,
        DeviceType::VirtualGpu => 2,
        DeviceType::Other => 3,
        DeviceType::Cpu => 4,
    }
}

fn choose_format(formats: &[TextureFormat]) -> TextureFormat {
    for format in [
        TextureFormat::Bgra8UnormSrgb,
        TextureFormat::Rgba8UnormSrgb,
        TextureFormat::Bgra8Unorm,
        TextureFormat::Rgba8Unorm,
    ] {
        if formats.contains(&format) {
            return format;
        }
    }
    formats[0]
}

/// Probe adapters without creating a window. Used by `pretty --print-info`.
pub fn probe_adapters(kind: RendererKind) -> Vec<AdapterInfo> {
    let candidates = kind.candidates();
    let mut backends = Backends::empty();
    for c in &candidates {
        match c {
            RendererKind::Vulkan if c.compile_enabled() => backends |= Backends::VULKAN,
            RendererKind::Gles if c.compile_enabled() => backends |= Backends::GL,
            _ => {}
        }
    }
    if backends.is_empty() {
        return Vec::new();
    }
    let instance = Instance::new(InstanceDescriptor {
        backends,
        ..InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(instance.enumerate_adapters(backends))
        .into_iter()
        .map(|a| a.get_info())
        .collect()
}
