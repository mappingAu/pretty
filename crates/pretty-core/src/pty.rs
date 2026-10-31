//! Cross-platform PTY session built on `portable-pty`.

use std::io::{Read, Write};
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{bounded, Receiver, Sender};
use log::{debug, warn};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::CoreError;

pub struct PtySession {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    reader_thread: Option<JoinHandle<()>>,
    data_rx: Option<Receiver<Vec<u8>>>,
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
        let reader_thread = std::thread::Builder::new()
            .name("pretty-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 65536];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                            wake();
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
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Some(pid) = self.child.process_id() {
            debug!("terminating PTY child {pid}");
        }
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
