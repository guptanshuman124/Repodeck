//! Creating a new SSH key for an account, and trusting github.com's host key.
//! Keys are only ever added (never overwritten); `~/.ssh/config` is never touched.

use crate::error::{AppError, AppResult};
use crate::git;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// GitHub's published SSH host key fingerprints (https://api.github.com/meta).
const GITHUB_FINGERPRINTS: &[&str] = &[
    "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU",
    "SHA256:p2QAMXNIC1TJYWeIOttrVc98/R1BUFWu3/LiyKgUfQM",
    "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s",
];

#[derive(Debug, Serialize)]
pub struct GeneratedKey {
    /// `~/.ssh/...` form, as shown in the account form.
    pub private_path: String,
    pub public_key: String,
    pub fingerprint: String,
}

fn ssh_dir() -> AppResult<PathBuf> {
    crate::scanner::home()
        .map(|h| h.join(".ssh"))
        .ok_or_else(|| AppError::msg("Could not find your home folder."))
}

/// "Work (Acme)" → "work_acme"
fn slug(label: &str) -> String {
    let mut s = String::new();
    for ch in label.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch);
        } else if !s.ends_with('_') {
            s.push('_');
        }
    }
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "repodeck".into() } else { s }
}

/// First `id_ed25519_<slug>[_N]` where neither the key nor its `.pub` exists.
fn free_name(dir: &Path, label: &str) -> PathBuf {
    let base = format!("id_ed25519_{}", slug(label));
    (1..)
        .map(|n| {
            if n == 1 {
                dir.join(&base)
            } else {
                dir.join(format!("{base}_{n}"))
            }
        })
        .find(|p| !p.exists() && !p.with_extension("pub").exists())
        .expect("unbounded range")
}

fn fingerprints(pub_file: &Path) -> AppResult<Vec<String>> {
    let mut c = git::ssh_keygen();
    c.arg("-lf").arg(pub_file);
    let o = git::run_limited(c, Duration::from_secs(15), None)?;
    if !o.ok() {
        return Err(AppError::msg(format!(
            "ssh-keygen could not read the key: {}",
            o.stderr.trim()
        )));
    }
    // "256 SHA256:abc… comment (ED25519)"
    Ok(o.stdout
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
        .collect())
}

pub fn generate(label: &str, email: &str) -> AppResult<GeneratedKey> {
    let dir = ssh_dir()?;
    let mut key = generate_in(&dir, label, email)?;
    if let Some(name) = Path::new(&key.private_path).file_name() {
        key.private_path = format!("~/.ssh/{}", name.to_string_lossy());
    }
    Ok(key)
}

fn generate_in(dir: &Path, label: &str, email: &str) -> AppResult<GeneratedKey> {
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let path = free_name(dir, label);
    let mut c = git::ssh_keygen();
    c.args(["-q", "-t", "ed25519", "-N", "", "-C"])
        .arg(email.trim())
        .arg("-f")
        .arg(&path);
    let o = git::run_limited(c, Duration::from_secs(30), None)?;
    if !o.ok() {
        return Err(AppError::msg(format!(
            "ssh-keygen failed: {}",
            o.stderr.trim()
        )));
    }
    let pub_path = path.with_extension("pub");
    let public_key = std::fs::read_to_string(&pub_path)?.trim().to_string();
    let fingerprint = fingerprints(&pub_path)?
        .into_iter()
        .next()
        .unwrap_or_default();
    Ok(GeneratedKey {
        private_path: path.to_string_lossy().into_owned(),
        public_key,
        fingerprint,
    })
}

#[derive(Debug, Serialize)]
pub struct SshKey {
    /// `~/.ssh/<name>` form, ready to store on an account.
    pub path: String,
    pub name: String,
    pub fingerprint: Option<String>,
    /// The key's comment, usually the email it was made for.
    pub comment: Option<String>,
    /// "ED25519", "RSA", …
    pub kind: Option<String>,
    pub has_public: bool,
    pub in_agent: bool,
}

/// Private keys in `~/.ssh`. Only names, fingerprints and comments are read: never key material.
pub fn list_keys() -> AppResult<Vec<SshKey>> {
    let dir = ssh_dir()?;
    let mut keys = list_keys_in(&dir)?;
    let agent = agent_fingerprints();
    for k in &mut keys {
        k.in_agent = k.fingerprint.as_ref().is_some_and(|f| agent.contains(f));
        k.path = format!("~/.ssh/{}", k.name);
    }
    Ok(keys)
}

