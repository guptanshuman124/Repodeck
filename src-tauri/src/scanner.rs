//! Discovery: walk scan roots in parallel and classify repo-inside-repo cases.

use globset::{Glob, GlobSet, GlobSetBuilder};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct RootSpec {
    pub id: i64,
    pub path: PathBuf,
    pub max_depth: usize,
    pub follow_symlinks: bool,
    pub stop_at_repo: bool,
}

#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    pub bare: bool,
}

#[derive(Debug, Clone)]
pub struct Classified {
    pub path: PathBuf,
    pub name: String,
    pub kind: &'static str,
    pub is_bare: bool,
    pub parent: Option<PathBuf>,
    pub embedded_warning: bool,
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\"))
        && let Some(h) = home()
    {
        return h.join(rest);
    }
    if p == "~"
        && let Some(h) = home()
    {
        return h;
    }
    PathBuf::from(p)
}

/// Exclude rules: bare names match any folder name, patterns with a slash match full paths.
pub struct Excludes {
    names: GlobSet,
    paths: GlobSet,
}

impl Excludes {
    pub fn new(globs: &[String]) -> Excludes {
        let mut names = GlobSetBuilder::new();
        let mut paths = GlobSetBuilder::new();
        for g in globs {
            let g = g.trim();
            if g.is_empty() {
                continue;
            }
            if g.contains('/') || g.contains('\\') || g.starts_with('~') {
                let expanded = expand_tilde(g).to_string_lossy().replace('\\', "/");
                if let Ok(glob) = Glob::new(&expanded) {
                    paths.add(glob);
                }
            } else if let Ok(glob) = Glob::new(g) {
                names.add(glob);
            }
        }
        Excludes {
            names: names.build().unwrap_or_else(|_| GlobSet::empty()),
            paths: paths.build().unwrap_or_else(|_| GlobSet::empty()),
        }
    }

    fn excluded(&self, name: &str, full: &Path) -> bool {
        self.names.is_match(name)
            || self
                .paths
                .is_match(full.to_string_lossy().replace('\\', "/"))
    }
}

/// Walk one root. Calls `progress(dirs_seen, repos_found)` periodically.
pub fn walk(
    root: &RootSpec,
    excludes: Arc<Excludes>,
    threads: usize,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(usize, usize),
) -> Vec<Found> {
    let found: Arc<Mutex<Vec<Found>>> = Arc::new(Mutex::new(Vec::new()));
    let dirs = Arc::new(AtomicUsize::new(0));
    let stop_at_repo = root.stop_at_repo;
    let (f2, d2, c2) = (found.clone(), dirs.clone(), cancel.clone());

    let walker = jwalk::WalkDirGeneric::<((), ())>::new(&root.path)
        .skip_hidden(false)
        .follow_links(root.follow_symlinks)
        // One extra level so a repo at max_depth still gets its `.git` looked at.
        .max_depth(root.max_depth + 1)
        .parallelism(jwalk::Parallelism::RayonNewPool(threads.max(1)))
        .process_read_dir(move |depth, path, _state, children| {
            if c2.load(Ordering::Relaxed) {
                children.clear();
                return;
            }
            d2.fetch_add(1, Ordering::Relaxed);
            if depth.is_none() {
                return; // the synthetic parent of the root
            }
            let mut has_git = false;
            let (mut head, mut objects, mut refs) = (false, false, false);
            for c in children.iter().flatten() {
                match c.file_name.to_str() {
                    Some(".git") => has_git = true,
                    Some("HEAD") if c.file_type.is_file() => head = true,
                    Some("objects") if c.file_type.is_dir() => objects = true,
                    Some("refs") if c.file_type.is_dir() => refs = true,
                    _ => {}
                }
            }
            let bare = !has_git && head && objects && refs;
            if has_git || bare {
                f2.lock().push(Found {
                    path: path.to_path_buf(),
                    bare,
                });
            }
            // Never descend into .git; never descend into bare repos.
            children.retain(|c| match c {
                Ok(e) => {
                    if e.file_name == ".git" {
                        return false;
                    }
                    if e.file_type.is_dir() {
                        let name = e.file_name.to_string_lossy();
                        return !excludes.excluded(&name, &path.join(&e.file_name));
                    }
                    false // files are never interesting past this point
                }
                Err(_) => false,
            });
            if bare || (has_git && stop_at_repo) {
                for c in children.iter_mut().flatten() {
                    c.read_children = None;
                }
            }
        });

    let mut n = 0usize;
    for _ in walker {
        n += 1;
        if n.is_multiple_of(512) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            progress(dirs.load(Ordering::Relaxed), found.lock().len());
        }
    }
    progress(dirs.load(Ordering::Relaxed), found.lock().len());

    std::mem::take(&mut *found.lock())
}

fn read_gitdir_file(repo: &Path) -> Option<PathBuf> {
    let s = std::fs::read_to_string(repo.join(".git")).ok()?;
    let rel = s.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let p = PathBuf::from(rel);
    let p = if p.is_absolute() { p } else { repo.join(p) };
    Some(dunce::canonicalize(&p).unwrap_or(p))
}

