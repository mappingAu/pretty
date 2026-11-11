//! PTY integration smoke test.
//!
//! Exercises the real `portable-pty` path: spawn a child, write to it, read
//! its output, resize and poll the child-exit API that the application needs
//! in order to close a finished pane (Pretty's own event loop never receives
//! `alacritty_terminal`'s `ChildExit` event).

#![cfg(unix)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use pretty_core::{CoreError, PtyExitStatus, PtySession};

fn wait_for_output(
    rx: &crossbeam_channel::Receiver<Vec<u8>>,
    needle: &[u8],
    timeout: Duration,
) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut data = Vec::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                data.extend_from_slice(&chunk);
                if data.windows(needle.len()).any(|window| window == needle) {
                    return data;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    data
}

#[test]
fn pty_roundtrip_resize_and_child_exit_status() {
    if !std::path::Path::new("/bin/sh").exists() {
        eprintln!("skipping: /bin/sh is not available");
        return;
    }
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let mut pty = PtySession::spawn(
        vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            r#"read x; printf 'got:%s' "$x"; exit 7"#.to_owned(),
        ],
        Vec::new(),
        None,
        40,
        10,
        8,
        16,
        wake,
    )
    .expect("spawn PTY");
    let rx = pty.take_receiver().expect("PTY receiver");

    // The child is waiting for a line, so it must still be running.
    assert!(
        pty.try_wait().expect("try_wait").is_none(),
        "child unexpectedly exited before input"
    );
    pty.write(b"hello\n").expect("write to PTY");
    let output = wait_for_output(&rx, b"got:hello", Duration::from_secs(10));
    assert!(
        output.windows(9).any(|window| window == b"got:hello"),
        "PTY output: {:?}",
        String::from_utf8_lossy(&output)
    );

    // Resize the PTY (the app does this on every window/pane resize).
    pty.resize(50, 12, 8, 16).expect("resize PTY");

    let mut status: Option<PtyExitStatus> = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(found) = pty.try_wait().expect("try_wait") {
            status = Some(found);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = status.expect("child exit status was never reported");
    assert_eq!(status.code, 7, "exit status: {status:?}");
    assert!(status.signal.is_none(), "exit status: {status:?}");
    assert!(!status.success());
    assert!(
        pty.child_pid().is_some(),
        "portable-pty should expose the child pid"
    );
}

#[test]
fn pty_rejects_empty_command() {
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let err = match PtySession::spawn(Vec::new(), Vec::new(), None, 10, 5, 8, 16, wake) {
        Ok(_) => panic!("empty argv must be rejected"),
        Err(err) => err,
    };
    assert!(matches!(err, CoreError::Config(_)), "error: {err:?}");
}

/// Stress test (ignored by default): close a pane while the bounded reader
/// channel is full. `PtySession::drop` used to join a reader thread blocked in
/// a blocking channel send forever, hanging the UI thread that closes the pane.
#[test]
#[ignore = "stress test: fills the 64 MiB PTY reader channel; run with --ignored"]
fn dropping_a_pty_with_a_full_reader_channel_does_not_deadlock() {
    if !std::path::Path::new("/bin/sh").exists() {
        eprintln!("skipping: /bin/sh is not available");
        return;
    }
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let mut pty = PtySession::spawn(
        vec!["/bin/sh".to_owned(), "-c".to_owned(), "cat /dev/zero".to_owned()],
        Vec::new(),
        None,
        200,
        50,
        8,
        16,
        wake,
    )
    .expect("spawn PTY");
    let rx = pty.take_receiver().expect("PTY receiver");
    let deadline = Instant::now() + Duration::from_secs(20);
    while rx.len() < 1024 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        rx.len(),
        1024,
        "the reader channel did not fill, so the deadlock path was not exercised"
    );
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);
    let handle = std::thread::spawn(move || {
        drop(pty);
        let _ = done_tx.send(());
    });
    let finished = done_rx.recv_timeout(Duration::from_secs(5));
    assert!(
        finished.is_ok(),
        "PtySession::drop deadlocked while the reader channel was full"
    );
    handle.join().ok();
    drop(rx);
}
