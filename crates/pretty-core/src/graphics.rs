//! kitty graphics protocol (APC `G`).
//!
//! Implemented: PNG/RGB/RGBA transmission, chunked base64, transmit-only and
//! transmit+display, placement, deletion, queries, pixel/cell offsets,
//! cell-sized scaling, z-index, bounded resource use and protocol responses.
//! Not implemented (reported as protocol errors): zlib compression, file/SHM
//! transmission, animation frames.
//!
//! Resource policy (terminal output is untrusted):
//! - PNG dimensions are validated from the IHDR chunk *before* the decoder is
//!   allowed to allocate pixels, and the decoder itself runs with explicit
//!   width/height/allocation limits (defence against decompression bombs).
//! - The decoded-pixel, image-count, resident-byte and placement budgets are
//!   enforced on every insert, including retransmissions of an existing image
//!   ID: otherwise a client could slowly grow resident memory by re-sending a
//!   small ID with ever-larger payloads.
//! - Auto-assigned image IDs never overwrite an image the client addressed by an
//!   explicit ID (the two namespaces are kept separate).
//! - Errors are not written to the PTY when the client asked for `q=2`
//!   (suppress all responses); `q=1` suppresses successful responses only.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use base64::Engine;
use image::{ImageFormat, ImageReader};
use log::{debug, warn};

use crate::frame::SharedImage;

const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_IMAGES: usize = 256;
const MAX_IMAGE_BYTES: usize = 256 * 1024 * 1024;
const MAX_PLACEMENTS: usize = 1024;
/// Upper bound on the base64-decoded payload accumulated across chunks.
const MAX_PENDING_BYTES: usize = MAX_IMAGE_PIXELS as usize * 4 + 1024 * 1024;

/// Resource budgets for one [`GraphicsStore`]. Defaults are the constants above;
/// tests build stores with tiny budgets so the enforcement paths stay cheap.
#[derive(Clone, Copy, Debug)]
struct Limits {
    max_pixels: u64,
    max_images: usize,
    max_image_bytes: usize,
    max_placements: usize,
    max_pending_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_pixels: MAX_IMAGE_PIXELS,
            max_images: MAX_IMAGES,
            max_image_bytes: MAX_IMAGE_BYTES,
            max_placements: MAX_PLACEMENTS,
            max_pending_bytes: MAX_PENDING_BYTES,
        }
    }
}

#[derive(Clone)]
struct StoredImage {
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
    generation: u64,
}

#[derive(Clone, Debug)]
struct Placement {
    image_id: u32,
    placement_id: u32,
    /// Absolute grid line at placement time (negative lines are history).
    anchor_line: i32,
    anchor_col: usize,
    x_offset: i32,
    y_offset: i32,
    cols: Option<u32>,
    rows: Option<u32>,
    z: i32,
    generation: u64,
    auto_id: bool,
}

struct Pending {
    action: char,
    format: u32,
    id: u32,
    width: Option<u32>,
    height: Option<u32>,
    data: Vec<u8>,
    quiet: u8,
    control: Control,
    anchor_line: i32,
    anchor_col: usize,
    auto_id: bool,
}

#[derive(Default, Clone, Debug)]
struct Control {
    action: Option<char>,
    format: Option<u32>,
    id: Option<u32>,
    placement_id: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    cols: Option<u32>,
    rows: Option<u32>,
    x: i32,
    y: i32,
    z: i32,
    more: bool,
    quiet: u8,
    delete: Option<char>,
}

#[derive(Clone, Debug)]
pub struct GraphicsEvent {
    pub response: Option<String>,
}

/// Frame-facing image placement.
#[derive(Clone, Debug)]
pub struct FrameImage {
    pub image_id: u32,
    pub placement_id: u32,
    pub anchor_line: i32,
    pub anchor_col: usize,
    pub x_offset: i32,
    pub y_offset: i32,
    pub cols: Option<u32>,
    pub rows: Option<u32>,
    pub z: i32,
    pub image: SharedImage,
    pub generation: u64,
}

pub struct GraphicsStore {
    images: HashMap<u32, StoredImage>,
    placements: Vec<Placement>,
    next_id: u32,
    next_placement_id: u32,
    generation: u64,
    pending: Option<Pending>,
    limits: Limits,
    /// Number of image commands processed; used for diagnostics/tests.
    pub commands: u64,
}

