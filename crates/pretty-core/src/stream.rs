//! Streaming interceptor between the PTY and the VTE parser.
//!
//! The underlying `vte` parser knows DCS but not APC. kitty graphics uses APC,
//! so those sequences are removed from the stream and delivered separately.
//! OSC 9 / OSC 777 notifications are passed through *and* reported to the app.
//! All state is retained across `feed` boundaries: escape sequences split over
//! PTY reads must work identically to contiguous input.

use log::warn;

const MAX_APC: usize = 64 * 1024 * 1024;
const MAX_OSC: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamItem {
    /// Bytes that must be passed to the VTE parser.
    Bytes(Vec<u8>),
    /// Payload of an APC sequence (without `ESC _ G` and `ESC \`).
    Graphics(Vec<u8>),
    /// Parsed OSC notification.
    Notification { title: String, body: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Apc,
    Osc,
    /// Discard an oversized APC/OSC until its terminator.
    Ignore,
}

pub struct StreamFilter {
    mode: Mode,
    pending_esc: bool,
    /// Accumulated raw bytes that will be sent to the VTE parser.
    bytes: Vec<u8>,
    /// APC/OSC payload or discarded data.
    payload: Vec<u8>,
}

impl Default for StreamFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamFilter {
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            pending_esc: false,
            bytes: Vec::new(),
            payload: Vec::new(),
        }
    }

    pub fn feed(&mut self, input: &[u8]) -> Vec<StreamItem> {
        let mut out = Vec::new();
        for &b in input {
            match self.mode {
                Mode::Normal => self.normal_byte(b),
                Mode::Apc => {
                    self.payload.push(b);
                    if self.payload.ends_with(b"\x1b\\") {
                        let mut payload = std::mem::take(&mut self.payload);
                        payload.truncate(payload.len() - 2);
                        self.flush_bytes(&mut out);
                        // Strip the leading "G" that identifies the kitty
                        // graphics APC command.
                        if payload.first() == Some(&b'G') {
                            out.push(StreamItem::Graphics(payload[1..].to_vec()));
                        } else if !payload.is_empty() {
                            warn!("ignoring unsupported APC sequence of {} bytes", payload.len());
                        }
                        self.mode = Mode::Normal;
                    } else if self.payload.len() > MAX_APC {
                        warn!("APC sequence exceeds {MAX_APC} bytes; discarding");
                        self.payload.clear();
                        self.mode = Mode::Ignore;
                    }
                }
                Mode::Osc => {
                    if b == 0x07 {
                        self.finish_osc(false, &mut out);
                    } else {
                        self.payload.push(b);
                        if self.payload.ends_with(b"\x1b\\") {
                            self.finish_osc(true, &mut out);
                        } else if self.payload.len() > MAX_OSC {
                            warn!("OSC sequence exceeds {MAX_OSC} bytes; passing through without parsing");
                            // Already copied to `bytes`; skip notification parsing
                            // and wait for the real terminator.
                            self.mode = Mode::Ignore;
                            self.payload.clear();
                        }
                    }
                }
                Mode::Ignore => {
                    self.payload.push(b);
                    if b == 0x07 || self.payload.ends_with(b"\x1b\\") {
                        self.payload.clear();
                        self.mode = Mode::Normal;
                    } else if self.payload.len() > MAX_APC {
                        self.payload.clear();
                    }
                }
            }
        }
        self.flush_bytes(&mut out);
        out
    }

    fn normal_byte(&mut self, b: u8) {
        if self.pending_esc {
            self.pending_esc = false;
            match b {
                b'_' => {
                    self.clear_stale_esc();
                    self.mode = Mode::Apc;
                    self.payload.clear();
                }
                b']' => {
                    self.bytes.push(0x1b);
                    self.bytes.push(b']');
                    self.mode = Mode::Osc;
                    self.payload.clear();
                }
                _ => {
                    self.bytes.push(0x1b);
                    self.bytes.push(b);
                }
            }
            return;
        }
        if b == 0x1b {
            self.pending_esc = true;
        } else {
            self.bytes.push(b);
        }
    }

    fn clear_stale_esc(&mut self) {
        // The ESC was withheld for lookahead and belongs to the APC introducer;
        // it must not be sent to the VTE parser.
    }

    fn finish_osc(&mut self, st: bool, out: &mut Vec<StreamItem>) {
        if st {
            // payload includes the ESC \ terminator.
            let len = self.payload.len();
            self.payload.truncate(len.saturating_sub(2));
        }
        let payload = std::mem::take(&mut self.payload);
        // `ESC ]` was added when the sequence started; append the payload and
        // terminator now that the complete sequence is available.
        self.bytes.extend_from_slice(&payload);
        if st {
            self.bytes.extend_from_slice(b"\x1b\\");
        } else {
            self.bytes.push(0x07);
        }
        self.mode = Mode::Normal;
        if let Some((title, body)) = parse_notification(&payload) {
            self.flush_bytes(out);
            out.push(StreamItem::Notification { title, body });
        }
    }

    fn flush_bytes(&mut self, out: &mut Vec<StreamItem>) {
        if !self.bytes.is_empty() {
            out.push(StreamItem::Bytes(std::mem::take(&mut self.bytes)));
        }
    }

    /// Finish a partial feed. Used by tests and shutdown: a lone trailing ESC
    /// is emitted so the VTE parser can decide what to do with it.
    pub fn finish(&mut self) -> Vec<StreamItem> {
        let mut out = Vec::new();
        if self.mode == Mode::Normal && self.pending_esc {
            self.pending_esc = false;
            self.bytes.push(0x1b);
        } else if self.mode == Mode::Apc || self.mode == Mode::Osc || self.mode == Mode::Ignore {
            warn!("unterminated escape sequence at stream end");
            self.mode = Mode::Normal;
            self.payload.clear();
        }
        self.flush_bytes(&mut out);
        out
    }
}

