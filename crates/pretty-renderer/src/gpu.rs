//! GPU pipelines shared by the terminal renderer.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use pretty_core::FrameImage;
use crate::context::RendererError;

pub const MAX_SOLID_QUADS: usize = 65536;
pub const MAX_IMAGE_QUADS: usize = 8192;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Globals {
    pub screen: [f32; 2],
    pub _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SolidInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ImageInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub uv: [f32; 4],
}

pub struct GlobalsBinding {
    pub buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub layout: wgpu::BindGroupLayout,
}

impl GlobalsBinding {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pretty-globals-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pretty-globals-buffer"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pretty-globals-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        Self { buffer, bind_group, layout }
    }

    pub fn update(&self, queue: &wgpu::Queue, width: u32, height: u32) {
        queue.write_buffer(
            &self.buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [width as f32, height as f32],
                _pad: [0.0, 0.0],
            }),
        );
    }
}

pub struct SolidRenderer {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    count: u32,
    capacity: usize,
}

impl SolidRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        globals: &GlobalsBinding,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pretty-solid-shader"),
            source: wgpu::ShaderSource::Wgsl(SOLID_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pretty-solid-layout"),
            bind_group_layouts: &[Some(&globals.layout)],
            immediate_size: 0,
        });
        let attrs = [
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 16,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x4,
            },
        ];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SolidInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attrs,
        })];
        let targets = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pretty-solid-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let buffer = create_instance_buffer::<SolidInstance>(
            device,
            "pretty-solid-instances",
            MAX_SOLID_QUADS,
        );
        Self { pipeline, buffer, count: 0, capacity: MAX_SOLID_QUADS }
    }

    pub fn prepare(&mut self, queue: &wgpu::Queue, mut instances: Vec<SolidInstance>) {
        if instances.len() > self.capacity {
            log::warn!("solid instance overflow: {} > {}", instances.len(), self.capacity);
            instances.truncate(self.capacity);
        }
        self.count = instances.len() as u32;
        if !instances.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&instances));
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, globals: &GlobalsBinding) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &globals.bind_group, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

struct ImageTexture {
    bind_group: wgpu::BindGroup,
}

pub struct ImageRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    capacity: usize,
    textures: HashMap<(u32, u64), ImageTexture>,
    batches: Vec<ImageBatch>,
    count: u32,
}

#[derive(Clone, Copy, Debug)]
struct ImageBatch {
    offset: u32,
    count: u32,
    key: (u32, u64),
}

pub struct ImageItem {
    pub key: (u32, u64),
    pub width: u32,
    pub height: u32,
    pub rgba: std::sync::Arc<[u8]>,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub alpha: f32,
}

impl ImageRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        globals: &GlobalsBinding,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pretty-image-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pretty-image-shader"),
            source: wgpu::ShaderSource::Wgsl(IMAGE_SHADER.into()),
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pretty-image-pipeline-layout"),
            bind_group_layouts: &[Some(&globals.layout), Some(&layout)],
            immediate_size: 0,
        });
        let attrs = [
            wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x4 },
            wgpu::VertexAttribute { offset: 16, shader_location: 1, format: wgpu::VertexFormat::Float32x4 },
            wgpu::VertexAttribute { offset: 32, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
        ];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ImageInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attrs,
        })];
        let targets = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pretty-image-pipeline"),
            layout: Some(&pipe_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let buffer = create_instance_buffer::<ImageInstance>(
            device,
            "pretty-image-instances",
            MAX_IMAGE_QUADS,
        );
        Self {
            pipeline,
            layout,
            buffer,
            capacity: MAX_IMAGE_QUADS,
            textures: HashMap::new(),
            batches: Vec::new(),
            count: 0,
        }
    }

    fn ensure_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sampler: &wgpu::Sampler,
        item: &ImageItem,
    ) {
        if self.textures.contains_key(&item.key) {
            return;
        }
        if item.width == 0 || item.height == 0 {
            return;
        }
        let size = wgpu::Extent3d {
            width: item.width,
            height: item.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pretty-kitty-image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pretty-kitty-image-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &item.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(item.width * 4),
                rows_per_image: Some(item.height),
            },
            size,
        );
        // Keep the texture alive through the bind group/view.
        self.textures.insert(item.key, ImageTexture { bind_group });
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[ImageItem],
    ) -> Result<(), RendererError> {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pretty-image-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mut instances = Vec::with_capacity(items.len());
        self.batches.clear();
        for item in items {
            self.ensure_texture(device, queue, &sampler, item);
            if !self.textures.contains_key(&item.key) {
                continue;
            }
            let inst = ImageInstance {
                rect: item.rect,
                color: [1.0, 1.0, 1.0, item.alpha],
                uv: item.uv,
            };
            let offset = instances.len() as u32;
            let last = self.batches.last_mut();
            if let Some(batch) = last.filter(|b| b.key == item.key) {
                batch.count += 1;
            } else {
                self.batches.push(ImageBatch { offset, count: 1, key: item.key });
            }
            instances.push(inst);
        }
        if instances.len() > self.capacity {
            log::warn!("image instance overflow");
            instances.truncate(self.capacity);
            self.batches.retain(|b| b.offset < self.capacity as u32);
            for b in &mut self.batches {
                b.count = b.count.min(self.capacity as u32 - b.offset);
            }
        }
        self.count = instances.len() as u32;
        if !instances.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&instances));
        }
        Ok(())
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, globals: &GlobalsBinding) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &globals.bind_group, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        for batch in &self.batches {
            if batch.count == 0 {
                continue;
            }
            let Some(tex) = self.textures.get(&batch.key) else { continue };
            pass.set_bind_group(1, &tex.bind_group, &[]);
            pass.draw(0..6, batch.offset..batch.offset + batch.count);
        }
    }
}

