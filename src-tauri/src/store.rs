//! Reads and writes against the encrypted cache. The UI only ever sees data
//! that came through here.

use crate::db::{Db, now};
use crate::error::{AppError, AppResult};
use crate::graph::Row;
use crate::inspect::{ConfigEntry, Inspection};
use crate::scanner::Classified;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRoot {
    pub id: i64,
    pub path: String,
    pub max_depth: i64,
    pub follow_symlinks: bool,
    pub stop_at_repo: bool,
    pub enabled: bool,
    pub last_scanned_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Option<i64>,
    pub label: String,
    pub user_name: String,
    pub user_email: String,
    pub host: String,
    pub auth_method: String,
    pub ssh_key_path: Option<String>,
    pub expected_username: Option<String>,
    /// Comma-separated owners/orgs this account is expected for (mismatch rule).
    pub owners: Option<String>,
    /// Computed when listing; ignored when saving.
    #[serde(default, skip_deserializing)]
    pub health: Option<Health>,
}

/// Can this account sign in? Key problems are found offline; the last sign-in test is stored.
#[derive(Debug, Clone, Serialize)]
pub struct Health {
    /// "ok", "warn", "error" or "unknown" (never tested)
    pub level: &'static str,
    /// "ok", "untested", "no_key", "missing", "public_key" or a check status like "auth_failed"
    pub kind: String,
    pub text: String,
    pub username: Option<String>,
    pub checked_at: Option<i64>,
    /// For "public_key": the private key to use instead.
    pub fix_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteInfo {
    pub name: String,
    pub url: String,
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub is_github: bool,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub level: &'static str,
    pub text: String,
    pub fix: Option<Fix>,
}

/// One-click fix offered next to an issue.
#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    /// "switch", "link_switch", "edit_account", "repair_key", "trust_host" or "add_account"
    pub kind: &'static str,
    pub account_id: Option<i64>,
    pub label: String,
}

