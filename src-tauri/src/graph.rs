//! Commit graph: topo-ordered history plus a lane assignment pass, like
//! `git log --graph` but as data the UI draws.

use crate::git;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A line drawn within one row. `y` is 0 = top edge, 1 = node centre, 2 = bottom edge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Seg {
    pub x1: u16,
    pub y1: u8,
    pub x2: u16,
    pub y2: u8,
    pub c: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub hash: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: i64,
    pub subject: String,
    pub refs: String,
    pub lane: u16,
    pub segments: Vec<Seg>,
}

pub struct RawCommit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: i64,
    pub subject: String,
    pub refs: String,
}

pub fn read_log(repo: &Path, limit: usize) -> Result<Vec<RawCommit>, String> {
    let mut c = git::git(repo);
    c.args([
        "log",
        "--branches",
        "--tags",
        "HEAD",
        "--topo-order",
        "-n",
        &limit.to_string(),
        "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e",
        "--",
    ]);
    let o = git::run(c).map_err(|e| e.to_string())?;
    if !o.ok() {
        let err = o.stderr.trim();
        // Empty repository: no commits yet.
        if err.contains("does not have any commits")
            || err.contains("unknown revision")
            || err.contains("bad revision")
        {
            return Ok(vec![]);
        }
        return Err(err.to_string());
    }
    Ok(o.stdout
        .split('\x1e')
        .filter_map(|rec| {
            let rec = rec.trim_start_matches('\n');
            if rec.is_empty() {
                return None;
            }
            let f: Vec<&str> = rec.split('\x1f').collect();
            if f.len() < 7 {
                return None;
            }
            Some(RawCommit {
                hash: f[0].to_string(),
                parents: f[1].split_whitespace().map(str::to_string).collect(),
                author_name: f[2].to_string(),
                author_email: f[3].to_string(),
                authored_at: f[4].parse().unwrap_or(0),
                subject: f[5].to_string(),
                refs: f[6].trim().to_string(),
            })
        })
        .collect())
}

/// Assign each commit a column and compute the line segments for its row.
/// Lanes never move sideways, so pass-through lines stay straight.
pub fn layout(commits: Vec<RawCommit>) -> Vec<Row> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    for c in commits {
        let mut segs = Vec::new();
        let before = lanes.clone();
        let incoming: Vec<usize> = before
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_deref() == Some(c.hash.as_str()))
            .map(|(i, _)| i)
            .collect();
        let col = match incoming.first() {
            Some(&i) => i,
            None => match lanes.iter().position(Option::is_none) {
                Some(i) => i,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            },
        };
        for &i in &incoming {
            segs.push(Seg {
                x1: i as u16,
                y1: 0,
                x2: col as u16,
                y2: 1,
                c: i as u16,
            });
            lanes[i] = None;
        }
        for (pi, p) in c.parents.iter().enumerate() {
            let existing = lanes.iter().position(|l| l.as_deref() == Some(p.as_str()));
            if pi == 0 && lanes[col].is_none() {
                match existing {
                    // The first parent pulls a lane on its right back into this column.
                    Some(j) if j > col => lanes[j] = None,
                    Some(j) => {
                        segs.push(Seg {
                            x1: col as u16,
                            y1: 1,
                            x2: j as u16,
                            y2: 2,
                            c: col as u16,
                        });
                        continue;
                    }
                    None => {}
                }
                lanes[col] = Some(p.clone());
                segs.push(Seg {
                    x1: col as u16,
                    y1: 1,
                    x2: col as u16,
                    y2: 2,
                    c: col as u16,
                });
                continue;
            }
            if let Some(j) = existing {
                segs.push(Seg {
                    x1: col as u16,
                    y1: 1,
                    x2: j as u16,
                    y2: 2,
                    c: j.max(col) as u16,
                });
                continue;
            }
            let target = match lanes
                .iter()
                .enumerate()
                .position(|(k, l)| l.is_none() && k != col)
            {
                Some(k) => k,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            };
            lanes[target] = Some(p.clone());
            segs.push(Seg {
                x1: col as u16,
                y1: 1,
                x2: target as u16,
                y2: 2,
                c: target as u16,
            });
        }
        for (i, l) in before.iter().enumerate() {
            if let Some(h) = l
                && h != &c.hash
            {
                let j = lanes
                    .iter()
                    .position(|l| l.as_ref() == Some(h))
                    .unwrap_or(i);
                segs.push(Seg {
                    x1: i as u16,
                    y1: 0,
                    x2: j as u16,
                    y2: 2,
                    c: i as u16,
                });
            }
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        rows.push(Row {
            hash: c.hash,
            parents: c.parents,
            author_name: c.author_name,
            author_email: c.author_email,
            authored_at: c.authored_at,
            subject: c.subject,
            refs: c.refs,
            lane: col as u16,
            segments: segs,
        });
    }
    rows
}

#[derive(Debug, Serialize)]
pub struct ChangedFile {
    pub status: String,
    pub path: String,
}

/// Changed files for one commit, read on demand and never cached.
pub fn changed_files(
    repo: &Path,
    hash: &str,
    first_parent: Option<&str>,
) -> Result<Vec<ChangedFile>, String> {
    if !hash.chars().all(|c| c.is_ascii_hexdigit())
        || first_parent.is_some_and(|p| !p.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Err("invalid commit id".into());
    }
    let mut c = git::git(repo);
    match first_parent {
        Some(p) => c.args(["diff", "--name-status", "--no-renames", p, hash, "--"]),
        None => c.args([
            "show",
            "--format=",
            "--name-status",
            "--no-renames",
            hash,
            "--",
        ]),
    };
    let o = git::run(c).map_err(|e| e.to_string())?;
    if !o.ok() {
        return Err(o.stderr.trim().to_string());
    }
    Ok(o.stdout
        .lines()
        .filter_map(|l| {
            let (s, p) = l.split_once('\t')?;
            Some(ChangedFile {
                status: s.to_string(),
                path: p.to_string(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rc(h: &str, p: &[&str]) -> RawCommit {
        RawCommit {
            hash: h.into(),
            parents: p.iter().map(|s| s.to_string()).collect(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at: 0,
            subject: String::new(),
            refs: String::new(),
        }
    }

    #[test]
    fn merge_and_branch_lanes() {
        // m merges a (main) and b (feature); both come from r.
        let rows = layout(vec![
            rc("m", &["a", "b"]),
            rc("b", &["r"]),
            rc("a", &["r"]),
            rc("r", &[]),
        ]);
        let lanes: Vec<u16> = rows.iter().map(|r| r.lane).collect();
        assert_eq!(lanes, vec![0, 1, 0, 0]);
        // The feature lane folds back into lane 0 on a's row, so r sits in lane 0.
        assert!(rows[2].segments.contains(&Seg {
            x1: 1,
            y1: 0,
            x2: 0,
            y2: 2,
            c: 1
        }));
        assert_eq!(
            rows[3]
                .segments
                .iter()
                .filter(|s| s.y1 == 0 && s.y2 == 1)
                .count(),
            1
        );
    }
}
