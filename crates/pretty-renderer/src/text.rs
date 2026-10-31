//! Font discovery, shaping, glyph rasterization and GPU glyph atlases.
//!
//! JetBrains Mono is bundled and used for Latin/monospace text. Additional
//! system fonts are discovered through fontdb for CJK and symbols, while Noto
//! emoji fonts are bundled. Shaping is performed per style/font run with
//! rustybuzz, preserving `liga`/`calt`; glyphs are rasterized through swash into
//! R8 (mask) or RGBA (color) atlases.

use std::collections::HashMap;
use std::sync::Arc;

use pretty_core::Frame;
use swash::scale::{
    image::{Content, Image as SwashImage},
    Render, ScaleContext, Source, StrikeWith,
};
use swash::FontRef;
use crate::theme::Theme;

const JETBRAINS_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.otf");
const JETBRAINS_BOLD: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.otf");
const JETBRAINS_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Italic.otf");
const JETBRAINS_BOLD_ITALIC: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-BoldItalic.otf");
const NOTO_COLOR_EMOJI: &[u8] = include_bytes!("../../../assets/fonts/NotoColorEmoji.ttf");
const NOTO_EMOJI: &[u8] = include_bytes!("../../../assets/fonts/NotoEmoji-Regular.ttf");

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub cell_w: f32,
    pub cell_h: f32,
    pub baseline: f32,
    pub underline: f32,
    pub stroke: f32,
    pub padding: f32,
}

impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            cell_w: 9.0,
            cell_h: 18.0,
            baseline: 14.0,
            underline: 2.0,
            stroke: 1.0,
            padding: 10.0,
        }
    }
}

#[derive(Clone)]
struct FontFace {
    data: Arc<[u8]>,
    index: u32,
    family: String,
}

impl FontFace {
    fn ref_swash(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, self.index as usize)
    }
}

pub struct FontManager {
    faces: Vec<FontFace>,
    /// regular, bold, italic, bold-italic
    primary: [usize; 4],
    db: fontdb::Database,
    coverage: HashMap<(usize, char), bool>,
    cell_width_cache: HashMap<usize, f32>,
    fallback_cache: HashMap<char, Option<usize>>,
    loaded_system: HashMap<String, Option<usize>>,
    metrics: CellMetrics,
    font_size_px: f32,
    scale_factor: f32,
    features: Vec<rustybuzz::Feature>,
    ligatures: bool,
}

