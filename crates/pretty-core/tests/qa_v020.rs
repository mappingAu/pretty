//! Regression tests added by the v0.2.0 QA pass.
//!
//! Every test in this file is a regression guard for a defect that was found
//! during the pre-release audit and fixed in `pretty-core`. The tests are
//! written against the public crate API (except for a couple of `Frame`
//! constructors, whose fields are public) so they also catch wiring mistakes
//! between the stream filter, the terminal state and the frame snapshot.

use pretty_core::graphics::GraphicsStore;
use pretty_core::{CoreConfig, Frame, FrameCell, TermSize, Terminal};

fn empty_frame(cols: usize, rows: usize) -> Frame {
    Frame {
        cols,
        rows,
        cells: vec![FrameCell::default(); cols * rows],
        cursor: None,
        display_offset: 0,
        history_size: 0,
        hyperlinks: Vec::new(),
        images: Vec::new(),
        generation: 1,
    }
}

/// QA-1: "Select all" must select the whole scrollback, not only the current
/// viewport. Before the fix `Terminal::select_all` used `-display_offset` as
/// the top row, so the oldest history lines were never copied.
#[test]
fn select_all_includes_scrollback_history() {
    let (mut term, _rx) = Terminal::new(TermSize::new(20, 4), &CoreConfig::default());
    for i in 0..30 {
        term.advance(format!("line{i:02}\r\n").as_bytes());
    }
    assert!(term.history_size() > 0, "test setup must produce scrollback");
    term.select_all();
    let text = term.selection_text().expect("select all produced a selection");
    assert!(
        text.contains("line00"),
        "select all must include the oldest scrollback line; got first line {:?}",
        text.lines().next()
    );
    assert!(
        text.contains("line29"),
        "select all must include the newest line; got last line {:?}",
        text.lines().last()
    );
}

/// QA-2: Copying a frame selection that contains a wide character must not
/// insert an extra space for the wide-character spacer cell.
#[test]
fn selection_text_does_not_insert_spaces_for_wide_spacers() {
    let mut frame = empty_frame(3, 1);
    let mut wide = FrameCell::default();
    wide.c = '中';
    wide.wide = true;
    wide.attrs.selected = true;
    let mut spacer = FrameCell::default();
    spacer.wide_spacer = true;
    spacer.attrs.selected = true;
    let mut ascii = FrameCell::default();
    ascii.c = 'a';
    ascii.attrs.selected = true;
    frame.cells = vec![wide, spacer, ascii];

    assert_eq!(
        frame.selection_text_from_cells(),
        "中a",
        "wide-character spacer cells must not become a literal space"
    );
}

/// QA-2b: A selection that starts on a later row must not produce leading
/// blank lines, because that silently changes the copied text (and the
/// previous implementation also leaked a leading newline for every empty row).
#[test]
fn selection_text_has_no_leading_blank_lines() {
    let mut frame = empty_frame(1, 3);
    let mut selected = FrameCell::default();
    selected.c = 'x';
    selected.attrs.selected = true;
    frame.cells[2] = selected;
    assert_eq!(frame.selection_text_from_cells(), "x");
}

/// QA-3: Error responses for kitty graphics commands must be bounded. A
/// malformed control value (here a multi-megabyte `i=` field, which fits in
/// the 64 MiB APC budget) used to be echoed back verbatim to the PTY; that is
/// both an unbounded allocation and a way to block the UI thread on a PTY
/// write that the client may never drain.
#[test]
fn graphics_error_response_is_bounded() {
    let mut store = GraphicsStore::new();
    let payload = format!("a=t,i={}", "9".repeat(256 * 1024));
    let response = store
        .handle(payload.as_bytes(), 0, 0)
        .response
        .expect("invalid control must still report an error");
    assert!(
        response.len() <= 512,
        "error response echoed attacker-controlled data: {} bytes",
        response.len()
    );
    assert!(response.contains("EINVAL"), "response: {response:?}");
    assert!(store.image_count() == 0);
}
