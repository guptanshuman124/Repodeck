//! The allow-listed IPC surface. Commands take repo and account ids, never raw
//! paths, so the UI cannot point git at arbitrary folders.

use crate::accounts::{self, ConfigChange};
use crate::core::{Core, DbState, SETTING_DEFAULTS};
use crate::error::{AppError, AppResult};
use crate::graph::{self, ChangedFile, Row};
use crate::store::{self, Account, MatrixCell, RepoDetail, RepoRow, ScanRoot};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

type S<'a> = State<'a, Arc<Core>>;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::msg(e.to_string()))?
}

#[derive(Serialize)]
pub struct AppStatus {
    db: DbState,
    git_ok: bool,
    git_version: Option<String>,
    git_too_old: bool,
    onboarded: bool,
    platform: &'static str,
}

#[tauri::command]
pub async fn app_status(core: S<'_>) -> AppResult<AppStatus> {
    let core = core.inner().clone();
    blocking(move || {
        let v = crate::git::version();
        Ok(AppStatus {
            db: core.db_state.lock().clone(),
            git_ok: v.is_some(),
            git_too_old: v
                .as_ref()
                .is_some_and(|(maj, min, _)| (*maj, *min) < (2, 26)),
            git_version: v.map(|v| v.2),
            onboarded: core.setting("onboarded") == "true",
            platform: std::env::consts::OS,
        })
    })
    .await
}

#[tauri::command]
pub async fn unlock(core: S<'_>, passphrase: String) -> AppResult<()> {
    let core = core.inner().clone();
    blocking(move || core.unlock_with_passphrase(&passphrase)).await
}

#[tauri::command]
pub async fn reset_data(core: S<'_>) -> AppResult<()> {
    let core = core.inner().clone();
    blocking(move || core.reset_data()).await
}

// ---------- roots ----------

#[tauri::command]
pub fn list_roots(core: S<'_>) -> AppResult<Vec<ScanRoot>> {
    store::list_roots(&*core.db()?)
}

#[tauri::command]
pub fn add_root(core: S<'_>, path: String, stop_at_repo: Option<bool>) -> AppResult<i64> {
    store::add_root(&*core.db()?, &path, stop_at_repo.unwrap_or(false))
}

#[tauri::command]
pub fn update_root(core: S<'_>, root: ScanRoot) -> AppResult<()> {
    store::update_root(&*core.db()?, &root)
}

#[tauri::command]
pub fn remove_root(core: S<'_>, id: i64) -> AppResult<()> {
    let core = core.inner().clone();
    store::remove_root(&*core.db()?, id)?;
    std::thread::spawn(move || core.rewatch());
    Ok(())
}

#[tauri::command]
pub fn get_excludes(core: S<'_>) -> AppResult<Vec<String>> {
    store::global_excludes(&*core.db()?)
}

#[tauri::command]
pub fn set_excludes(core: S<'_>, globs: Vec<String>) -> AppResult<()> {
    store::set_global_excludes(&*core.db()?, &globs)
}

#[tauri::command]
pub fn start_scan(core: S<'_>, root_ids: Option<Vec<i64>>) -> AppResult<u64> {
    core.inner().start_scan(root_ids)
}

#[tauri::command]
pub fn cancel_job(core: S<'_>, id: u64) {
    core.cancel_job(id)
}

// ---------- repos ----------

#[tauri::command]
pub async fn list_repos(core: S<'_>) -> AppResult<Vec<RepoRow>> {
    let d = core.db()?;
    blocking(move || store::repo_rows(&d, None)).await
}

#[tauri::command]
pub async fn repo_detail(core: S<'_>, id: i64) -> AppResult<RepoDetail> {
    let d = core.db()?;
    blocking(move || store::repo_detail(&d, id)).await
}

/// Manual refresh: one repo (forced), or everything that changed.
#[tauri::command]
pub async fn refresh_repos(
    core: S<'_>,
    ids: Option<Vec<i64>>,
    force: Option<bool>,
) -> AppResult<usize> {
    let core = core.inner().clone();
    blocking(move || Ok(core.refresh(ids.as_deref(), force.unwrap_or(false)))).await
}

