//! kitty graphics protocol (APC `G`).
//!
//! Implemented: PNG/RGB/RGBA transmission, chunked base64, transmit-only and
//! transmit+display, placement, deletion, queries, pixel/cell offsets,
//! cell-sized scaling, z-index, bounded resource use and protocol responses.
//! Not implemented (reported as protocol errors): zlib compression, file/SHM
//! transmission, animation frames.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use base64::Engine;
use image::ImageFormat;
use log::{debug, warn};

use crate::frame::SharedImage;

const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_IMAGES: usize = 256;
const MAX_PLACEMENTS: usize = 1024;

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
        Self {
            images: HashMap::new(),
            placements: Vec::new(),
            next_id: 1,
            next_placement_id: 1,
            generation: 1,
            pending: None,
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

    /// Handle a kitty graphics APC payload (the bytes after `ESC _ G`).
    ///
    /// `cursor_line` and `display_offset` are used to anchor images to terminal
    /// content rather than to the viewport.
    pub fn handle(
        &mut self,
        payload: &[u8],
        cursor_row: usize,
        cursor_col: usize,
        display_offset: usize,
    ) -> GraphicsEvent {
        self.commands += 1;
        let Ok(text) = std::str::from_utf8(payload) else {
            return self.error("invalid UTF-8 in graphics command", None);
        };
        let (control_text, data_text) = match text.split_once(';') {
            Some((c, d)) => (c, d),
            None => (text, ""),
        };
        let control = match parse_control(control_text) {
            Ok(c) => c,
            Err(e) => return self.error(&e, None),
        };
        let anchor_line = cursor_row as i32 - display_offset as i32;

        // Continuation chunks are allowed to omit the action. kitty's chunked
        // transfers always have `m=1` on every non-final chunk.
        let is_continuation = self.pending.is_some() && control.action.is_none();
        if is_continuation {
            let Some(mut pending) = self.pending.take() else {
                return self.error("internal chunk state error", None);
            };
            let decoded = match decode_data(data_text) {
                Ok(d) => d,
                Err(e) => {
                    self.pending = None;
                    return self.error(&e, Some(pending.id));
                }
            };
            pending.data.extend_from_slice(&decoded);
            if pending.data.len() > MAX_IMAGE_PIXELS as usize * 4 + 1024 * 1024 {
                return self.error("image payload too large", Some(pending.id));
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
            return self.error("new transmission while chunked transfer pending", control.id);
        }

        let action = control.action.unwrap_or('t');
        match action {
            't' | 'T' => {
                if control.quiet == 2 {
                    // still process
                }
                let format = control.format.unwrap_or(32);
                let id = match control.id {
                    Some(id) if id > 0 => id,
                    _ => {
                        let id = self.next_id;
                        self.next_id = self.next_id.wrapping_add(1).max(1);
                        id
                    }
                };
                let decoded = match decode_data(data_text) {
                    Ok(d) => d,
                    Err(e) => return self.error(&e, Some(id)),
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
                    anchor_col: cursor_col,
                    control,
                };
                if pending.control.more {
                    self.pending = Some(pending);
                    return GraphicsEvent { response: None };
                }
                self.finish_transmission(pending)
            }
            'p' => self.place(&control, anchor_line, cursor_col),
            'd' => self.delete(&control),
            'q' => self.query(&control),
            'f' => self.error("animation frames are not implemented", control.id),
            'a' => self.error("animation control is not implemented", control.id),
            other => self.error(&format!("unsupported graphics action '{other}'"), control.id),
        }
    }

    fn finish_transmission(&mut self, pending: Pending) -> GraphicsEvent {
        let id = pending.id;
        let decoded = match decode_payload(pending.format, &pending.data, pending.width, pending.height) {
            Ok(v) => v,
            Err(e) => return self.error(&e, Some(id)),
        };
        let (width, height, rgba) = decoded;
        if width == 0 || height == 0 {
            return self.error("zero-sized image", Some(id));
        }
        if width as u64 * height as u64 > MAX_IMAGE_PIXELS {
            return self.error("image exceeds 32 megapixel limit", Some(id));
        }
        if self.images.len() >= MAX_IMAGES && !self.images.contains_key(&id) {
            return self.error("image table full (256 images)", Some(id));
        }
        let generation = self.bump_generation();
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
            };
            self.add_placement(p);
        }
        let mut response = String::new();
        if pending.quiet != 2 {
            response = format!("\x1b_Gi={id};OK\x1b\\");
        }
        debug!(
            "kitty graphics: stored image {id} ({}x{}) format {}, chunked={}",
            width,
            height,
            pending.format,
            pending.control.more
        );
        GraphicsEvent { response: (!response.is_empty()).then_some(response) }
    }

    fn place(&mut self, control: &Control, anchor_line: i32, anchor_col: usize) -> GraphicsEvent {
        let Some(image_id) = control.id else {
            return self.error("placement requires image id (i)", None);
        };
        if !self.images.contains_key(&image_id) {
            return self.error("image not found", Some(image_id));
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
        };
        self.add_placement(p);
        GraphicsEvent { response: (control.quiet != 2).then(|| format!("\x1b_Gi={image_id};OK\x1b\\")) }
    }

    fn add_placement(&mut self, p: Placement) {
        if self.placements.len() >= MAX_PLACEMENTS {
            self.placements.remove(0);
        }
        self.placements.push(p);
    }

    fn delete(&mut self, control: &Control) -> GraphicsEvent {
        let mode = control.delete.unwrap_or('a');
        match mode {
            'a' => {
                self.images.clear();
                self.placements.clear();
            }
            'i' | 'I' => {
                let Some(id) = control.id else {
                    return self.error("delete requires image id", None);
                };
                self.images.remove(&id);
                self.placements.retain(|p| p.image_id != id);
            }
            'p' => {
                let Some(pid) = control.placement_id else {
                    return self.error("delete placement requires p", control.id);
                };
                self.placements.retain(|p| p.placement_id != pid);
            }
            'z' => {
                let z = control.z;
                self.placements.retain(|p| p.z != z);
            }
            other => return self.error(&format!("unsupported delete mode '{other}'"), control.id),
        }
        self.generation += 1;
        GraphicsEvent { response: (control.quiet != 2).then(|| "\x1b_G;OK\x1b\\".to_owned()) }
    }

    fn query(&mut self, control: &Control) -> GraphicsEvent {
        if let Some(id) = control.id {
            if self.images.contains_key(&id) {
                GraphicsEvent {
                    response: Some(format!("\x1b_Gi={id};OK\x1b\\")),
                }
            } else {
                self.error("image not found", Some(id))
            }
        } else {
            GraphicsEvent {
                response: Some(format!(
                    "\x1b_Gi={};OK\x1b\\",
                    self.images.keys().next().copied().unwrap_or(0)
                )),
            }
        }
    }

    fn error(&self, msg: &str, id: Option<u32>) -> GraphicsEvent {
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
    pub fn frame_images(&self, display_offset: usize) -> Vec<FrameImage> {
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
                // Include display offset indirectly: anchor_line is absolute.
                generation: p.generation.max(img.generation),
            });
        }
        let _ = display_offset;
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
            'q' => c.quiet = parse_u32(value, "q")? as u8,
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