impl Default for GraphicsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphicsStore {
    pub fn new() -> Self {
        Self::with_limits(Limits::default())
    }

    fn with_limits(limits: Limits) -> Self {
        Self {
            images: HashMap::new(),
            placements: Vec::new(),
            next_id: 1,
            next_placement_id: 1,
            generation: 1,
            pending: None,
            limits,
            commands: 0,
        }
    }

    pub fn clear(&mut self) {
        self.images.clear();
        self.placements.clear();
        self.pending = None;
        self.generation += 1;
    }

    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    pub fn placement_count(&self) -> usize {
        self.placements.len()
    }

    /// Bytes currently held by decoded image pixel buffers.
    pub fn resident_bytes(&self) -> usize {
        self.images.values().map(|image| image.rgba.len()).sum()
    }

    /// Handle a kitty graphics APC payload (the bytes after `ESC _ G`).
    ///
    /// `cursor_line`/`cursor_col` are the cursor's grid coordinates at command
    /// time (the same coordinate space the renderer sees at `display_offset=0`;
    /// the renderer adds the current display offset when drawing).
    pub fn handle(
        &mut self,
        payload: &[u8],
        cursor_row: usize,
        cursor_col: usize,
    ) -> GraphicsEvent {
        self.commands += 1;
        let Ok(text) = std::str::from_utf8(payload) else {
            return self.error("invalid UTF-8 in graphics command", None, 0);
        };
        let (control_text, data_text) = match text.split_once(';') {
            Some((c, d)) => (c, d),
            None => (text, ""),
        };
        let control = match parse_control(control_text) {
            Ok(c) => c,
            Err(e) => return self.error(&e, None, 0),
        };
        if log::log_enabled!(log::Level::Debug) {
            debug!(
                "kitty graphics command action={:?} id={:?} placement={:?} quiet={} more={} bytes={}",
                control.action,
                control.id,
                control.placement_id,
                control.quiet,
                control.more,
                data_text.len()
            );
        }
        // The anchor is a viewport-independent grid line: the renderer maps it
        // to a screen row by adding the live display offset. Do not bake the
        // current scroll offset into it, or every image written while the user
        // is scrolled up would be permanently displaced.
        let anchor_line = cursor_row as i32;
        let anchor_col = cursor_col;

        // Continuation chunks are allowed to omit the action. kitty's chunked
        // transfers always have `m=1` on every non-final chunk.
        let is_continuation = self.pending.is_some() && control.action.is_none();
        if is_continuation {
            let Some(mut pending) = self.pending.take() else {
                return self.error("internal chunk state error", None, 0);
            };
            let decoded = match decode_data(data_text) {
                Ok(d) => d,
                Err(e) => {
                    self.pending = None;
                    return self.error(&e, Some(pending.id), pending.quiet);
                }
            };
            pending.data.extend_from_slice(&decoded);
            if pending.data.len() > self.limits.max_pending_bytes {
                self.pending = None;
                return self.error("image payload too large", Some(pending.id), pending.quiet);
            }
            if control.more {
                self.pending = Some(pending);
                return GraphicsEvent { response: None };
            }
            return self.finish_transmission(pending);
        }

        // A new transmission while a chunked transfer is pending is an error.
        if self.pending.is_some() {
            self.pending = None;
            return self.error(
                "new transmission while chunked transfer pending",
                control.id,
                control.quiet,
            );
        }

        let action = control.action.unwrap_or('t');
        match action {
            't' | 'T' => {
                let format = control.format.unwrap_or(32);
                let (id, auto_id) = match control.id {
                    Some(id) if id > 0 => (id, false),
                    _ => (self.alloc_image_id(), true),
                };
                let decoded = match decode_data(data_text) {
                    Ok(d) => d,
                    Err(e) => return self.error(&e, Some(id), control.quiet),
                };
                let pending = Pending {
                    action,
                    format,
                    id,
                    width: control.width,
                    height: control.height,
                    data: decoded,
                    quiet: control.quiet,
                    anchor_line,
                    anchor_col,
                    auto_id,
                    control,
                };
                if pending.data.len() > self.limits.max_pending_bytes {
                    return self.error("image payload too large", Some(id), pending.quiet);
                }
                if pending.control.more {
                    self.pending = Some(pending);
                    return GraphicsEvent { response: None };
                }
                self.finish_transmission(pending)
            }
            'p' => self.place(&control, anchor_line, anchor_col),
            'd' => self.delete(&control),
            'q' => self.query(&control),
            'f' => self.error(
                "animation frames are not implemented",
                control.id,
                control.quiet,
            ),
            'a' => self.error(
                "animation control is not implemented",
                control.id,
                control.quiet,
            ),
            other => self.error(
                &format!("unsupported graphics action '{other}'"),
                control.id,
                control.quiet,
            ),
        }
    }