fn issue(level: &'static str, text: impl Into<String>, fix: Option<Fix>) -> Issue {
    Issue {
        level,
        text: text.into(),
        fix,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub parent_repo_id: Option<i64>,
    pub parent_name: Option<String>,
    pub root_id: Option<i64>,
    pub is_bare: bool,
    pub head_branch: Option<String>,
    pub is_dirty: Option<bool>,
    pub last_commit_at: Option<i64>,
    pub effective_name: Option<String>,
    pub effective_email: Option<String>,
    pub email_scope: Option<String>,
    pub active_account_id: Option<i64>,
    pub linked_account_ids: Vec<i64>,
    pub remote: Option<RemoteInfo>,
    pub last_fetch_at: Option<i64>,
    pub last_fetch_result: Option<String>,
    pub inspected: bool,
    pub status: &'static str,
    pub issues: Vec<Issue>,
    /// The account this repo most likely belongs to (owner rule, recent authors, single link).
    pub suggested_account_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushCheck {
    pub account_id: i64,
    pub status: String,
    pub detail: Option<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoDetail {
    pub row: RepoRow,
    pub config: Vec<ConfigEntry>,
    pub remotes: Vec<RemoteInfo>,
    pub branches: Vec<crate::inspect::Branch>,
    pub push_checks: Vec<PushCheck>,
    pub authors: Vec<(String, i64)>,
    pub inspect_error: Option<String>,
}

// ---------- roots and excludes ----------

pub fn list_roots(db: &Db) -> AppResult<Vec<ScanRoot>> {
    let c = db.conn.lock();
    let mut st = c.prepare(
        "SELECT id, path, max_depth, follow_symlinks, stop_at_repo, enabled, last_scanned_at FROM scan_roots ORDER BY path",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok(ScanRoot {
                id: r.get(0)?,
                path: r.get(1)?,
                max_depth: r.get(2)?,
                follow_symlinks: r.get(3)?,
                stop_at_repo: r.get(4)?,
                enabled: r.get(5)?,
                last_scanned_at: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn add_root(db: &Db, path: &str, stop_at_repo: bool) -> AppResult<i64> {
    let canon =
        dunce::canonicalize(path).map_err(|_| AppError::msg("That folder does not exist."))?;
    if !canon.is_dir() {
        return Err(AppError::msg("Scan roots must be folders."));
    }
    let p = canon.to_string_lossy().into_owned();
    let c = db.conn.lock();
    c.execute(
        "INSERT INTO scan_roots(path, stop_at_repo) VALUES (?1, ?2) ON CONFLICT(path) DO NOTHING",
        params![p, stop_at_repo],
    )?;
    Ok(
        c.query_row("SELECT id FROM scan_roots WHERE path=?1", params![p], |r| {
            r.get(0)
        })?,
    )
}

pub fn update_root(db: &Db, r: &ScanRoot) -> AppResult<()> {
    db.conn.lock().execute(
        "UPDATE scan_roots SET max_depth=?2, follow_symlinks=?3, stop_at_repo=?4, enabled=?5 WHERE id=?1",
        params![r.id, r.max_depth.clamp(1, 32), r.follow_symlinks, r.stop_at_repo, r.enabled],
    )?;
    Ok(())
}

/// Removing a root forgets the repos that were only reachable through it.
pub fn remove_root(db: &Db, id: i64) -> AppResult<()> {
    let c = db.conn.lock();
    c.execute("DELETE FROM repos WHERE root_id=?1", params![id])?;
    c.execute("DELETE FROM scan_roots WHERE id=?1", params![id])?;
    Ok(())
}

pub fn global_excludes(db: &Db) -> AppResult<Vec<String>> {
    let c = db.conn.lock();
    let mut st = c.prepare("SELECT glob FROM exclude_rules WHERE root_id IS NULL ORDER BY id")?;
    let v = st
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(v)
}

pub fn excludes_for(db: &Db, root_id: i64) -> AppResult<Vec<String>> {
    let c = db.conn.lock();
    let mut st = c.prepare("SELECT glob FROM exclude_rules WHERE root_id IS NULL OR root_id=?1")?;
    let v = st
        .query_map(params![root_id], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(v)
}

pub fn set_global_excludes(db: &Db, globs: &[String]) -> AppResult<()> {
    let mut c = db.conn.lock();
    let tx = c.transaction()?;
    tx.execute("DELETE FROM exclude_rules WHERE root_id IS NULL", [])?;
    for g in globs.iter().map(|g| g.trim()).filter(|g| !g.is_empty()) {
        globset::Glob::new(g).map_err(|e| AppError::msg(format!("Bad pattern \"{g}\": {e}")))?;
        tx.execute(
            "INSERT INTO exclude_rules(root_id, glob) VALUES (NULL, ?1)",
            params![g],
        )?;
    }
    tx.commit()?;
    Ok(())
}

// ---------- repos ----------

/// Store scan results for one root. Returns ids of every repo found.
/// A repo reached through two roots is stored once, keyed by canonical path.
pub fn save_scan(db: &Db, root_id: i64, found: &[Classified]) -> AppResult<Vec<i64>> {
    let mut c = db.conn.lock();
    let tx = c.transaction()?;
    let t = now();
    let mut ids = Vec::with_capacity(found.len());
    let mut by_path: HashMap<String, i64> = HashMap::new();
    for f in found {
        let p = f.path.to_string_lossy().into_owned();
        tx.execute(
            "INSERT INTO repos(canonical_path, name, kind, root_id, is_bare, embedded_warning, last_scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(canonical_path) DO UPDATE SET
               name=excluded.name, kind=excluded.kind, is_bare=excluded.is_bare,
               embedded_warning=excluded.embedded_warning, last_scanned_at=excluded.last_scanned_at,
               root_id=COALESCE(repos.root_id, excluded.root_id)",
            params![p, f.name, f.kind, root_id, f.is_bare, f.embedded_warning, t],
        )?;
        let id: i64 = tx.query_row(
            "SELECT id FROM repos WHERE canonical_path=?1",
            params![p],
            |r| r.get(0),
        )?;
        by_path.insert(p, id);
        ids.push(id);
    }
    for f in found {
        let id = by_path[&f.path.to_string_lossy().into_owned()];
        let parent = match &f.parent {
            Some(pp) => {
                let ps = pp.to_string_lossy().into_owned();
                match by_path.get(&ps) {
                    Some(&pid) => Some(pid),
                    None => tx
                        .query_row(
                            "SELECT id FROM repos WHERE canonical_path=?1",
                            params![ps],
                            |r| r.get(0),
                        )
                        .optional()?,
                }
            }
            None => None,
        };
        tx.execute(
            "UPDATE repos SET parent_repo_id=?2 WHERE id=?1",
            params![id, parent],
        )?;
    }
    // Repos under this root that were not found again are gone.
    let keep: HashSet<i64> = ids.iter().copied().collect();
    let existing: Vec<i64> = {
        let mut st = tx.prepare("SELECT id FROM repos WHERE root_id=?1")?;

        st.query_map(params![root_id], |r| r.get(0))?
            .collect::<Result<Vec<i64>, _>>()?
    };
    for id in existing.into_iter().filter(|id| !keep.contains(id)) {
        tx.execute("DELETE FROM repos WHERE id=?1", params![id])?;
    }
    tx.execute(
        "UPDATE scan_roots SET last_scanned_at=?2 WHERE id=?1",
        params![root_id, t],
    )?;
    tx.commit()?;
    Ok(ids)
}

/// Register one repo (e.g. a fresh clone) without rescanning its root.
pub fn add_repo(db: &Db, root_id: i64, path: &Path, name: &str) -> AppResult<i64> {
    let c = db.conn.lock();
    let p = path.to_string_lossy().into_owned();
    c.execute(
        "INSERT INTO repos(canonical_path, name, kind, root_id, is_bare, embedded_warning, last_scanned_at)
         VALUES (?1, ?2, 'normal', ?3, 0, 0, ?4)
         ON CONFLICT(canonical_path) DO UPDATE SET last_scanned_at=excluded.last_scanned_at",
        params![p, name, root_id, now()],
    )?;
    Ok(c.query_row(
        "SELECT id FROM repos WHERE canonical_path=?1",
        params![p],
        |r| r.get(0),
    )?)
}

pub struct RepoRef {
    pub id: i64,
    pub path: std::path::PathBuf,
    pub bare: bool,
    pub fingerprint: Option<String>,
    pub global_fingerprint: Option<String>,
}

pub fn repo_refs(db: &Db, ids: Option<&[i64]>) -> AppResult<Vec<RepoRef>> {
    let c = db.conn.lock();
    let mut st = c.prepare(
        "SELECT id, canonical_path, is_bare, fingerprint, global_fingerprint FROM repos",
    )?;
    let filter: Option<HashSet<i64>> = ids.map(|v| v.iter().copied().collect());
    let v = st
        .query_map([], |r| {
            Ok(RepoRef {
                id: r.get(0)?,
                path: std::path::PathBuf::from(r.get::<_, String>(1)?),
                bare: r.get(2)?,
                fingerprint: r.get(3)?,
                global_fingerprint: r.get(4)?,
            })
        })?
        .filter_map(Result::ok)
        .filter(|r| filter.as_ref().is_none_or(|f| f.contains(&r.id)))
        .collect();
    Ok(v)
}

pub fn repo_ref(db: &Db, id: i64) -> AppResult<RepoRef> {
    repo_refs(db, Some(&[id]))?
        .pop()
        .ok_or_else(|| AppError::msg("Repo not found."))
}

pub fn save_inspection(db: &Db, id: i64, fp: &str, gfp: &str, ins: &Inspection) -> AppResult<()> {
    let mut c = db.conn.lock();
    let tx = c.transaction()?;
    let t = now();
    tx.execute(
        "UPDATE repos SET fingerprint=?2, global_fingerprint=?3, inspected_at=?4, head_branch=?5, is_dirty=?6,
           last_commit_at=?7, branches_json=?8, recent_emails_json=?9, inspect_error=NULL WHERE id=?1",
        params![
            id,
            fp,
            gfp,
            t,
            ins.head_branch,
            ins.is_dirty,
            ins.last_commit_at,
            serde_json::to_string(&ins.branches)?,
            serde_json::to_string(&ins.recent_emails)?
        ],
    )?;
    tx.execute(
        "DELETE FROM identity_snapshots WHERE repo_id=?1",
        params![id],
    )?;
    for (i, e) in ins.config.iter().enumerate() {
        tx.execute(
            "INSERT INTO identity_snapshots(repo_id, ord, key, value, scope, origin_file, read_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, i as i64, e.key, e.value, e.scope, e.origin_file, t],
        )?;
    }
    tx.execute("DELETE FROM remotes WHERE repo_id=?1", params![id])?;
    for (name, url) in &ins.remotes {
        let p = crate::remote_url::parse(url);
        tx.execute(
            "INSERT OR REPLACE INTO remotes(repo_id, name, url, host, owner, repo_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                name,
                url,
                p.as_ref().map(|p| p.real_host.clone()),
                p.as_ref().map(|p| p.owner.clone()),
                p.as_ref().map(|p| p.repo.clone())
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn save_inspect_error(db: &Db, id: i64, err: &str) -> AppResult<()> {
    db.conn.lock().execute(
        "UPDATE repos SET inspect_error=?2, inspected_at=?3 WHERE id=?1",
        params![id, err, now()],
    )?;
    Ok(())
}

pub fn save_fetch_result(db: &Db, id: i64, result: &str) -> AppResult<()> {
    db.conn.lock().execute(
        "UPDATE repos SET last_fetch_at=?2, last_fetch_result=?3 WHERE id=?1",
        params![id, now(), result],
    )?;
    Ok(())
}

fn remote_info(
    name: String,
    url: String,
    host: Option<String>,
    owner: Option<String>,
    repo: Option<String>,
) -> RemoteInfo {
    let is_github = host
        .as_deref()
        .is_some_and(|h| h.eq_ignore_ascii_case("github.com"));
    let web_url = match (&host, &owner, &repo) {
        (Some(h), Some(o), Some(r)) if is_github => Some(format!("https://{h}/{o}/{r}")),
        _ => None,
    };
    RemoteInfo {
        name,
        url,
        host,
        owner,
        repo,
        is_github,
        web_url,
    }
}

fn all_remotes(
    c: &rusqlite::Connection,
    only: Option<i64>,
) -> AppResult<HashMap<i64, Vec<RemoteInfo>>> {
    let mut st = c.prepare(
        "SELECT repo_id, name, url, host, owner, repo_name FROM remotes WHERE (?1 IS NULL OR repo_id=?1) ORDER BY repo_id, name",
    )?;
    let mut m: HashMap<i64, Vec<RemoteInfo>> = HashMap::new();
    for row in st.query_map(params![only], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            remote_info(r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?),
        ))
    })? {
        let (id, ri) = row?;
        m.entry(id).or_default().push(ri);
    }
    Ok(m)
}

fn all_config(
    c: &rusqlite::Connection,
    only: Option<i64>,
) -> AppResult<HashMap<i64, Vec<ConfigEntry>>> {
    let mut st = c.prepare(
        "SELECT repo_id, key, value, scope, origin_file FROM identity_snapshots WHERE (?1 IS NULL OR repo_id=?1) ORDER BY repo_id, ord",
    )?;
    let mut m: HashMap<i64, Vec<ConfigEntry>> = HashMap::new();
    for row in st.query_map(params![only], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            ConfigEntry {
                key: r.get(1)?,
                value: r.get(2)?,
                scope: r.get(3)?,
                origin_file: r.get(4)?,
            },
        ))
    })? {
        let (id, e) = row?;
        m.entry(id).or_default().push(e);
    }
    Ok(m)
}

pub fn status_words(s: &str) -> &'static str {
    match s {
        "can_push" => "can push",
        "read_only" => "read only",
        "no_access" => "no access",
        "wrong_account" => "signs in as a different user",
        "auth_failed" => "sign-in failed",
        "host_untrusted" => "host not in known_hosts, run `ssh -T git@github.com` once",
        "offline" => "offline",
        _ => "unknown",
    }
}

/// Repo rows with issues computed from cached data only (no git, no network).
pub fn repo_rows(db: &Db, only: Option<i64>) -> AppResult<Vec<RepoRow>> {
    let accounts: HashMap<i64, Account> = list_accounts(db)?
        .into_iter()
        .map(|a| (a.id.unwrap(), a))
        .collect();
    let c = db.conn.lock();
    let config = all_config(&c, only)?;
    let remotes = all_remotes(&c, only)?;
    let mut links: HashMap<i64, Vec<(i64, bool)>> = HashMap::new();
    {
        let mut st = c.prepare("SELECT repo_id, account_id, is_active FROM repo_accounts WHERE (?1 IS NULL OR repo_id=?1)")?;
        for r in st.query_map(params![only], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })? {
            let (repo, acc, active) = r?;
            links.entry(repo).or_default().push((acc, active));
        }
    }
    let mut checks: HashMap<(i64, i64), String> = HashMap::new();
    {
        let mut st = c.prepare(
            "SELECT repo_id, account_id, status FROM push_checks WHERE (?1 IS NULL OR repo_id=?1)",
        )?;
        for r in st.query_map(params![only], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (repo, acc, s) = r?;
            checks.insert((repo, acc), s);
        }
    }
    let mut st = c.prepare(
        "SELECT r.id, r.canonical_path, r.name, r.kind, r.parent_repo_id, p.name, r.root_id, r.is_bare, r.embedded_warning,
                r.head_branch, r.is_dirty, r.last_commit_at, r.recent_emails_json, r.last_fetch_at, r.last_fetch_result,
                r.inspected_at, r.inspect_error
         FROM repos r LEFT JOIN repos p ON p.id = r.parent_repo_id
         WHERE (?1 IS NULL OR r.id=?1) ORDER BY lower(r.name), r.canonical_path",
    )?;
    let mut sorted_accounts: Vec<&Account> = accounts.values().collect();
    sorted_accounts.sort_by_key(|a| a.id);
    let email_owner: HashMap<String, i64> = accounts
        .values()
        .map(|a| (a.user_email.to_lowercase(), a.id.unwrap()))
        .collect();

    let rows = st
        .query_map(params![only], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, bool>(7)?,
                r.get::<_, bool>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<bool>>(10)?,
                r.get::<_, Option<i64>>(11)?,
                r.get::<_, Option<String>>(12)?,
                r.get::<_, Option<i64>>(13)?,
                r.get::<_, Option<String>>(14)?,
                r.get::<_, Option<i64>>(15)?,
                r.get::<_, Option<String>>(16)?,
            ))
        })?
        .filter_map(Result::ok)
        .map(|(id, path, name, kind, parent_repo_id, parent_name, root_id, is_bare, embedded, head_branch, is_dirty, last_commit_at, recent_json, last_fetch_at, last_fetch_result, inspected_at, inspect_error)| {
            let cfg = config.get(&id).map(Vec::as_slice).unwrap_or(&[]);
            let eff_email = crate::inspect::effective(cfg, "user.email");
            let eff_name = crate::inspect::effective(cfg, "user.name");
            let repo_links = links.get(&id).cloned().unwrap_or_default();
            let active = repo_links.iter().find(|l| l.1).map(|l| l.0);
            let rs = remotes.get(&id).cloned().unwrap_or_default();
            let remote = rs.iter().find(|r| r.name == "origin").or(rs.first()).cloned();
            let recent: Vec<String> = recent_json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();

            let email = eff_email.map(|e| e.value.to_lowercase());
            let linked = |acc: i64| repo_links.iter().any(|l| l.0 == acc);
            let use_fix = |acc: &Account| {
                let aid = acc.id.unwrap_or_default();
                Fix {
                    kind: if linked(aid) { "switch" } else { "link_switch" },
                    account_id: Some(aid),
                    label: format!("Use {}", acc.label),
                }
            };
            let owner = remote.as_ref().and_then(|r| r.owner.clone());
            let owner_accounts: Vec<&Account> = match &owner {
                Some(o) => sorted_accounts
                    .iter()
                    .copied()
                    .filter(|a| a.owners.as_deref().unwrap_or("").split(',').any(|x| x.trim().eq_ignore_ascii_case(o)))
                    .collect(),
                None => Vec::new(),
            };
            // The account this repo most likely belongs to.
            let suggested = if is_bare {
                None
            } else {
                owner_accounts.first().and_then(|a| a.id).or_else(|| {
                    let mut counts: HashMap<i64, usize> = HashMap::new();
                    for e in &recent {
                        if let Some(aid) = email_owner.get(e) {
                            *counts.entry(*aid).or_default() += 1;
                        }
                    }
                    counts.into_iter().max_by_key(|(aid, n)| (*n, -aid)).map(|(aid, _)| aid)
                }).or_else(|| (repo_links.len() == 1).then(|| repo_links[0].0))
            };

            let mut issues = Vec::new();
            if let Some(err) = &inspect_error {
                issues.push(issue("error", format!("Could not read this repo: {err}"), None));
            }
            if !is_bare && inspected_at.is_some() && inspect_error.is_none() {
                let active_acc = active.and_then(|a| accounts.get(&a));
                // The active account can't sign in: most urgent, since every push will fail.
                if let Some(a) = active_acc
                    && let Some(h) = &a.health
                    && h.level == "error"
                {
                    let fix = if h.kind == "public_key" && h.fix_path.is_some() {
                        Fix { kind: "repair_key", account_id: a.id, label: "Fix key".into() }
                    } else {
                        Fix { kind: "edit_account", account_id: a.id, label: format!("Fix {}", a.label) }
                    };
                    issues.push(issue("error", format!("{} can't sign in: {}", a.label, h.text), Some(fix)));
                }
                if email.is_none() {
                    let fix = match suggested.and_then(|s| accounts.get(&s)) {
                        Some(acc) => Some(use_fix(acc)),
                        None if accounts.is_empty() => Some(Fix { kind: "add_account", account_id: None, label: "Add an account".into() }),
                        None => None,
                    };
                    issues.push(issue("warn", "No commit email is set, so commits will fail or use a guessed address.", fix));
                }
                if let Some(a) = active_acc
                    && email.as_deref() != Some(&a.user_email.to_lowercase())
                {
                    issues.push(issue(
                        "warn",
                        format!(
                            "Active account is {} ({}), but Git will commit as {}.",
                            a.label,
                            a.user_email,
                            email.as_deref().unwrap_or("nobody")
                        ),
                        Some(Fix { kind: "switch", account_id: a.id, label: format!("Re-apply {}", a.label) }),
                    ));
                }
                for e in &recent {
                    if Some(e) == email.as_ref() {
                        continue;
                    }
                    if let Some(other) = email_owner.get(e).and_then(|id| accounts.get(id))
                        && Some(other.id.unwrap()) != active
                    {
                        issues.push(issue(
                            "warn",
                            format!("Recent commits were authored as {} ({}).", other.label, e),
                            Some(use_fix(other)),
                        ));
                    }
                }
                if let Some(o) = &owner
                    && !owner_accounts.is_empty()
                    && !owner_accounts.iter().any(|a| Some(a.user_email.to_lowercase()) == email)
                {
                    issues.push(issue(
                        "warn",
                        format!(
                            "{o} is a {} remote, but commits go out as {}.",
                            owner_accounts[0].label,
                            email.as_deref().unwrap_or("nobody")
                        ),
                        Some(use_fix(owner_accounts[0])),
                    ));
                }
                if let Some(a) = active
                    && let Some(s) = checks.get(&(id, a))
                    && matches!(s.as_str(), "read_only" | "no_access" | "wrong_account" | "auth_failed" | "host_untrusted")
                {
                    let label = accounts.get(&a).map(|a| a.label.as_str()).unwrap_or("Active account");
                    let fix = if s == "host_untrusted" {
                        Fix { kind: "trust_host", account_id: Some(a), label: "Trust github.com".into() }
                    } else {
                        Fix { kind: "edit_account", account_id: Some(a), label: format!("Check {label}") }
                    };
                    issues.push(issue("error", format!("{label}: {}.", status_words(s)), Some(fix)));
                }
                if last_fetch_result.as_deref() == Some("auth_failed") {
                    let fix = active_acc.map(|a| Fix { kind: "edit_account", account_id: a.id, label: format!("Check {}", a.label) });
                    issues.push(issue("error", "Last fetch failed to sign in.", fix));
                }
            }
            if embedded {
                issues.push(issue(
                    "warn",
                    "The parent repo tracks this folder as an embedded repo instead of ignoring it.",
                    None,
                ));
            }
            let status = if inspected_at.is_none() {
                "pending"
            } else if issues.iter().any(|i| i.level == "error") {
                "error"
            } else if !issues.is_empty() {
                "warn"
            } else {
                "ok"
            };
            RepoRow {
                id,
                path,
                name,
                kind,
                parent_repo_id,
                parent_name,
                root_id,
                is_bare,
                head_branch,
                is_dirty,
                last_commit_at,
                effective_name: eff_name.map(|e| e.value.clone()),
                effective_email: eff_email.map(|e| e.value.clone()),
                email_scope: eff_email.map(|e| e.scope.clone()),
                active_account_id: active,
                linked_account_ids: repo_links.iter().map(|l| l.0).collect(),
                remote,
                last_fetch_at,
                last_fetch_result,
                inspected: inspected_at.is_some(),
                status,
                issues,
                suggested_account_id: suggested,
            }
        })
        .collect();
    Ok(rows)
}