pub fn image_items_for_pane(
    images: &[FrameImage],
    origin: (f32, f32),
    cell_w: f32,
    cell_h: f32,
    padding: f32,
    display_offset: usize,
    content_clip: (f32, f32, f32, f32),
    under: bool,
) -> Vec<ImageItem> {
    let mut out = Vec::new();
    for image in images.iter() {
        if (image.z < 0) != under {
            continue;
        }
        let row = image.anchor_line + display_offset as i32;
        if row < 0 && image.rows.is_none() {
            continue;
        }
        let x = origin.0 + padding + image.anchor_col as f32 * cell_w + image.x_offset as f32;
        let y = origin.1 + padding + row as f32 * cell_h + image.y_offset as f32;
        let natural_w = image.image.width as f32;
        let natural_h = image.image.height as f32;
        let mut w = image.cols.filter(|c| *c > 0).map(|c| c as f32 * cell_w).unwrap_or(natural_w);
        let mut h = image.rows.filter(|r| *r > 0).map(|r| r as f32 * cell_h).unwrap_or(natural_h);
        if image.cols.is_none() && image.rows.is_some() {
            w = natural_w * (h / natural_h);
        }
        if image.rows.is_none() && image.cols.is_some() {
            h = natural_h * (w / natural_w);
        }
        let x0 = x.max(content_clip.0);
        let y0 = y.max(content_clip.1);
        let x1 = (x + w).min(content_clip.0 + content_clip.2);
        let y1 = (y + h).min(content_clip.1 + content_clip.3);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let u0 = (x0 - x) / w;
        let v0 = (y0 - y) / h;
        let u1 = (x1 - x) / w;
        let v1 = (y1 - y) / h;
        out.push(ImageItem {
            key: (image.image_id, image.generation),
            width: image.image.width,
            height: image.image.height,
            rgba: image.image.rgba.clone(),
            rect: [x0, y0, x1 - x0, y1 - y0],
            uv: [u0, v0, u1, v1],
            alpha: 1.0,
        });
    }
    out
}

fn create_instance_buffer<T>(device: &wgpu::Device, label: &str, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (capacity * std::mem::size_of::<T>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}



const SOLID_SHADER: &str = r#"
struct Globals { screen: vec2<f32>, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) color: vec4<f32> };
@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @location(0) rect: vec4<f32>, @location(1) color: vec4<f32>) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0));
    var out: VsOut;
    let c = corners[vi];
    let p = rect.xy + c * rect.zw;
    out.pos = vec4<f32>(p.x / globals.screen.x * 2.0 - 1.0, 1.0 - p.y / globals.screen.y * 2.0, 0.0, 1.0);
    out.color = color;
    return out;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb * in.color.a, in.color.a);
}
"#;

const IMAGE_SHADER: &str = r#"
struct Globals { screen: vec2<f32>, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var image_tex: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> };
@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @location(0) rect: vec4<f32>, @location(1) color: vec4<f32>, @location(2) uvr: vec4<f32>) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0));
    var out: VsOut;
    let c = corners[vi];
    let p = rect.xy + c * rect.zw;
    out.pos = vec4<f32>(p.x / globals.screen.x * 2.0 - 1.0, 1.0 - p.y / globals.screen.y * 2.0, 0.0, 1.0);
    out.uv = uvr.xy + c * (uvr.zw - uvr.xy);
    out.color = color;
    return out;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(image_tex, image_sampler, in.uv);
    let a = c.a * in.color.a;
    return vec4<f32>(c.rgb * a, a);
}
"#;