    fn finish_transmission(&mut self, pending: Pending) -> GraphicsEvent {
        let id = pending.id;
        let decoded = match decode_payload(
            pending.format,
            &pending.data,
            pending.width,
            pending.height,
            self.limits.max_pixels,
        ) {
            Ok(v) => v,
            Err(e) => return self.error(&e, Some(id), pending.quiet),
        };
        let (width, height, rgba) = decoded;
        if width == 0 || height == 0 {
            return self.error("zero-sized image", Some(id), pending.quiet);
        }
        if width as u64 * height as u64 > self.limits.max_pixels {
            return self.error(
                &format!(
                    "image exceeds {} megapixel limit",
                    self.limits.max_pixels / (1024 * 1024)
                ),
                Some(id),
                pending.quiet,
            );
        }
        let image_bytes = rgba.len();
        if image_bytes > self.limits.max_image_bytes {
            return self.error("image exceeds resident byte budget", Some(id), pending.quiet);
        }
        // Animation clients (mpv's kitty VO is one) commonly allocate a fresh
        // image ID per frame and do not delete the previous placement. Keep the
        // newest frames and bound both the image count and resident bytes so a
        // long video cannot exhaust memory or choke the renderer.
        //
        // This runs even when the ID already exists: re-sending a known ID with
        // a much larger payload must not bypass the resident-byte budget.
        let generation = self.bump_generation();
        self.evict_until(
            self.limits.max_images.saturating_sub(1),
            self.limits.max_image_bytes.saturating_sub(image_bytes),
            Some(id),
        );
        self.images.insert(
            id,
            StoredImage {
                width,
                height,
                rgba: Arc::from(rgba.into_boxed_slice()),
                generation,
            },
        );
        if pending.action == 'T' {
            let p = Placement {
                image_id: id,
                placement_id: self.alloc_placement_id(pending.control.placement_id),
                anchor_line: pending.anchor_line,
                anchor_col: pending.anchor_col,
                x_offset: pending.control.x,
                y_offset: pending.control.y,
                cols: pending.control.cols,
                rows: pending.control.rows,
                z: pending.control.z,
                generation,
                auto_id: pending.auto_id,
            };
            debug!(
                "kitty graphics: image {id} anchored line {} col {} z={} cols={:?} rows={:?} auto={} opaque24={}",
                p.anchor_line, p.anchor_col, p.z, p.cols, p.rows, p.auto_id, pending.format == 24
            );
            // mpv and similar animation clients use auto-assigned image ids
            // with an opaque RGB frame at the same cell for every frame and
            // quiet=2 (so they cannot delete by id). Treat such a transmission
            // as replacing the previous opaque auto-id frame at that exact
            // anchor instead of stacking hundreds of full-screen images.
            if p.auto_id && pending.format == 24 {
                let mut removed_images = Vec::new();
                self.placements.retain(|q| {
                    let same_anchor = q.auto_id
                        && q.image_id != id
                        && q.anchor_line == p.anchor_line
                        && q.anchor_col == p.anchor_col
                        && q.z == p.z
                        && q.x_offset == p.x_offset
                        && q.y_offset == p.y_offset
                        && q.cols == p.cols
                        && q.rows == p.rows;
                    if same_anchor {
                        removed_images.push(q.image_id);
                    }
                    !same_anchor
                });
                for old in removed_images {
                    if !self.placements.iter().any(|q| q.image_id == old) {
                        self.images.remove(&old);
                    }
                }
            }
            self.add_placement(p);
        }
        let response = (pending.quiet == 0).then(|| format!("\x1b_Gi={id};OK\x1b\\"));
        debug!(
            "kitty graphics: stored image {id} ({}x{}) format {}, chunked={}",
            width,
            height,
            pending.format,
            pending.control.more
        );
        GraphicsEvent { response }
    }

