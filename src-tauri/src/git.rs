//! Spawning the user's own `git` (and `ssh`). Every call is non-interactive,
//! uses the C locale so messages can be parsed, and never takes optional locks.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

fn base(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

pub fn git(dir: &Path) -> Command {
    let mut c = base("git");
    c.arg("-C").arg(dir);
    c
}

pub fn git_global() -> Command {
    base("git")
}

pub fn ssh() -> Command {
    base("ssh")
}

pub fn ssh_keygen() -> Command {
    base("ssh-keygen")
}

pub fn ssh_keyscan() -> Command {
    base("ssh-keyscan")
}

pub fn ssh_add() -> Command {
    base("ssh-add")
}

/// Run to completion with a quick path (no timeout).
pub fn run(mut cmd: Command) -> std::io::Result<Out> {
    let o = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    Ok(Out {
        code: o.status.code(),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        timed_out: false,
        cancelled: false,
    })
}

/// Run with a timeout and an optional cancel flag; the child is killed on either.
pub fn run_limited(
    mut cmd: Command,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> std::io::Result<Out> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = so.read_to_end(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = se.read_to_end(&mut s);
        s
    });
    let start = Instant::now();
    let (mut timed_out, mut cancelled) = (false, false);
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break Some(st);
        }
        if start.elapsed() >= timeout {
            timed_out = true;
        } else if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            cancelled = true;
        }
        if timed_out || cancelled {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    Ok(Out {
        code: status.and_then(|s| s.code()),
        stdout: String::from_utf8_lossy(&t_out.join().unwrap_or_default()).into_owned(),
        stderr: String::from_utf8_lossy(&t_err.join().unwrap_or_default()).into_owned(),
        timed_out,
        cancelled,
    })
}

/// Returns (major, minor, full string) when git is on PATH.
pub fn version() -> Option<(u32, u32, String)> {
    let mut c = git_global();
    c.arg("--version");
    let out = run(c).ok()?;
    if !out.ok() {
        return None;
    }
    let full = out.stdout.trim().to_string();
    let v = full.split_whitespace().nth(2)?;
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor, full))
}