impl FontManager {
    pub fn new(theme: &Theme, scale_factor: f32, max_texture: u32) -> Result<Self, String> {
        let mut faces = Vec::new();
        for (data, name) in [
            (JETBRAINS_REGULAR, "JetBrains Mono"),
            (JETBRAINS_BOLD, "JetBrains Mono Bold"),
            (JETBRAINS_ITALIC, "JetBrains Mono Italic"),
            (JETBRAINS_BOLD_ITALIC, "JetBrains Mono Bold Italic"),
        ] {
            faces.push(FontFace {
                data: Arc::from(data),
                index: 0,
                family: name.to_owned(),
            });
        }
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        if theme.font_family != "JetBrains Mono" {
            let regular = load_system_family(&db, &theme.font_family, fontdb::Weight::NORMAL, fontdb::Style::Normal);
            if let Some(regular) = regular {
                let bold = load_system_family(&db, &theme.font_family, fontdb::Weight::BOLD, fontdb::Style::Normal)
                    .unwrap_or_else(|| regular.clone());
                let italic = load_system_family(&db, &theme.font_family, fontdb::Weight::NORMAL, fontdb::Style::Italic)
                    .unwrap_or_else(|| regular.clone());
                let bold_italic = load_system_family(&db, &theme.font_family, fontdb::Weight::BOLD, fontdb::Style::Italic)
                    .unwrap_or_else(|| bold.clone());
                faces = vec![regular, bold, italic, bold_italic];
                log::info!("using configured font family '{}'", theme.font_family);
            } else {
                log::warn!(
                    "font family '{}' not found; falling back to bundled JetBrains Mono",
                    theme.font_family
                );
            }
        }
        let mut fm = Self {
            faces,
            primary: [0, 1, 2, 3],
            db,
            coverage: HashMap::new(),
            cell_width_cache: HashMap::new(),
            fallback_cache: HashMap::new(),
            loaded_system: HashMap::new(),
            metrics: CellMetrics::default(),
            font_size_px: 0.0,
            scale_factor,
            features: Vec::new(),
            ligatures: theme.ligatures,
        };
        // Emoji fonts are bundled and take part in fallback. Color emoji first.
        for data in [NOTO_EMOJI, NOTO_COLOR_EMOJI] {
            fm.faces.push(FontFace {
                data: Arc::from(data),
                index: 0,
                family: "Noto Emoji".into(),
            });
        }
        // Preload plausible fallback families once. Missing families are simply
        // skipped; per-character lookup scans remaining system faces lazily.
        for family in [
            "Noto Sans Mono CJK SC",
            "Noto Sans CJK SC",
            "Noto Sans Mono",
            "Noto Sans",
            "DejaVu Sans",
            "Symbola",
        ] {
            if fm.loaded_system.contains_key(family) {
                continue;
            }
            let id = fm.db.query(&fontdb::Query {
                families: &[fontdb::Family::Name(family)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            });
            let loaded = id.and_then(|id| fm.load_face_id(id));
            fm.loaded_system.insert(family.to_owned(), loaded);
        }
        fm.set_size(theme, scale_factor, max_texture)?;
        Ok(fm)
    }

    fn load_face_id(&mut self, id: fontdb::ID) -> Option<usize> {
        let (data, index, family) = self.db.with_face_data(id, |data, index| {
            let family = self
                .db
                .face(id)
                .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
                .unwrap_or_else(|| "System font".into());
            (data.to_vec(), index, family)
        })?;
        self.faces.push(FontFace {
            data: Arc::from(data.into_boxed_slice()),
            index,
            family,
        });
        Some(self.faces.len() - 1)
    }

    pub fn metrics(&self) -> CellMetrics {
        self.metrics.clone()
    }

    pub fn family_name(&self) -> String {
        self.faces
            .get(self.primary[0])
            .map(|f| f.family.clone())
            .unwrap_or_else(|| "JetBrains Mono".into())
    }

    pub fn set_size(&mut self, theme: &Theme, scale_factor: f32, _max_texture: u32) -> Result<(), String> {
        self.scale_factor = scale_factor;
        self.font_size_px = (theme.font_size * scale_factor).max(4.0);
        self.ligatures = theme.ligatures;
        let face = self.faces.get(self.primary[0]).ok_or("missing primary font")?;
        let font = face.ref_swash().ok_or("failed to parse primary font")?;
        let m = font.metrics(&[]).scale(self.font_size_px);
        let gm = font.glyph_metrics(&[]).scale(self.font_size_px);
        let glyph = font.charmap().map('M');
        let advance = if glyph != 0 { gm.advance_width(glyph) } else { 0.0 };
        let cell_w = if advance > 0.0 {
            advance.round().max(1.0)
        } else {
            (self.font_size_px * 0.6).round().max(1.0)
        };
        let ascent = m.ascent.max(1.0);
        let descent = (-m.descent).max(1.0);
        let line_height = (ascent + descent + m.leading.max(0.0)) * theme.line_height.max(0.8);
        let cell_h = line_height.round().max(1.0);
        let baseline = ((cell_h - (ascent + descent)) / 2.0 + ascent).round();
        self.metrics = CellMetrics {
            cell_w,
            cell_h,
            baseline,
            underline: (m.underline_offset.abs().max(1.0)).round(),
            stroke: m.stroke_size.max(1.0).round(),
            padding: (theme.padding * scale_factor).round(),
        };
        self.cell_width_cache.clear();
        self.coverage.clear();
        self.features = parse_features(&theme.font_features, theme.ligatures);
        Ok(())
    }

    fn face_for(&mut self, c: char, bold: bool, italic: bool) -> usize {
        let primary = self.primary[(bold as usize) | ((italic as usize) << 1)];
        if self.has_glyph(primary, c) {
            return primary;
        }
        if let Some(cached) = self.fallback_cache.get(&c) {
            return cached.unwrap_or(primary);
        }
        let mut result = None;
        // Search preloaded (emoji + CJK) and lazily loaded system faces.
        for idx in 0..self.faces.len() {
            if self.has_glyph(idx, c) {
                result = Some(idx);
                break;
            }
        }
        if result.is_none() {
            let ids: Vec<fontdb::ID> = self.db.faces().map(|f| f.id).collect();
            for id in ids {
                if self.faces.len() > 512 {
                    break;
                }
                let Some(loaded) = self.load_face_id(id) else { continue };
                if self.has_glyph(loaded, c) {
                    result = Some(loaded);
                    break;
                }
            }
        }
        // Prefer a color emoji glyph if the codepoint is an emoji and the
        // primary font only has a monochrome fallback. Noto Color Emoji is
        // face index 4 and has the broadest coverage.
        if is_emoji(c) && self.faces.len() > 4 && self.has_glyph(4, c) {
            result = Some(4);
        }
        self.fallback_cache.insert(c, result);
        result.unwrap_or(primary)
    }

    fn has_glyph(&mut self, face_idx: usize, c: char) -> bool {
        if let Some(v) = self.coverage.get(&(face_idx, c)) {
            return *v;
        }
        let has = self
            .faces
            .get(face_idx)
            .and_then(|f| f.ref_swash())
            .map(|font| font.charmap().map(c) != 0)
            .unwrap_or(false);
        self.coverage.insert((face_idx, c), has);
        has
    }


    pub fn shape(&self, font_idx: usize, text: &[char], clusters: &[u32]) -> Vec<ShapedGlyph> {
        let Some(face) = self.faces.get(font_idx) else { return Vec::new() };
        let Some(rb) = rustybuzz::Face::from_slice(&face.data, face.index) else {
            return Vec::new();
        };
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        for (ch, cluster) in text.iter().zip(clusters) {
            buffer.add(*ch, *cluster);
        }
        buffer.guess_segment_properties();
        let glyphs = rustybuzz::shape(&rb, &self.features, buffer);
        let scale = self.font_size_px / rb.units_per_em().max(1) as f32;
        let infos = glyphs.glyph_infos();
        let positions = glyphs.glyph_positions();
        let mut out = Vec::with_capacity(infos.len());
        for (info, pos) in infos.iter().zip(positions) {
            out.push(ShapedGlyph {
                glyph_id: info.glyph_id as u16,
                cluster: info.cluster,
                x: pos.x_offset as f32 * scale,
                y: -pos.y_offset as f32 * scale,
            });
        }
        out
    }
}

pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub cluster: u32,
    pub x: f32,
    pub y: f32,
}