    fn place(&mut self, control: &Control, anchor_line: i32, anchor_col: usize) -> GraphicsEvent {
        let Some(image_id) = control.id else {
            return self.error("placement requires image id (i)", None, control.quiet);
        };
        if !self.images.contains_key(&image_id) {
            return self.error("image not found", Some(image_id), control.quiet);
        }
        let generation = self.bump_generation();
        let p = Placement {
            image_id,
            placement_id: self.alloc_placement_id(control.placement_id),
            anchor_line,
            anchor_col,
            x_offset: control.x,
            y_offset: control.y,
            cols: control.cols,
            rows: control.rows,
            z: control.z,
            generation,
            auto_id: false,
        };
        self.add_placement(p);
        GraphicsEvent {
            response: (control.quiet == 0).then(|| format!("\x1b_Gi={image_id};OK\x1b\\")),
        }
    }

    fn add_placement(&mut self, p: Placement) {
        if self.placements.len() >= self.limits.max_placements {
            // Drop the oldest placement and any image that becomes unreferenced.
            let old = self.placements.remove(0);
            if !self.placements.iter().any(|q| q.image_id == old.image_id) {
                self.images.remove(&old.image_id);
            }
        }
        self.placements.push(p);
    }

    /// Evict least-recently-created images until both limits fit, preserving
    /// `keep` (the image currently being stored). This is intentionally a
    /// resource guard, not part of normal protocol semantics: clients that need
    /// deterministic lifetimes should use kitty's delete commands.
    fn evict_until(&mut self, max_count: usize, max_bytes: usize, keep: Option<u32>) {
        let mut bytes: usize = self.images.values().map(|im| im.rgba.len()).sum();
        while self.images.len() > max_count || bytes > max_bytes {
            let victim = self
                .images
                .iter()
                .filter(|(id, _)| Some(**id) != keep)
                .min_by_key(|(_, im)| im.generation)
                .map(|(id, _)| *id);
            let Some(victim) = victim else { break };
            if let Some(im) = self.images.remove(&victim) {
                bytes = bytes.saturating_sub(im.rgba.len());
                self.placements.retain(|p| p.image_id != victim);
                debug!("kitty graphics: evicted image {victim} to stay within resource budget");
            }
        }
    }

    fn delete(&mut self, control: &Control) -> GraphicsEvent {
        let mode = control.delete.unwrap_or('a');
        match mode.to_ascii_lowercase() {
            'a' => {
                self.images.clear();
                self.placements.clear();
            }
            'i' => {
                let Some(id) = control.id else {
                    return self.error("delete requires image id", None, control.quiet);
                };
                self.images.remove(&id);
                self.placements.retain(|p| p.image_id != id);
            }
            'p' => {
                let Some(pid) = control.placement_id else {
                    return self.error(
                        "delete placement requires p",
                        control.id,
                        control.quiet,
                    );
                };
                self.placements.retain(|p| p.placement_id != pid);
            }
            'z' => {
                let z = control.z;
                self.placements.retain(|p| p.z != z);
            }
            other => {
                return self.error(
                    &format!("unsupported delete mode '{other}'"),
                    control.id,
                    control.quiet,
                )
            }
        }
        debug!("kitty graphics delete mode {mode:?} id={:?} placement={:?}", control.id, control.placement_id);
        self.generation += 1;
        GraphicsEvent {
            response: (control.quiet == 0).then(|| "\x1b_G;OK\x1b\\".to_owned()),
        }
    }

    fn query(&mut self, control: &Control) -> GraphicsEvent {
        if let Some(id) = control.id {
            if self.images.contains_key(&id) {
                GraphicsEvent {
                    response: (control.quiet == 0)
                        .then(|| format!("\x1b_Gi={id};OK\x1b\\")),
                }
            } else {
                self.error("image not found", Some(id), control.quiet)
            }
        } else {
            GraphicsEvent {
                response: (control.quiet == 0).then(|| {
                    format!(
                        "\x1b_Gi={};OK\x1b\\",
                        self.images.keys().next().copied().unwrap_or(0)
                    )
                }),
            }
        }
    }

    fn error(&self, msg: &str, id: Option<u32>, quiet: u8) -> GraphicsEvent {
        if quiet == 2 {
            // kitty: q=2 suppresses every response, including errors.
            debug!("kitty graphics error suppressed by q=2: {msg}");
            return GraphicsEvent { response: None };
        }
        let clean: String = msg
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let idpart = id.map(|i| format!("i={i};")).unwrap_or_default();
        warn!("kitty graphics error: {clean}");
        GraphicsEvent {
            response: Some(format!("\x1b_G{idpart}EINVAL:{clean}\x1b\\")),
        }
    }

