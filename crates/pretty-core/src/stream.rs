//! Streaming interceptor between the PTY and the VTE parser.
//!
//! The underlying `vte` parser knows DCS but not APC. kitty graphics uses APC,
//! so those sequences are removed from the stream and delivered separately.
//! OSC 9 / OSC 777 notifications are passed through *and* reported to the app.
//! All state is retained across `feed` boundaries: escape sequences split over
//! PTY reads must work identically to contiguous input.
//!
//! Resource and safety policy:
//! - APC payloads are bounded and scanned incrementally instead of being
//!   re-scanned from the start on every PTY read (which would be quadratic in
//!   the payload size and trivially CPU bound by a malicious program).
//! - Discarded (oversized) OSC sequences are still *terminated* in the byte
//!   stream handed to the VTE parser, otherwise the parser would stay inside an
//!   unterminated OSC string and silently swallow or reinterpret following
//!   output.
//! - Discarded sequences never retain their payload, so a hostile stream cannot
//!   make the filter buffer megabytes while it is looking for a terminator.
//! - Desktop notification text is sanitized and rate limited: terminal output is
//!   untrusted and must not be able to inject control/bidi characters into
//!   notification daemons or produce unbounded notification spam.

use std::time::{Duration, Instant};

use log::{debug, warn};

const MAX_APC: usize = 64 * 1024 * 1024;
const MAX_OSC: usize = 4 * 1024 * 1024;

/// Maximum number of desktop notifications forwarded per
/// [`NOTIFICATION_WINDOW`] from a single terminal.
const NOTIFICATION_BURST: u32 = 8;
const NOTIFICATION_WINDOW: Duration = Duration::from_secs(1);
/// Upper bounds on notification title/body length (in Unicode scalar values).
const MAX_NOTIFICATION_TITLE: usize = 256;
const MAX_NOTIFICATION_BODY: usize = 4096;

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

/// Outcome of a single [`StreamFilter::feed_apc`] attempt.
enum ApcProgress {
    /// More input is needed (everything was appended to the pending payload).
    Waiting,
    /// An APC was delivered; `consumed` input bytes belonged to it.
    Done { consumed: usize },
}