fn load_system_family(
    db: &fontdb::Database,
    family: &str,
    weight: fontdb::Weight,
    style: fontdb::Style,
) -> Option<FontFace> {
    let id = db.query(&fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight,
        stretch: fontdb::Stretch::Normal,
        style,
    })?;
    let (data, index) = db.with_face_data(id, |data, index| (data.to_vec(), index))?;
    let name = db
        .face(id)
        .and_then(|face| face.families.first())
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| family.to_owned());
    Some(FontFace {
        data: Arc::from(data.into_boxed_slice()),
        index,
        family: name,
    })
}

fn parse_features(specs: &[String], ligatures: bool) -> Vec<rustybuzz::Feature> {
    let mut out = Vec::new();
    let mut has_liga = false;
    let mut has_calt = false;
    for spec in specs {
        if let Ok(feature) = spec.parse::<rustybuzz::Feature>() {
            let tag = feature.tag.to_bytes();
            if tag == *b"liga" {
                has_liga = true;
            }
            if tag == *b"calt" {
                has_calt = true;
            }
            out.push(feature);
        } else {
            log::warn!("ignoring invalid OpenType feature '{spec}'");
        }
    }
    if !ligatures {
        out.push(rustybuzz::Feature::new(
            rustybuzz::ttf_parser::Tag::from_bytes(b"liga"),
            0,
            ..,
        ));
        out.push(rustybuzz::Feature::new(
            rustybuzz::ttf_parser::Tag::from_bytes(b"calt"),
            0,
            ..,
        ));
    } else {
        if !has_liga {
            out.push(rustybuzz::Feature::new(
                rustybuzz::ttf_parser::Tag::from_bytes(b"liga"),
                1,
                ..,
            ));
        }
        if !has_calt {
            out.push(rustybuzz::Feature::new(
                rustybuzz::ttf_parser::Tag::from_bytes(b"calt"),
                1,
                ..,
            ));
        }
    }
    out
}