fn list_keys_in(dir: &Path) -> AppResult<Vec<SshKey>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_file() || p.extension().is_some_and(|x| x == "pub") {
            continue;
        }
        let mut head = [0u8; 64];
        let n = std::fs::File::open(&p)
            .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
            .unwrap_or(0);
        let head = String::from_utf8_lossy(&head[..n]);
        if !(head.starts_with("-----BEGIN") && head.contains("PRIVATE KEY")) {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let pub_path = p.with_file_name(format!("{name}.pub"));
        let has_public = pub_path.is_file();
        // "256 SHA256:abc… comment words (ED25519)"
        let line = has_public
            .then(|| {
                let mut c = git::ssh_keygen();
                c.arg("-lf").arg(&pub_path);
                git::run_limited(c, Duration::from_secs(10), None).ok()
            })
            .flatten()
            .filter(|o| o.ok())
            .map(|o| o.stdout.trim().to_string());
        let (fingerprint, comment, kind) = match line.as_deref().map(parse_fingerprint_line) {
            Some((f, c, k)) => (Some(f), c, k),
            None => (None, None, None),
        };
        out.push(SshKey {
            path: p.to_string_lossy().into_owned(),
            name,
            fingerprint,
            comment,
            kind,
            has_public,
            in_agent: false,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// "256 SHA256:abc you@x.com (ED25519)" → (fingerprint, comment, kind)
fn parse_fingerprint_line(l: &str) -> (String, Option<String>, Option<String>) {
    let mut parts = l.splitn(3, ' ');
    let _bits = parts.next();
    let fp = parts.next().unwrap_or_default().to_string();
    let rest = parts.next().unwrap_or_default().trim();
    let (comment, kind) = match rest.rfind(" (") {
        Some(i) if rest.ends_with(')') => {
            (&rest[..i], Some(rest[i + 2..rest.len() - 1].to_string()))
        }
        _ => (rest, None),
    };
    let comment = (!comment.is_empty() && comment != "no comment").then(|| comment.to_string());
    (fp, comment, kind)
}

fn agent_fingerprints() -> Vec<String> {
    let mut c = git::ssh_add();
    c.arg("-l");
    git::run_limited(c, Duration::from_secs(5), None)
        .ok()
        .filter(|o| o.ok())
        .map(|o| {
            o.stdout
                .lines()
                .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Is this file a public key? By name (`.pub`) or by content (`ssh-ed25519 AAAA…`).
pub fn is_public_key(path: &Path) -> bool {
    if path.extension().is_some_and(|e| e == "pub") {
        return true;
    }
    let mut head = [0u8; 32];
    let n = std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .unwrap_or(0);
    let head = String::from_utf8_lossy(&head[..n]);
    ["ssh-", "ecdsa-", "sk-ssh-", "sk-ecdsa-"]
        .iter()
        .any(|p| head.starts_with(p))
}

/// For a `.pub` path, the private key next to it, if there is one.
pub fn private_for(public: &Path) -> Option<PathBuf> {
    let name = public.file_name()?.to_string_lossy();
    let stem = name.strip_suffix(".pub")?;
    let p = public.with_file_name(stem);
    (p.is_file() && !is_public_key(&p)).then_some(p)
}

/// Health of an SSH key path, offline: (kind, text, fix_path).
/// kind: "ok", "no_key", "missing" or "public_key".
pub fn key_problem(key_path: Option<&str>) -> (&'static str, String, Option<String>) {
    let Some(k) = key_path.map(str::trim).filter(|k| !k.is_empty()) else {
        return ("no_key", "No SSH key set.".into(), None);
    };
    let p = crate::scanner::expand_tilde(k);
    if !p.is_file() {
        return ("missing", format!("Key file not found: {k}"), None);
    }
    if is_public_key(&p) {
        let fix = private_for(&p).map(|f| {
            // Keep the `~/` form when the user wrote it that way.
            match (k.starts_with("~/"), f.file_name()) {
                (true, Some(n)) => {
                    let parent = Path::new(k).parent().unwrap_or(Path::new("~"));
                    parent.join(n).to_string_lossy().into_owned()
                }
                _ => f.to_string_lossy().into_owned(),
            }
        });
        let text = match &fix {
            Some(f) => format!("This is the public key. The private key is {f}."),
            None => "This is the public key, and no private key was found next to it.".into(),
        };
        return ("public_key", text, fix);
    }
    ("ok", String::new(), None)
}

/// Make a key pair in `dir` for other modules' tests; returns the private key path.
#[cfg(test)]
pub fn generate_for_test(dir: &Path, label: &str) -> String {
    generate_in(dir, label, "test@example.com")
        .unwrap()
        .private_path
}

fn matches_published(fp: &str) -> bool {
    GITHUB_FINGERPRINTS.contains(&fp)
}

/// Add github.com's host keys to `~/.ssh/known_hosts`, but only if every key the server
/// offers matches GitHub's published fingerprints. Does nothing if github.com is already known.
pub fn trust_github() -> AppResult<()> {
    let known = ssh_dir()?.join("known_hosts");
    if known.exists() {
        let mut c = git::ssh_keygen();
        c.args(["-F", "github.com", "-f"]).arg(&known);
        if git::run_limited(c, Duration::from_secs(15), None)?.ok() {
            return Ok(());
        }
    }

    let mut c = git::ssh_keyscan();
    c.args(["-T", "10", "-t", "ed25519,ecdsa,rsa", "github.com"]);
    let o = git::run_limited(c, Duration::from_secs(20), None)?;
    let lines: Vec<&str> = o
        .stdout
        .lines()
        .filter(|l| l.starts_with("github.com "))
        .collect();
    if lines.is_empty() {
        return Err(AppError::msg(
            "Could not reach github.com to read its host key. Check your connection.",
        ));
    }

    let tmp = std::env::temp_dir().join(format!("repodeck-hostkeys-{}", std::process::id()));
    std::fs::write(&tmp, lines.join("\n") + "\n")?;
    let fps = fingerprints(&tmp);
    let _ = std::fs::remove_file(&tmp);
    let fps = fps?;
    if fps.len() != lines.len() || !fps.iter().all(|f| matches_published(f)) {
        return Err(AppError::msg(
            "github.com's host key didn't match GitHub's published fingerprints, so it was not trusted.",
        ));
    }

    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let dir = ssh_dir()?;
    std::fs::create_dir_all(&dir)?;
    let mut f = opts.open(&known)?;
    // Start on a fresh line even if the file doesn't end with one.
    let needs_newline = std::fs::read(&known)
        .map(|b| b.last().is_some_and(|c| *c != b'\n'))
        .unwrap_or(false);
    if needs_newline {
        writeln!(f)?;
    }
    for l in lines {
        writeln!(f, "{l}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Work"), "work");
        assert_eq!(slug("  Work (Acme) "), "work_acme");
        assert_eq!(slug("Été!!"), "t");
        assert_eq!(slug("***"), "repodeck");
    }

    #[test]
    fn detects_public_keys() {
        let dir = std::env::temp_dir().join(format!("repodeck-pub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let k = generate_in(&dir, "Home", "a@b.c").unwrap();
        let private = PathBuf::from(&k.private_path);
        let public = private.with_extension("pub");
        assert!(!is_public_key(&private));
        assert!(is_public_key(&public));
        assert_eq!(private_for(&public), Some(private.clone()));

        // A public key saved without the .pub name is still detected by its content.
        let renamed = dir.join("looks_private");
        std::fs::copy(&public, &renamed).unwrap();
        assert!(is_public_key(&renamed));

        assert_eq!(key_problem(Some(private.to_str().unwrap())).0, "ok");
        let (kind, _, fix) = key_problem(Some(public.to_str().unwrap()));
        assert_eq!((kind, fix.as_deref()), ("public_key", private.to_str()));
        assert_eq!(
            key_problem(Some(dir.join("nope").to_str().unwrap())).0,
            "missing"
        );
        assert_eq!(key_problem(None).0, "no_key");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_private_keys_only() {
        let dir = std::env::temp_dir().join(format!("repodeck-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        generate_in(&dir, "Work", "me@work.example").unwrap();
        let lone = generate_in(&dir, "Lone", "lone@x.y").unwrap();
        std::fs::remove_file(format!("{}.pub", lone.private_path)).unwrap();
        std::fs::write(dir.join("known_hosts"), "github.com ssh-ed25519 AAAA\n").unwrap();
        std::fs::write(dir.join("stray.pub"), "ssh-ed25519 AAAA x\n").unwrap();

        let keys = list_keys_in(&dir).unwrap();
        let names: Vec<_> = keys.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, ["id_ed25519_lone", "id_ed25519_work"]);
        let work = &keys[1];
        assert!(work.has_public);
        assert_eq!(work.comment.as_deref(), Some("me@work.example"));
        assert_eq!(work.kind.as_deref(), Some("ED25519"));
        assert!(work.fingerprint.as_deref().unwrap().starts_with("SHA256:"));
        assert!(!keys[0].has_public && keys[0].fingerprint.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_fingerprint_lines() {
        assert_eq!(
            parse_fingerprint_line("256 SHA256:abc Ada Work <a@b.c> (ED25519)"),
            (
                "SHA256:abc".into(),
                Some("Ada Work <a@b.c>".into()),
                Some("ED25519".into())
            )
        );
        assert_eq!(
            parse_fingerprint_line("3072 SHA256:x no comment (RSA)").1,
            None
        );
    }

    #[test]
    fn published_fingerprints() {
        assert!(matches_published(
            "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU"
        ));
        assert!(!matches_published(
            "SHA256:WMPVQtvtnjLBqMYqBM0Ks7r79tLOLlOYV2ONZZcnvcM"
        ));
    }

    #[test]
    fn generates_without_overwriting() {
        let dir = std::env::temp_dir().join(format!("repodeck-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let a = generate_in(&dir, "Work", "ada@work.example").unwrap();
        assert!(a.private_path.ends_with("id_ed25519_work"));
        assert!(a.public_key.starts_with("ssh-ed25519 "));
        assert!(a.public_key.ends_with("ada@work.example"));
        assert!(a.fingerprint.starts_with("SHA256:"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a.private_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o077,
                0,
                "private key must not be group/world readable"
            );
        }
        let before = std::fs::read(&a.private_path).unwrap();

        let b = generate_in(&dir, "Work", "ada@work.example").unwrap();
        assert!(b.private_path.ends_with("id_ed25519_work_2"));
        assert_eq!(std::fs::read(&a.private_path).unwrap(), before);
        assert_ne!(a.fingerprint, b.fingerprint);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
