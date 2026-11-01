//! High-level terminal renderer: cells, text, images, chrome and egui.

use std::sync::Arc;

use pretty_core::{
    ColorSpec, CursorShape, Frame, Rect, UnderlineStyle,
};
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::backend::RendererKind;
use crate::context::{GpuContext, RendererError};
use crate::gpu::{
    image_items_for_pane, GlobalsBinding, ImageRenderer, SolidInstance, SolidRenderer,
};
use crate::text::TextRenderer;
use crate::theme::{linear, Theme};

pub struct PaneDraw<'a> {
    pub rect: Rect,
    pub frame: &'a Frame,
    pub focused: bool,
}

pub struct UiPaint {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
    Retry,
}

pub struct Renderer {
    pub gpu: GpuContext,
    pub theme: Theme,
    pub scale_factor: f32,
    globals: GlobalsBinding,
    solid: SolidRenderer,
    solid_fg: SolidRenderer,
    images_under: ImageRenderer,
    images_over: ImageRenderer,
    text: TextRenderer,
    egui: egui_wgpu::Renderer,
    clear: wgpu::Color,
    frame_count: u64,
}

impl Renderer {
    pub fn new(
        window: Arc<Window>,
        display_handle: OwnedDisplayHandle,
        requested: RendererKind,
        theme: Theme,
        transparent: bool,
        vsync: bool,
        scale_factor: f32,
        allow_software: bool,
    ) -> Result<Self, RendererError> {
        let gpu = GpuContext::new(
            window,
            display_handle,
            requested,
            transparent,
            vsync,
            allow_software,
        )?;
        let mut theme = theme;
        if !gpu.info.transparent {
            // The compositor applies window opacity on X11; avoid applying the
            // configured opacity twice in the opaque swapchain.
            theme.opacity = 1.0;
        }
        let globals = GlobalsBinding::new(&gpu.device);
        let format = gpu.config.format;
        let text = TextRenderer::new(
            &gpu.device,
            &gpu.queue,
            format,
            &theme,
            scale_factor,
            &globals.layout,
            gpu.adapter.limits().max_texture_dimension_2d,
        )
        .map_err(RendererError::Font)?;
        let solid = SolidRenderer::new(&gpu.device, format, &globals);
        let solid_fg = SolidRenderer::new(&gpu.device, format, &globals);
        let images_under = ImageRenderer::new(&gpu.device, format, &globals);
        let images_over = ImageRenderer::new(&gpu.device, format, &globals);
        let egui = egui_wgpu::Renderer::new(
            &gpu.device,
            format,
            egui_wgpu::RendererOptions::default(),
        );
        let clear = if gpu.info.transparent {
            wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
        } else {
            let c = theme.background;
            wgpu::Color {
                r: (c.r as f64 / 255.0).powf(2.2),
                g: (c.g as f64 / 255.0).powf(2.2),
                b: (c.b as f64 / 255.0).powf(2.2),
                a: 1.0,
            }
        };
        log::info!(
            "renderer initialized: backend={:?} format={:?} alpha={:?} egui=on",
            gpu.info.wgpu_backend,
            format,
            gpu.info.alpha_mode
        );
        Ok(Self {
            gpu,
            theme,
            scale_factor,
            globals,
            solid,
            solid_fg,
            images_under,
            images_over,
            text,
            egui,
            clear,
            frame_count: 0,
        })
    }

    pub fn cell_metrics(&self) -> crate::text::CellMetrics {
        self.text.metrics.clone()
    }

    pub fn font_family(&self) -> &str {
        &self.text.font_family
    }

    pub fn set_theme(&mut self, theme: Theme) -> Result<(), RendererError> {
        let rebuild = self.text.needs_rebuild(&theme, self.scale_factor);
        let mut theme = theme;
        if !self.gpu.info.transparent {
            theme.opacity = 1.0;
        }
        let bg = theme.background;
        self.clear = if self.gpu.info.transparent {
            wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
        } else {
            wgpu::Color {
                r: (bg.r as f64 / 255.0).powf(2.2),
                g: (bg.g as f64 / 255.0).powf(2.2),
                b: (bg.b as f64 / 255.0).powf(2.2),
                a: 1.0,
            }
        };
        self.theme = theme;
        if rebuild {
            let t = self.theme.clone();
            let format = self.gpu.config.format;
            let max = self.gpu.adapter.limits().max_texture_dimension_2d;
            self.text.rebuild(
                &self.gpu.device,
                &self.gpu.queue,
                format,
                &t,
                self.scale_factor,
                &self.globals.layout,
                max,
            )
            .map_err(RendererError::Font)?;
            log::info!(
                "font rebuilt: {} @ {:.1}px, cell {}x{}",
                self.text.font_family,
                t.font_size * self.scale_factor,
                self.text.metrics.cell_w,
                self.text.metrics.cell_h
            );
        }
        Ok(())
    }

