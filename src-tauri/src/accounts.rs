//! Applying an account to a repo (repo-local config only) and checking push access.

use crate::git;
use crate::remote_url;
use crate::store::Account;
use serde::Serialize;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[derive(Debug, Serialize, serde::Deserialize, Clone)]
pub struct ConfigChange {
    pub key: String,
    pub current: Option<String>,
    pub next: Option<String>,
}

fn ssh_command_for(key_path: &str) -> String {
    let p = crate::scanner::expand_tilde(key_path)
        .to_string_lossy()
        .replace('\\', "/");
    format!("ssh -i \"{p}\" -o IdentitiesOnly=yes")
}

/// The repo-local keys an account controls, and the values it wants.
fn desired(repo_host: &str, account: Option<&Account>) -> Vec<(String, Option<String>)> {
    let cred_key = format!("credential.https://{repo_host}.username");
    match account {
        None => vec![
            ("user.name".into(), None),
            ("user.email".into(), None),
            ("core.sshCommand".into(), None),
            (cred_key, None),
        ],
        Some(a) => {
            let ssh = (a.auth_method == "ssh")
                .then(|| a.ssh_key_path.as_deref().map(ssh_command_for))
                .flatten();
            let user = (a.auth_method == "https")
                .then(|| a.expected_username.clone())
                .flatten();
            vec![
                ("user.name".into(), Some(a.user_name.clone())),
                ("user.email".into(), Some(a.user_email.clone())),
                ("core.sshCommand".into(), ssh),
                (cred_key, user),
            ]
        }
    }
}

fn local_get(repo: &Path, key: &str) -> Option<String> {
    let mut c = git::git(repo);
    c.args(["config", "--local", "--get", key]);
    git::run(c)
        .ok()
        .filter(|o| o.ok())
        .map(|o| o.stdout.trim_end_matches(['\n', '\r']).to_string())
}

/// Exactly what would change in this repo's .git/config. Unchanged keys are omitted.
pub fn preview(repo: &Path, repo_host: &str, account: Option<&Account>) -> Vec<ConfigChange> {
    desired(repo_host, account)
        .into_iter()
        .filter_map(|(key, next)| {
            let current = local_get(repo, &key);
            (current != next).then_some(ConfigChange { key, current, next })
        })
        .collect()
}

pub fn apply(repo: &Path, changes: &[ConfigChange]) -> Result<(), String> {
    for ch in changes {
        let mut c = git::git(repo);
        match &ch.next {
            Some(v) => c.args(["config", "--local", &ch.key, v]),
            None => c.args(["config", "--local", "--unset-all", &ch.key]),
        };
        let o = git::run(c).map_err(|e| e.to_string())?;
        // Exit 5 on --unset-all means the key was already absent.
        if !o.ok() && !(ch.next.is_none() && o.code == Some(5)) {
            return Err(format!("git config {} failed: {}", ch.key, o.stderr.trim()));
        }
    }
    Ok(())
}

pub fn command_lines(changes: &[ConfigChange]) -> Vec<String> {
    changes
        .iter()
        .map(|c| match &c.next {
            Some(v) => format!(
                "git config --local {} \"{}\"",
                c.key,
                v.replace('"', "\\\"")
            ),
            None => format!("git config --local --unset-all {}", c.key),
        })
        .collect()
}

#[derive(Debug, Serialize, Clone)]
pub struct CheckResult {
    pub status: &'static str,
    pub detail: String,
    pub username: Option<String>,
}

fn batch_ssh(key: &str) -> String {
    let p = crate::scanner::expand_tilde(key)
        .to_string_lossy()
        .replace('\\', "/");
    format!("ssh -i \"{p}\" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15")
}

