//! Pretty renderer abstraction and wgpu implementation.
//!
//! Vulkan and OpenGL ES 3.0+ are first-class. Metal (macOS) and Direct3D 12
//! (Windows) are real wgpu backends too: they are compiled in with the `metal`
//! and `dx` Cargo features, and `auto` prefers the platform-native API. A
//! backend that is not compiled in, or that cannot exist on the host operating
//! system, is rejected with a typed error and never silently substituted.

pub mod backend;
pub mod context;
pub mod gpu;
pub mod renderer;
pub mod text;
pub mod theme;

pub use backend::{driver_for, BackendError, RendererKind};
pub use context::{probe_adapters, GpuInfo, RendererError};
pub use renderer::{PaneDraw, RenderOutcome, Renderer, UiPaint};
pub use text::CellMetrics;
pub use theme::Theme;
