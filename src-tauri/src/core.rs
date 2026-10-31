//! App state plus the background machinery: inspection pool, jobs, watchers, scheduler.

use crate::db::{self, Db, KeyOutcome, now};
use crate::error::{AppError, AppResult};
use crate::scanner::{self, Excludes, RootSpec};
use crate::{accounts, inspect, store};
use notify::{RecursiveMode, Watcher};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DbState {
    Ready,
    NeedsPassphrase { first_time: bool },
    KeyLost,
    Error { message: String },
}

pub struct Core {
    pub app: AppHandle,
    pub data_dir: PathBuf,
    db: RwLock<Option<Arc<Db>>>,
    pub db_state: Mutex<DbState>,
    jobs: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next_job: AtomicU64,
    scanning: AtomicBool,
    in_flight: Mutex<HashSet<i64>>,
    pub focused: AtomicBool,
    last_focus_pass: Mutex<Option<Instant>>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    watched: Arc<Mutex<HashMap<PathBuf, Vec<i64>>>>,
    dirty: Arc<Mutex<HashMap<i64, Instant>>>,
    polling: AtomicBool,
    last_poll: Mutex<Instant>,
}

#[derive(Serialize, Clone)]
pub struct JobEvent {
    pub id: u64,
    pub kind: &'static str,
    pub done: usize,
    pub total: usize,
    pub label: String,
    pub finished: bool,
    pub cancelled: bool,
    pub summary: HashMap<String, usize>,
}

#[derive(Serialize, Clone)]
pub struct JobItem {
    pub job_id: u64,
    pub kind: &'static str,
    pub repo_id: i64,
    pub account_id: Option<i64>,
    pub status: String,
    pub detail: String,
}

#[derive(Serialize, Clone)]
pub struct CloneAccess {
    pub account_id: i64,
    pub status: &'static str,
    pub detail: String,
}

pub const SETTING_DEFAULTS: &[(&str, &str)] = &[
    ("editor", "code"),
    ("fetch_concurrency", "auto"),
    ("fetch_prune", "false"),
    ("watch_threshold", "300"),
    ("poll_interval_secs", "60"),
    ("rescan_hours", "24"),
    ("battery_saver", "false"),
    ("max_commits", "10000"),
    ("stale_branch_days", "90"),
    ("onboarded", "false"),
];

impl Core {
    pub fn new(app: AppHandle, data_dir: PathBuf) -> Arc<Core> {
        Arc::new(Core {
            app,
            data_dir,
            db: RwLock::new(None),
            db_state: Mutex::new(DbState::Error {
                message: "starting".into(),
            }),
            jobs: Mutex::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            scanning: AtomicBool::new(false),
            in_flight: Mutex::new(HashSet::new()),
            focused: AtomicBool::new(true),
            last_focus_pass: Mutex::new(None),
            watcher: Mutex::new(None),
            watched: Arc::new(Mutex::new(HashMap::new())),
            dirty: Arc::new(Mutex::new(HashMap::new())),
            polling: AtomicBool::new(false),
            last_poll: Mutex::new(Instant::now()),
        })
    }

    pub fn db(&self) -> AppResult<Arc<Db>> {
        self.db
            .read()
            .clone()
            .ok_or_else(|| AppError::msg("The database is locked."))
    }

    // ---------- unlocking ----------

    pub fn try_open_with_keychain(self: &Arc<Self>) {
        let state = match db::keychain_key(&self.data_dir) {
            KeyOutcome::Ready(k) => match Db::open(&db::db_path(&self.data_dir), &k) {
                Ok(d) => {
                    *self.db.write() = Some(Arc::new(d));
                    DbState::Ready
                }
                Err(_) => DbState::KeyLost,
            },
            KeyOutcome::NeedsPassphrase { first_time } => DbState::NeedsPassphrase { first_time },
            KeyOutcome::Lost => DbState::KeyLost,
        };
        let ready = state == DbState::Ready;
        *self.db_state.lock() = state;
        if ready {
            self.after_unlock();
        }
    }

