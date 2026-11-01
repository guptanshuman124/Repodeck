//! Per-repo inspection: cheap fingerprints decide staleness, then git resolves
//! identity, remotes and branches exactly as a commit or push would.

use crate::git;
use serde::{Deserialize, Serialize};
use std::hash::Hasher;
use std::path::{Path, PathBuf};

pub struct GitPaths {
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

pub fn git_paths(repo: &Path, bare: bool) -> Option<GitPaths> {
    if bare {
        return Some(GitPaths {
            git_dir: repo.to_path_buf(),
            common_dir: repo.to_path_buf(),
        });
    }
    let dotgit = repo.join(".git");
    let git_dir = if dotgit.is_dir() {
        dotgit
    } else {
        let s = std::fs::read_to_string(&dotgit).ok()?;
        let rel = s.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
        let p = PathBuf::from(rel);
        if p.is_absolute() { p } else { repo.join(p) }
    };
    let common_dir = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(s) => {
            let p = PathBuf::from(s.trim());
            if p.is_absolute() { p } else { git_dir.join(p) }
        }
        Err(_) => git_dir.clone(),
    };
    Some(GitPaths {
        git_dir,
        common_dir,
    })
}

/// FNV-1a; stable across runs, unlike the std hasher.
struct Fnv(u64);
impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

fn stat_into(h: &mut Fnv, p: &Path) {
    match std::fs::metadata(p) {
        Ok(m) => {
            let mt = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            h.write(&mt.to_le_bytes());
            h.write(&m.len().to_le_bytes());
        }
        Err(_) => h.write(b"-"),
    }
}

fn refs_into(h: &mut Fnv, dir: &Path, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        h.write(e.file_name().as_encoded_bytes());
        if p.is_dir() {
            if depth < 8 {
                refs_into(h, &p, depth + 1);
            }
        } else {
            stat_into(h, &p);
        }
    }
}

/// Modification times and sizes of HEAD, index, config, packed-refs and refs/.
pub fn fingerprint(paths: &GitPaths) -> String {
    let mut h = Fnv(0xcbf29ce484222325);
    stat_into(&mut h, &paths.git_dir.join("HEAD"));
    stat_into(&mut h, &paths.git_dir.join("index"));
    stat_into(&mut h, &paths.common_dir.join("config"));
    stat_into(&mut h, &paths.common_dir.join("packed-refs"));
    refs_into(&mut h, &paths.common_dir.join("refs"), 0);
    format!("{:016x}", h.finish())
}

pub fn global_config_files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = crate::scanner::home() {
        v.push(h.join(".gitconfig"));
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| h.join(".config"));
        v.push(xdg.join("git").join("config"));
    }
    v
}

/// Covers the global config files and the files their `[include]` / `[includeIf]` sections pull
/// in, so editing e.g. `~/.gitconfig-work` marks every repo stale.
pub fn global_fingerprint() -> String {
    let mut h = Fnv(0xcbf29ce484222325);
    for f in global_config_files() {
        stat_into(&mut h, &f);
        for inc in include_paths(&f) {
            stat_into(&mut h, &inc);
        }
    }
    format!("{:016x}", h.finish())
}

/// `path = …` entries from include sections of one config file (one level deep).
fn include_paths(config: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(config) else {
        return Vec::new();
    };
    let base = config.parent().unwrap_or(Path::new("."));
    let mut in_include = false;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            in_include = header
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("include");
        } else if in_include {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if k.trim().eq_ignore_ascii_case("path") {
                let v = v.trim().trim_matches('"');
                let p = crate::scanner::expand_tilde(v);
                out.push(if p.is_absolute() { p } else { base.join(p) });
            }
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub scope: String,
    pub origin_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub name: String,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub gone: bool,
    pub last_commit_at: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct Inspection {
    pub config: Vec<ConfigEntry>,
    pub remotes: Vec<(String, String)>,
    pub branches: Vec<Branch>,
    pub head_branch: Option<String>,
    pub is_dirty: Option<bool>,
    pub last_commit_at: Option<i64>,
    pub recent_emails: Vec<String>,
}

/// `git config --show-origin --show-scope` for identity, ssh command, credential username and remotes.
pub fn read_config(repo: &Path) -> Result<Vec<ConfigEntry>, String> {
    let mut c = git::git(repo);
    c.args([
        "config",
        "--show-origin",
        "--show-scope",
        "--get-regexp",
        r"^(user\.(name|email)|core\.sshcommand|credential\..*\.?username|remote\..*\.url)$",
    ]);
    let o = git::run(c).map_err(|e| e.to_string())?;
    // Exit 1 just means "no matching keys".
    if !o.ok() && o.code != Some(1) {
        return Err(o.stderr.trim().to_string());
    }
    let globals: Vec<String> = global_config_files()
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    Ok(o.stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let scope = parts.next()?.to_string();
            let origin = parts
                .next()?
                .strip_prefix("file:")
                .unwrap_or("")
                .to_string();
            let kv = parts.next()?;
            let (key, value) = kv.split_once(' ').unwrap_or((kv, ""));
            let normalized = origin.replace('\\', "/");
            // Values pulled in by include/includeIf report the scope of the including file.
            let included = match scope.as_str() {
                "global" => !globals.iter().any(|g| g == &normalized),
                "local" => !(normalized == "config" || normalized.ends_with("/config")),
                _ => false,
            };
            let scope = if included {
                "includeIf".to_string()
            } else {
                scope
            };
            Some(ConfigEntry {
                key: key.to_string(),
                value: value.to_string(),
                scope,
                origin_file: origin,
            })
        })
        .collect())
}