    /// Allocate an auto image ID in kitty's "image number" namespace without
    /// overwriting any image the client addressed by an explicit numeric ID.
    fn alloc_image_id(&mut self) -> u32 {
        let attempts = self.images.len().saturating_add(1);
        for _ in 0..=attempts {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if !self.images.contains_key(&id) {
                return id;
            }
        }
        // Pathological state (every candidate taken); fall back to the counter
        // value and let the normal eviction policy bound memory.
        self.next_id
    }

    fn alloc_placement_id(&mut self, requested: Option<u32>) -> u32 {
        if let Some(id) = requested.filter(|id| *id > 0) {
            id
        } else {
            let id = self.next_placement_id;
            self.next_placement_id = self.next_placement_id.wrapping_add(1).max(1);
            id
        }
    }

    fn bump_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// Images visible in the current viewport. The renderer clips them to the
    /// pane rectangle and sorts by z-index.
    pub fn frame_images(&self, _display_offset: usize) -> Vec<FrameImage> {
        let mut out = Vec::with_capacity(self.placements.len());
        for p in &self.placements {
            let Some(img) = self.images.get(&p.image_id) else { continue };
            out.push(FrameImage {
                image_id: p.image_id,
                placement_id: p.placement_id,
                anchor_line: p.anchor_line,
                anchor_col: p.anchor_col,
                x_offset: p.x_offset,
                y_offset: p.y_offset,
                cols: p.cols,
                rows: p.rows,
                z: p.z,
                image: SharedImage {
                    width: img.width,
                    height: img.height,
                    rgba: img.rgba.clone(),
                },
                // The anchor is a grid line; the renderer adds the live display
                // offset when drawing, so nothing is added here.
                generation: p.generation.max(img.generation),
            });
        }
        out
    }

    pub fn placement_ids(&self) -> HashSet<u32> {
        self.placements.iter().map(|p| p.placement_id).collect()
    }
}

fn parse_control(text: &str) -> Result<Control, String> {
    let mut c = Control::default();
    if text.is_empty() {
        return Ok(c);
    }
    for part in text.split(',') {
        if part.is_empty() {
            continue;
        }
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("malformed graphics control '{part}'"))?;
        let mut chars = key.chars();
        let k = chars.next().ok_or("empty graphics key")?;
        if chars.next().is_some() {
            return Err(format!("multi-character graphics key '{key}'"));
        }
        match k {
            'a' => c.action = value.chars().next(),
            'f' => c.format = Some(parse_u32(value, "f")?),
            'i' => c.id = Some(parse_u32(value, "i")?),
            'p' => c.placement_id = Some(parse_u32(value, "p")?),
            's' => c.width = Some(parse_u32(value, "s")?),
            'v' => c.height = Some(parse_u32(value, "v")?),
            'c' => c.cols = Some(parse_u32(value, "c")?),
            'r' => c.rows = Some(parse_u32(value, "r")?),
            'x' => c.x = parse_i32(value, "x")?,
            'y' => c.y = parse_i32(value, "y")?,
            'z' => c.z = parse_i32(value, "z")?,
            'm' => c.more = parse_u32(value, "m")? != 0,
            // kitty defines q=0 (all responses), q=1 (errors only) and q=2
            // (nothing); treat larger values like 2.
            'q' => {
                c.quiet = match parse_u32(value, "q")? {
                    0 => 0,
                    1 => 1,
                    _ => 2,
                }
            }
            'd' => c.delete = value.chars().next(),
            'o' => {
                if value == "z" {
                    return Err("zlib graphics compression is not implemented".into());
                }
            }
            // Keys that are accepted but not acted on yet: source rect (X/Y/w/h),
            // image number (I), virtual placements (U), etc.
            'I' | 'X' | 'Y' | 'w' | 'h' | 'U' | 'C' | 'S' | 't' => {}
            other => return Err(format!("unsupported graphics control key '{other}'")),
        }
    }
    Ok(c)
}

fn parse_u32(value: &str, key: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("invalid numeric value for {key}: '{value}'"))
}

fn parse_i32(value: &str, key: &str) -> Result<i32, String> {
    value
        .parse()
        .map_err(|_| format!("invalid numeric value for {key}: '{value}'"))
}

fn decode_data(data: &str) -> Result<Vec<u8>, String> {
    let engine = base64::engine::general_purpose::STANDARD;
    engine
        .decode(data.as_bytes())
        .map_err(|e| format!("invalid base64 payload: {e}"))
}