pub fn repo_row(db: &Db, id: i64) -> AppResult<RepoRow> {
    repo_rows(db, Some(id))?
        .pop()
        .ok_or_else(|| AppError::msg("Repo not found."))
}

pub fn repo_detail(db: &Db, id: i64) -> AppResult<RepoDetail> {
    let row = repo_row(db, id)?;
    let c = db.conn.lock();
    let config = all_config(&c, Some(id))?.remove(&id).unwrap_or_default();
    let remotes = all_remotes(&c, Some(id))?.remove(&id).unwrap_or_default();
    let (branches_json, inspect_error): (Option<String>, Option<String>) = c.query_row(
        "SELECT branches_json, inspect_error FROM repos WHERE id=?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let branches = branches_json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let mut st = c.prepare(
        "SELECT account_id, status, detail, checked_at FROM push_checks WHERE repo_id=?1",
    )?;
    let push_checks = st
        .query_map(params![id], |r| {
            Ok(PushCheck {
                account_id: r.get(0)?,
                status: r.get(1)?,
                detail: r.get(2)?,
                checked_at: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut st = c.prepare(
        "SELECT lower(author_email), count(*) FROM commits WHERE repo_id=?1 GROUP BY lower(author_email) ORDER BY count(*) DESC LIMIT 20",
    )?;
    let authors = st
        .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RepoDetail {
        row,
        config,
        remotes,
        branches,
        push_checks,
        authors,
        inspect_error,
    })
}

// ---------- accounts ----------

pub fn list_accounts(db: &Db) -> AppResult<Vec<Account>> {
    let c = db.conn.lock();
    let mut checks: HashMap<i64, (String, Option<String>, Option<String>, i64)> = HashMap::new();
    {
        let mut st = c.prepare(
            "SELECT account_id, status, detail, username, checked_at FROM account_checks",
        )?;
        for r in st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?),
            ))
        })? {
            let (id, v) = r?;
            checks.insert(id, v);
        }
    }
    let mut st = c.prepare(
        "SELECT id, label, user_name, user_email, host, auth_method, ssh_key_path, expected_username, owners FROM accounts ORDER BY lower(label)",
    )?;
    let mut v = st
        .query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                label: r.get(1)?,
                user_name: r.get(2)?,
                user_email: r.get(3)?,
                host: r.get(4)?,
                auth_method: r.get(5)?,
                ssh_key_path: r.get(6)?,
                expected_username: r.get(7)?,
                owners: r.get(8)?,
                health: None,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for a in &mut v {
        a.health = Some(health(a, a.id.and_then(|id| checks.get(&id))));
    }
    Ok(v)
}