#[derive(Serialize)]
pub struct GraphPage {
    rows: Vec<Row>,
    offset: usize,
    has_more: bool,
}

const PAGE: usize = 500;

#[tauri::command]
pub async fn get_graph(core: S<'_>, id: i64, page: usize) -> AppResult<GraphPage> {
    let core = core.inner().clone();
    blocking(move || {
        let d = core.db()?;
        let r = store::repo_ref(&d, id)?;
        let paths = crate::inspect::git_paths(&r.path, r.bare)
            .ok_or_else(|| AppError::msg("Not a git repository."))?;
        let fp = crate::inspect::fingerprint(&paths);
        let offset = page * PAGE;
        if let Some((rows, has_more)) = store::graph_cached(&d, id, &fp, offset, PAGE)? {
            return Ok(GraphPage {
                rows,
                offset,
                has_more,
            });
        }
        let cap = core
            .setting("max_commits")
            .parse::<usize>()
            .unwrap_or(10_000)
            .max(PAGE);
        let want = (offset + PAGE).min(cap);
        let raw = graph::read_log(&r.path, want).map_err(AppError::msg)?;
        let complete = raw.len() < want;
        let rows = graph::layout(raw);
        store::graph_store(&d, id, &fp, &rows, complete)?;
        let has_more = !complete && want < cap;
        let page_rows = rows.into_iter().skip(offset).take(PAGE).collect();
        Ok(GraphPage {
            rows: page_rows,
            offset,
            has_more,
        })
    })
    .await
}

#[tauri::command]
pub async fn commit_files(
    core: S<'_>,
    id: i64,
    hash: String,
    parent: Option<String>,
) -> AppResult<Vec<ChangedFile>> {
    let d = core.db()?;
    blocking(move || {
        let r = store::repo_ref(&d, id)?;
        graph::changed_files(&r.path, &hash, parent.as_deref()).map_err(AppError::msg)
    })
    .await
}

// ---------- accounts ----------

#[tauri::command]
pub fn list_accounts(core: S<'_>) -> AppResult<Vec<Account>> {
    store::list_accounts(&*core.db()?)
}

#[tauri::command]
pub async fn save_account(core: S<'_>, account: Account) -> AppResult<i64> {
    let core = core.inner().clone();
    blocking(move || {
        let id = store::save_account(&*core.db()?, &account)?;
        let _ = tauri::Emitter::emit(&core.app, "repos-reload", ());
        Ok(id)
    })
    .await
}

#[tauri::command]
pub fn delete_account(core: S<'_>, id: i64) -> AppResult<()> {
    store::delete_account(&*core.db()?, id)?;
    let _ = tauri::Emitter::emit(&core.app, "repos-reload", ());
    Ok(())
}

#[tauri::command]
pub fn link_account(core: S<'_>, repo_ids: Vec<i64>, account_id: i64) -> AppResult<()> {
    store::link(&*core.db()?, &repo_ids, account_id)?;
    let _ = tauri::Emitter::emit(&core.app, "repos-reload", ());
    Ok(())
}

#[tauri::command]
pub fn unlink_account(core: S<'_>, repo_id: i64, account_id: i64) -> AppResult<()> {
    let d = core.db()?;
    if store::active_account(&d, repo_id)?.and_then(|a| a.id) == Some(account_id) {
        return Err(AppError::msg(
            "Switch this repo to another account or to the global identity before unlinking.",
        ));
    }
    store::unlink(&d, repo_id, account_id)?;
    let _ = tauri::Emitter::emit(&core.app, "repos-reload", ());
    Ok(())
}

#[derive(Serialize)]
pub struct Preview {
    changes: Vec<ConfigChange>,
    commands: Vec<String>,
    config_file: String,
}

fn repo_host(row: &RepoRow) -> String {
    row.remote
        .as_ref()
        .and_then(|r| r.host.clone())
        .unwrap_or_else(|| "github.com".into())
}