fn is_emoji(c: char) -> bool {
    let cp = c as u32;
    (0x1F000..=0x1FAFF).contains(&cp)
        || (0x2600..=0x27BF).contains(&cp)
        || (0x1F900..=0x1F9FF).contains(&cp)
}

#[derive(Clone, Copy, Debug)]
pub struct RasterGlyph {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub color: bool,
}

#[derive(Clone, Debug)]
pub struct AtlasGlyph {
    pub uv: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub color: bool,
}

struct Atlas {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    color: bool,
    dirty: bool,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
}

impl Atlas {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        width: u32,
        height: u32,
        color: bool,
    ) -> Self {
        let format = if color {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::R8Unorm
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(if color { "pretty-color-atlas" } else { "pretty-mono-atlas" }),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pretty-atlas-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pretty-atlas-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let bpp = if color { 4 } else { 1 };
        Self {
            texture,
            bind_group,
            width,
            height,
            pixels: vec![0u8; (width * height * bpp) as usize],
            color,
            dirty: false,
            shelf_x: 1,
            shelf_y: 1,
            shelf_h: 0,
        }
    }

    fn bpp(&self) -> u32 {
        if self.color { 4 } else { 1 }
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let pad = 1;
        let w = w + pad * 2;
        let h = h + pad * 2;
        if w + 2 > self.width {
            return None;
        }
        if self.shelf_x + w + 1 > self.width {
            self.shelf_x = 1;
            self.shelf_y += self.shelf_h + 1;
            self.shelf_h = 0;
        }
        if self.shelf_y + h + 1 > self.height {
            return None;
        }
        let (x, y) = (self.shelf_x, self.shelf_y);
        self.shelf_x += w;
        self.shelf_h = self.shelf_h.max(h);
        Some((x + pad, y + pad))
    }

    fn upload_r8(&mut self, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        if !self.color {
            for row in 0..h {
                let dst = ((y + row) * self.width + x) as usize;
                let src = (row * w) as usize;
                let len = w as usize;
                if dst + len <= self.pixels.len() && src + len <= data.len() {
                    self.pixels[dst..dst + len].copy_from_slice(&data[src..src + len]);
                }
            }
        } else {
            for row in 0..h {
                for col in 0..w {
                    let src = ((row * w + col) * 4) as usize;
                    let dst = (((y + row) * self.width + x + col) * 4) as usize;
                    if src + 4 <= data.len() && dst + 4 <= self.pixels.len() {
                        self.pixels[dst..dst + 4].copy_from_slice(&data[src..src + 4]);
                    }
                }
            }
        }
        self.dirty = true;
    }

    fn upload_rgba(&mut self, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        for row in 0..h {
            for col in 0..w {
                let src = ((row * w + col) * 4) as usize;
                let dst = (((y + row) * self.width + x + col) * 4) as usize;
                if src + 4 <= data.len() && dst + 4 <= self.pixels.len() {
                    self.pixels[dst..dst + 4].copy_from_slice(&data[src..src + 4]);
                }
            }
        }
        self.dirty = true;
    }

    fn flush(&mut self, queue: &wgpu::Queue) {
        if !self.dirty {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * self.bpp()),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        self.dirty = false;
    }
}

