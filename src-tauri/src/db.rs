//! Encrypted local store: SQLCipher database whose key lives in the OS keychain,
//! with an Argon2id passphrase fallback for machines without a keychain.

use crate::error::{AppError, AppResult};
use parking_lot::Mutex;
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

const KEYCHAIN_SERVICE: &str = "com.repodeck.app";
const KEYCHAIN_ACCOUNT: &str = "database-key";
const DB_FILE: &str = "repodeck.db";
const SALT_FILE: &str = "passphrase.salt";

pub const DEFAULT_EXCLUDES: &[&str] = &[
    "node_modules",
    ".venv",
    "venv",
    "target",
    "vendor",
    "build",
    "dist",
    ".cache",
    "~/Library",
    "**/AppData",
    "$RECYCLE.BIN",
    ".Trash",
];

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS scan_roots (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  max_depth INTEGER NOT NULL DEFAULT 8,
  follow_symlinks INTEGER NOT NULL DEFAULT 0,
  stop_at_repo INTEGER NOT NULL DEFAULT 0,
  enabled INTEGER NOT NULL DEFAULT 1,
  last_scanned_at INTEGER
);
CREATE TABLE IF NOT EXISTS exclude_rules (
  id INTEGER PRIMARY KEY,
  root_id INTEGER REFERENCES scan_roots(id) ON DELETE CASCADE,
  glob TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS repos (
  id INTEGER PRIMARY KEY,
  canonical_path TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  parent_repo_id INTEGER REFERENCES repos(id) ON DELETE SET NULL,
  root_id INTEGER REFERENCES scan_roots(id) ON DELETE SET NULL,
  is_bare INTEGER NOT NULL DEFAULT 0,
  embedded_warning INTEGER NOT NULL DEFAULT 0,
  fingerprint TEXT,
  global_fingerprint TEXT,
  last_scanned_at INTEGER,
  inspected_at INTEGER,
  head_branch TEXT,
  is_dirty INTEGER,
  last_commit_at INTEGER,
  branches_json TEXT,
  recent_emails_json TEXT,
  last_fetch_at INTEGER,
  last_fetch_result TEXT,
  inspect_error TEXT
);
CREATE TABLE IF NOT EXISTS accounts (
  id INTEGER PRIMARY KEY,
  label TEXT NOT NULL,
  user_name TEXT NOT NULL,
  user_email TEXT NOT NULL,
  host TEXT NOT NULL DEFAULT 'github.com',
  auth_method TEXT NOT NULL DEFAULT 'ssh',
  ssh_key_path TEXT,
  expected_username TEXT,
  owners TEXT
);
CREATE TABLE IF NOT EXISTS repo_accounts (
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  is_active INTEGER NOT NULL DEFAULT 0,
  linked_at INTEGER NOT NULL,
  applied_at INTEGER,
  PRIMARY KEY (repo_id, account_id)
);
CREATE TABLE IF NOT EXISTS identity_snapshots (
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  ord INTEGER NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  scope TEXT NOT NULL,
  origin_file TEXT NOT NULL,
  read_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_identity_repo ON identity_snapshots(repo_id);
CREATE TABLE IF NOT EXISTS remotes (
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  host TEXT,
  owner TEXT,
  repo_name TEXT,
  PRIMARY KEY (repo_id, name)
);
CREATE TABLE IF NOT EXISTS push_checks (
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  detail TEXT,
  checked_at INTEGER NOT NULL,
  PRIMARY KEY (repo_id, account_id)
);
CREATE TABLE IF NOT EXISTS graph_meta (
  repo_id INTEGER PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
  fingerprint TEXT NOT NULL,
  row_count INTEGER NOT NULL,
  complete INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS commits (
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  ord INTEGER NOT NULL,
  hash TEXT NOT NULL,
  parents TEXT NOT NULL,
  author_name TEXT NOT NULL,
  author_email TEXT NOT NULL,
  authored_at INTEGER NOT NULL,
  subject TEXT NOT NULL,
  refs TEXT NOT NULL,
  lane INTEGER NOT NULL,
  segments TEXT NOT NULL,
  PRIMARY KEY (repo_id, ord)
);
CREATE TABLE IF NOT EXISTS account_checks (
  account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  detail TEXT,
  username TEXT,
  checked_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

pub struct Db {
    pub conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path, key: &[u8; 32]) -> AppResult<Db> {
        let conn = Connection::open(path)?;
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex::encode(key)))?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|_| AppError::msg("The database could not be decrypted with this key."))?;
        conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        let db = Db {
            conn: Mutex::new(conn),
        };
        db.seed()?;
        Ok(db)
    }

    fn seed(&self) -> AppResult<()> {
        let c = self.conn.lock();
        let seeded: Option<String> = c
            .query_row("SELECT value FROM settings WHERE key='seeded'", [], |r| {
                r.get(0)
            })
            .optional()?;
        if seeded.is_none() {
            for g in DEFAULT_EXCLUDES {
                c.execute(
                    "INSERT INTO exclude_rules(root_id, glob) VALUES (NULL, ?1)",
                    params![g],
                )?;
            }
            c.execute(
                "INSERT INTO settings(key, value) VALUES ('seeded', '1')",
                [],
            )?;
        }
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.conn
            .lock()
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.conn.lock().execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

/// How the database key is obtained on this machine.
pub enum KeyOutcome {
    Ready([u8; 32]),
    NeedsPassphrase { first_time: bool },
    Lost,
}

pub fn db_path(dir: &Path) -> PathBuf {
    dir.join(DB_FILE)
}

/// Load the key from the OS keychain, creating one on first launch.
pub fn keychain_key(dir: &Path) -> KeyOutcome {
    let db_exists = db_path(dir).exists();
    if dir.join(SALT_FILE).exists() {
        return KeyOutcome::NeedsPassphrase { first_time: false };
    }
    let entry = match keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
        Ok(e) => e,
        Err(_) => {
            return KeyOutcome::NeedsPassphrase {
                first_time: !db_exists,
            };
        }
    };
    match entry.get_password() {
        Ok(h) => match hex::decode(h.trim()) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut k = [0u8; 32];
                k.copy_from_slice(&bytes);
                KeyOutcome::Ready(k)
            }
            _ => KeyOutcome::Lost,
        },
        Err(keyring::Error::NoEntry) => {
            if db_exists {
                return KeyOutcome::Lost;
            }
            let mut k = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut k);
            match entry.set_password(&hex::encode(k)) {
                Ok(()) => KeyOutcome::Ready(k),
                Err(_) => KeyOutcome::NeedsPassphrase { first_time: true },
            }
        }
        Err(_) => KeyOutcome::NeedsPassphrase {
            first_time: !db_exists,
        },
    }
}

/// Derive the key from a passphrase with Argon2id. The salt is not secret.
pub fn passphrase_key(dir: &Path, passphrase: &str) -> AppResult<[u8; 32]> {
    let salt_path = dir.join(SALT_FILE);
    let salt = match std::fs::read(&salt_path) {
        Ok(s) if s.len() >= 16 => s,
        _ => {
            let mut s = vec![0u8; 16];
            rand::thread_rng().fill_bytes(&mut s);
            std::fs::write(&salt_path, &s)?;
            s
        }
    };
    let mut key = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), &salt, &mut key)
        .map_err(|e| AppError::msg(format!("key derivation failed: {e}")))?;
    Ok(key)
}

/// Remove the local database and key material so the app can start fresh.
pub fn reset(dir: &Path) -> AppResult<()> {
    for f in [DB_FILE, "repodeck.db-wal", "repodeck.db-shm", SALT_FILE] {
        let p = dir.join(f);
        if p.exists() {
            std::fs::remove_file(p)?;
        }
    }
    if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
        let _ = e.delete_credential();
    }
    Ok(())
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