/// What switching to `account_id` (or to the global identity when None) would write.
#[tauri::command]
pub async fn preview_switch(
    core: S<'_>,
    repo_id: i64,
    account_id: Option<i64>,
) -> AppResult<Preview> {
    let d = core.db()?;
    blocking(move || {
        let row = store::repo_row(&d, repo_id)?;
        let acc = account_id.map(|a| store::account(&d, a)).transpose()?;
        let changes = accounts::preview(Path::new(&row.path), &repo_host(&row), acc.as_ref());
        let paths = crate::inspect::git_paths(Path::new(&row.path), row.is_bare)
            .ok_or_else(|| AppError::msg("Not a git repository."))?;
        Ok(Preview {
            commands: accounts::command_lines(&changes),
            changes,
            config_file: paths
                .common_dir
                .join("config")
                .to_string_lossy()
                .into_owned(),
        })
    })
    .await
}

/// Recompute the preview and write it only if it still matches what the user confirmed,
/// then mark the account active. Shared by single and bulk switches.
fn switch_checked(
    d: &crate::db::Db,
    row: &RepoRow,
    acc: Option<&Account>,
    expected: &[ConfigChange],
) -> AppResult<()> {
    let changes = accounts::preview(Path::new(&row.path), &repo_host(row), acc);
    let same = changes.len() == expected.len()
        && changes
            .iter()
            .zip(expected)
            .all(|(a, b)| a.key == b.key && a.current == b.current && a.next == b.next);
    if !same {
        return Err(AppError::msg(
            "The repo's config changed since the preview. Review it again.",
        ));
    }
    accounts::apply(Path::new(&row.path), &changes).map_err(AppError::msg)?;
    store::set_active(d, row.id, acc.and_then(|a| a.id))?;
    Ok(())
}

/// Writes only after the user confirmed the preview. Recomputes it so nothing unseen is written.
#[tauri::command]
pub async fn apply_switch(
    core: S<'_>,
    repo_id: i64,
    account_id: Option<i64>,
    expected: Vec<ConfigChange>,
) -> AppResult<RepoRow> {
    let core = core.inner().clone();
    blocking(move || {
        let d = core.db()?;
        let row = store::repo_row(&d, repo_id)?;
        let acc = account_id.map(|a| store::account(&d, a)).transpose()?;
        switch_checked(&d, &row, acc.as_ref(), &expected)?;
        core.refresh(Some(&[repo_id]), true);
        store::repo_row(&d, repo_id)
    })
    .await
}

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct BulkItem {
    repo_id: i64,
    name: String,
    /// Empty when the repo already matches.
    changes: Vec<ConfigChange>,
}

#[derive(Serialize)]
pub struct BulkResult {
    applied: usize,
    skipped: Vec<(String, String)>,
}

/// Preview of switching many repos to one account (or to the global identity).
#[tauri::command]
pub async fn preview_bulk_switch(
    core: S<'_>,
    repo_ids: Vec<i64>,
    account_id: Option<i64>,
) -> AppResult<Vec<BulkItem>> {
    let d = core.db()?;
    blocking(move || {
        let acc = account_id.map(|a| store::account(&d, a)).transpose()?;
        let mut out = Vec::new();
        for id in repo_ids {
            let row = store::repo_row(&d, id)?;
            if row.is_bare {
                continue;
            }
            let changes = accounts::preview(Path::new(&row.path), &repo_host(&row), acc.as_ref());
            out.push(BulkItem {
                repo_id: id,
                name: row.name,
                changes,
            });
        }
        Ok(out)
    })
    .await
}

/// Apply a confirmed bulk preview. Repos whose config changed since the preview are skipped.
#[tauri::command]
pub async fn apply_bulk_switch(
    core: S<'_>,
    account_id: Option<i64>,
    expected: Vec<BulkItem>,
) -> AppResult<BulkResult> {
    let core = core.inner().clone();
    blocking(move || {
        let d = core.db()?;
        let acc = account_id.map(|a| store::account(&d, a)).transpose()?;
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        for item in &expected {
            let res = store::repo_row(&d, item.repo_id)
                .and_then(|row| switch_checked(&d, &row, acc.as_ref(), &item.changes));
            match res {
                Ok(()) => applied.push(item.repo_id),
                Err(e) => skipped.push((item.name.clone(), e.to_string())),
            }
        }
        if !applied.is_empty() {
            core.refresh(Some(&applied), true);
        }
        let _ = tauri::Emitter::emit(&core.app, "repos-reload", ());
        Ok(BulkResult {
            applied: applied.len(),
            skipped,
        })
    })
    .await
}

