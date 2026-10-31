//! Pretty renderer abstraction and wgpu implementation.
//!
//! Vulkan and OpenGL ES 3.0+ are first-class. Metal and Direct3D are reserved
//! interface skeletons: they return explicit errors and never claim support.

pub mod backend;
pub mod context;
pub mod gpu;
pub mod renderer;
pub mod text;
pub mod theme;

pub use backend::{driver_for, RendererKind, RESERVED_BACKENDS};
pub use context::{probe_adapters, GpuInfo, RendererError};
pub use renderer::{PaneDraw, RenderOutcome, Renderer, UiPaint};
pub use text::CellMetrics;
pub use theme::Theme;