    pub fn set_scale_factor(&mut self, scale_factor: f32) -> Result<(), RendererError> {
        if (scale_factor - self.scale_factor).abs() < 0.001 {
            return Ok(());
        }
        self.scale_factor = scale_factor;
        let theme = self.theme.clone();
        self.set_theme(theme)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.reconfigure(width, height);
    }

    pub fn render(
        &mut self,
        panes: &[PaneDraw<'_>],
        mut ui: Option<UiPaint>,
    ) -> Result<RenderOutcome, RendererError> {
        let (width, height) = self.gpu.size();
        if width == 0 || height == 0 {
            return Ok(RenderOutcome::Skipped);
        }
        // Apply egui text/atlas uploads before acquiring the swapchain. This
        // guarantees a dropped UI frame cannot lose a required font atlas.
        if let Some(ui) = ui.as_mut() {
            for (id, deltas) in &ui.textures_delta.set {
                for delta in deltas {
                    self.egui
                        .update_texture(&self.gpu.device, &self.gpu.queue, *id, delta);
                }
            }
        }
        let skip_ui = |slf: &mut Self, ui: &mut Option<UiPaint>| {
            if let Some(ui) = ui {
                for id in &ui.textures_delta.free {
                    slf.egui.free_texture(id);
                }
                ui.textures_delta.clear();
            }
        };
        let surface_texture = match self.gpu.current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                log::warn!("surface texture is suboptimal; reconfiguring after this frame");
                t
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                skip_ui(self, &mut ui);
                return Ok(RenderOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                log::warn!("surface lost/outdated; reconfiguring swapchain");
                self.gpu.reconfigure(width, height);
                skip_ui(self, &mut ui);
                return Ok(RenderOutcome::Retry);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                skip_ui(self, &mut ui);
                return Err(RendererError::SurfaceLost(
                    "surface texture validation error".into(),
                ));
            }
        };

        self.globals.update(&self.gpu.queue, width, height);
        let (bg, fg) = self.build_chrome(panes);
        let pane_refs: Vec<(Rect, &Frame)> =
            panes.iter().map(|p| (p.rect, p.frame)).collect();
        self.text.layout_panes(&pane_refs, &self.theme);
        self.text.flush_textures(&self.gpu.queue);

        let mut under_items = Vec::new();
        let mut over_items = Vec::new();
        for pane in panes {
            let m = self.text.metrics;
            let inner = (
                pane.rect.x,
                pane.rect.y,
                pane.rect.w,
                pane.rect.h,
            );
            under_items.extend(image_items_for_pane(
                &pane.frame.images,
                (pane.rect.x, pane.rect.y),
                m.cell_w,
                m.cell_h,
                m.padding,
                pane.frame.display_offset,
                inner,
                true,
            ));
            over_items.extend(image_items_for_pane(
                &pane.frame.images,
                (pane.rect.x, pane.rect.y),
                m.cell_w,
                m.cell_h,
                m.padding,
                pane.frame.display_offset,
                inner,
                false,
            ));
        }
        self.images_under
            .prepare(&self.gpu.device, &self.gpu.queue, &under_items)?;
        self.images_over
            .prepare(&self.gpu.device, &self.gpu.queue, &over_items)?;
        self.solid.prepare(&self.gpu.queue, bg);
        self.solid_fg.prepare(&self.gpu.queue, fg);

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pretty-encoder") });
        let mut egui_callbacks = Vec::new();
        let mut egui_ui = ui;
        if let Some(ui) = egui_ui.as_mut() {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [width, height],
                pixels_per_point: ui.pixels_per_point.max(0.1),
            };
            egui_callbacks = self.egui.update_buffers(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                &ui.primitives,
                &screen,
            );
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pretty-main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.solid.draw(&mut pass, &self.globals);
            self.images_under.draw(&mut pass, &self.globals);
            self.text.draw(&mut pass, &self.globals.bind_group);
            self.images_over.draw(&mut pass, &self.globals);
            self.solid_fg.draw(&mut pass, &self.globals);
        }