fn health(a: &Account, check: Option<&(String, Option<String>, Option<String>, i64)>) -> Health {
    if a.auth_method == "ssh" {
        let (kind, text, fix_path) = crate::sshkeys::key_problem(a.ssh_key_path.as_deref());
        if kind != "ok" {
            return Health {
                level: "error",
                kind: kind.into(),
                text,
                username: None,
                checked_at: None,
                fix_path,
            };
        }
    }
    match check {
        Some((status, detail, username, at)) => {
            let ok = status == "can_push";
            Health {
                level: if ok {
                    "ok"
                } else if matches!(status.as_str(), "offline" | "unknown") {
                    "warn"
                } else {
                    "error"
                },
                kind: if ok { "ok".into() } else { status.clone() },
                text: if ok {
                    username
                        .as_deref()
                        .map(|u| format!("Signs in as @{u}"))
                        .unwrap_or_else(|| "Signs in".into())
                } else {
                    detail
                        .clone()
                        .unwrap_or_else(|| status_words(status).into())
                },
                username: username.clone(),
                checked_at: Some(*at),
                fix_path: None,
            }
        }
        None => Health {
            level: "unknown",
            kind: "untested".into(),
            text: "Not tested yet".into(),
            username: None,
            checked_at: None,
            fix_path: None,
        },
    }
}