fn read_branches(repo: &Path) -> Vec<Branch> {
    let mut c = git::git(repo);
    c.args([
        "for-each-ref",
        "--format=%(HEAD)%00%(refname:short)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(committerdate:unix)",
        "refs/heads",
    ]);
    let Ok(o) = git::run(c) else { return vec![] };
    o.stdout
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\0').collect();
            if f.len() < 5 {
                return None;
            }
            let track = f[3];
            let num = |word: &str| {
                track.split(", ").find_map(|p| {
                    p.strip_prefix(word)
                        .and_then(|n| n.trim().parse::<u32>().ok())
                })
            };
            let has_up = !f[2].is_empty();
            Some(Branch {
                name: f[1].to_string(),
                is_head: f[0] == "*",
                upstream: has_up.then(|| f[2].to_string()),
                ahead: if has_up && track != "gone" {
                    Some(num("ahead ").unwrap_or(0))
                } else {
                    None
                },
                behind: if has_up && track != "gone" {
                    Some(num("behind ").unwrap_or(0))
                } else {
                    None
                },
                gone: track == "gone",
                last_commit_at: f[4].parse().ok(),
            })
        })
        .collect()
}

fn read_head(paths: &GitPaths) -> Option<String> {
    let s = std::fs::read_to_string(paths.git_dir.join("HEAD")).ok()?;
    s.trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
}

pub fn inspect(repo: &Path, bare: bool) -> Result<Inspection, String> {
    let paths = git_paths(repo, bare).ok_or("not a git repository")?;
    let config = read_config(repo)?;
    let remotes = config
        .iter()
        .filter_map(|e| {
            let name = e.key.strip_prefix("remote.")?.strip_suffix(".url")?;
            Some((name.to_string(), e.value.clone()))
        })
        .collect();
    let branches = read_branches(repo);
    let head_branch = read_head(&paths);

    let is_dirty = if bare {
        None
    } else {
        let mut c = git::git(repo);
        c.args(["status", "--porcelain", "--untracked-files=no"]);
        git::run(c)
            .ok()
            .filter(|o| o.ok())
            .map(|o| !o.stdout.trim().is_empty())
    };

    let mut c = git::git(repo);
    c.args(["log", "-n", "30", "--format=%ae%x09%ct", "HEAD", "--"]);
    let (mut recent_emails, mut last_commit_at) = (Vec::<String>::new(), None);
    if let Ok(o) = git::run(c) {
        for (i, l) in o.stdout.lines().enumerate() {
            let (email, ts) = l.split_once('\t').unwrap_or((l, ""));
            if i == 0 {
                last_commit_at = ts.parse().ok();
            }
            let email = email.to_lowercase();
            if !recent_emails.contains(&email) {
                recent_emails.push(email);
            }
        }
    }
    Ok(Inspection {
        config,
        remotes,
        branches,
        head_branch,
        is_dirty,
        last_commit_at,
        recent_emails,
    })
}

/// The value git will actually use: the last entry wins.
pub fn effective<'a>(config: &'a [ConfigEntry], key: &str) -> Option<&'a ConfigEntry> {
    config
        .iter()
        .rev()
        .find(|e| e.key.eq_ignore_ascii_case(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_include_paths() {
        let dir = std::env::temp_dir().join(format!("repodeck-inc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("gitconfig");
        // An absolute path on this OS (C:\… on Windows), written with forward slashes as git
        // configs do, since `\` is an escape character there.
        let abs = dir.join("abs").join("work.inc");
        let abs_in_config = abs.to_string_lossy().replace('\\', "/");
        std::fs::write(
            &cfg,
            format!(
                "[user]\n\tname = A\n\tpath = not-an-include\n\
                 [includeIf \"gitdir:~/work/\"]\n\tpath = \"{abs_in_config}\"\n\
                 [include]\n\tpath = relative.inc\n"
            ),
        )
        .unwrap();
        let got = include_paths(&cfg);
        assert_eq!(got, vec![abs, dir.join("relative.inc")]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