        if let Some(ui) = egui_ui.as_ref() {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [width, height],
                pixels_per_point: ui.pixels_per_point.max(0.1),
            };
            // egui-wgpu requires a 'static render pass. The pass is dropped
            // before the encoder is finished, so no encoder operation can occur
            // while it is alive.
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pretty-egui-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui.render(&mut pass, &ui.primitives, &screen);
        }

        let mut command_buffers = egui_callbacks;
        command_buffers.push(encoder.finish());
        self.gpu.queue.submit(command_buffers);
        self.gpu.queue.present(surface_texture);
        if let Some(mut ui) = egui_ui {
            for id in &ui.textures_delta.free {
                self.egui.free_texture(id);
            }
            ui.textures_delta.clear();
        }
        self.frame_count = self.frame_count.wrapping_add(1);
        Ok(RenderOutcome::Presented)
    }

    fn build_chrome(&self, panes: &[PaneDraw<'_>]) -> (Vec<SolidInstance>, Vec<SolidInstance>) {
        let mut bg = Vec::new();
        let mut fg = Vec::new();
        let m = self.text.metrics;
        let t = &self.theme;
        for pane in panes {
            push_rect(
                &mut bg,
                pane.rect,
                linear(t.background, t.opacity.max(0.05).min(1.0)),
            );
            let (cursor_row, cursor_col, cursor_shape) = pane
                .frame
                .cursor
                .filter(|c| c.visible && !c.off)
                .map(|c| (c.row, c.col, c.shape))
                .unwrap_or((usize::MAX, usize::MAX, CursorShape::Hidden));
            let cursor_block = matches!(cursor_shape, CursorShape::Block | CursorShape::HollowBlock);
            let content_x = pane.rect.x + m.padding;
            let content_y = pane.rect.y + m.padding;
            for row in 0..pane.frame.rows {
                for col in 0..pane.frame.cols {
                    let Some(cell) = pane.frame.cell(row, col) else { continue };
                    if cell.wide_spacer {
                        continue;
                    }
                    let cursor_here = cursor_block && row == cursor_row && col == cursor_col;
                    let default_bg = matches!(cell.bg, ColorSpec::Background);
                    let cell_w = if cell.wide { m.cell_w * 2.0 } else { m.cell_w };
                    if !default_bg || cell.attrs.selected || cursor_here {
                        let color = t.resolve(cell.bg, &cell.attrs, cursor_here, true);
                        push_rect(
                            &mut bg,
                            Rect::new(
                                content_x + col as f32 * m.cell_w,
                                content_y + row as f32 * m.cell_h,
                                cell_w,
                                m.cell_h,
                            ),
                            color,
                        );
                    }
                    // Underline decorations.
                    let underline_color = cell
                        .attrs
                        .underline_color
                        .map(|color| t.resolve(color, &cell.attrs, cursor_here, false))
                        .unwrap_or_else(|| t.resolve(cell.fg, &cell.attrs, cursor_here, false));
                    let x = content_x + col as f32 * m.cell_w;
                    let y = content_y + row as f32 * m.cell_h + m.baseline + m.underline;
                    match cell.attrs.underline {
                        UnderlineStyle::Single => {
                            push_rect(&mut bg, Rect::new(x, y, cell_w, m.stroke), underline_color)
                        }
                        UnderlineStyle::Double => {
                            push_rect(&mut bg, Rect::new(x, y, cell_w, m.stroke), underline_color);
                            push_rect(
                                &mut bg,
                                Rect::new(x, y + m.stroke + 1.0, cell_w, m.stroke),
                                underline_color,
                            );
                        }
                        UnderlineStyle::Curly => {
                            // A low-cost wave approximation using small segments.
                            let mut offset = 0.0;
                            while offset < cell_w {
                                let dy = if ((offset / 3.0) as i32) % 2 == 0 { 0.0 } else { 2.0 };
                                push_rect(
                                    &mut bg,
                                    Rect::new(x + offset, y + dy, (m.cell_w / 3.0).min(cell_w - offset), m.stroke),
                                    underline_color,
                                );
                                offset += (m.cell_w / 3.0).max(1.0);
                            }
                        }
                        UnderlineStyle::Dotted | UnderlineStyle::Dashed => {
                            let step: f32 = if cell.attrs.underline == UnderlineStyle::Dotted {
                                3.0
                            } else {
                                6.0
                            };
                            let mut offset = 0.0;
                            while offset < cell_w {
                                push_rect(
                                    &mut bg,
                                    Rect::new(x + offset, y, (step / 2.0).min(cell_w - offset), m.stroke),
                                    underline_color,
                                );
                                offset += step;
                            }
                        }
                        UnderlineStyle::None => {}
                    }
                    if cell.attrs.hyperlink.is_some() && cell.attrs.underline == UnderlineStyle::None {
                        push_rect(
                            &mut bg,
                            Rect::new(x, y, cell_w, 1.0),
                            linear(t.accent, 0.7),
                        );
                    }
                    if cell.attrs.strikeout && !cell.wide_spacer {
                        let sy = content_y + row as f32 * m.cell_h + m.baseline * 0.62;
                        push_rect(&mut fg, Rect::new(x, sy, cell_w, m.stroke), underline_color);
                    }
                }
            }
            // Cursor variants that draw over text.
            if let Some(cursor) = pane.frame.cursor.filter(|c| c.visible) {
                let x = content_x + cursor.col as f32 * m.cell_w;
                let y = content_y + cursor.row as f32 * m.cell_h;
                let color = linear(t.cursor, if cursor.off { 0.30 } else { 0.9 });
                if cursor.off {
                    // Keep the cursor location legible during the blink-off
                    // phase instead of making it disappear entirely.
                    push_outline(&mut fg, Rect::new(x, y, m.cell_w, m.cell_h), 1.0, color);
                } else {
                    match cursor.shape {
                        CursorShape::Beam => {
                            push_rect(&mut fg, Rect::new(x, y, 2.0, m.cell_h), color)
                        }
                        CursorShape::Underline => {
                            push_rect(&mut fg, Rect::new(x, y + m.cell_h - 2.0, m.cell_w, 2.0), color)
                        }
                        CursorShape::HollowBlock => {
                            push_outline(&mut fg, Rect::new(x, y, m.cell_w, m.cell_h), 1.0, color)
                        }
                        _ => {}
                    }
                }
            }
            // Scrollbar.
            if pane.frame.history_size > 0 {
                let track_x = pane.rect.x + pane.rect.w - 6.0;
                let track_y = pane.rect.y + 4.0;
                let track_h = (pane.rect.h - 8.0).max(1.0);
                push_rect(
                    &mut bg,
                    Rect::new(track_x, track_y, 3.0, track_h),
                    linear(t.border, 0.25),
                );
                let total = (pane.frame.history_size + pane.frame.rows) as f32;
                let visible = pane.frame.rows as f32 / total;
                let thumb_h = (track_h * visible).max(16.0);
                let max_offset = pane.frame.history_size as f32;
                let frac = if max_offset > 0.0 {
                    1.0 - (pane.frame.display_offset as f32 / max_offset).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let thumb_y = track_y + (track_h - thumb_h) * frac;
                push_rect(
                    &mut bg,
                    Rect::new(track_x, thumb_y, 3.0, thumb_h),
                    linear(t.accent, 0.65),
                );
            }
            let border = if pane.focused {
                linear(t.accent, 0.8)
            } else {
                linear(t.border, 0.5)
            };
            push_outline(&mut fg, pane.rect.inset(0.5), 1.0, border);
        }
        (bg, fg)
    }
}

fn push_rect(out: &mut Vec<SolidInstance>, rect: Rect, color: [f32; 4]) {
    if rect.w <= 0.0 || rect.h <= 0.0 || color[3] <= 0.0 {
        return;
    }
    out.push(SolidInstance {
        rect: [rect.x, rect.y, rect.w, rect.h],
        color,
    });
}

fn push_outline(out: &mut Vec<SolidInstance>, rect: Rect, width: f32, color: [f32; 4]) {
    push_rect(out, Rect::new(rect.x, rect.y, rect.w, width), color);
    push_rect(out, Rect::new(rect.x, rect.y + rect.h - width, rect.w, width), color);
    push_rect(out, Rect::new(rect.x, rect.y, width, rect.h), color);
    push_rect(out, Rect::new(rect.x + rect.w - width, rect.y, width, rect.h), color);
}