/// Step 1 for SSH accounts: which GitHub user does this key belong to?
pub fn whoami(account: &Account, cancel: Option<&AtomicBool>) -> CheckResult {
    let Some(key) = account.ssh_key_path.as_deref() else {
        return CheckResult {
            status: "unknown",
            detail: "No SSH key set.".into(),
            username: None,
        };
    };
    let host = remote_url::resolve_ssh_host(&account.host);
    let mut c = git::ssh();
    c.args(["-T", "-i"])
        .arg(crate::scanner::expand_tilde(key))
        .args([
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
        ])
        .arg(format!("git@{host}"));
    let o = match git::run_limited(c, Duration::from_secs(30), cancel) {
        Ok(o) => o,
        Err(e) => {
            return CheckResult {
                status: "unknown",
                detail: format!("Could not run ssh: {e}"),
                username: None,
            };
        }
    };
    if o.timed_out {
        return CheckResult {
            status: "offline",
            detail: "ssh timed out.".into(),
            username: None,
        };
    }
    let text = format!("{}{}", o.stdout, o.stderr);
    // GitHub: "Hi USER! You've successfully authenticated..."
    if let Some(rest) = text.split("Hi ").nth(1)
        && let Some(user) = rest.split(['!', ' ']).next().filter(|u| !u.is_empty())
    {
        let user = user.split('/').next().unwrap_or(user).to_string(); // deploy keys say owner/repo
        if let Some(exp) = account
            .expected_username
            .as_deref()
            .filter(|e| !e.is_empty())
            && !exp.eq_ignore_ascii_case(&user)
        {
            return CheckResult {
                status: "wrong_account",
                detail: format!("This key signs in as {user}, not {exp}."),
                username: Some(user),
            };
        }
        return CheckResult {
            status: "can_push",
            detail: format!("Signed in as {user}."),
            username: Some(user),
        };
    }
    classify_failure(&text)
}

pub fn classify_failure(text: &str) -> CheckResult {
    let t = text.to_lowercase();
    let (status, hint) = if t.contains("host key verification failed") {
        (
            "host_untrusted",
            "Your Mac has never connected to this host over SSH, so it is not in ~/.ssh/known_hosts yet. Run `ssh -T git@github.com` in a terminal once and answer yes.",
        )
    } else if t.contains("permission denied (publickey")
        || t.contains("authentication failed")
        || t.contains("could not read username")
        || t.contains("terminal prompts disabled")
        || t.contains("invalid username or password")
    {
        ("auth_failed", "Sign-in failed.")
    } else if t.contains("repository not found")
        || t.contains("not found") && t.contains("repository")
    {
        ("no_access", "This account cannot see the repository.")
    } else if (t.contains("permission to") && t.contains("denied"))
        || t.contains("403")
        || t.contains("write access to repository not granted")
    {
        ("read_only", "This account can read but not push.")
    } else if t.contains("could not resolve host")
        || t.contains("network is unreachable")
        || t.contains("timed out")
        || t.contains("connection refused")
        || t.contains("no route to host")
    {
        ("offline", "Network unavailable.")
    } else {
        ("unknown", "Unrecognised response.")
    };
    let last = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("");
    CheckResult {
        status,
        detail: format!("{hint} {last}").trim().to_string(),
        username: None,
    }
}

/// The remote URL rewritten to the account's own auth method (SSH or HTTPS).
/// URLs that don't parse (local paths, other hosts' formats) are used as given.
pub fn url_for(url: &str, account: &Account) -> String {
    match (remote_url::parse(url), account.auth_method.as_str()) {
        (Some(p), "ssh") => format!("git@{}:{}/{}.git", p.real_host, p.owner, p.repo),
        (Some(p), _) => format!("https://{}/{}/{}.git", p.real_host, p.owner, p.repo),
        (None, _) => url.to_string(),
    }
}

/// Make a git command authenticate as `account`, without prompting.
pub fn with_auth(c: &mut Command, account: &Account) {
    if account.auth_method == "ssh" {
        if let Some(k) = account.ssh_key_path.as_deref() {
            c.env("GIT_SSH_COMMAND", batch_ssh(k));
        }
    } else if let Some(u) = account
        .expected_username
        .as_deref()
        .filter(|u| !u.is_empty())
    {
        c.arg("-c").arg(format!("credential.username={u}"));
    }
}