#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct GlyphInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub uv: [f32; 4],
}

pub struct TextRenderer {
    pub metrics: CellMetrics,
    pub font_family: String,
    font_size_px: f32,
    scale_factor: f32,
    fonts: FontManager,
    mono_atlas: Atlas,
    color_atlas: Atlas,
    glyph_cache: HashMap<(usize, u16), AtlasGlyph>,
    mono_pipeline: wgpu::RenderPipeline,
    color_pipeline: wgpu::RenderPipeline,
    mono_buffer: wgpu::Buffer,
    color_buffer: wgpu::Buffer,
    mono_capacity: usize,
    color_capacity: usize,
    mono_count: u32,
    color_count: u32,
    pending_mono: Vec<GlyphInstance>,
    pending_color: Vec<GlyphInstance>,
    atlas_layout: wgpu::BindGroupLayout,
    scale_context: ScaleContext,
    theme_size: f32,
    theme_family: String,
    theme_ligatures: bool,
}

impl TextRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        theme: &Theme,
        scale_factor: f32,
        body_layout: &wgpu::BindGroupLayout,
        max_texture: u32,
    ) -> Result<Self, String> {
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pretty-atlas-layout"),
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
        let fonts = FontManager::new(theme, scale_factor, max_texture)?;
        let metrics = fonts.metrics();
        let atlas_size = if max_texture >= 2048 { 2048 } else { 1024 };
        let color_size = if max_texture >= 1024 { 1024 } else { 512 };
        let mono_atlas = Atlas::new(device, &atlas_layout, atlas_size, atlas_size, false);
        let color_atlas = Atlas::new(device, &atlas_layout, color_size, color_size, true);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pretty-text-shader"),
            source: wgpu::ShaderSource::Wgsl(TEXT_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pretty-text-pipeline-layout"),
            bind_group_layouts: &[Some(body_layout), Some(&atlas_layout)],
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
            wgpu::VertexAttribute {
                offset: 32,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x4,
            },
        ];
        let layouts = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GlyphInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attrs,
        })];
        let targets = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let make_pipeline = |label: &str, fragment: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &layouts,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let mono_pipeline = make_pipeline("pretty-glyph-pipeline", "fs_glyph");
        let color_pipeline = make_pipeline("pretty-glyph-color-pipeline", "fs_color");
        let mono_buffer = create_quad_buffer(device, "pretty-glyph-buffer", 32768);
        let color_buffer = create_quad_buffer(device, "pretty-color-glyph-buffer", 8192);
        let _ = queue;
        Ok(Self {
            metrics,
            font_family: fonts.family_name(),
            font_size_px: theme.font_size * scale_factor,
            scale_factor,
            fonts,
            mono_atlas,
            color_atlas,
            glyph_cache: HashMap::new(),
            mono_pipeline,
            color_pipeline,
            mono_buffer,
            color_buffer,
            mono_capacity: 32768,
            color_capacity: 8192,
            mono_count: 0,
            color_count: 0,
            pending_mono: Vec::new(),
            pending_color: Vec::new(),
            atlas_layout,
            scale_context: ScaleContext::new(),
            theme_size: theme.font_size,
            theme_family: theme.font_family.clone(),
            theme_ligatures: theme.ligatures,
        })
    }

    pub fn needs_rebuild(&self, theme: &Theme, scale_factor: f32) -> bool {
        (theme.font_size - self.theme_size).abs() > 0.01
            || theme.font_family != self.theme_family
            || theme.ligatures != self.theme_ligatures
            || (scale_factor - self.scale_factor).abs() > 0.001
    }

    pub fn rebuild(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        theme: &Theme,
        scale_factor: f32,
        body_layout: &wgpu::BindGroupLayout,
        max_texture: u32,
    ) -> Result<(), String> {
        let new = Self::new(device, queue, format, theme, scale_factor, body_layout, max_texture)?;
        *self = new;
        Ok(())
    }

    pub fn cell_size(&self) -> (f32, f32) {
        (self.metrics.cell_w, self.metrics.cell_h)
    }

    pub fn atlas_bind_groups(&self) -> (&wgpu::BindGroup, &wgpu::BindGroup) {
        (&self.mono_atlas.bind_group, &self.color_atlas.bind_group)
    }

    /// Rasterize a glyph (or fetch the cached atlas entry). R8 glyphs go to the
    /// mono atlas, RGBA color glyphs to the color atlas.
    fn glyph(&mut self, font_idx: usize, glyph_id: u16) -> Option<AtlasGlyph> {
        if let Some(g) = self.glyph_cache.get(&(font_idx, glyph_id)) {
            return Some(g.clone());
        }
        let data = Arc::clone(&self.fonts.faces.get(font_idx)?.data);
        let index = self.fonts.faces[font_idx].index as usize;
        let font_ref = FontRef::from_index(&data, index)?;
        let mut scaler = self
            .scale_context
            .builder(font_ref)
            .size(self.font_size_px)
            .hint(true)
            .build();
        let sources = [
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
            Source::Bitmap(StrikeWith::BestFit),
        ];
        let render = Render::new(&sources);
        let mut image = SwashImage::new();
        if !render.render_into(&mut scaler, glyph_id, &mut image) {
            return None;
        }
        let w = image.placement.width;
        let h = image.placement.height;
        if w == 0 || h == 0 || image.data.is_empty() {
            return None;
        }
        let is_color = image.content == Content::Color;
        let atlas = if is_color { &mut self.color_atlas } else { &mut self.mono_atlas };
        if atlas.width < w + 4 || atlas.height < h + 4 {
            log::warn!("glyph {glyph_id} too large for atlas ({w}x{h})");
            return None;
        }
        let (x, y) = atlas.alloc(w, h)?;
        match image.content {
            Content::Mask => atlas.upload_r8(x, y, w, h, &image.data),
            Content::SubpixelMask => {
                let mut alpha = vec![0u8; (w * h) as usize];
                for i in 0..(w * h) as usize {
                    alpha[i] = image
                        .data
                        .get(i * 4..i * 4 + 4)
                        .map(|p| ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8)
                        .unwrap_or(0);
                }
                atlas.upload_r8(x, y, w, h, &alpha);
            }
            Content::Color => atlas.upload_rgba(x, y, w, h, &image.data),
        }
        let uv = [
            x as f32 / atlas.width as f32,
            y as f32 / atlas.height as f32,
            (x + w) as f32 / atlas.width as f32,
            (y + h) as f32 / atlas.height as f32,
        ];
        let glyph = AtlasGlyph {
            uv,
            width: w,
            height: h,
            left: image.placement.left,
            top: image.placement.top,
            color: is_color,
        };
        if self.glyph_cache.len() > 20000 {
            self.glyph_cache.clear();
        }
        self.glyph_cache.insert((font_idx, glyph_id), glyph.clone());
        Some(glyph)
    }

    /// Shape and lay out all visible panes. Returns the number of mono and
    /// color quads; instances are kept internally for `draw`.
    pub fn layout_panes(
        &mut self,
        panes: &[(pretty_core::Rect, &Frame)],
        theme: &Theme,
    ) -> (usize, usize) {
        let mut mono = Vec::new();
        let mut color = Vec::new();
        for (rect, frame) in panes {
            self.layout_one(frame, theme, (rect.x, rect.y), &mut mono, &mut color);
        }
        if mono.len() > self.mono_capacity {
            log::warn!("mono glyph instance overflow ({} > {})", mono.len(), self.mono_capacity);
            mono.truncate(self.mono_capacity);
        }
        if color.len() > self.color_capacity {
            log::warn!("color glyph instance overflow ({} > {})", color.len(), self.color_capacity);
            color.truncate(self.color_capacity);
        }
        self.mono_count = mono.len() as u32;
        self.color_count = color.len() as u32;
        self.pending_mono = mono;
        self.pending_color = color;
        (self.mono_count as usize, self.color_count as usize)
    }

    fn layout_one(
        &mut self,
        frame: &Frame,
        theme: &Theme,
        origin: (f32, f32),
        mono: &mut Vec<GlyphInstance>,
        color: &mut Vec<GlyphInstance>,
    ) {
        let cw = self.metrics.cell_w;
        let ch = self.metrics.cell_h;
        let padding = self.metrics.padding;
        let x0 = origin.0 + padding;
        let y0 = origin.1 + padding;
        let (cursor_row, cursor_col, cursor_shape) = frame
            .cursor
            .filter(|c| c.visible)
            .map(|c| (c.row, c.col, c.shape))
            .unwrap_or((usize::MAX, usize::MAX, pretty_core::CursorShape::Hidden));
        let cursor_block = matches!(
            cursor_shape,
            pretty_core::CursorShape::Block | pretty_core::CursorShape::HollowBlock
        );

        for row in 0..frame.rows {
            let mut col = 0;
            while col < frame.cols {
                let Some(cell) = frame.cell(row, col) else { break };
                if cell.wide_spacer {
                    col += 1;
                    continue;
                }
                let font_idx = self.fonts.face_for(cell.c, cell.attrs.bold, cell.attrs.italic);
                // Group a same-font, same-style run, excluding wide cells which
                // are shaped individually and fallback fonts which may have
                // different metrics.
                let mut end = col + 1;
                if !cell.wide && font_idx < 4 {
                    while end < frame.cols {
                        let Some(next) = frame.cell(row, end) else { break };
                        if next.wide_spacer || next.wide {
                            break;
                        }
                        if next.attrs.bold != cell.attrs.bold
                            || next.attrs.italic != cell.attrs.italic
                        {
                            break;
                        }
                        let nf = self
                            .fonts
                            .face_for(next.c, next.attrs.bold, next.attrs.italic);
                        if nf != font_idx {
                            break;
                        }
                        end += 1;
                    }
                }
                if cell.wide {
                    end = col + 1;
                }
                let mut chars = Vec::new();
                let mut clusters = Vec::new();
                for c in col..end {
                    if let Some(cell) = frame.cell(row, c) {
                        let mut first = true;
                        for ch in std::iter::once(cell.c).chain(cell.zero_width.iter().copied()) {
                            chars.push(ch);
                            clusters.push((c - col) as u32);
                            let _ = first;
                            first = false;
                        }
                    }
                }
                let shaped = self.fonts.shape(font_idx, &chars, &clusters);
                for glyph in shaped {
                    let cell_index = col + glyph.cluster as usize;
                    let Some(cell) = frame.cell(row, cell_index) else { continue };
                    if cell.c.is_whitespace() && cell.zero_width.is_empty() {
                        continue;
                    }
                    let cursor_here = cursor_block && row == cursor_row && cell_index == cursor_col;
                    let fg = theme.resolve(cell.fg, &cell.attrs, cursor_here, false);
                    let Some(raster) = self.glyph(font_idx, glyph.glyph_id) else {
                        continue;
                    };
                    let cell_x = x0 + (col + glyph.cluster as usize) as f32 * cw + glyph.x;
                    let baseline = y0 + row as f32 * ch + self.metrics.baseline + glyph.y;
                    let mut gw = raster.width as f32;
                    let mut gh = raster.height as f32;
                    let mut gx = cell_x + raster.left as f32;
                    let mut gy = baseline - raster.top as f32;
                    if raster.color {
                        let max_w = if cell.wide { cw * 2.0 } else { cw };
                        let max_h = ch;
                        let scale = (max_w / gw).min(max_h / gh).min(2.0);
                        if scale < 1.0 {
                            let old_w = gw;
                            let old_h = gh;
                            gw *= scale;
                            gh *= scale;
                            gx = x0 + (col as f32) * cw + (max_w - gw) / 2.0;
                            gy = y0 + (row as f32 + 1.0) * ch - gh;
                            let _ = old_w;
                            let _ = old_h;
                        }
                    }
                    let inst = GlyphInstance {
                        rect: [gx, gy, gw, gh],
                        color: if raster.color { [1.0, 1.0, 1.0, 1.0] } else { fg },
                        uv: raster.uv,
                    };
                    if raster.color {
                        color.push(inst);
                    } else {
                        mono.push(inst);
                    }
                }
                col = end.max(col + 1);
            }
        }
    }

    pub fn flush_textures(&mut self, queue: &wgpu::Queue) {
        self.mono_atlas.flush(queue);
        self.color_atlas.flush(queue);
        if !self.pending_mono.is_empty() {
            queue.write_buffer(
                &self.mono_buffer,
                0,
                bytemuck::cast_slice(&self.pending_mono),
            );
        }
        if !self.pending_color.is_empty() {
            queue.write_buffer(
                &self.color_buffer,
                0,
                bytemuck::cast_slice(&self.pending_color),
            );
        }
    }

    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        body: &wgpu::BindGroup,
    ) {
        if self.mono_count > 0 {
            pass.set_pipeline(&self.mono_pipeline);
            pass.set_bind_group(0, body, &[]);
            pass.set_bind_group(1, &self.mono_atlas.bind_group, &[]);
            pass.set_vertex_buffer(0, self.mono_buffer.slice(..));
            pass.draw(0..6, 0..self.mono_count);
        }
        if self.color_count > 0 {
            pass.set_pipeline(&self.color_pipeline);
            pass.set_bind_group(0, body, &[]);
            pass.set_bind_group(1, &self.color_atlas.bind_group, &[]);
            pass.set_vertex_buffer(0, self.color_buffer.slice(..));
            pass.draw(0..6, 0..self.color_count);
        }
    }

    pub fn _atlas_layout(&self) -> &wgpu::BindGroupLayout {
        &self.atlas_layout
    }
}

fn create_quad_buffer(device: &wgpu::Device, label: &str, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (capacity * std::mem::size_of::<GlyphInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}


const TEXT_SHADER: &str = r#"
struct Globals {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

var<private> QUAD: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
);

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uvr: vec4<f32>,
) -> VsOut {
    var out: VsOut;
    let c = QUAD[vi];
    let p = rect.xy + c * rect.zw;
    let ndc = vec2<f32>(p.x / globals.screen.x * 2.0 - 1.0, 1.0 - p.y / globals.screen.y * 2.0);
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uvr.xy + c * (uvr.zw - uvr.xy);
    out.color = color;
    return out;
}

@fragment
fn fs_glyph(in: VsOut) -> @location(0) vec4<f32> {
    let mask = textureSample(atlas_tex, atlas_sampler, in.uv).r;
    let a = in.color.a * mask;
    return vec4<f32>(in.color.rgb * a, a);
}

@fragment
fn fs_color(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(atlas_tex, atlas_sampler, in.uv);
    let a = c.a * in.color.a;
    return vec4<f32>(c.rgb * a, a);
}
"#;