fn gitmodules_paths(repo: &Path) -> HashSet<PathBuf> {
    let Ok(s) = std::fs::read_to_string(repo.join(".gitmodules")) else {
        return HashSet::new();
    };
    s.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (k, v) = l.split_once('=')?;
            (k.trim() == "path").then(|| PathBuf::from(v.trim().trim_matches('"')))
        })
        .collect()
}

fn embedded_in_parent(parent: &Path, rel: &Path) -> bool {
    let mut c = crate::git::git(parent);
    c.args(["ls-files", "-s", "--"]).arg(rel);
    crate::git::run(c)
        .map(|o| o.stdout.lines().any(|l| l.starts_with("160000")))
        .unwrap_or(false)
}

/// Classify found repos as normal, submodule, worktree, nested or bare.
pub fn classify(found: Vec<Found>) -> Vec<Classified> {
    let mut by_path: HashMap<PathBuf, bool> = HashMap::new();
    for f in found {
        let p = dunce::canonicalize(&f.path).unwrap_or(f.path);
        by_path.entry(p).or_insert(f.bare);
    }
    let set: HashSet<PathBuf> = by_path.keys().cloned().collect();
    let mut modules_cache: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
    let mut paths: Vec<_> = by_path.into_iter().collect();
    paths.sort_by(|a, b| a.0.cmp(&b.0));

    paths
        .into_iter()
        .map(|(path, bare)| {
            let raw_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let name = raw_name
                .strip_suffix(".git")
                .filter(|n| !n.is_empty())
                .unwrap_or(&raw_name)
                .to_string();
            let ancestor = path
                .ancestors()
                .skip(1)
                .find(|a| set.contains(*a))
                .map(Path::to_path_buf);
            if bare {
                return Classified {
                    path,
                    name,
                    kind: "bare",
                    is_bare: true,
                    parent: ancestor,
                    embedded_warning: false,
                };
            }
            let dotgit_is_file = path.join(".git").is_file();
            if dotgit_is_file && let Some(gitdir) = read_gitdir_file(&path) {
                let g = gitdir.to_string_lossy().replace('\\', "/");
                if g.contains("/worktrees/") {
                    // <main>/.git/worktrees/<name> -> <main>
                    let main = gitdir.parent().and_then(Path::parent).and_then(|dotgit| {
                        if dotgit.file_name().is_some_and(|n| n == ".git") {
                            dotgit.parent().map(Path::to_path_buf)
                        } else {
                            Some(dotgit.to_path_buf()) // bare main repo
                        }
                    });
                    return Classified {
                        path,
                        name,
                        kind: "worktree",
                        is_bare: false,
                        parent: main,
                        embedded_warning: false,
                    };
                }
                if g.contains("/modules/") {
                    return Classified {
                        path,
                        name,
                        kind: "submodule",
                        is_bare: false,
                        parent: ancestor,
                        embedded_warning: false,
                    };
                }
            }
            if let Some(anc) = ancestor {
                let rel = path
                    .strip_prefix(&anc)
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                let modules = modules_cache
                    .entry(anc.clone())
                    .or_insert_with(|| gitmodules_paths(&anc));
                if modules.contains(&rel) {
                    return Classified {
                        path,
                        name,
                        kind: "submodule",
                        is_bare: false,
                        parent: Some(anc),
                        embedded_warning: false,
                    };
                }
                let embedded = embedded_in_parent(&anc, &rel);
                return Classified {
                    path,
                    name,
                    kind: "nested",
                    is_bare: false,
                    parent: Some(anc),
                    embedded_warning: embedded,
                };
            }
            Classified {
                path,
                name,
                kind: "normal",
                is_bare: false,
                parent: None,
                embedded_warning: false,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
        assert!(ok, "git {args:?} failed in {dir:?}");
    }

    #[test]
    fn walks_and_classifies() {
        let tmp = std::env::temp_dir().join(format!("repodeck-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let main = tmp.join("main");
        std::fs::create_dir_all(main.join("node_modules/dep")).unwrap();
        git(&main, &["init", "-q"]);
        git(&main.join("node_modules/dep"), &["init", "-q"]);
        let nested = main.join("tools/inner");
        std::fs::create_dir_all(&nested).unwrap();
        git(&nested, &["init", "-q"]);
        git(
            &main,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        git(&main, &["worktree", "add", "-q", "../wt"]);
        git(&tmp, &["init", "-q", "--bare", "bare.git"]);

        let root = RootSpec {
            id: 1,
            path: tmp.clone(),
            max_depth: 8,
            follow_symlinks: false,
            stop_at_repo: false,
        };
        let ex = Arc::new(Excludes::new(
            &crate::db::DEFAULT_EXCLUDES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        ));
        let found = walk(&root, ex, 2, Arc::new(AtomicBool::new(false)), |_, _| {});
        let mut c = classify(found);
        c.sort_by(|a, b| a.name.cmp(&b.name));
        let kinds: Vec<_> = c.iter().map(|r| (r.name.as_str(), r.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("bare", "bare"),
                ("inner", "nested"),
                ("main", "normal"),
                ("wt", "worktree")
            ]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
