//! Cross-platform PTY session built on `portable-pty`.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, SendTimeoutError, Sender};
use log::{debug, warn};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::CoreError;

/// Exit status of a PTY child process, as reported by [`PtySession::try_wait`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyExitStatus {
    /// Exit code (1 when the process was terminated by a signal).
    pub code: u32,
    /// Signal name on Unix if the process was terminated by a signal.
    pub signal: Option<String>,
}

impl PtyExitStatus {
    /// Whether the child completed successfully.
    pub fn success(&self) -> bool {
        self.signal.is_none() && self.code == 0
    }
}

pub struct PtySession {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    reader_thread: Option<JoinHandle<()>>,
    data_rx: Option<Receiver<Vec<u8>>>,
    /// Set before joining the reader so a thread blocked on a full channel
    /// exits instead of deadlocking `Drop`.
    stop_reader: Arc<AtomicBool>,
    cols: u16,
    rows: u16,
    cell_w: u16,
    cell_h: u16,
}

impl PtySession {
    pub fn spawn(
        argv: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<std::path::PathBuf>,
        cols: u16,
        rows: u16,
        cell_w: u16,
        cell_h: u16,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, CoreError> {
        if argv.is_empty() {
            return Err(CoreError::Config("empty PTY command".into()));
        }
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: cell_w.saturating_mul(cols),
                pixel_height: cell_h.saturating_mul(rows),
            })
            .map_err(|e| CoreError::Pty(e.to_string()))?;

        let mut cmd = CommandBuilder::new(&argv[0]);
        if argv.len() > 1 {
            cmd.args(&argv[1..]);
        }
        for (key, value) in &env {
            cmd.env(key, value);
        }
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| CoreError::Pty(format!("spawn {}: {e}", argv[0])))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| CoreError::Pty(format!("reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| CoreError::Pty(format!("writer: {e}")))?;

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(1024);
        let stop_reader = Arc::new(AtomicBool::new(false));
        let reader_stop = stop_reader.clone();
        let reader_thread = std::thread::Builder::new()
            .name("pretty-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 65536];
                loop {
                    if reader_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut pending = Some(buf[..n].to_vec());
                            while let Some(bytes) = pending.take() {
                                if reader_stop.load(Ordering::Relaxed) {
                                    return;
                                }
                                // A bounded timeout (rather than a blocking
                                // send) keeps `PtySession::drop` from hanging
                                // forever when the app closes a pane while the
                                // 1024-slot reader channel is full.
                                match tx.send_timeout(bytes, Duration::from_millis(50)) {
                                    Ok(()) => wake(),
                                    Err(SendTimeoutError::Timeout(bytes)) => pending = Some(bytes),
                                    Err(SendTimeoutError::Disconnected(_)) => return,
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            debug!("PTY reader closed: {e}");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| CoreError::Pty(format!("spawn reader thread: {e}")))?;

        debug!("spawned PTY session {}x{} argv={argv:?}", cols, rows);
        Ok(Self {
            master: Some(pair.master),
            writer: Some(writer),
            child,
            reader_thread: Some(reader_thread),
            data_rx: Some(rx),
            stop_reader,
            cols,
            rows,
            cell_w,
            cell_h,
        })
    }

    pub fn take_receiver(&mut self) -> Option<Receiver<Vec<u8>>> {
        self.data_rx.take()
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "PTY closed"));
        };
        writer.write_all(bytes)?;
        writer.flush()
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) -> Result<(), CoreError> {
        if (cols, rows, cell_w, cell_h) == (self.cols, self.rows, self.cell_w, self.cell_h) {
            return Ok(());
        }
        let Some(master) = self.master.as_ref() else {
            return Err(CoreError::Pty("PTY closed".into()));
        };
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: cell_w.saturating_mul(cols),
                pixel_height: cell_h.saturating_mul(rows),
            })
            .map_err(|e| CoreError::Pty(format!("resize: {e}")))?;
        self.cols = cols;
        self.rows = rows;
        self.cell_w = cell_w;
        self.cell_h = cell_h;
        Ok(())
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Check whether the child process has exited, without blocking.
    ///
    /// `pretty-app` owns the pane/tab lifecycle, so it has to poll this (for
    /// example when the PTY reader channel disconnects or on an idle tick) to
    /// close the pane of a finished shell. `alacritty_terminal`'s `ChildExit`
    /// event is only emitted by its own event loop, which Pretty does not use.
    /// The exit status is cached by the OS child handle, so a later `Drop`
    /// still reaps the process correctly.
    pub fn try_wait(&mut self) -> std::io::Result<Option<PtyExitStatus>> {
        let status = self.child.try_wait()?;
        Ok(status.map(|status| PtyExitStatus {
            code: status.exit_code(),
            signal: status.signal().map(str::to_owned),
        }))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Some(pid) = self.child.process_id() {
            debug!("terminating PTY child {pid}");
        }
        // Stop the reader before joining it: if it is blocked in a channel
        // send because nobody drains the output (pane being closed), killing
        // the child alone would not unblock it.
        self.stop_reader.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.writer.take();
        self.master.take();
        if let Some(handle) = self.reader_thread.take() {
            // Killing the child closes the slave side, so the reader exits.
            if let Err(err) = handle.join() {
                warn!("PTY reader thread panicked: {err:?}");
            }
        }
    }
}