/// Can this account read the repo at `url`? `git ls-remote` only lists refs; nothing is written.
pub fn probe_access(url: &str, account: &Account, cancel: Option<&AtomicBool>) -> CheckResult {
    let mut c = git::git_global();
    with_auth(&mut c, account);
    c.args(["ls-remote", "--heads"]).arg(url_for(url, account));
    let o = match git::run_limited(c, Duration::from_secs(30), cancel) {
        Ok(o) => o,
        Err(e) => {
            return CheckResult {
                status: "unknown",
                detail: format!("Could not run git: {e}"),
                username: None,
            };
        }
    };
    if o.cancelled {
        return CheckResult {
            status: "unknown",
            detail: "Cancelled.".into(),
            username: None,
        };
    }
    if o.timed_out {
        return CheckResult {
            status: "offline",
            detail: "Timed out after 30 s.".into(),
            username: None,
        };
    }
    if o.ok() {
        return CheckResult {
            status: "can_read",
            detail: "This account can read the repository.".into(),
            username: None,
        };
    }
    let mut r = classify_failure(&o.stderr);
    // ls-remote on a path that isn't a repo says "does not appear to be a git repository".
    if r.status == "unknown" && o.stderr.contains("does not appear to be a git repository") {
        r.status = "no_access";
    }
    r
}

/// `-c key=value` arguments for `git clone`, which writes them into the new repo's
/// `.git/config` before fetching: the same keys an account switch writes.
pub fn clone_args(url: &str, account: &Account) -> Vec<String> {
    let host = remote_url::parse(url)
        .map(|p| p.real_host)
        .unwrap_or_else(|| account.host.clone());
    desired(&host, Some(account))
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| ["-c".to_string(), format!("{k}={v}")]))
        .flatten()
        .collect()
}