#[tauri::command]
pub async fn push_check(
    core: S<'_>,
    repo_id: i64,
    account_id: i64,
) -> AppResult<accounts::CheckResult> {
    let core = core.inner().clone();
    blocking(move || core.check_one(repo_id, account_id)).await
}

#[tauri::command]
pub async fn probe_clone_access(
    core: S<'_>,
    url: String,
) -> AppResult<Vec<crate::core::CloneAccess>> {
    let core = core.inner().clone();
    blocking(move || core.probe_clone_access(&url)).await
}

#[tauri::command]
pub fn start_clone(core: S<'_>, url: String, dest: String, account_id: i64) -> AppResult<u64> {
    core.inner()
        .start_clone(url, crate::scanner::expand_tilde(&dest), account_id)
}

#[tauri::command]
pub fn start_fetch(core: S<'_>, repo_ids: Vec<i64>) -> AppResult<u64> {
    core.inner().start_fetch(repo_ids)
}

#[tauri::command]
pub fn start_access_scan(
    core: S<'_>,
    changed_only: bool,
    repo_ids: Option<Vec<i64>>,
) -> AppResult<u64> {
    core.inner().start_access_scan(changed_only, repo_ids)
}

#[tauri::command]
pub fn access_matrix(core: S<'_>) -> AppResult<Vec<MatrixCell>> {
    store::all_push_checks(&*core.db()?)
}

// ---------- open in ----------

fn editor_candidates(editor: &str) -> Vec<String> {
    let mut v = vec![editor.to_string()];
    if editor == "code" {
        #[cfg(target_os = "macos")]
        v.push("/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code".into());
        #[cfg(target_os = "windows")]
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            v.push(
                Path::new(&local)
                    .join("Programs/Microsoft VS Code/bin/code.cmd")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        #[cfg(target_os = "linux")]
        v.extend(["/usr/bin/code".to_string(), "/snap/bin/code".to_string()]);
    }
    v
}

#[tauri::command]
pub fn open_in_editor(core: S<'_>, repo_id: i64) -> AppResult<()> {
    let d = core.db()?;
    let r = store::repo_ref(&d, repo_id)?;
    if r.bare {
        return Err(AppError::msg("Bare repos have no working tree to open."));
    }
    let editor = core.setting("editor");
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("code").to_string();
    let extra: Vec<String> = parts.map(str::to_string).collect();
    for cand in editor_candidates(&program) {
        let mut c = std::process::Command::new(&cand);
        c.args(&extra)
            .arg(&r.path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        if c.spawn().is_ok() {
            return Ok(());
        }
    }
    if program == "code" {
        let p = r.path.to_string_lossy().replace('\\', "/");
        let p = if p.starts_with('/') {
            p
        } else {
            format!("/{p}")
        };
        return core
            .app
            .opener()
            .open_url(format!("vscode://file{p}"), None::<&str>)
            .map_err(|e| AppError::msg(e.to_string()));
    }
    Err(AppError::msg(format!(
        "Could not start \"{editor}\". Check the editor command in Settings."
    )))
}

fn url_path_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tauri::command]
pub fn open_on_github(
    core: S<'_>,
    repo_id: i64,
    branch: Option<String>,
    commit: Option<String>,
) -> AppResult<()> {
    let row = store::repo_row(&*core.db()?, repo_id)?;
    let base = row
        .remote
        .and_then(|r| r.web_url)
        .ok_or_else(|| AppError::msg("This repo has no GitHub remote."))?;
    let url = match (commit, branch) {
        (Some(c), _) if c.chars().all(|ch| ch.is_ascii_hexdigit()) => format!("{base}/commit/{c}"),
        (_, Some(b)) => format!("{base}/tree/{}", url_path_escape(&b)),
        _ => base,
    };
    core.app
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::msg(e.to_string()))
}