    pub fn unlock_with_passphrase(self: &Arc<Self>, pass: &str) -> AppResult<()> {
        if pass.chars().count() < 8 {
            return Err(AppError::msg("Use at least 8 characters."));
        }
        let key = db::passphrase_key(&self.data_dir, pass)?;
        let d = Db::open(&db::db_path(&self.data_dir), &key)
            .map_err(|_| AppError::msg("Wrong passphrase."))?;
        *self.db.write() = Some(Arc::new(d));
        *self.db_state.lock() = DbState::Ready;
        self.after_unlock();
        Ok(())
    }

    pub fn reset_data(self: &Arc<Self>) -> AppResult<()> {
        self.cancel_all();
        *self.watcher.lock() = None;
        *self.db.write() = None;
        db::reset(&self.data_dir)?;
        self.try_open_with_keychain();
        Ok(())
    }

    fn after_unlock(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            me.rewatch();
            // Cache first: the UI already has rows; now refresh only what changed.
            me.refresh(None, false);
        });
    }

    // ---------- settings ----------

    pub fn setting(&self, key: &str) -> String {
        self.db()
            .ok()
            .and_then(|d| d.setting(key))
            .or_else(|| {
                SETTING_DEFAULTS
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.to_string())
            })
            .unwrap_or_default()
    }

    fn setting_num(&self, key: &str, fallback: u64) -> u64 {
        self.setting(key).parse().unwrap_or(fallback)
    }

    fn battery_saver(&self) -> bool {
        self.setting("battery_saver") == "true"
    }

    /// Two threads on low-end machines, up to four otherwise.
    pub fn threads(&self) -> usize {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        if cores <= 2 || self.battery_saver() {
            2
        } else {
            cores.min(4)
        }
    }

    // ---------- jobs ----------

    fn new_job(&self) -> (u64, Arc<AtomicBool>) {
        let id = self.next_job.fetch_add(1, Ordering::Relaxed);
        let flag = Arc::new(AtomicBool::new(false));
        self.jobs.lock().insert(id, flag.clone());
        (id, flag)
    }

    fn end_job(&self, id: u64) {
        self.jobs.lock().remove(&id);
    }

    pub fn cancel_job(&self, id: u64) {
        if let Some(f) = self.jobs.lock().get(&id) {
            f.store(true, Ordering::Relaxed);
        }
    }

    fn cancel_all(&self) {
        for f in self.jobs.lock().values() {
            f.store(true, Ordering::Relaxed);
        }
    }

    fn emit_job(&self, e: JobEvent) {
        let _ = self.app.emit("job", e);
    }

    fn emit_rows(&self, ids: &[i64]) {
        let Ok(d) = self.db() else { return };
        for id in ids {
            if let Ok(row) = store::repo_row(&d, *id) {
                let _ = self.app.emit("repo-updated", row);
            }
        }
    }

    /// Run `f` over `items` on a small pool; stops early when `cancel` is set.
    fn pool<T: Sync, F: Fn(&T) + Sync>(items: &[T], threads: usize, cancel: &AtomicBool, f: F) {
        let next = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..threads.max(1).min(items.len().max(1)) {
                s.spawn(|| {
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        f(item);
                    }
                });
            }
        });
    }

    // ---------- inspection ----------

    /// Re-inspect repos whose fingerprint (or the global config) changed. `force` skips the check.
    /// Returns how many repos were re-read.
    pub fn refresh(self: &Arc<Self>, ids: Option<&[i64]>, force: bool) -> usize {
        self.refresh_with(ids, force, None, &AtomicBool::new(false))
    }

    fn refresh_with(
        self: &Arc<Self>,
        ids: Option<&[i64]>,
        force: bool,
        progress: Option<&(dyn Fn(usize, usize) + Sync)>,
        cancel: &AtomicBool,
    ) -> usize {
        let Ok(d) = self.db() else { return 0 };
        let Ok(refs) = store::repo_refs(&d, ids) else {
            return 0;
        };
        let gfp = inspect::global_fingerprint();
        let _ = d.set_setting("global_fingerprint", &gfp);
        let changed = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let total = refs.len();
        Self::pool(&refs, self.threads(), cancel, |r| {
            if !self.in_flight.lock().insert(r.id) {
                done.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let fp = inspect::git_paths(&r.path, r.bare).map(|p| inspect::fingerprint(&p));
            let stale = force
                || fp.is_none()
                || r.fingerprint.as_deref() != fp.as_deref()
                || r.global_fingerprint.as_deref() != Some(gfp.as_str());
            if stale {
                match (fp, inspect::inspect(&r.path, r.bare)) {
                    (Some(fp), Ok(ins)) => {
                        let _ = store::save_inspection(&d, r.id, &fp, &gfp, &ins);
                    }
                    (_, Err(e)) => {
                        let _ = store::save_inspect_error(&d, r.id, &e);
                    }
                    (None, Ok(_)) => {
                        let _ = store::save_inspect_error(&d, r.id, "The .git folder is missing.");
                    }
                }
                changed.fetch_add(1, Ordering::Relaxed);
                self.emit_rows(&[r.id]);
            }
            self.in_flight.lock().remove(&r.id);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(p) = progress {
                p(n, total);
            }
        });
        changed.load(Ordering::Relaxed)
    }

    // ---------- scanning ----------

    pub fn start_scan(self: &Arc<Self>, root_ids: Option<Vec<i64>>) -> AppResult<u64> {
        let d = self.db()?;
        if self.scanning.swap(true, Ordering::SeqCst) {
            return Err(AppError::msg("A scan is already running."));
        }
        let (job, cancel) = self.new_job();
        let me = self.clone();
        std::thread::spawn(move || {
            let roots: Vec<_> = store::list_roots(&d)
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.enabled && root_ids.as_ref().is_none_or(|ids| ids.contains(&r.id)))
                .collect();
            let mut all_ids = Vec::new();
            for root in &roots {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let spec = RootSpec {
                    id: root.id,
                    path: PathBuf::from(&root.path),
                    max_depth: root.max_depth.max(1) as usize,
                    follow_symlinks: root.follow_symlinks,
                    stop_at_repo: root.stop_at_repo,
                };
                if !spec.path.is_dir() {
                    continue;
                }
                let ex = Arc::new(Excludes::new(
                    &store::excludes_for(&d, root.id).unwrap_or_default(),
                ));
                let label_root = root.path.clone();
                let found =
                    scanner::walk(&spec, ex, me.threads(), cancel.clone(), |dirs, repos| {
                        me.emit_job(JobEvent {
                            id: job,
                            kind: "scan",
                            done: repos,
                            total: 0,
                            label: format!("Scanning {label_root}: {dirs} folders, {repos} repos"),
                            finished: false,
                            cancelled: false,
                            summary: HashMap::new(),
                        });
                    });
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let classified = scanner::classify(found);
                if let Ok(ids) = store::save_scan(&d, spec.id, &classified) {
                    all_ids.extend(ids);
                }
                let _ = me.app.emit("repos-reload", ());
            }
            let progress = |n: usize, total: usize| {
                if n.is_multiple_of(10) || n == total {
                    me.emit_job(JobEvent {
                        id: job,
                        kind: "scan",
                        done: n,
                        total,
                        label: format!("Reading {n} of {total} repos"),
                        finished: false,
                        cancelled: false,
                        summary: HashMap::new(),
                    });
                }
            };
            if !cancel.load(Ordering::Relaxed) {
                me.refresh_with(Some(&all_ids), false, Some(&progress), &cancel);
                if root_ids.is_none() {
                    let _ = d.set_setting("last_full_scan", &now().to_string());
                }
            }
            let cancelled = cancel.load(Ordering::Relaxed);
            me.emit_job(JobEvent {
                id: job,
                kind: "scan",
                done: all_ids.len(),
                total: all_ids.len(),
                label: if cancelled {
                    "Scan cancelled".into()
                } else {
                    format!("Found {} repos", all_ids.len())
                },
                finished: true,
                cancelled,
                summary: HashMap::from([("repos".to_string(), all_ids.len())]),
            });
            let _ = me.app.emit("repos-reload", ());
            me.end_job(job);
            me.scanning.store(false, Ordering::SeqCst);
            me.rewatch();
        });
        Ok(job)
    }

    // ---------- fetch all ----------

    pub fn start_fetch(self: &Arc<Self>, repo_ids: Vec<i64>) -> AppResult<u64> {
        let d = self.db()?;
        let (job, cancel) = self.new_job();
        let me = self.clone();
        let concurrency = match self.setting("fetch_concurrency").parse::<usize>() {
            Ok(n) if n > 0 => n.min(16),
            _ => {
                if self.threads() <= 2 {
                    2
                } else {
                    4
                }
            }
        };
        let prune = self.setting("fetch_prune") == "true";
        std::thread::spawn(move || {
            let rows = store::repo_rows(&d, None).unwrap_or_default();
            let wanted: HashSet<i64> = repo_ids.into_iter().collect();
            let targets: Vec<_> = rows
                .into_iter()
                .filter(|r| wanted.contains(&r.id))
                .collect();
            let total = targets.len();
            let done = AtomicUsize::new(0);
            let summary = Mutex::new(HashMap::<String, usize>::new());
            Self::pool(&targets, concurrency, &cancel, |r| {
                let status = if r.remote.is_none() {
                    "no_remote".to_string()
                } else {
                    let mut c = crate::git::git(Path::new(&r.path));
                    c.args(["fetch", "--all", "--quiet"]);
                    if prune {
                        c.arg("--prune");
                    }
                    match crate::git::run_limited(c, Duration::from_secs(60), Some(&cancel)) {
                        Ok(o) if o.cancelled => "cancelled".into(),
                        Ok(o) if o.timed_out => "timed_out".into(),
                        Ok(o) if o.ok() => "fetched".into(),
                        Ok(o) => {
                            let t = o.stderr.to_lowercase();
                            if t.contains("host key verification failed") {
                                "host_untrusted".into()
                            } else if t.contains("permission denied")
                                || t.contains("authentication failed")
                                || t.contains("could not read username")
                            {
                                "auth_failed".into()
                            } else if t.contains("could not resolve host")
                                || t.contains("network is unreachable")
                                || t.contains("timed out")
                            {
                                "offline".into()
                            } else {
                                "failed".into()
                            }
                        }
                        Err(_) => "failed".into(),
                    }
                };
                if status != "cancelled" {
                    let _ = store::save_fetch_result(&d, r.id, &status);
                }
                *summary.lock().entry(status.clone()).or_default() += 1;
                let _ = me.app.emit(
                    "job-item",
                    JobItem {
                        job_id: job,
                        kind: "fetch",
                        repo_id: r.id,
                        account_id: None,
                        status,
                        detail: String::new(),
                    },
                );
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                me.emit_job(JobEvent {
                    id: job,
                    kind: "fetch",
                    done: n,
                    total,
                    label: format!("Fetched {n} of {total}"),
                    finished: false,
                    cancelled: false,
                    summary: HashMap::new(),
                });
            });
            let ids: Vec<i64> = targets.iter().map(|r| r.id).collect();
            me.refresh(Some(&ids), false);
            me.emit_rows(&ids);
            let cancelled = cancel.load(Ordering::Relaxed);
            me.emit_job(JobEvent {
                id: job,
                kind: "fetch",
                done: done.load(Ordering::Relaxed),
                total,
                label: if cancelled {
                    "Fetch cancelled".into()
                } else {
                    "Fetch finished".into()
                },
                finished: true,
                cancelled,
                summary: summary.into_inner(),
            });
            me.end_job(job);
        });
        Ok(job)
    }

    // ---------- clone as an account ----------

    /// Which accounts can read `url`? Runs `git ls-remote` per account, four at a time.
    pub fn probe_clone_access(&self, url: &str) -> AppResult<Vec<CloneAccess>> {
        let d = self.db()?;
        let accs = store::list_accounts(&d)?;
        let out = Mutex::new(Vec::with_capacity(accs.len()));
        Self::pool(&accs, 4, &AtomicBool::new(false), |a| {
            let r = accounts::probe_access(url, a, None);
            out.lock().push(CloneAccess {
                account_id: a.id.unwrap_or_default(),
                status: r.status,
                detail: r.detail,
            });
        });
        Ok(out.into_inner())
    }

    /// Clone `url` into `dest` as `account_id`, then register it with that account active.
    /// `dest` must be new (or an empty folder) inside an enabled scan root.
    pub fn start_clone(
        self: &Arc<Self>,
        url: String,
        dest: PathBuf,
        account_id: i64,
    ) -> AppResult<u64> {
        let d = self.db()?;
        let acc = store::account(&d, account_id)?;
        if url.trim().is_empty() {
            return Err(AppError::msg("Enter the repository URL."));
        }
        let name = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty() && n != "." && n != "..")
            .ok_or_else(|| AppError::msg("Enter a folder name for the clone."))?;
        let parent = dest
            .parent()
            .and_then(|p| dunce::canonicalize(p).ok())
            .ok_or_else(|| AppError::msg("The destination folder does not exist."))?;
        let root = store::list_roots(&d)?
            .into_iter()
            .filter(|r| r.enabled)
            .find(|r| dunce::canonicalize(&r.path).is_ok_and(|rp| parent.starts_with(rp)))
            .ok_or_else(|| AppError::msg("Choose a folder inside one of your scan folders."))?;
        let dest = parent.join(&name);
        let created = match std::fs::read_dir(&dest) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(AppError::msg(format!(
                        "{} already exists and is not empty.",
                        dest.display()
                    )));
                }
                false
            }
            Err(_) => true,
        };

        let (job, cancel) = self.new_job();
        let me = self.clone();
        std::thread::spawn(move || {
            me.emit_job(JobEvent {
                id: job,
                kind: "clone",
                done: 0,
                total: 0,
                label: format!("Cloning {name} as {}", acc.label),
                finished: false,
                cancelled: false,
                summary: HashMap::new(),
            });
            let mut c = crate::git::git_global();
            accounts::with_auth(&mut c, &acc);
            c.arg("clone")
                .args(accounts::clone_args(&url, &acc))
                .arg("--")
                .arg(accounts::url_for(&url, &acc))
                .arg(&dest);
            let res = crate::git::run_limited(c, Duration::from_secs(30 * 60), Some(&cancel));
            let (status, detail) = match &res {
                Ok(o) if o.ok() => ("cloned".to_string(), String::new()),
                Ok(o) if o.cancelled => ("cancelled".into(), String::new()),
                Ok(o) if o.timed_out => ("timed_out".into(), "Timed out after 30 minutes.".into()),
                Ok(o) => {
                    let r = accounts::classify_failure(&o.stderr);
                    (r.status.to_string(), r.detail)
                }
                Err(e) => ("failed".into(), format!("Could not run git: {e}")),
            };
            let mut repo_id = None;
            if status == "cloned" {
                let path = dunce::canonicalize(&dest).unwrap_or(dest.clone());
                let registered = store::add_repo(&d, root.id, &path, &name).and_then(|id| {
                    store::link(&d, &[id], account_id)?;
                    store::set_active(&d, id, Some(account_id))?;
                    Ok(id)
                });
                if let Ok(id) = registered {
                    me.refresh(Some(&[id]), true);
                    repo_id = Some(id);
                }
                let _ = me.app.emit("repos-reload", ());
                me.rewatch();
            } else if created {
                // Only remove what this clone created.
                let _ = std::fs::remove_dir_all(&dest);
            }
            if let Some(id) = repo_id {
                let _ = me.app.emit(
                    "job-item",
                    JobItem {
                        job_id: job,
                        kind: "clone",
                        repo_id: id,
                        account_id: Some(account_id),
                        status: status.clone(),
                        detail: detail.clone(),
                    },
                );
            }
            let cancelled = status == "cancelled";
            me.emit_job(JobEvent {
                id: job,
                kind: "clone",
                done: 1,
                total: 1,
                label: match status.as_str() {
                    "cloned" => format!("Cloned {name} as {}", acc.label),
                    "cancelled" => "Clone cancelled".into(),
                    _ => format!("Clone failed: {}", detail.trim()),
                },
                finished: true,
                cancelled,
                summary: if cancelled {
                    HashMap::new()
                } else {
                    HashMap::from([(status, 1)])
                },
            });
            me.end_job(job);
        });
        Ok(job)
    }

    // ---------- push check and access scan ----------

    pub fn check_one(&self, repo_id: i64, account_id: i64) -> AppResult<accounts::CheckResult> {
        let d = self.db()?;
        let row = store::repo_row(&d, repo_id)?;
        let acc = store::account(&d, account_id)?;
        let remote = row
            .remote
            .ok_or_else(|| AppError::msg("This repo has no remote to check."))?;
        let who = (acc.auth_method == "ssh").then(|| accounts::whoami(&acc, None));
        let res = accounts::push_check(Path::new(&row.path), &remote.url, &acc, who.as_ref(), None);
        store::save_push_check(&d, repo_id, account_id, res.status, &res.detail)?;
        self.emit_rows(&[repo_id]);
        Ok(res)
    }

    pub fn start_access_scan(
        self: &Arc<Self>,
        changed_only: bool,
        repo_ids: Option<Vec<i64>>,
    ) -> AppResult<u64> {
        let d = self.db()?;
        let accs = store::list_accounts(&d)?;
        if accs.is_empty() {
            return Err(AppError::msg("Add an account first."));
        }
        let (job, cancel) = self.new_job();
        let me = self.clone();
        std::thread::spawn(move || {
            let rows: Vec<_> = store::repo_rows(&d, None)
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.remote.as_ref().is_some_and(|x| x.is_github))
                .filter(|r| repo_ids.as_ref().is_none_or(|ids| ids.contains(&r.id)))
                .collect();
            // Identity per account, once.
            let mut who: HashMap<i64, Option<accounts::CheckResult>> = HashMap::new();
            for a in &accs {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                me.emit_job(JobEvent {
                    id: job,
                    kind: "access",
                    done: 0,
                    total: 0,
                    label: format!("Checking who {} signs in as", a.label),
                    finished: false,
                    cancelled: false,
                    summary: HashMap::new(),
                });
                let w = (a.auth_method == "ssh").then(|| accounts::whoami(a, Some(&cancel)));
                if let Some(r) = &w
                    && !cancel.load(Ordering::Relaxed)
                {
                    let _ = store::save_account_check(
                        &d,
                        a.id.unwrap(),
                        r.status,
                        &r.detail,
                        r.username.as_deref(),
                    );
                }
                who.insert(a.id.unwrap(), w);
            }
            let week = 7 * 24 * 3600;
            let pairs: Vec<(usize, usize)> = (0..rows.len())
                .flat_map(|ri| (0..accs.len()).map(move |ai| (ri, ai)))
                .filter(|(ri, ai)| {
                    !changed_only
                        || store::last_check_at(&d, rows[*ri].id, accs[*ai].id.unwrap())
                            .is_none_or(|t| now() - t > week)
                })
                .collect();
            let total = pairs.len();
            let done = AtomicUsize::new(0);
            let summary = Mutex::new(HashMap::<String, usize>::new());
            let concurrency = if me.threads() <= 2 { 2 } else { 4 };
            Self::pool(&pairs, concurrency, &cancel, |(ri, ai)| {
                let (r, a) = (&rows[*ri], &accs[*ai]);
                let aid = a.id.unwrap();
                let url = &r.remote.as_ref().unwrap().url;
                let res = accounts::push_check(
                    Path::new(&r.path),
                    url,
                    a,
                    who.get(&aid).and_then(Option::as_ref),
                    Some(&cancel),
                );
                if !cancel.load(Ordering::Relaxed) {
                    let _ = store::save_push_check(&d, r.id, aid, res.status, &res.detail);
                    *summary.lock().entry(res.status.to_string()).or_default() += 1;
                    let _ = me.app.emit(
                        "job-item",
                        JobItem {
                            job_id: job,
                            kind: "access",
                            repo_id: r.id,
                            account_id: Some(aid),
                            status: res.status.into(),
                            detail: res.detail.clone(),
                        },
                    );
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                me.emit_job(JobEvent {
                    id: job,
                    kind: "access",
                    done: n,
                    total,
                    label: format!("Checked {n} of {total} repo/account pairs"),
                    finished: false,
                    cancelled: false,
                    summary: HashMap::new(),
                });
            });
            let cancelled = cancel.load(Ordering::Relaxed);
            let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
            me.emit_rows(&ids);
            me.emit_job(JobEvent {
                id: job,
                kind: "access",
                done: done.load(Ordering::Relaxed),
                total,
                label: if cancelled {
                    "Access scan cancelled".into()
                } else {
                    "Access scan finished".into()
                },
                finished: true,
                cancelled,
                summary: summary.into_inner(),
            });
            me.end_job(job);
        });
        Ok(job)
    }

    // ---------- watching and scheduling ----------

    /// Watch a few files per repo, or fall back to polling above the threshold.
    pub fn rewatch(self: &Arc<Self>) {
        *self.watcher.lock() = None;
        self.watched.lock().clear();
        let Ok(d) = self.db() else { return };
        let refs = store::repo_refs(&d, None).unwrap_or_default();
        let threshold = self.setting_num("watch_threshold", 300) as usize;
        if refs.len() > threshold {
            self.polling.store(true, Ordering::Relaxed);
            return;
        }
        let watched = self.watched.clone();
        let dirty = self.dirty.clone();
        let handler = move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            let map = watched.lock();
            for p in &ev.paths {
                let s = p.to_string_lossy();
                if s.ends_with(".lock")
                    || s.contains("/objects/")
                    || s.contains("\\objects\\")
                    || s.contains("/logs/")
                    || s.contains("\\logs\\")
                {
                    continue;
                }
                if let Some(ids) = p.ancestors().take(8).find_map(|a| map.get(a)) {
                    let mut q = dirty.lock();
                    for id in ids {
                        q.insert(*id, Instant::now());
                    }
                }
            }
        };
        let mut w = match notify::recommended_watcher(handler) {
            Ok(w) => w,
            Err(_) => {
                self.polling.store(true, Ordering::Relaxed);
                return;
            }
        };
        let mut map: HashMap<PathBuf, Vec<i64>> = HashMap::new();
        let mut failures = 0;
        for r in &refs {
            let Some(p) = inspect::git_paths(&r.path, r.bare) else {
                continue;
            };
            let mut targets = vec![
                (p.git_dir.clone(), RecursiveMode::NonRecursive),
                (p.common_dir.join("refs"), RecursiveMode::Recursive),
            ];
            if p.common_dir != p.git_dir {
                targets.push((p.common_dir.clone(), RecursiveMode::NonRecursive));
            }
            for (t, mode) in targets {
                let entry = map.entry(t.clone()).or_default();
                if entry.is_empty() && w.watch(&t, mode).is_err() {
                    failures += 1;
                }
                entry.push(r.id);
            }
        }
        *self.watched.lock() = map;
        if failures > refs.len() / 2 && !refs.is_empty() {
            // Out of watch handles (Linux inotify limits): poll instead.
            self.polling.store(true, Ordering::Relaxed);
            return;
        }
        self.polling.store(false, Ordering::Relaxed);
        *self.watcher.lock() = Some(w);
    }

    pub fn on_focus(self: &Arc<Self>, focused: bool) {
        self.focused.store(focused, Ordering::Relaxed);
        if !focused {
            return;
        }
        let mut last = self.last_focus_pass.lock();
        if last.is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
            return;
        }
        *last = Some(Instant::now());
        let me = self.clone();
        std::thread::spawn(move || {
            me.refresh(None, false);
        });
    }

    /// Debounces watcher events (2 s) and runs polling and periodic rescans.
    pub fn start_background(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            let mut last_global = String::new();
            let mut tick = 0u64;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                tick += 1;
                if me.db().is_err() {
                    continue;
                }
                let ready: Vec<i64> = {
                    let mut q = me.dirty.lock();
                    let ids: Vec<i64> = q
                        .iter()
                        .filter(|(_, t)| t.elapsed() >= Duration::from_secs(2))
                        .map(|(id, _)| *id)
                        .collect();
                    for id in &ids {
                        q.remove(id);
                    }
                    ids
                };
                if !ready.is_empty() {
                    me.refresh(Some(&ready), false);
                }
                if !tick.is_multiple_of(30) {
                    continue; // the rest runs every 15 s
                }
                let saver = me.battery_saver();
                let g = inspect::global_fingerprint();
                if !last_global.is_empty() && g != last_global {
                    me.refresh(None, false);
                }
                last_global = g;
                if me.polling.load(Ordering::Relaxed) {
                    let every =
                        me.setting_num("poll_interval_secs", 60) * if saver { 4 } else { 1 };
                    if me.last_poll.lock().elapsed() >= Duration::from_secs(every) {
                        *me.last_poll.lock() = Instant::now();
                        me.refresh(None, false);
                    }
                }
                let hours = me.setting_num("rescan_hours", 24);
                let last_scan: i64 = me.setting("last_full_scan").parse().unwrap_or(0);
                let idle = !me.focused.load(Ordering::Relaxed);
                if !saver
                    && hours > 0
                    && idle
                    && last_scan > 0
                    && now() - last_scan >= (hours * 3600) as i64
                {
                    let _ = me.start_scan(None);
                }
            }
        });
    }
}