/// Dimensions declared in a PNG IHDR chunk, if `data` starts like a PNG.
///
/// Reading this before decoding is what makes PNG decompression bombs
/// impossible: the IHDR declares the full decoded size, so an image that would
/// exceed the pixel budget can be rejected without allocating it.
fn png_ihdr_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if data.len() < 24 || data[..8] != SIGNATURE || &data[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(data[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(data[20..24].try_into().ok()?);
    Some((width, height))
}

fn decode_png(
    data: &[u8],
    width: Option<u32>,
    height: Option<u32>,
    max_pixels: u64,
) -> Result<(u32, u32, Vec<u8>), String> {
    if let Some((w, h)) = png_ihdr_dimensions(data) {
        if w as u64 * h as u64 > max_pixels {
            return Err(format!(
                "image exceeds {} megapixel limit",
                max_pixels / (1024 * 1024)
            ));
        }
    }
    let mut reader = ImageReader::new(std::io::Cursor::new(data));
    reader.set_format(ImageFormat::Png);
    let mut limits = image::Limits::default();
    let dim_limit = max_pixels.min(u32::MAX as u64) as u32;
    limits.max_image_width = Some(dim_limit);
    limits.max_image_height = Some(dim_limit);
    // Decoded output is at most 4 bytes per pixel; leave headroom for the
    // decoder's own row/scratch buffers.
    limits.max_alloc = Some(
        max_pixels
            .saturating_mul(8)
            .max(64 * 1024 * 1024)
            .min(MAX_IMAGE_BYTES as u64 * 2),
    );
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|e| format!("PNG decode failed: {e}"))?;
    let (w, h) = (img.width(), img.height());
    if w as u64 * h as u64 > max_pixels {
        return Err(format!(
            "image exceeds {} megapixel limit",
            max_pixels / (1024 * 1024)
        ));
    }
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if let Some(sw) = width {
        if sw != 0 && sw != w {
            return Err(format!("PNG width {w} does not match s={sw}"));
        }
    }
    if let Some(sh) = height {
        if sh != 0 && sh != h {
            return Err(format!("PNG height {h} does not match v={sh}"));
        }
    }
    Ok((w, h, rgba.into_raw()))
}

fn decode_payload(
    format: u32,
    data: &[u8],
    width: Option<u32>,
    height: Option<u32>,
    max_pixels: u64,
) -> Result<(u32, u32, Vec<u8>), String> {
    match format {
        100 => decode_png(data, width, height, max_pixels),
        24 => {
            let (w, h) = raw_dims(data, width, height, 3, max_pixels)?;
            let mut rgba = Vec::with_capacity((w as usize * h as usize) * 4);
            for px in data.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            Ok((w, h, rgba))
        }
        32 => {
            let (w, h) = raw_dims(data, width, height, 4, max_pixels)?;
            Ok((w, h, data.to_vec()))
        }
        other => Err(format!("unsupported graphics format f={other}")),
    }
}

fn raw_dims(
    data: &[u8],
    width: Option<u32>,
    height: Option<u32>,
    bpp: u32,
    max_pixels: u64,
) -> Result<(u32, u32), String> {
    let (Some(w), Some(h)) = (width, height) else {
        return Err("raw image requires s= and v= dimensions".into());
    };
    if w == 0 || h == 0 {
        return Err("raw image dimensions must be nonzero".into());
    }
    if w as u64 * h as u64 > max_pixels {
        return Err(format!(
            "image exceeds {} megapixel limit",
            max_pixels / (1024 * 1024)
        ));
    }
    let expected = w as u64 * h as u64 * bpp as u64;
    if data.len() as u64 != expected {
        return Err(format!(
            "raw image payload mismatch: expected {expected} bytes, got {}",
            data.len()
        ));
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small budgets so the enforcement paths are cheap to exercise.
    fn tiny_limits() -> Limits {
        Limits {
            max_pixels: 4096,
            max_images: 8,
            max_image_bytes: 800,
            max_placements: 16,
            max_pending_bytes: 64 * 1024,
        }
    }

    fn transmit_png(store: &mut GraphicsStore) -> GraphicsEvent {
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR4nGOsWvT9DAMewIRPcvgoAAD3iwLvLo6ZVwAAAABJRU5ErkJggg==";
        store.handle(format!("a=T,f=100,i=7;{b64}").as_bytes(), 2, 3)
    }

    fn transmit_raw(store: &mut GraphicsStore, id: Option<u32>, w: u32, h: u32, fill: u8) {
        let data = vec![fill; (w * h * 4) as usize];
        let b64 = base64::engine::general_purpose::STANDARD.encode(data);
        let id = id.map(|id| format!(",i={id}")).unwrap_or_default();
        let action = "a=T";
        store.handle(format!("{action},f=32,s={w},v={h}{id};{b64}").as_bytes(), 0, 0);
    }

    fn png_with_declared_size(w: u32, h: u32) -> Vec<u8> {
        let mut data = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        data.extend_from_slice(&13u32.to_be_bytes());
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&w.to_be_bytes());
        data.extend_from_slice(&h.to_be_bytes());
        data.extend_from_slice(&[8, 6, 0, 0, 0]);
        data.extend_from_slice(&0u32.to_be_bytes());
        // Deliberately malformed image data: the pre-decode size check must
        // reject the image before the decoder ever looks at it.
        data.extend_from_slice(b"not a real png");
        data
    }

    #[test]
    fn stores_and_places() {
        let mut s = GraphicsStore::new();
        let ev = transmit_png(&mut s);
        assert!(ev.response.as_deref().unwrap_or("").contains("OK"), "response: {:?}", ev.response);
        assert_eq!(s.image_count(), 1);
        assert_eq!(s.placement_count(), 1);
        let frames = s.frame_images(0);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].anchor_line, 2);
        assert_eq!(frames[0].anchor_col, 3);
    }

    #[test]
    fn chunked_rgba() {
        let mut s = GraphicsStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3, 255]);
        let (first, second) = b64.split_at(4);
        let ev = s.handle(format!("a=t,f=32,s=1,v=1,i=4,m=1;{first}").as_bytes(), 0, 0);
        assert!(ev.response.is_none());
        let ev = s.handle(format!("m=0;{second}").as_bytes(), 0, 0);
        assert!(ev.response.as_deref().unwrap_or("").contains("OK"), "response: {:?}", ev.response);
        assert_eq!(s.image_count(), 1);
    }

    #[test]
    fn rejects_bad_raw_size() {
        let mut s = GraphicsStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
        let ev = s.handle(format!("a=t,f=24,s=2,v=2;{b64}").as_bytes(), 0, 0);
        assert!(ev.response.unwrap().contains("EINVAL"));
        assert_eq!(s.image_count(), 0);
    }

    #[test]
    fn auto_opaque_frames_replace_at_the_same_anchor() {
        let mut s = GraphicsStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode([10u8, 20, 30]);
        let ev = s.handle(format!("a=T,f=24,s=1,v=1,q=2;{b64}").as_bytes(), 1, 0);
        assert!(ev.response.is_none());
        let first = s.frame_images(0)[0].image_id;
        let ev = s.handle(format!("a=T,f=24,s=1,v=1,q=2;{b64}").as_bytes(), 1, 0);
        assert!(ev.response.is_none());
        let frames = s.frame_images(0);
        assert_eq!(frames.len(), 1, "new opaque auto-id frame should replace the old one");
        assert_ne!(frames[0].image_id, first);
        assert_eq!(s.image_count(), 1);
    }

    #[test]
    fn explicit_image_ids_are_not_collapsed() {
        let mut s = GraphicsStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode([10u8, 20, 30]);
        s.handle(format!("a=T,f=24,s=1,v=1,i=10;{b64}").as_bytes(), 1, 0);
        s.handle(format!("a=T,f=24,s=1,v=1,i=11;{b64}").as_bytes(), 1, 0);
        assert_eq!(s.image_count(), 2);
        assert_eq!(s.placement_count(), 2);
    }

    #[test]
    fn auto_ids_do_not_overwrite_explicit_ids() {
        let mut s = GraphicsStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode([10u8, 20, 30]);
        s.handle(format!("a=T,f=24,s=1,v=1,i=1;{b64}").as_bytes(), 1, 0);
        assert_eq!(s.image_count(), 1);
        // The first auto-assigned id starts at 1, which is already taken by the
        // explicit image: it must skip ahead instead of replacing it.
        s.handle(format!("a=T,f=24,s=1,v=1;{b64}").as_bytes(), 1, 0);
        assert_eq!(s.image_count(), 2, "auto id must not overwrite explicit image 1");
        let ids: Vec<u32> = s.frame_images(0).iter().map(|f| f.image_id).collect();
        assert!(ids.contains(&1));
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn png_decompression_bomb_rejected_before_decode() {
        let mut s = GraphicsStore::new();
        // 20000x20000 = 400 megapixels: far beyond the 32 MP budget. The image
        // data is malformed, so this can only pass if the IHDR pre-check runs
        // before the decoder.
        let png = png_with_declared_size(20_000, 20_000);
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let ev = s.handle(format!("a=t,f=100;{b64}").as_bytes(), 0, 0);
        let response = ev.response.unwrap_or_default();
        assert!(response.contains("EINVAL"), "response: {response:?}");
        assert!(response.contains("megapixel"), "response: {response:?}");
        assert_eq!(s.image_count(), 0);
    }

    #[test]
    fn png_limits_are_enforced_for_tiny_stores() {
        let mut s = GraphicsStore::with_limits(tiny_limits());
        let png = png_with_declared_size(128, 128); // 16k pixels > 4096 budget
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let ev = s.handle(format!("a=t,f=100;{b64}").as_bytes(), 0, 0);
        assert!(ev.response.unwrap_or_default().contains("megapixel"));
        assert_eq!(s.image_count(), 0);
    }

    #[test]
    fn retransmitting_a_known_id_cannot_exceed_the_byte_budget() {
        let limits = Limits {
            max_pixels: 4096,
            max_images: 8,
            max_image_bytes: 800,
            max_placements: 16,
            max_pending_bytes: 64 * 1024,
        };
        let mut s = GraphicsStore::with_limits(limits);
        // Three small images: 4x4 RGBA = 64 bytes each, 192 bytes resident.
        for id in 1..=3 {
            transmit_raw(&mut s, Some(id), 4, 4, id as u8);
        }
        assert_eq!(s.resident_bytes(), 192);
        // Re-send image 2 with a payload that fits the byte budget on its own
        // but would push the store over it if the old contents of all images
        // were kept.
        transmit_raw(&mut s, Some(2), 7, 25, 9); // 7*25*4 = 700 bytes
        assert!(
            s.resident_bytes() <= 800,
            "resident bytes exceeded budget: {}",
            s.resident_bytes()
        );
        assert_eq!(s.image_count(), 1, "old images should be evicted");
        assert_eq!(s.resident_bytes(), 700);
    }

    #[test]
    fn resident_bytes_are_evicted_for_new_ids() {
        let mut s = GraphicsStore::with_limits(tiny_limits());
        for id in 1..=8 {
            transmit_raw(&mut s, Some(id), 4, 4, id as u8);
        }
        assert!(s.resident_bytes() <= 800);
        assert!(s.image_count() <= 8);
    }

    #[test]
    fn quiet_two_suppresses_error_responses() {
        let mut s = GraphicsStore::new();
        let ev = s.handle(b"a=d,d=x,q=2", 0, 0);
        assert!(ev.response.is_none(), "q=2 must suppress errors");
        let ev = s.handle(b"a=d,d=x", 0, 0);
        assert!(ev.response.unwrap_or_default().contains("EINVAL"));
    }

    #[test]
    fn quiet_one_suppresses_success_only() {
        let mut s = GraphicsStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode([10u8, 20, 30]);
        let ev = s.handle(format!("a=T,f=24,s=1,v=1,i=3,q=1;{b64}").as_bytes(), 0, 0);
        assert!(ev.response.is_none(), "q=1 must suppress OK responses");
    }

    #[test]
    fn delete_all() {
        let mut s = GraphicsStore::new();
        transmit_png(&mut s);
        let ev = s.handle(b"a=d,d=a", 0, 0);
        assert!(ev.response.unwrap().contains("OK"));
        assert_eq!(s.image_count(), 0);
        assert_eq!(s.placement_count(), 0);
    }

    #[test]
    fn anchor_does_not_depend_on_scroll_offset() {
        let mut s = GraphicsStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode([10u8, 20, 30]);
        // Cursor grid line 4, whatever the current scroll offset happens to be.
        s.handle(format!("a=T,f=24,s=1,v=1,i=5;{b64}").as_bytes(), 4, 2);
        let frames = s.frame_images(0);
        assert_eq!(frames[0].anchor_line, 4);
        assert_eq!(frames[0].anchor_col, 2);
    }
}