// ---------- SSH keys ----------

#[tauri::command]
pub async fn generate_ssh_key(
    label: String,
    email: String,
) -> AppResult<crate::sshkeys::GeneratedKey> {
    blocking(move || crate::sshkeys::generate(&label, &email)).await
}

/// Which GitHub user does this key sign in as? Works before the account is saved;
/// with `account_id`, the result is stored as that account's health.
#[tauri::command]
pub async fn test_ssh_key(
    core: S<'_>,
    key_path: String,
    host: String,
    expected_username: Option<String>,
    account_id: Option<i64>,
) -> AppResult<accounts::CheckResult> {
    let core = core.inner().clone();
    let host = if host.trim().is_empty() {
        "github.com".to_string()
    } else {
        host.trim().to_string()
    };
    let acc = Account {
        id: None,
        label: String::new(),
        user_name: String::new(),
        user_email: String::new(),
        host,
        auth_method: "ssh".into(),
        ssh_key_path: Some(key_path),
        expected_username,
        owners: None,
        health: None,
    };
    blocking(move || {
        let r = accounts::whoami(&acc, None);
        if let Some(id) = account_id {
            let d = core.db()?;
            // Only record tests of the key the account actually uses.
            if store::account(&d, id)?.ssh_key_path == acc.ssh_key_path {
                store::save_account_check(&d, id, r.status, &r.detail, r.username.as_deref())?;
                let _ = tauri::Emitter::emit(&core.app, "repos-reload", ());
            }
        }
        Ok(r)
    })
    .await
}

/// Point an account that was saved with its public key at the private key, and update
/// `core.sshCommand` in the repos where it's active. Nothing else in those configs changes.
#[tauri::command]
pub async fn repair_account_key(core: S<'_>, account_id: i64) -> AppResult<usize> {
    let core = core.inner().clone();
    blocking(move || {
        let d = core.db()?;
        let acc = store::account(&d, account_id)?;
        let fix = acc
            .health
            .as_ref()
            .and_then(|h| h.fix_path.clone())
            .ok_or_else(|| {
                AppError::msg(
                    "There's no private key to switch to. Edit the account and choose its key.",
                )
            })?;
        store::set_account_key(&d, account_id, &fix)?;
        let acc = store::account(&d, account_id)?;
        let mut touched = Vec::new();
        for row in store::repo_rows(&d, None)? {
            if row.active_account_id != Some(account_id) {
                continue;
            }
            let changes: Vec<ConfigChange> =
                accounts::preview(Path::new(&row.path), &repo_host(&row), Some(&acc))
                    .into_iter()
                    .filter(|c| c.key == "core.sshCommand")
                    .collect();
            if !changes.is_empty() {
                accounts::apply(Path::new(&row.path), &changes).map_err(AppError::msg)?;
                touched.push(row.id);
            }
        }
        if !touched.is_empty() {
            core.refresh(Some(&touched), true);
        }
        let _ = tauri::Emitter::emit(&core.app, "repos-reload", ());
        Ok(touched.len())
    })
    .await
}

/// Opens the host's "new SSH key" page; the only URL this command can open.
#[tauri::command]
pub fn open_ssh_key_page(core: S<'_>, host: String) -> AppResult<()> {
    let host = host.trim().to_lowercase();
    let host = if host.is_empty() {
        "github.com".to_string()
    } else {
        host
    };
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(AppError::msg("That host name isn't valid."));
    }
    core.app
        .opener()
        .open_url(format!("https://{host}/settings/ssh/new"), None::<&str>)
        .map_err(|e| AppError::msg(e.to_string()))
}

#[derive(Serialize)]
pub struct KeyInfo {
    #[serde(flatten)]
    key: crate::sshkeys::SshKey,
    /// Labels of accounts that use this key.
    used_by: Vec<String>,
}