/// Latest sign-in test for an account (from the account form or the access scan).
pub fn save_account_check(
    db: &Db,
    account_id: i64,
    status: &str,
    detail: &str,
    username: Option<&str>,
) -> AppResult<()> {
    db.conn.lock().execute(
        "INSERT INTO account_checks(account_id, status, detail, username, checked_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(account_id) DO UPDATE SET status=excluded.status, detail=excluded.detail,
           username=excluded.username, checked_at=excluded.checked_at",
        params![account_id, status, detail, username, now()],
    )?;
    Ok(())
}

pub fn set_account_key(db: &Db, account_id: i64, key_path: &str) -> AppResult<()> {
    db.conn.lock().execute(
        "UPDATE accounts SET ssh_key_path=?2 WHERE id=?1",
        params![account_id, key_path],
    )?;
    Ok(())
}

pub fn account(db: &Db, id: i64) -> AppResult<Account> {
    list_accounts(db)?
        .into_iter()
        .find(|a| a.id == Some(id))
        .ok_or_else(|| AppError::msg("Account not found."))
}

pub fn save_account(db: &Db, a: &Account) -> AppResult<i64> {
    if a.label.trim().is_empty() || a.user_name.trim().is_empty() || !a.user_email.contains('@') {
        return Err(AppError::msg("Label, name and a valid email are required."));
    }
    if !matches!(a.auth_method.as_str(), "ssh" | "https") {
        return Err(AppError::msg("Auth method must be ssh or https."));
    }
    let key = a
        .ssh_key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut key = key.map(str::to_string);
    if a.auth_method == "ssh" {
        let k = key
            .clone()
            .ok_or_else(|| AppError::msg("SSH accounts need the path to a private key."))?;
        match crate::sshkeys::key_problem(Some(&k)) {
            ("ok", _, _) => {}
            // A .pub path with its private key next to it: store the private key instead.
            ("public_key", _, Some(fix)) => key = Some(fix),
            ("public_key", _, None) => {
                return Err(AppError::msg(
                    "That's a public key (.pub). Choose the private key: the same file name without .pub.",
                ));
            }
            (_, text, _) => return Err(AppError::msg(text)),
        }
    }
    let host = if a.host.trim().is_empty() {
        "github.com"
    } else {
        a.host.trim()
    };
    let opt = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let c = db.conn.lock();
    match a.id {
        Some(id) => {
            c.execute(
                "UPDATE accounts SET label=?2, user_name=?3, user_email=?4, host=?5, auth_method=?6, ssh_key_path=?7,
                 expected_username=?8, owners=?9 WHERE id=?1",
                params![id, a.label.trim(), a.user_name.trim(), a.user_email.trim(), host, a.auth_method, key, opt(&a.expected_username), opt(&a.owners)],
            )?;
            Ok(id)
        }
        None => {
            c.execute(
                "INSERT INTO accounts(label, user_name, user_email, host, auth_method, ssh_key_path, expected_username, owners)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![a.label.trim(), a.user_name.trim(), a.user_email.trim(), host, a.auth_method, key, opt(&a.expected_username), opt(&a.owners)],
            )?;
            Ok(c.last_insert_rowid())
        }
    }
}