/// Parse OSC payloads (without `ESC ]` and terminator) for notification forms.
pub fn parse_notification(payload: &[u8]) -> Option<(String, String)> {
    let text = std::str::from_utf8(payload).ok()?;
    if let Some(rest) = text.strip_prefix("777;notify;") {
        let mut parts = rest.splitn(2, ';');
        let title = parts.next().unwrap_or("Pretty");
        let body = parts.next().unwrap_or("");
        return Some((title.to_owned(), body.to_owned()));
    }
    if let Some(body) = text.strip_prefix("9;") {
        // OSC 9;4 is a progress report, not a notification.
        if body.starts_with("4;") {
            return None;
        }
        return Some(("Pretty".to_owned(), body.to_owned()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(items: Vec<StreamItem>) -> Vec<u8> {
        items
            .into_iter()
            .flat_map(|i| match i {
                StreamItem::Bytes(b) => b,
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn plain_text_passthrough() {
        let mut f = StreamFilter::new();
        assert_eq!(bytes(f.feed(b"hello")), b"hello");
    }

    #[test]
    fn graphics_apc_split_across_reads() {
        let mut f = StreamFilter::new();
        let a = f.feed(b"before\x1b_Gi=1;");
        assert_eq!(bytes(a), b"before");
        let b = f.feed(b"AAAA\x1b");
        assert!(bytes(b).is_empty());
        let c = f.feed(b"\\after");
        assert_eq!(bytes(c.clone()), b"after");
        let graphics: Vec<_> = c
            .into_iter()
            .filter_map(|i| match i {
                StreamItem::Graphics(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(graphics.len(), 1);
        assert_eq!(graphics[0], b"i=1;AAAA");
    }

    #[test]
    fn osc_title_passthrough_and_notification() {
        let mut f = StreamFilter::new();
        let items = f.feed(b"a\x1b]0;title\x07b");
        let raw = bytes(items.clone());
        assert_eq!(raw, b"a\x1b]0;title\x07b");
        assert!(!items.is_empty());

        let mut f = StreamFilter::new();
        let items = f.feed(b"\x1b]777;notify;Build;done\x07");
        assert!(matches!(
            &items[1],
            StreamItem::Notification { title, body } if title == "Build" && body == "done"
        ));
    }

    #[test]
    fn osc_st_split() {
        let mut f = StreamFilter::new();
        let _ = f.feed(b"\x1b]9;hi");
        let items = f.feed(b"\x1b\\");
        assert!(items.iter().any(|i| matches!(
            i,
            StreamItem::Notification { body, .. } if body == "hi"
        )));
    }

    #[test]
    fn esc_not_selective() {
        let mut f = StreamFilter::new();
        assert_eq!(bytes(f.feed(b"\x1b[31m")), b"\x1b[31m");
        assert_eq!(bytes(f.feed(b"\x1b")), b"");
        assert_eq!(bytes(f.feed(b"c")), b"\x1bc");
    }
}
