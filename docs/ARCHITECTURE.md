# Architecture

How RepoDeck's code fits together. For what the app does from a user's point of view, see
[HOW-IT-WORKS.md](HOW-IT-WORKS.md).

## The big picture

```
┌──────────────────────── RepoDeck process ────────────────────────┐
│                                                                  │
│  WebView (Svelte UI)                Rust core (Tauri)            │
│  ┌────────────────────┐   invoke   ┌──────────────────────────┐  │
│  │ components/*.svelte │ ────────▶ │ commands.rs (43 commands) │  │
│  │ state.svelte.ts     │           │        │                  │  │
│  │ api.ts              │ ◀──────── │ core.rs: jobs, pool,      │  │
│  └────────────────────┘   events   │ watcher, scheduler        │  │
│                                    │   │      │       │         │  │
│                                    │ scanner inspect accounts  │  │
│                                    │ graph   remote_url        │  │
│                                    │   │                       │  │
│                                    │ store.rs ── db.rs ─────────┼──┼─▶ repodeck.db (SQLCipher)
│                                    └───┬──────────────────────┘  │       key ◀── OS keychain
└────────────────────────────────────────┼─────────────────────────┘
                                         │ child processes
                                         ▼
                              git · ssh · editor CLI
```

- **One process.** Tauri runs the Rust core and hosts the UI in the system WebView (WKWebView on
  macOS, WebView2 on Windows, WebKitGTK on Linux).
- **The UI never touches the filesystem or runs programs.** It calls Rust commands and listens for
  events. All Git work happens in Rust.
- **Git is used through its CLI.** RepoDeck runs your installed `git` and `ssh` as child
  processes, so identity resolution, includeIf rules, credential helpers and ssh config behave
  exactly as they do in your terminal. Processes are only spawned when a repo's fingerprint has
  changed or you ask for something, which keeps this cheap.

## Start-up sequence

1. `main.rs` calls `repodeck_lib::run()` in `lib.rs`.
2. On macOS/Linux, `inherit_shell_path()` runs your login shell once to copy its `PATH`. Apps
   launched from Finder or a desktop menu otherwise get a minimal `PATH` and can't find Homebrew's
   git or the `code` CLI. This runs before any thread starts, which is what makes `set_var` safe.
3. `setup` creates the app data folder, builds `Core`, and calls `try_open_with_keychain()`:
   - key in the keychain → open the database (`DbState::Ready`)
   - no keychain service → `NeedsPassphrase` (UI shows the passphrase screen)
   - database exists but its key is gone → `KeyLost` (UI offers "start fresh")