pub struct StreamFilter {
    mode: Mode,
    pending_esc: bool,
    /// Accumulated raw bytes that will be sent to the VTE parser.
    bytes: Vec<u8>,
    /// APC/OSC payload that is being assembled.
    payload: Vec<u8>,
    /// Number of `payload` bytes that are known not to start an APC
    /// terminator. Kept across `feed` calls so a large APC is scanned once.
    apc_scanned: usize,
    /// Previous byte while discarding an oversized sequence (for `ESC \`).
    ignore_prev_esc: bool,
    /// Desktop-notification rate limiting state.
    notification_window: Instant,
    notification_count: u32,
    max_apc: usize,
    max_osc: usize,
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
            apc_scanned: 0,
            ignore_prev_esc: false,
            notification_window: Instant::now(),
            notification_count: 0,
            max_apc: MAX_APC,
            max_osc: MAX_OSC,
        }
    }

    #[cfg(test)]
    fn with_limits(max_apc: usize, max_osc: usize) -> Self {
        let mut filter = Self::new();
        filter.max_apc = max_apc;
        filter.max_osc = max_osc;
        filter
    }

    pub fn feed(&mut self, input: &[u8]) -> Vec<StreamItem> {
        let mut out = Vec::new();
        self.process_input(input, &mut out);
        self.flush_bytes(&mut out);
        out
    }

    /// Run the normal (byte-oriented) state machine over `input`, switching to
    /// the slice-scanning APC fast path whenever an APC introducer is consumed.
    /// Avoiding per-byte work *and* recursion for multi-megabyte kitty payloads
    /// keeps both CPU and stack use bounded.
    fn process_input(&mut self, input: &[u8], out: &mut Vec<StreamItem>) {
        // One owned copy: the tail after an APC terminator must be reprocessed
        // while the original `input` slice stays borrowed by the caller.
        let current = input.to_vec();
        let mut start = 0usize;
        loop {
            if start >= current.len() {
                return;
            }
            if self.mode == Mode::Apc {
                self.flush_bytes(out);
                match self.feed_apc(&current[start..], out) {
                    ApcProgress::Done { consumed } => {
                        start += consumed;
                        if start >= current.len() {
                            return;
                        }
                        continue;
                    }
                    ApcProgress::Waiting => return,
                }
            }
            while start < current.len() && self.mode != Mode::Apc {
                let b = current[start];
                start += 1;
                match self.mode {
                    Mode::Normal => self.normal_byte(b),
                    Mode::Osc => self.osc_byte(b, out),
                    Mode::Ignore => self.ignore_byte(b),
                    Mode::Apc => unreachable!("APC is handled by feed_apc"),
                }
            }
            if start >= current.len() {
                return;
            }
        }
    }

    /// Fast path for an APC sequence. Appends `input` to the pending payload,
    /// looks for an `ESC \` terminator and, on success, pushes the decoded APC
    /// item and reports how many `input` bytes it consumed (the terminator may
    /// span the boundary between the pending payload and `input`).
    fn feed_apc(&mut self, input: &[u8], out: &mut Vec<StreamItem>) -> ApcProgress {
        let old_len = self.payload.len();
        self.payload.extend_from_slice(input);
        let Some(pos) = self.find_apc_terminator(old_len) else {
            // Remember what was already scanned: re-scanning the whole payload
            // on every read would be O(n^2) for a large chunked image/APC.
            self.apc_scanned = self.payload.len().saturating_sub(1);
            if self.payload.len() > self.max_apc {
                warn!(
                    "APC sequence exceeds {} bytes; discarding the rest of the sequence",
                    self.max_apc
                );
                // The discarded payload may end with the first byte of an
                // `ESC \` terminator. Remember it for the ignore state machine,
                // otherwise a terminator split across reads is never
                // recognised and all following output is swallowed.
                self.ignore_prev_esc = self.payload.last() == Some(&0x1b);
                self.payload.clear();
                self.apc_scanned = 0;
                self.mode = Mode::Ignore;
            }
            return ApcProgress::Waiting;
        };
        let consumed = (pos + 2).saturating_sub(old_len);
        let mut payload = std::mem::take(&mut self.payload);
        // Drop the terminator and everything after it; the caller still owns
        // those bytes through `input` and will process them next.
        payload.truncate(pos);
        self.apc_scanned = 0;
        self.flush_bytes(out);
        if payload.first() == Some(&b'G') {
            out.push(StreamItem::Graphics(payload[1..].to_vec()));
        } else if !payload.is_empty() {
            warn!("ignoring unsupported APC sequence of {} bytes", payload.len());
        }
        self.mode = Mode::Normal;
        ApcProgress::Done { consumed }
    }

    /// Find `ESC \` in the not-yet-scanned part of the APC payload.
    fn find_apc_terminator(&self, old_len: usize) -> Option<usize> {
        let len = self.payload.len();
        if len < 2 {
            return None;
        }
        // Start one byte before the previously scanned region: a terminator can
        // straddle the boundary between two PTY reads.
        let search_from = self
            .apc_scanned
            .min(old_len.saturating_sub(1))
            .min(len - 2);
        self.payload[search_from..]
            .windows(2)
            .position(|w| w == b"\x1b\\")
            .map(|offset| search_from + offset)
    }

    fn normal_byte(&mut self, b: u8) {
        if self.pending_esc {
            self.pending_esc = false;
            match b {
                b'_' => {
                    // The withheld ESC introducer belongs to the APC sequence;
                    // it must not be sent to the VTE parser.
                    self.mode = Mode::Apc;
                    self.payload.clear();
                    self.apc_scanned = 0;
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

    fn osc_byte(&mut self, b: u8, out: &mut Vec<StreamItem>) {
        if b == 0x07 {
            self.finish_osc(false, out);
            return;
        }
        self.payload.push(b);
        if self.payload.ends_with(b"\x1b\\") {
            self.finish_osc(true, out);
        } else if self.payload.len() > self.max_osc {
            warn!(
                "OSC sequence exceeds {} bytes; discarding the rest of the sequence",
                self.max_osc
            );
            self.mode = Mode::Ignore;
            self.payload.clear();
            self.ignore_prev_esc = b == 0x1b;
            // `ESC ]` was already handed to the VTE parser. Close it with BEL so
            // the parser returns to ground state instead of interpreting the
            // following output as (more) OSC payload.
            self.bytes.push(0x07);
        }
    }

    fn ignore_byte(&mut self, b: u8) {
        let is_st = b == b'\\' && self.ignore_prev_esc;
        self.ignore_prev_esc = b == 0x1b;
        if b == 0x07 || is_st {
            self.ignore_prev_esc = false;
            self.mode = Mode::Normal;
        }
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
            if self.notification_allowed() {
                out.push(StreamItem::Notification { title, body });
            } else {
                debug!("dropping OSC notification for {title:?}: rate limit exceeded");
            }
        }
    }

    /// Token-bucket style limiter: terminal output cannot spam the desktop with
    /// thousands of notifications from a single pane.
    fn notification_allowed(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.notification_window) >= NOTIFICATION_WINDOW {
            self.notification_window = now;
            self.notification_count = 0;
        }
        if self.notification_count >= NOTIFICATION_BURST {
            return false;
        }
        self.notification_count += 1;
        true
    }

    fn flush_bytes(&mut self, out: &mut Vec<StreamItem>) {
        if !self.bytes.is_empty() {
            out.push(StreamItem::Bytes(std::mem::take(&mut self.bytes)));
        }
    }

    /// Finish a partial feed. Used by tests and shutdown: a lone trailing ESC
    /// is emitted so the VTE parser can decide what to do with it, and a
    /// partially forwarded OSC is closed so the parser cannot be left in an
    /// unterminated string state.
    pub fn finish(&mut self) -> Vec<StreamItem> {
        let mut out = Vec::new();
        match self.mode {
            Mode::Normal => {
                if self.pending_esc {
                    self.pending_esc = false;
                    self.bytes.push(0x1b);
                }
            }
            Mode::Apc => {
                warn!("unterminated APC sequence at stream end");
                self.mode = Mode::Normal;
                self.payload.clear();
                self.apc_scanned = 0;
            }
            Mode::Osc => {
                warn!("unterminated OSC sequence at stream end");
                self.bytes.push(0x07);
                self.mode = Mode::Normal;
                self.payload.clear();
            }
            Mode::Ignore => {
                warn!("unterminated escape sequence at stream end");
                self.mode = Mode::Normal;
                self.ignore_prev_esc = false;
            }
        }
        self.flush_bytes(&mut out);
        out
    }
}

/// Parse OSC payloads (without `ESC ]` and terminator) for notification forms.
///
/// The terminal output is untrusted: control characters (including `ESC`/`BEL`,
/// which some notification daemons interpret) and bidi/zero-width format
/// characters (UI spoofing) are stripped, and both fields are length capped.
pub fn parse_notification(payload: &[u8]) -> Option<(String, String)> {
    let text = std::str::from_utf8(payload).ok()?;
    let (raw_title, raw_body) = if let Some(rest) = text.strip_prefix("777;notify;") {
        let mut parts = rest.splitn(2, ';');
        (parts.next().unwrap_or("Pretty"), parts.next().unwrap_or(""))
    } else if let Some(body) = text.strip_prefix("9;") {
        // OSC 9;4 is a progress report, not a notification.
        if body.starts_with("4;") {
            return None;
        }
        ("Pretty", body)
    } else {
        return None;
    };
    let title = sanitize_notification_field(raw_title, MAX_NOTIFICATION_TITLE, "Pretty");
    let body = sanitize_notification_field(raw_body, MAX_NOTIFICATION_BODY, "");
    Some((title, body))
}

fn sanitize_notification_field(raw: &str, max_chars: usize, fallback: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(max_chars.saturating_mul(4)));
    let mut count = 0usize;
    for c in raw.chars() {
        if count >= max_chars {
            break;
        }
        if is_spoofing_format_char(c) {
            continue;
        }
        let mapped = if c.is_control() {
            // Line breaks/tabs would let untrusted output forge the structure
            // of a notification; render them as a plain space instead.
            if matches!(c, '\t' | '\n' | '\r') {
                Some(' ')
            } else {
                None
            }
        } else {
            Some(c)
        };
        if let Some(c) = mapped {
            out.push(c);
            count += 1;
        }
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

/// Unicode format characters that can reorder or hide notification text.
fn is_spoofing_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{061c}' // Arabic letter mark
            | '\u{200b}'..='\u{200f}' // zero width space/joiners, LRM/RLM
            | '\u{202a}'..='\u{202e}' // bidi embedding/override
            | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
            | '\u{2066}'..='\u{2069}' // bidi isolates
            | '\u{feff}' // zero width no-break space / BOM
    )
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

    fn graphics(items: &[StreamItem]) -> Vec<&[u8]> {
        items
            .iter()
            .filter_map(|i| match i {
                StreamItem::Graphics(p) => Some(p.as_slice()),
                _ => None,
            })
            .collect()
    }

    fn notifications(items: &[StreamItem]) -> Vec<(&str, &str)> {
        items
            .iter()
            .filter_map(|i| match i {
                StreamItem::Notification { title, body } => {
                    Some((title.as_str(), body.as_str()))
                }
                _ => None,
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
    fn apc_large_payload_scanned_incrementally() {
        // Feed a large APC in small reads: the payload must be delivered
        // intact and bytes after the terminator must not be swallowed.
        let payload = "i=9;".to_owned() + &"A".repeat(256 * 1024);
        let mut f = StreamFilter::new();
        let mut items = Vec::new();
        let mut wire = b"\x1b_G".to_vec();
        wire.extend_from_slice(payload.as_bytes());
        wire.extend_from_slice(b"\x1b\\tail");
        for chunk in wire.chunks(1024) {
            items.extend(f.feed(chunk));
        }
        let expected = payload.as_bytes();
        assert_eq!(graphics(&items), vec![expected]);
        assert_eq!(bytes(items), b"tail");
    }

    #[test]
    fn many_complete_apcs_in_one_read_do_not_recurse() {
        let mut f = StreamFilter::new();
        let mut wire = Vec::new();
        for _ in 0..10_000 {
            wire.extend_from_slice(b"\x1b_Gi=1;QUJD\x1b\\");
        }
        let items = f.feed(&wire);
        assert_eq!(graphics(&items).len(), 10_000);
    }

    #[test]
    fn oversized_osc_is_terminated_for_the_vte_parser() {
        let mut f = StreamFilter::with_limits(1024, 64);
        let mut wire = b"\x1b]0;".to_vec();
        wire.extend(std::iter::repeat(b'x').take(256));
        wire.extend_from_slice(b"\x07hello\x07");
        let items = f.feed(&wire);
        let raw = bytes(items.clone());
        // The unterminated OSC must be closed with BEL, and the following text
        // must be forwarded as normal bytes.
        assert_eq!(raw, b"\x1b]\x07hello\x07", "raw passthrough: {raw:?}");
        assert!(notifications(&items).is_empty());
    }

    #[test]
    fn oversized_osc_with_st_terminator_recovers() {
        let mut f = StreamFilter::with_limits(1024, 32);
        let mut wire = b"\x1b]777;notify;t;".to_vec();
        wire.extend(std::iter::repeat(b'y').take(128));
        wire.extend_from_slice(b"\x1b\\after");
        let items = f.feed(&wire);
        assert_eq!(bytes(items), b"\x1b]\x07after");
    }

    #[test]
    fn discarded_oversized_osc_does_not_buffer_payload() {
        let mut f = StreamFilter::with_limits(1024, 32);
        let mut wire = b"\x1b]0;".to_vec();
        wire.extend(std::iter::repeat(b'z').take(64));
        let _ = f.feed(&wire);
        assert_eq!(f.mode, Mode::Ignore);
        // Keep the terminator away for a long time: all of this must be
        // discarded without being retained.
        for _ in 0..64 {
            let junk = vec![b'j'; 16 * 1024];
            let _ = f.feed(&junk);
            assert!(f.payload.is_empty(), "ignore mode retained payload");
        }
        let items = f.feed(b"\x07safe");
        assert_eq!(bytes(items), b"safe");
    }

    #[test]
    fn oversized_apc_is_discarded_and_does_not_buffer() {
        let mut f = StreamFilter::with_limits(64, 4096);
        let mut wire = b"\x1b_G".to_vec();
        wire.extend(std::iter::repeat(b'A').take(256));
        let _ = f.feed(&wire);
        assert_eq!(f.mode, Mode::Ignore);
        assert!(f.payload.is_empty());
        let items = f.feed(b"\x1b\\ok");
        assert!(graphics(&items).is_empty());
        assert_eq!(bytes(items), b"ok");
    }

    #[test]
    fn oversized_apc_recognises_terminator_split_across_reads() {
        // The oversized payload ends with the ESC half of the `ESC \`
        // terminator; the backslash arrives in the next read. The filter must
        // still leave discard mode and forward the following output instead of
        // swallowing the rest of the stream.
        let mut f = StreamFilter::with_limits(32, 4096);
        let mut wire = b"\x1b_G".to_vec();
        wire.extend(std::iter::repeat(b'A').take(32));
        wire.push(0x1b);
        assert!(bytes(f.feed(&wire)).is_empty());
        assert_eq!(f.mode, Mode::Ignore);
        let items = f.feed(b"\\visible");
        assert!(graphics(&items).is_empty());
        assert_eq!(bytes(items), b"visible");
    }

    #[test]
    fn oversized_apc_without_split_terminator_still_discards() {
        // The discarded payload ends with data, so a later backslash is not a
        // terminator and must not end discard mode early. Only BEL recovers.
        let mut f = StreamFilter::with_limits(32, 4096);
        let mut wire = b"\x1b_G".to_vec();
        wire.extend(std::iter::repeat(b'A').take(33));
        let _ = f.feed(&wire);
        assert_eq!(f.mode, Mode::Ignore);
        assert!(bytes(f.feed(b"\\not-a-terminator")).is_empty());
        assert_eq!(f.mode, Mode::Ignore);
        let items = f.feed(b"\x07after-bel");
        assert_eq!(bytes(items), b"after-bel");
    }

    #[test]
    fn unterminated_osc_at_stream_end_is_closed() {
        let mut f = StreamFilter::new();
        let first = f.feed(b"text\x1b]0;unfinished");
        // The introducer is streamed through immediately.
        assert_eq!(bytes(first), b"text\x1b]");
        assert_eq!(bytes(f.finish()), b"\x07");
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
    fn notification_text_is_sanitized() {
        let mut f = StreamFilter::new();
        // Ctrl characters and bidi overrides in the title/body must not survive.
        let items = f.feed(b"\x1b]9;hi\x1b[31m\xe2\x80\xaethere\x1b[0m\x07");
        let notifs = notifications(&items);
        assert_eq!(notifs.len(), 1);
        let (title, body) = notifs[0];
        assert_eq!(title, "Pretty");
        assert!(!body.contains('\x1b'));
        assert!(!body.contains('\u{202e}'));
        assert_eq!(body, "hi[31mthere[0m");
    }

    #[test]
    fn notification_fields_are_capped() {
        let mut f = StreamFilter::new();
        let long = "a".repeat(MAX_NOTIFICATION_BODY + 100);
        let wire = format!("\x1b]9;{long}\x07");
        let items = f.feed(wire.as_bytes());
        let notifs = notifications(&items);
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].1.chars().count(), MAX_NOTIFICATION_BODY);
    }

    #[test]
    fn notification_spam_is_rate_limited() {
        let mut f = StreamFilter::new();
        let mut items = Vec::new();
        for _ in 0..64 {
            items.extend(f.feed(b"\x1b]9;spam\x07"));
        }
        let count = notifications(&items).len() as u32;
        assert_eq!(count, NOTIFICATION_BURST, "rate limit must drop the burst");
        // A new window allows notifications again.
        f.notification_window = Instant::now() - Duration::from_secs(2);
        let items = f.feed(b"\x1b]9;later\x07");
        assert_eq!(notifications(&items).len(), 1);
    }

    #[test]
    fn esc_not_selective() {
        let mut f = StreamFilter::new();
        assert_eq!(bytes(f.feed(b"\x1b[31m")), b"\x1b[31m");
        assert_eq!(bytes(f.feed(b"\x1b")), b"");
        assert_eq!(bytes(f.feed(b"c")), b"\x1bc");
    }
}
