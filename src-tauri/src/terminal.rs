//! Built-in terminal: one shell per PTY, started in a repo's folder. Output goes to the UI as raw
//! bytes (the UI decodes them, so UTF-8 split across reads is fine); input, resizes and kills
//! come back through commands keyed by session id.

use crate::error::{AppError, AppResult};
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

#[derive(Default)]
pub struct Terminals {
    sessions: Mutex<HashMap<u32, Session>>,
    next: AtomicU32,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(2),
        cols: cols.max(10),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// The program and arguments to run. An explicit setting is used as written; otherwise the
/// user's login shell (Unix) or PowerShell (Windows).
fn shell_command(setting: &str) -> CommandBuilder {
    let mut parts = setting.split_whitespace();
    if let Some(program) = parts.next() {
        let mut c = CommandBuilder::new(program);
        c.args(parts);
        return c;
    }
    #[cfg(windows)]
    {
        let mut c = CommandBuilder::new("powershell.exe");
        c.arg("-NoLogo");
        c
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if cfg!(target_os = "macos") {
                    "/bin/zsh".into()
                } else {
                    "/bin/bash".into()
                }
            });
        let mut c = CommandBuilder::new(shell);
        // Login shell, so the user's profile (PATH, prompt, nvm, …) loads as in their terminal.
        c.arg("-l");
        c
    }
}

impl Terminals {
    /// Starts a shell in `cwd`. `on_data` gets every chunk of output; `on_exit` gets the exit
    /// code once the shell has ended and its output has been delivered.
    pub fn open(
        self: &Arc<Self>,
        cwd: &Path,
        shell: &str,
        cols: u16,
        rows: u16,
        on_data: impl Fn(Vec<u8>) + Send + 'static,
        on_exit: impl FnOnce(u32, u32) + Send + 'static,
    ) -> AppResult<u32> {
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .map_err(fail)?;
        let mut cmd = shell_command(shell);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "RepoDeck");
        #[cfg(not(windows))]
        if std::env::var_os("LANG").is_none() {
            // Apps started from the Dock/Finder get no locale, which breaks UTF-8 in shells.
            cmd.env("LANG", "en_US.UTF-8");
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(fail)?;
        // The child holds its own copy; keeping ours would stop the reader ever seeing EOF.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(fail)?;
        let writer = pair.master.take_writer().map_err(fail)?;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.sessions.lock().insert(
            id,
            Session {
                master: pair.master,
                writer,
                killer: child.clone_killer(),
            },
        );

        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name(format!("term-read-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => on_data(buf[..n].to_vec()),
                    }
                }
                let _ = done_tx.send(());
            })
            .map_err(fail)?;

        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name(format!("term-wait-{id}"))
            .spawn(move || {
                let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
                // Let the last output arrive first. On Windows the reader only ends once the
                // PTY is closed, hence the time limit before closing it ourselves.
                if done_rx.recv_timeout(Duration::from_millis(300)).is_err() {
                    me.sessions.lock().remove(&id);
                    let _ = done_rx.recv_timeout(Duration::from_millis(300));
                }
                me.sessions.lock().remove(&id);
                on_exit(id, code);
            })
            .map_err(fail)?;
        Ok(id)
    }

    fn with<T>(&self, id: u32, f: impl FnOnce(&mut Session) -> AppResult<T>) -> AppResult<T> {
        let mut s = self.sessions.lock();
        let session = s
            .get_mut(&id)
            .ok_or_else(|| AppError::msg("This terminal has already ended."))?;
        f(session)
    }

    pub fn write(&self, id: u32, data: &[u8]) -> AppResult<()> {
        self.with(id, |s| {
            s.writer
                .write_all(data)
                .and_then(|_| s.writer.flush())
                .map_err(|e| AppError::msg(e.to_string()))
        })
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> AppResult<()> {
        self.with(id, |s| {
            s.master
                .resize(size(cols, rows))
                .map_err(|e| AppError::msg(e.to_string()))
        })
    }

    /// Ends the shell. Its exit is still reported through `on_exit`.
    pub fn close(&self, id: u32) {
        if let Some(mut s) = self.sessions.lock().remove(&id) {
            let _ = s.killer.kill();
        }
    }

    pub fn close_all(&self) {
        for (_, mut s) in self.sessions.lock().drain() {
            let _ = s.killer.kill();
        }
    }
}

fn fail(e: impl std::fmt::Display) -> AppError {
    AppError::msg(format!("Couldn't start the terminal: {e}"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn runs_in_folder_and_reports_exit() {
        let dir = std::env::temp_dir().join(format!("repodeck-term-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let t = Arc::new(Terminals::default());
        let out = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (exit_tx, exit_rx) = mpsc::channel();
        let sink = Arc::clone(&out);
        let id = t
            .open(
                &dir,
                "/bin/sh",
                80,
                24,
                move |b| sink.lock().extend(b),
                move |_, code| exit_tx.send(code).unwrap(),
            )
            .unwrap();
        t.write(id, b"pwd; exit 3\n").unwrap();
        let code = exit_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(code, 3);
        let text = String::from_utf8_lossy(&out.lock()).to_string();
        assert!(text.contains(dir.to_str().unwrap()), "output was: {text}");
        assert!(t.write(id, b"x").is_err(), "session should be gone");
        std::fs::remove_dir_all(&dir).ok();
    }
}