4. `start_background()` starts the scheduler thread (see [Background work](#background-work)).
5. The UI loads. `routes/+page.svelte` calls `app_status` and shows the first gate that applies:
   **git missing → unlock → first-run setup → main screen**. Once the database is ready,
   `app.loadAll()` fetches roots, repos, accounts and settings, and `app.listen()` subscribes to
   backend events.

## Rust modules (`src-tauri/src/`)

| Module | Responsibility |
| --- | --- |
| `lib.rs` | Module wiring, plugins (dialog, opener), `setup`, window-focus hook, command registration |
| `commands.rs` | Every `#[tauri::command]`. Thin: validates input, loads by ID, calls into the modules below, runs blocking work on `spawn_blocking` |
| `core.rs` | `Core`, the shared app state: database handle, job registry with cancel flags, the inspection pool, scan / fetch / access-scan / clone jobs, file watcher, scheduler, settings defaults |
| `db.rs` | Opening SQLCipher, the schema, keychain key (`keychain_key`), passphrase key (`passphrase_key`, Argon2id), `reset` |
| `store.rs` | All SQL. Saving scan and inspection results, accounts and links, push-check results, the graph cache, and `repo_rows()`, which builds list rows and computes the offline problems |
| `scanner.rs` | Walking scan roots with `jwalk`, exclude globs, classifying submodule / worktree / nested / bare, detecting embedded repos |
| `inspect.rs` | Fingerprints, reading config with origins and scopes, branches with ahead/behind, dirty state, recent author emails |
| `accounts.rs` | Switch preview and apply, `whoami` over ssh, the dry-run push check, the clone access probe and clone arguments, authenticating a git command as an account (`with_auth`, `url_for`), classifying Git/SSH errors into statuses |
| `graph.rs` | Reading the topo-ordered log and assigning lanes; changed files for a commit |
| `sshkeys.rs` | Generating a new ed25519 key per account with `ssh-keygen` (never overwriting), and trusting github.com by checking `ssh-keyscan` output against GitHub's published fingerprints before appending to `known_hosts` |
| `remote_url.rs` | Parsing remote URLs into host / owner / repo; resolving ssh host aliases with `ssh -G` |
| `git.rs` | Building `git` / `ssh` commands with a safe environment, running them with a timeout and cancel flag |
| `error.rs` | `AppError`, serialised to the UI as a plain message |

### Running external commands (`git.rs`)

Every `git` and `ssh` call goes through `git.rs`, which sets:

- `GIT_TERMINAL_PROMPT=0`, `GCM_INTERACTIVE=never`, and `BatchMode=yes` for ssh: never wait for
  a password prompt nobody can see.
- `GIT_OPTIONAL_LOCKS=0`: reading status must not take the index lock and get in the way of the
  user's own Git commands.
- `LC_ALL=C`: stable English output to parse.
- `CREATE_NO_WINDOW` on Windows, so no console windows flash up.

`run_limited(cmd, timeout, cancel)` polls the child and kills it on timeout or cancel; long jobs
(fetch, push checks) use it so **Cancel** takes effect immediately.

## The UI (`src/`)

SvelteKit with the static adapter and SSR turned off (`routes/+layout.ts`), so the build is plain
files in `build/` that Tauri serves from disk.

| File | Responsibility |
| --- | --- |
| `routes/+page.svelte` | Start-up gates and the toast |
| `lib/api.ts` | Types matching the Rust structs, one wrapper per command (`api.listRepos()` → `invoke("list_repos")`), status wording (`checkWords`, `scopeWords`) and helpers |
| `lib/state.svelte.ts` | `app`, one reactive object (Svelte 5 runes) with repos, accounts, roots, settings, filters, grouping, the single selection (`selectedId`, for the detail panel) and the multi-selection (`selectedIds`), running jobs. `listRows` derives the filtered list with nested repos under their parent and group headers; `visible` is the repo entries only. `suggestions` groups repos by their suggested account. `runFix()` carries out an issue's fix. `listen()` wires backend events into it |
| `components/Main.svelte` | Toolbar, filters, tabs, keyboard shortcuts |
| `components/RepoList.svelte` | Virtualised repo list with the account (▾) and ⋯ menus |
| `components/RepoDetail.svelte` | Overview / Commit graph / Technical tabs for the selected repo |
| `components/CommitGraph.svelte` | SVG lanes, paging, changed files |
| `components/SwitchDialog.svelte` | Preview and confirm an account switch |
| `components/BulkSwitchDialog.svelte` | One preview and confirm for switching many repos (multi-select or a suggestion strip) |
| `components/CloneDialog.svelte` | Clone a repo as a chosen account, with a per-account access probe |
| `components/Accounts.svelte` | Account cards with health (Test, Fix key, Edit), the access matrix, link suggestions |
| `components/AccountWizard.svelte` | Adding an account in three steps: who, sign-in (key picker from `~/.ssh`, generate, auto-test), where it applies (owners, then bulk switch) |
| `components/AccountForm.svelte` | Editing an existing account (Generate key, Test, Trust github.com) |
| `components/Settings.svelte` | Folders, excludes, refresh and fetch settings, data reset |
| `components/Setup.svelte` | Three-step first run |
| `components/Unlock.svelte` | Passphrase and lost-key screens |
| `components/JobBar.svelte` | Progress, cancel, summaries and "retry failed" for running jobs |

## How the UI and the core talk

### Commands (UI → core)

The UI calls `invoke("<command>", args)` through the wrappers in `api.ts`. Commands take
**repo and account IDs, not paths**: the core looks the path up in its own database. That way
even a compromised UI can't make the core run Git in an arbitrary folder. The paths the UI
sends are a new scan root and a clone destination, both from the native folder picker; a clone
destination is rejected unless it lies inside an enabled scan root. A clone URL is only ever
passed to `git` as a URL (after `--`), never used as a local path by RepoDeck. The UI can't
open arbitrary web pages either: `open_on_github` builds the URL from a repo's remote, and
`open_ssh_key_page` only opens `https://<host>/settings/ssh/new` for a validated host name.
`generate_ssh_key` takes a label and email, never a path, and picks a fresh file name itself.

Long-running work (scan, fetch all, access scan, clone) returns a **job ID** immediately and continues
on a background thread. `cancel_job(id)` sets that job's cancel flag.

### Events (core → UI)

| Event | Payload | Sent when | UI reaction |
| --- | --- | --- | --- |
| `repo-updated` | One `RepoRow` | A repo was re-inspected | Replace that row in `app.repos` |
| `repos-reload` | none | Repos were added or removed (scan, root change) | Reload the list (bursts coalesced) |
| `job` | `JobEvent` (id, kind, done/total, label, summary, finished) | Job progress | Update `app.jobs`, which drives `JobBar` |
| `job-item` | `JobItem` (job id, repo, result) | Each repo finishes in a fetch or access scan, or a clone succeeds | Collect failures for "Retry failed"; select a freshly cloned repo |

### Permissions

Tauri only lets the WebView call what the capability file allows:

- `src-tauri/build.rs` declares every app command in `COMMANDS`, which generates an
  `allow-<command>` permission for each.
- `src-tauri/capabilities/default.json` grants the main window exactly those permissions, plus
  `core:default` (events, window) and `dialog:allow-open` (the folder and key pickers). The opener
  plugin is used only from Rust and has no UI permission.
- `tauri.conf.json` sets a strict CSP: scripts and styles only from the app itself, no remote
  connections other than Tauri's IPC.

### Adding a command

1. Write the function in `commands.rs` with `#[tauri::command]`. Take IDs rather than paths. Wrap
   anything that runs Git or touches the database heavily in `blocking(...)`.
2. Register it in `generate_handler![...]` in `lib.rs`.
3. Add its name to `COMMANDS` in `build.rs`.
4. Add `"allow-<name-with-dashes>"` to `capabilities/default.json`.
5. Add a typed wrapper to `api` in `src/lib/api.ts`.

If you skip step 3 or 4 the call fails at runtime with a "not allowed" error.

## Data flow examples

### A scan

```
Setup / Settings ── start_scan ──▶ core.start_scan (job thread)
   scanner::walk(root)  ── parallel walk, excludes, classify
   store::save_scan()   ── insert new repos, drop vanished ones   ──▶ event repos-reload
   core.refresh(new)    ── inspection pool (2–4 threads)
      inspect::fingerprint → changed? → inspect::inspect → store::save_inspection
                                                         ──▶ event repo-updated (per repo)
   core.rewatch()       ── re-register file watches
   ──▶ event job (progress … finished, summary)
```

### Switching a repo's account

```
RepoList ▾ menu → app.switchReq → SwitchDialog
   preview_switch(repo, account) → accounts::preview() → shows current → new + git commands
   [Confirm] apply_switch(repo, account, expected)
      recompute preview; differs from `expected`? → refuse ("config changed, review again")
      accounts::apply() → git config --local … (repo's .git/config only)
      store: mark account active → core.refresh(repo, force) ──▶ event repo-updated
```

### Cloning as an account

```
CloneDialog → probe_clone_access(url)
   accounts::probe_access per account (git ls-remote, 4 at a time) → Can read / No access / …
   [Clone] start_clone(url, dest, account) → core.start_clone (job thread)
      validate: dest new or empty, parent inside an enabled scan root
      git clone -c <accounts::clone_args> -- <accounts::url_for(url)> <dest>   (GIT_SSH_COMMAND = account key)
         clone_args = the same keys `desired()` gives an account switch
      store::add_repo → store::link → store::set_active → core.refresh(repo, force)
      ──▶ events repos-reload, job-item (selects the repo), job (finished)
      failure/cancel → remove dest if the clone created it
```

### Problems and their fixes

```
store::repo_rows()
   per repo: suggested_account_id = owner rule → most recent author's account → only linked account
   per rule: Issue { level, text, fix: Fix { kind, account_id, label } }
   account health (store::list_accounts → health(): sshkeys::key_problem + account_checks)
      active account unhealthy → error issue with "Fix key" / "Fix <account>"
UI: RepoList / RepoDetail show fix.label → app.runFix(repo, fix)
   switch / link_switch → SwitchDialog (preview → apply_switch; set_active links if needed)
   repair_key → repair_account_key: point at the private key, rewrite core.sshCommand only
   trust_host → trust_github_host → push_check again
   edit_account / add_account → Accounts screen with that editor open
```

### Switching many repos

```
multi-select (⌘/Shift-click, ⌘A) or a suggestion strip → app.bulkReq → BulkSwitchDialog
   preview_bulk_switch(repo_ids, account) → accounts::preview per repo ("6 will change, 1 matches")
   [Apply] apply_bulk_switch(account, previews)
      per repo: switch_checked() — the same guard as apply_switch: re-preview, skip if changed
      → { applied, skipped[] } → refresh touched repos → repos-reload
```

### Opening the commit graph

```
CommitGraph → get_graph(repo, page)
   fingerprint(repo) == cached fingerprint? → return rows from the commits table
   otherwise: graph::read_log(`git log --branches --tags HEAD --topo-order`, up to (page+1)×500)
              graph::layout() → lanes + line segments → store::graph_store() → return the page
```

## Background work

`core.rs` owns all threads besides Tauri's own:

- **Inspection pool.** `refresh()` checks fingerprints with 2–4 threads (`threads()`: two on
  2-core machines or in battery saver, else up to four). `in_flight` stops two refreshes from
  inspecting the same repo at once.
- **Jobs.** Scan, fetch and access scan each run on their own thread with their own small pool.
  Only one scan runs at a time.
- **Watcher.** `rewatch()` registers `notify` watches on each repo's `.git` (non-recursive) and
  `refs/`. Events for locks, `objects/` and `logs/` are ignored. A matching event marks the
  repo dirty with a timestamp. Above `watch_threshold` (300) repos, or when most watches fail,
  it switches to polling.
- **Scheduler** (`start_background()`), ticking every 500 ms:
  - every tick: refresh repos that have been dirty for at least 2 s (debounce)
  - every 15 s: check the global config fingerprint, poll if in polling mode
    (`poll_interval_secs`, ×4 in battery saver), and start the idle rescan once
    `rescan_hours` have passed while the window is unfocused
- **Focus.** The window's `Focused` event calls `on_focus()`, which runs a refresh at most every
  5 seconds.

### Fingerprints

`inspect::fingerprint()` hashes (FNV-1a) the size and modification time of `HEAD`, `index`,
`config`, `packed-refs` and every file under `refs/`. It never reads object data, so checking
hundreds of repos takes milliseconds. `global_fingerprint()` does the same for `~/.gitconfig`,
`~/.config/git/config` and the files their `[include]` / `[includeIf]` sections point to; a
change there marks every repo stale. The system config isn't tracked (it rarely changes); the
**Re-read now** (repo panel) and **Refresh** in a repo's ⋯ menu force a re-read of that repo.

## Database

SQLCipher (SQLite with AES-256 encryption) through `rusqlite` with the
`bundled-sqlcipher-vendored-openssl` feature, so there is no system dependency. The connection
lives in `Db { conn: Mutex<Connection> }`. The schema is in `db.rs` and created with
`CREATE TABLE IF NOT EXISTS` on open.

| Table | Holds |
| --- | --- |
| `scan_roots` | Folders to scan, with max depth, stop-at-repo and enabled flags |
| `exclude_rules` | Exclude globs (global when `root_id` is null) |
| `repos` | One row per repo: path, kind, parent, fingerprints, HEAD branch, dirty, branches and recent emails (JSON), last fetch result, inspect error |
| `identity_snapshots` | The config entries read for each repo, with scope and origin file |
| `remotes` | Remote name, URL and parsed host / owner / repo |
| `accounts` | Account profiles (no secrets: the SSH key is a path) |
| `repo_accounts` | Which accounts are linked to which repo, and which is active |
| `push_checks` | Latest check result per (repo, account): the access matrix |
| `account_checks` | Latest sign-in test per account (status, GitHub username, time): part of account health |
| `graph_meta`, `commits` | Graph cache: laid-out rows per repo, keyed by fingerprint |
| `settings` | Key/value settings (defaults in `core::SETTING_DEFAULTS`) |

**Keys.** On first launch `db.rs` generates 32 random bytes and stores them hex-encoded in the
OS keychain (service `com.repodeck.app`, account `database-key`), via the `keyring` crate:
macOS Keychain, Windows Credential Manager, Linux Secret Service. Without a keychain the key is
derived from a passphrase with Argon2id, using a random salt saved in `passphrase.salt` next to
the database.

### Settings

| Key | Default | Meaning |
| --- | --- | --- |
| `editor` | `code` | Command for "Open in editor" |
| `fetch_concurrency` | `auto` | Parallel fetches (auto = 4, or 2 on low-end / battery saver) |
| `fetch_prune` | `false` | Add `--prune` to Fetch all |
| `watch_threshold` | `300` | Above this many repos, poll instead of watching |
| `poll_interval_secs` | `60` | Polling interval |
| `rescan_hours` | `24` | Idle rescan interval (0 = off) |
| `battery_saver` | `false` | Fewer threads, slower polling, no idle rescan |
| `max_commits` | `10000` | Commit graph limit |
| `stale_branch_days` | `90` | Branches older than this are shown as stale |
| `onboarded` | `false` | First-run setup finished |

## Tests

`cd src-tauri && cargo test` runs:

| Test | Covers |
| --- | --- |
| `remote_url` | URL parsing for https, `ssh://`, scp-style and aliases |
| `sshkeys` | Slugs for key names; generating keys never overwrites an existing one and keeps the private key `0600`; public keys are detected by name and content, with the private key found next to them; listing `~/.ssh` returns private keys only, with comment and type; host-key fingerprints must match GitHub's published ones |
| `store` | Against a real temp database and repo: `.pub` paths are corrected or refused on save; owner and author rules produce `link_switch` / `switch` fixes and the suggested account; a missing key makes the active account's repos an error with `edit_account`; stored sign-in tests become health |
| `commands` | A bulk switch skips a repo whose config changed after the preview and leaves it untouched |
| `inspect` | Finding `[include]` / `[includeIf]` paths in a global config |
| `scanner` | Walking, excludes, classification of nested / submodule / worktree |
| `graph` | Lane assignment for merges and branches |
| `accounts` (classification) | Mapping Git/SSH error text to statuses |
| `accounts` (clone) | Access probe returns Can read / No access against local repos; a clone made with `clone_args` has the account's name and email in its local config; SSH accounts get `core.sshCommand` |
| `accounts` (flow) | End to end against temp repos: switch → inspect shows the new identity with its source → dry-run push check against a local bare remote creates no ref → switch back restores the config |

`npm run check` type-checks the Svelte and TypeScript code.

## Dependencies worth knowing

| Crate / package | Why |
| --- | --- |
| `tauri` 2, `tauri-plugin-dialog`, `tauri-plugin-opener` | App shell, native folder/file pickers, opening URLs and revealing folders |
| `rusqlite` (bundled SQLCipher + vendored OpenSSL) | Encrypted cache with no system library dependency |
| `keyring` 3 | OS keychain access |
| `argon2` | Passphrase key derivation |
| `jwalk` | Parallel directory walk that can stop descending per folder |
| `globset` | Exclude patterns |
| `notify` 8 | File watching (FSEvents / ReadDirectoryChangesW / inotify) |
| `svelte` 5, `@sveltejs/kit` + `adapter-static` | UI |
| `@tauri-apps/api`, `@tauri-apps/plugin-dialog` | `invoke`, events, pickers from the UI |