/// Step 2: `git push --dry-run` to a throwaway ref, as this account, over its own auth method.
/// Dry runs never write; GitHub checks write permission when receive-pack starts.
pub fn push_check(
    repo: &Path,
    remote_url: &str,
    account: &Account,
    whoami_result: Option<&CheckResult>,
    cancel: Option<&AtomicBool>,
) -> CheckResult {
    if let Some(w) = whoami_result
        && w.status != "can_push"
    {
        return w.clone();
    }
    let mut c = git::git(repo);
    with_auth(&mut c, account);
    c.args(["push", "--dry-run", "--porcelain", "--no-verify"])
        .arg(url_for(remote_url, account))
        .arg("HEAD:refs/heads/repodeck-push-check");
    let o = match git::run_limited(c, Duration::from_secs(60), cancel) {
        Ok(o) => o,
        Err(e) => {
            return CheckResult {
                status: "unknown",
                detail: format!("Could not run git: {e}"),
                username: None,
            };
        }
    };
    let username = whoami_result.and_then(|w| w.username.clone());
    if o.cancelled {
        return CheckResult {
            status: "unknown",
            detail: "Cancelled.".into(),
            username,
        };
    }
    if o.timed_out {
        return CheckResult {
            status: "offline",
            detail: "Timed out after 60 s.".into(),
            username,
        };
    }
    if o.ok() {
        let who = username
            .as_deref()
            .map(|u| format!(" as {u}"))
            .unwrap_or_default();
        return CheckResult {
            status: "can_push",
            detail: format!("Dry-run push accepted{who}."),
            username,
        };
    }
    let text = format!("{}{}", o.stdout, o.stderr);
    if text.contains("src refspec HEAD does not match") {
        return CheckResult {
            status: "unknown",
            detail: "The repo has no commits yet, so there is nothing to test-push.".into(),
            username,
        };
    }
    let mut r = classify_failure(&text);
    r.username = username;
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_github_errors() {
        assert_eq!(classify_failure("ERROR: Permission to acme/x.git denied to bob.\nfatal: Could not read from remote repository.").status, "read_only");
        assert_eq!(
            classify_failure("git@github.com: Permission denied (publickey).").status,
            "auth_failed"
        );
        assert_eq!(
            classify_failure("ERROR: Repository not found.").status,
            "no_access"
        );
        assert_eq!(classify_failure("remote: Write access to repository not granted.\nfatal: unable to access 'https://github.com/a/b/': The requested URL returned error: 403").status, "read_only");
        assert_eq!(
            classify_failure("ssh: Could not resolve hostname github.com").status,
            "offline"
        );
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn switch_inspect_and_dry_run_push() {
        let tmp = std::env::temp_dir().join(format!("repodeck-flow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        git(&tmp, &["init", "-q", "--bare", "remote.git"]);
        let repo = tmp.join("work");
        git(&tmp, &["init", "-q", "work"]);
        git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                tmp.join("remote.git").to_str().unwrap(),
            ],
        );
        git(
            &repo,
            &[
                "-c",
                "user.name=x",
                "-c",
                "user.email=x@x",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "one",
            ],
        );

        let acc = Account {
            id: Some(1),
            label: "Work".into(),
            user_name: "Ada".into(),
            user_email: "ada@work.example".into(),
            host: "github.com".into(),
            auth_method: "https".into(),
            ssh_key_path: None,
            expected_username: Some("ada-w".into()),
            owners: None,
            health: None,
        };
        let changes = preview(&repo, "github.com", Some(&acc));
        assert_eq!(changes.len(), 3, "{changes:?}"); // name, email, credential username
        apply(&repo, &changes).unwrap();
        assert!(preview(&repo, "github.com", Some(&acc)).is_empty());

        let ins = crate::inspect::inspect(&repo, false).unwrap();
        let email = crate::inspect::effective(&ins.config, "user.email").unwrap();
        assert_eq!(
            (email.value.as_str(), email.scope.as_str()),
            ("ada@work.example", "local")
        );
        assert_eq!(ins.remotes.len(), 1);
        assert_eq!(ins.recent_emails, vec!["x@x".to_string()]);
        assert_eq!(ins.is_dirty, Some(false));

        let url = tmp.join("remote.git").to_string_lossy().into_owned();
        let res = push_check(&repo, &url, &acc, None, None);
        assert_eq!(res.status, "can_push", "{}", res.detail);
        // A dry run must not create the ref.
        let refs = Command::new("git")
            .arg("-C")
            .arg(tmp.join("remote.git"))
            .args(["for-each-ref"])
            .output()
            .unwrap();
        assert!(refs.stdout.is_empty());

        let back = preview(&repo, "github.com", None);
        apply(&repo, &back).unwrap();
        assert!(local_get(&repo, "user.email").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn account(auth: &str) -> Account {
        Account {
            id: Some(1),
            label: "Work".into(),
            user_name: "Ada".into(),
            user_email: "ada@work.example".into(),
            host: "github.com".into(),
            auth_method: auth.into(),
            ssh_key_path: Some("~/.ssh/id_work".into()),
            expected_username: Some("ada-w".into()),
            owners: None,
            health: None,
        }
    }

    #[test]
    fn probe_and_clone_as_account() {
        let tmp = std::env::temp_dir().join(format!("repodeck-clone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        git(&tmp, &["init", "-q", "--bare", "remote.git"]);
        let remote = tmp.join("remote.git");
        let remote_s = remote.to_str().unwrap();
        git(&tmp, &["init", "-q", "seed"]);
        let seed = tmp.join("seed");
        git(
            &seed,
            &[
                "-c",
                "user.name=x",
                "-c",
                "user.email=x@x",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "one",
            ],
        );
        git(&seed, &["push", "-q", remote_s, "HEAD:refs/heads/main"]);

        let acc = account("https");
        assert_eq!(probe_access(remote_s, &acc, None).status, "can_read");
        let missing = tmp.join("missing.git");
        assert_eq!(
            probe_access(missing.to_str().unwrap(), &acc, None).status,
            "no_access"
        );

        // SSH accounts also pin their key in the clone's config.
        let ssh_args = clone_args("git@github.com:acme/app.git", &account("ssh")).join(" ");
        assert!(ssh_args.contains("core.sshCommand=ssh -i"), "{ssh_args}");

        let dest = tmp.join("clone");
        let mut c = git::git_global();
        with_auth(&mut c, &acc);
        c.arg("clone")
            .args(clone_args(remote_s, &acc))
            .arg("--")
            .arg(url_for(remote_s, &acc))
            .arg(&dest);
        assert!(git::run(c).unwrap().ok());

        let ins = crate::inspect::inspect(&dest, false).unwrap();
        for (key, want) in [("user.name", "Ada"), ("user.email", "ada@work.example")] {
            let e = crate::inspect::effective(&ins.config, key).unwrap();
            assert_eq!(
                (e.value.as_str(), e.scope.as_str()),
                (want, "local"),
                "{key}"
            );
        }
        // The clone already matches the account, so switching to it would change nothing.
        assert!(preview(&dest, "github.com", Some(&acc)).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