/// Private keys found in ~/.ssh, with the accounts that use each one.
#[tauri::command]
pub async fn list_ssh_keys(core: S<'_>) -> AppResult<Vec<KeyInfo>> {
    let d = core.db()?;
    blocking(move || {
        let accounts = store::list_accounts(&d)?;
        let keys = crate::sshkeys::list_keys()?;
        Ok(keys
            .into_iter()
            .map(|k| {
                let full = crate::scanner::expand_tilde(&k.path);
                let used_by = accounts
                    .iter()
                    .filter(|a| {
                        a.ssh_key_path
                            .as_deref()
                            .is_some_and(|p| crate::scanner::expand_tilde(p) == full)
                    })
                    .map(|a| a.label.clone())
                    .collect();
                KeyInfo { key: k, used_by }
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn trust_github_host() -> AppResult<()> {
    blocking(crate::sshkeys::trust_github).await
}

#[tauri::command]
pub fn reveal_repo(core: S<'_>, repo_id: i64) -> AppResult<()> {
    let r = store::repo_ref(&*core.db()?, repo_id)?;
    core.app
        .opener()
        .reveal_item_in_dir(&r.path)
        .map_err(|e| AppError::msg(e.to_string()))
}

// ---------- settings ----------

#[tauri::command]
pub fn get_settings(core: S<'_>) -> AppResult<HashMap<String, String>> {
    core.db()?;
    Ok(SETTING_DEFAULTS
        .iter()
        .map(|(k, _)| (k.to_string(), core.setting(k)))
        .collect())
}

#[tauri::command]
pub fn set_settings(core: S<'_>, values: HashMap<String, String>) -> AppResult<()> {
    let d = core.db()?;
    let mut rewatch = false;
    for (k, v) in values {
        if !SETTING_DEFAULTS.iter().any(|(dk, _)| *dk == k) {
            return Err(AppError::msg(format!("Unknown setting {k}.")));
        }
        rewatch |= k == "watch_threshold";
        d.set_setting(&k, &v)?;
    }
    if rewatch {
        let core = core.inner().clone();
        std::thread::spawn(move || core.rewatch());
    }
    Ok(())
}

#[tauri::command]
pub fn clear_graph_cache(core: S<'_>) -> AppResult<()> {
    store::clear_graph_cache(&*core.db()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn switch_skips_repos_changed_after_preview() {
        let tmp = std::env::temp_dir().join(format!("repodeck-bulk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let d = crate::db::Db::open(&tmp.join("t.db"), &[3u8; 32]).unwrap();
        let acc_id = store::save_account(
            &d,
            &Account {
                id: None,
                label: "Work".into(),
                user_name: "Ada".into(),
                user_email: "ada@work.example".into(),
                host: "github.com".into(),
                auth_method: "https".into(),
                ssh_key_path: None,
                expected_username: None,
                owners: None,
                health: None,
            },
        )
        .unwrap();
        let acc = store::account(&d, acc_id).unwrap();
        let root = store::add_root(&d, tmp.to_str().unwrap(), false).unwrap();
        let mut rows = Vec::new();
        for name in ["one", "two"] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&tmp)
                    .args(["init", "-q", name])
                    .status()
                    .unwrap()
                    .success()
            );
            let path = dunce::canonicalize(tmp.join(name)).unwrap();
            let id = store::add_repo(&d, root, &path, name).unwrap();
            rows.push(store::repo_row(&d, id).unwrap());
        }
        let previews: Vec<_> = rows
            .iter()
            .map(|r| accounts::preview(Path::new(&r.path), "github.com", Some(&acc)))
            .collect();

        // Someone edits repo two's config after the preview was shown.
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&rows[1].path)
                .args(["config", "user.email", "x@y.z"])
                .status()
                .unwrap()
                .success()
        );

        assert!(switch_checked(&d, &rows[0], Some(&acc), &previews[0]).is_ok());
        assert!(switch_checked(&d, &rows[1], Some(&acc), &previews[1]).is_err());
        assert_eq!(
            store::repo_row(&d, rows[0].id).unwrap().active_account_id,
            Some(acc_id)
        );
        assert_eq!(
            store::repo_row(&d, rows[1].id).unwrap().active_account_id,
            None
        );
        let out = Command::new("git")
            .arg("-C")
            .arg(&rows[1].path)
            .args(["config", "user.email"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "x@y.z",
            "the skipped repo is untouched"
        );
        drop(d);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