pub fn delete_account(db: &Db, id: i64) -> AppResult<()> {
    db.conn
        .lock()
        .execute("DELETE FROM accounts WHERE id=?1", params![id])?;
    Ok(())
}

pub fn link(db: &Db, repo_ids: &[i64], account_id: i64) -> AppResult<()> {
    let mut c = db.conn.lock();
    let tx = c.transaction()?;
    for r in repo_ids {
        tx.execute(
            "INSERT INTO repo_accounts(repo_id, account_id, is_active, linked_at) VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(repo_id, account_id) DO NOTHING",
            params![r, account_id, now()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn unlink(db: &Db, repo_id: i64, account_id: i64) -> AppResult<()> {
    db.conn.lock().execute(
        "DELETE FROM repo_accounts WHERE repo_id=?1 AND account_id=?2",
        params![repo_id, account_id],
    )?;
    Ok(())
}

pub fn set_active(db: &Db, repo_id: i64, account_id: Option<i64>) -> AppResult<()> {
    let mut c = db.conn.lock();
    let tx = c.transaction()?;
    tx.execute(
        "UPDATE repo_accounts SET is_active=0 WHERE repo_id=?1",
        params![repo_id],
    )?;
    if let Some(a) = account_id {
        tx.execute(
            "INSERT INTO repo_accounts(repo_id, account_id, is_active, linked_at, applied_at) VALUES (?1, ?2, 1, ?3, ?3)
             ON CONFLICT(repo_id, account_id) DO UPDATE SET is_active=1, applied_at=excluded.applied_at",
            params![repo_id, a, now()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn active_account(db: &Db, repo_id: i64) -> AppResult<Option<Account>> {
    let id: Option<i64> = db
        .conn
        .lock()
        .query_row(
            "SELECT account_id FROM repo_accounts WHERE repo_id=?1 AND is_active=1",
            params![repo_id],
            |r| r.get(0),
        )
        .optional()?;
    id.map(|id| account(db, id)).transpose()
}

pub fn save_push_check(
    db: &Db,
    repo_id: i64,
    account_id: i64,
    status: &str,
    detail: &str,
) -> AppResult<()> {
    db.conn.lock().execute(
        "INSERT INTO push_checks(repo_id, account_id, status, detail, checked_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(repo_id, account_id) DO UPDATE SET status=excluded.status, detail=excluded.detail, checked_at=excluded.checked_at",
        params![repo_id, account_id, status, detail, now()],
    )?;
    Ok(())
}

pub fn last_check_at(db: &Db, repo_id: i64, account_id: i64) -> Option<i64> {
    db.conn
        .lock()
        .query_row(
            "SELECT checked_at FROM push_checks WHERE repo_id=?1 AND account_id=?2",
            params![repo_id, account_id],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
}

#[derive(Serialize)]
pub struct MatrixCell {
    pub repo_id: i64,
    pub account_id: i64,
    pub status: String,
    pub detail: Option<String>,
    pub checked_at: i64,
}

pub fn all_push_checks(db: &Db) -> AppResult<Vec<MatrixCell>> {
    let c = db.conn.lock();
    let mut st =
        c.prepare("SELECT repo_id, account_id, status, detail, checked_at FROM push_checks")?;
    let v = st
        .query_map([], |r| {
            Ok(MatrixCell {
                repo_id: r.get(0)?,
                account_id: r.get(1)?,
                status: r.get(2)?,
                detail: r.get(3)?,
                checked_at: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

// ---------- graph cache ----------

pub fn graph_cached(
    db: &Db,
    repo_id: i64,
    fingerprint: &str,
    offset: usize,
    limit: usize,
) -> AppResult<Option<(Vec<Row>, bool)>> {
    let c = db.conn.lock();
    let meta: Option<(String, i64, bool)> = c
        .query_row(
            "SELECT fingerprint, row_count, complete FROM graph_meta WHERE repo_id=?1",
            params![repo_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((fp, count, complete)) = meta else {
        return Ok(None);
    };
    if fp != fingerprint || (!complete && (count as usize) < offset + limit) {
        return Ok(None);
    }
    let mut st = c.prepare(
        "SELECT hash, parents, author_name, author_email, authored_at, subject, refs, lane, segments
         FROM commits WHERE repo_id=?1 AND ord>=?2 AND ord<?3 ORDER BY ord",
    )?;
    let rows = st
        .query_map(
            params![repo_id, offset as i64, (offset + limit) as i64],
            |r| {
                Ok(Row {
                    hash: r.get(0)?,
                    parents: r
                        .get::<_, String>(1)?
                        .split_whitespace()
                        .map(str::to_string)
                        .collect(),
                    author_name: r.get(2)?,
                    author_email: r.get(3)?,
                    authored_at: r.get(4)?,
                    subject: r.get(5)?,
                    refs: r.get(6)?,
                    lane: r.get(7)?,
                    segments: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let more = !complete || (count as usize) > offset + limit;
    Ok(Some((rows, more)))
}

pub fn graph_store(
    db: &Db,
    repo_id: i64,
    fingerprint: &str,
    rows: &[Row],
    complete: bool,
) -> AppResult<()> {
    let mut c = db.conn.lock();
    let tx = c.transaction()?;
    tx.execute("DELETE FROM commits WHERE repo_id=?1", params![repo_id])?;
    {
        let mut st = tx.prepare(
            "INSERT INTO commits(repo_id, ord, hash, parents, author_name, author_email, authored_at, subject, refs, lane, segments)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for (i, r) in rows.iter().enumerate() {
            st.execute(params![
                repo_id,
                i as i64,
                r.hash,
                r.parents.join(" "),
                r.author_name,
                r.author_email,
                r.authored_at,
                r.subject,
                r.refs,
                r.lane,
                serde_json::to_string(&r.segments)?
            ])?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO graph_meta(repo_id, fingerprint, row_count, complete) VALUES (?1, ?2, ?3, ?4)",
        params![repo_id, fingerprint, rows.len() as i64, complete],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn clear_graph_cache(db: &Db) -> AppResult<()> {
    let c = db.conn.lock();
    c.execute("DELETE FROM commits", [])?;
    c.execute("DELETE FROM graph_meta", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success(),
            "git {args:?}"
        );
    }

    fn new_account(label: &str, email: &str, key: Option<&str>, owners: Option<&str>) -> Account {
        Account {
            id: None,
            label: label.into(),
            user_name: label.into(),
            user_email: email.into(),
            host: "github.com".into(),
            auth_method: if key.is_some() { "ssh" } else { "https" }.into(),
            ssh_key_path: key.map(str::to_string),
            expected_username: None,
            owners: owners.map(str::to_string),
            health: None,
        }
    }

    #[test]
    fn issues_come_with_fixes_and_health() {
        let tmp = std::env::temp_dir().join(format!("repodeck-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let db = Db::open(&tmp.join("t.db"), &[7u8; 32]).unwrap();

        // Keys: one real pair, used once by its .pub path.
        let key = crate::sshkeys::generate_for_test(&tmp.join("keys"), "work");
        let public = format!("{key}.pub");

        // Saving a .pub path stores the private key instead; a lone .pub is refused.
        let work = save_account(
            &db,
            &new_account("Work", "me@acme.example", Some(&public), Some("acme")),
        )
        .unwrap();
        assert_eq!(
            account(&db, work).unwrap().ssh_key_path.as_deref(),
            Some(key.as_str())
        );
        let lone = tmp.join("lone.pub");
        std::fs::copy(&public, &lone).unwrap();
        assert!(
            save_account(
                &db,
                &new_account("X", "x@x.x", Some(lone.to_str().unwrap()), None)
            )
            .is_err()
        );

        let personal =
            save_account(&db, &new_account("Personal", "me@home.example", None, None)).unwrap();
        assert_eq!(account(&db, work).unwrap().health.unwrap().kind, "untested");

        // An acme repo that commits as the personal email.
        let repo = tmp.join("app");
        git(&tmp, &["init", "-q", "app"]);
        git(&repo, &["config", "user.email", "me@home.example"]);
        git(
            &repo,
            &["remote", "add", "origin", "git@github.com:acme/app.git"],
        );
        let root = add_root(&db, tmp.to_str().unwrap(), false).unwrap();
        let id = add_repo(&db, root, &dunce::canonicalize(&repo).unwrap(), "app").unwrap();
        let ins = crate::inspect::inspect(&repo, false).unwrap();
        save_inspection(&db, id, "fp", "gfp", &ins).unwrap();

        let row = repo_row(&db, id).unwrap();
        assert_eq!(row.suggested_account_id, Some(work));
        let owner_issue = row
            .issues
            .iter()
            .find(|i| i.text.contains("acme is a Work remote"))
            .unwrap();
        let fix = owner_issue.fix.as_ref().unwrap();
        assert_eq!((fix.kind, fix.account_id), ("link_switch", Some(work)));

        // Once Work is linked, the same fix becomes a plain switch.
        link(&db, &[id], work).unwrap();
        let row = repo_row(&db, id).unwrap();
        let fix = row.issues.iter().find_map(|i| i.fix.as_ref()).unwrap();
        assert_eq!((fix.kind, fix.label.as_str()), ("switch", "Use Work"));

        // An active account whose key file is gone: an error with "Fix Work".
        set_active(&db, id, Some(work)).unwrap();
        std::fs::remove_file(&key).unwrap();
        let w = account(&db, work).unwrap();
        assert_eq!(w.health.as_ref().unwrap().kind, "missing");
        let row = repo_row(&db, id).unwrap();
        assert_eq!(row.status, "error");
        let i = &row.issues[0];
        assert!(i.text.starts_with("Work can't sign in"), "{}", i.text);
        assert_eq!(i.fix.as_ref().unwrap().kind, "edit_account");

        // A stored sign-in result shows up as health.
        save_account_check(&db, personal, "can_push", "Signed in as me.", Some("me")).unwrap();
        let h = account(&db, personal).unwrap().health.unwrap();
        assert_eq!((h.level, h.text.as_str()), ("ok", "Signs in as @me"));
        drop(db);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
