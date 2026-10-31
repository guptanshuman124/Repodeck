//! Parse remote URLs into host/owner/repo and resolve SSH host aliases.

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Parsed {
    /// Host as written in the URL (may be an SSH alias).
    pub host: String,
    /// Real host after resolving SSH aliases.
    pub real_host: String,
    pub owner: String,
    pub repo: String,
    pub is_ssh: bool,
}

const KNOWN_HOSTS: &[&str] = &["github.com", "gitlab.com", "bitbucket.org"];

pub fn parse(url: &str) -> Option<Parsed> {
    let url = url.trim();
    let (host, path, is_ssh) = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next()?.to_string(), path.to_string(), false)
    } else if let Some(rest) = url
        .strip_prefix("ssh://")
        .or_else(|| url.strip_prefix("git+ssh://"))
    {
        let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next()?.to_string(), path.to_string(), true)
    } else if !url.contains("://") && url.contains(':') {
        // scp-like: [user@]host:owner/repo.git
        let (left, path) = url.split_once(':')?;
        if left.len() == 1 {
            return None; // Windows drive letter, a local path
        }
        let host = left.rsplit_once('@').map(|(_, h)| h).unwrap_or(left);
        (host.to_string(), path.to_string(), true)
    } else {
        return None;
    };
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.rsplit_once('/')?;
    if owner.is_empty() || repo.is_empty() || host.is_empty() {
        return None;
    }
    let real_host = if is_ssh {
        resolve_ssh_host(&host)
    } else {
        host.clone()
    };
    Some(Parsed {
        host,
        real_host,
        owner: owner.to_string(),
        repo: repo.to_string(),
        is_ssh,
    })
}

/// `ssh -G alias` prints the effective config without connecting. Cached per alias.
pub fn resolve_ssh_host(alias: &str) -> String {
    if KNOWN_HOSTS.contains(&alias) {
        return alias.to_string();
    }
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(h) = cache.lock().get(alias) {
        return h.clone();
    }
    let mut c = crate::git::ssh();
    c.arg("-G").arg(alias);
    let resolved = crate::git::run(c)
        .ok()
        .filter(|o| o.ok())
        .and_then(|o| {
            o.stdout
                .lines()
                .find_map(|l| l.strip_prefix("hostname ").map(|h| h.trim().to_string()))
        })
        .unwrap_or_else(|| alias.to_string());
    cache.lock().insert(alias.to_string(), resolved.clone());
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        let h = parse("https://github.com/acme/tool.git").unwrap();
        assert_eq!(
            (h.host.as_str(), h.owner.as_str(), h.repo.as_str(), h.is_ssh),
            ("github.com", "acme", "tool", false)
        );
        let s = parse("git@github.com:acme/tool.git").unwrap();
        assert_eq!(
            (s.owner.as_str(), s.repo.as_str(), s.is_ssh),
            ("acme", "tool", true)
        );
        let u = parse("ssh://git@github.com:22/acme/tool").unwrap();
        assert_eq!((u.host.as_str(), u.repo.as_str()), ("github.com", "tool"));
        let t = parse("https://user:tok@github.com/acme/tool").unwrap();
        assert_eq!(t.owner, "acme");
        assert!(parse("C:\\work\\repo").is_none());
        assert!(parse("/srv/git/repo.git").is_none());
    }
}