fn decode_payload(
    format: u32,
    data: &[u8],
    width: Option<u32>,
    height: Option<u32>,
) -> Result<(u32, u32, Vec<u8>), String> {
    match format {
        100 => {
            let img = image::load_from_memory_with_format(data, ImageFormat::Png)
                .map_err(|e| format!("PNG decode failed: {e}"))?;
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
        24 => {
            let (w, h) = raw_dims(data, width, height, 3)?;
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for px in data.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            Ok((w, h, rgba))
        }
        32 => {
            let (w, h) = raw_dims(data, width, height, 4)?;
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
) -> Result<(u32, u32), String> {
    let (Some(w), Some(h)) = (width, height) else {
        return Err("raw image requires s= and v= dimensions".into());
    };
    if w == 0 || h == 0 {
        return Err("raw image dimensions must be nonzero".into());
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

    fn transmit_png(store: &mut GraphicsStore) -> GraphicsEvent {
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR4nGOsWvT9DAMewIRPcvgoAAD3iwLvLo6ZVwAAAABJRU5ErkJggg==";
        store.handle(format!("a=T,f=100,i=7;{b64}").as_bytes(), 2, 3, 0)
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
        let ev = s.handle(format!("a=t,f=32,s=1,v=1,i=4,m=1;{first}").as_bytes(), 0, 0, 0);
        assert!(ev.response.is_none());
        let ev = s.handle(format!("m=0;{second}").as_bytes(), 0, 0, 0);
        assert!(ev.response.as_deref().unwrap_or("").contains("OK"), "response: {:?}", ev.response);
        assert_eq!(s.image_count(), 1);
    }

    #[test]
    fn rejects_bad_raw_size() {
        let mut s = GraphicsStore::new();
        let b64 = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
        let ev = s.handle(format!("a=t,f=24,s=2,v=2;{b64}").as_bytes(), 0, 0, 0);
        assert!(ev.response.unwrap().contains("EINVAL"));
        assert_eq!(s.image_count(), 0);
    }

    #[test]
    fn delete_all() {
        let mut s = GraphicsStore::new();
        transmit_png(&mut s);
        let ev = s.handle(b"a=d,d=a", 0, 0, 0);
        assert!(ev.response.unwrap().contains("OK"));
        assert_eq!(s.image_count(), 0);
        assert_eq!(s.placement_count(), 0);
    }
}
