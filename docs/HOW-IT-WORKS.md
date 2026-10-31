# How RepoDeck works

This document explains what RepoDeck does on your machine and why. For how the code is organised,
see [ARCHITECTURE.md](ARCHITECTURE.md).

## Finding repos

RepoDeck only scans the folders you add as **scan roots** (Settings → Folders). It never looks
anywhere else.

The scanner walks each root in parallel and treats a folder as a repo when it contains `.git`
(a folder, or a file for worktrees and submodules) or looks like a bare repo (`HEAD`, `objects/`
and `refs/` at the top level).

- **Skipped folders.** By default it skips `node_modules`, `.venv`, `venv`, `target`, `vendor`,
  `build`, `dist`, `.cache`, `~/Library`, `AppData`, `$RECYCLE.BIN` and `.Trash`. You can edit the
  list in Settings. Patterns without a `/` match a folder name anywhere; patterns with a `/` or a
  leading `~` match a path.
- **Depth.** It goes at most 8 levels deep per root (configurable per root).
- **Symlinks** are never followed, so links can't cause loops or escape a root.
- **Repos inside repos.** By default the scanner keeps going inside a repo, because people often
  keep several repos in one folder. Each root has a "Stop at first repo" option. What it finds
  inside is labelled:

  | Label | Meaning |
  | --- | --- |
  | Submodule | Registered in the parent's `.gitmodules` / tracked as a gitlink |
  | Worktree | A linked worktree (`.git` is a file pointing into another repo's `worktrees/`) |
  | Nested | An independent repo inside another repo's folder |
  | Bare | A repo without a working tree |

  When a parent repo tracks a nested repo as an *embedded repo* (a gitlink with no
  `.gitmodules` entry, usually from an accidental `git add`), RepoDeck warns you. Clones of that
  parent would get an empty folder.

If a repo's `.git` folder disappears, the repo shows an error until the next scan of its root,
which drops repos that are no longer there. A scan only ever removes RepoDeck's record, never
files.

## Showing who a repo commits as

For each repo RepoDeck runs `git config --list --show-origin --show-scope` in that repo. This is
the same resolution Git itself uses, so the answer is always what a real commit would use. For
`user.name`, `user.email`, `core.sshCommand` and the credential username it shows the effective
value and **where it came from**:

| Shown as | Meaning |
| --- | --- |
| this repo's config | `.git/config` |
| an includeIf rule | A file pulled in by `[includeIf "gitdir:…"]` in your global config |
| your global ~/.gitconfig | `~/.gitconfig` or `~/.config/git/config` |
| system config | Git's system-wide config |

From the remote URL it works out the host, owner and repo name (https, `ssh://` and
`git@host:owner/repo` forms). If the host is an alias from `~/.ssh/config` (such as
`github-work`), it is resolved with `ssh -G` to the real host name.

### Problems it flags, and their fixes

All of these are computed from local data, without touching the network. Each problem comes with
a one-click fix, shown next to it in the list and in the repo panel:

| Problem | Fix button |
| --- | --- |
| The active account can't sign in (its key file is missing, it points at a `.pub` public key, or its last sign-in test failed) | **Fix key** (switches a `.pub` path to the private key next to it) or **Fix &lt;account&gt;** (opens the account) |
| No `user.email` is set, so commits would fail or use a generated address | **Use &lt;account&gt;** for the suggested account, or **Add an account** |
| The repo's active account doesn't match the effective email (something else overrides it) | **Re-apply &lt;account&gt;** |
| Recent commits (the last 30) were authored with another account's email | **Use &lt;that account&gt;** |
| The remote belongs to an owner listed under another account ("Expected for owners") | **Use &lt;that account&gt;** |
| The last push check for the active account failed | **Trust github.com** for an untrusted host, otherwise **Check &lt;account&gt;** |
| The last fetch failed to sign in | **Check &lt;account&gt;** |
| An embedded repo (see above) | (fix it in Git: add a `.gitmodules` entry or ignore the folder) |

Fixes that change a repo's config always go through the same preview and confirm as a manual
switch.

**Suggested account.** For every repo RepoDeck works out which account it most likely belongs
to. The first of these that applies wins:
1. an account whose "Expected for owners" lists the remote's owner;
2. the account whose email made most of the recent commits;
3. the only linked account.

The suggestion powers the **Use …** fixes and the suggestion strips described below.

## Accounts and switching

An **account** is a saved profile: a label, commit name, commit email, host, and how it signs in
(an SSH private key path, or HTTPS through your credential helper). Only the key's *path* is
stored, never the key itself.

A repo can have several **linked** accounts with one **active**. Making an account active writes
these keys to that repo's `.git/config`, and nothing else:

| Key | Value |
| --- | --- |
| `user.name` | The account's name |
| `user.email` | The account's email |
| `core.sshCommand` | `ssh -i <key> -o IdentitiesOnly=yes` (SSH accounts) |
| `credential.https://<host>.username` | The GitHub username (HTTPS accounts) |

`IdentitiesOnly=yes` makes ssh offer only that key, so the right GitHub account is used even when
your agent holds several keys.

Before anything is written you see a **preview**: each key's current value, its new value, and the
exact `git config --local …` commands. When you confirm, RepoDeck recomputes the preview. If the
repo's config changed in between (for example you edited it in a terminal), it refuses and shows
you the new preview instead of overwriting your change. Choosing **Use global identity** removes
those keys again so the repo falls back to your global config.

### Switching many repos at once

- **Select several repos:** ⌘-click (Ctrl-click) adds or removes a repo, Shift-click adds a
  range, ⌘A (Ctrl+A) selects everything shown, and Esc clears the selection. The toolbar becomes
  a selection bar: **Use account…** and **Fetch**.
- **Suggestions:** when repos look like they belong to an account they don't use yet (see
  "Suggested account" above), a strip says so, for example "7 repos look like GlobalRhub repos
  but don't use it yet", with **Review and use GlobalRhub**. **Not now** hides that strip on
  this device.

Either way you get one dialog: "6 will change, 1 already matches", the per-repo changes if you
want them, and **Apply to 6 repos**. Each repo is re-checked before it's written. A repo whose
config changed after the preview is skipped and named in the result, never overwritten.

### Account health

Each account on the Accounts screen shows whether it can sign in:

| Badge | Meaning |
| --- | --- |
| OK | The last test signed in, as @user |
| Not tested | No test yet. Click **Test** |
| Key missing | The key file isn't there (deleted or moved) |
| Public key | The account points at the `.pub` file. **Fix key** switches to the private key next to it and updates `core.sshCommand` in the repos that use the account |
| Sign-in failed, Wrong account, … | The last test's result |

Tests run from the account card, the account form and the access scan, and the result is
remembered. When any account can't sign in, the **Accounts** tab shows a red dot, the summary
line says how many, and every repo using that account is flagged.

Saving an account with a `.pub` path stores the private key next to it instead. If there's no
private key there, the save is refused.

### Setting up an SSH key per account

GitHub lets a key belong to only one account, so each account needs its own key. **Accounts →
Add account** walks through three steps:

1. **Who you commit as:** label, name and email. Use the email GitHub knows for that account,
   so commits link to it.
2. **How you sign in.** For SSH, pick a key:
   - **Keys already in `~/.ssh`** are listed as cards. Each shows its file name, comment
     (usually the email it was made for), type, which accounts already use it, and whether it's
     loaded in ssh-agent. RepoDeck reads only names, fingerprints and comments, never key
     contents. Picking a card tests it right away and shows **Works · Signs in as @user**.
   - **Create a new key** shows the exact file it will create (`~/.ssh/id_ed25519_<label>`, or
     `_2`, `_3` if that's taken) and never replaces an existing key. Then **Copy key and open
     GitHub** copies the public key and opens GitHub's *New SSH key* page. Make sure the browser
     is signed in to the right GitHub account, paste, and save. RepoDeck re-tests every few
     seconds for up to two minutes and shows **Works** once GitHub accepts the key.
   - **Use a key file somewhere else…** takes a path. A `.pub` file is rejected on the spot.

   A successful test fills in the GitHub username, so later checks can tell a key that signs
   in as the wrong user. If the test says **Host not trusted**, click **Trust github.com**:
   RepoDeck checks GitHub's host keys against the fingerprints GitHub publishes (built into the
   app) and only then adds them to `~/.ssh/known_hosts`.
3. **Where it applies:** the GitHub owners (users or organisations) whose repos should use this
   account. They're prefilled from repos that already commit with this email. If any repos
   match, **Switch the N matching repos after saving** opens the bulk preview.

**Edit** on an existing account opens a single form with the same **Generate new key**, **Test**
and **Trust github.com** buttons.

Generated keys have **no passphrase**. The private key file is readable only by you (mode
`0600`) and protected by your disk encryption, which is how most GitHub keys are used. It also
means RepoDeck's checks and your pushes never need a prompt. If you want a passphrase, create the
key yourself (below) and load it into your ssh-agent.

**Doing it by hand:**

```sh
ssh-keygen -t ed25519 -C "you@example.com" -f ~/.ssh/id_ed25519_personal
ssh-add --apple-use-keychain ~/.ssh/id_ed25519_personal   # macOS, if you set a passphrase
pbcopy < ~/.ssh/id_ed25519_personal.pub                  # copy the PUBLIC key
```

Add the public key on GitHub under **Settings → SSH and GPG keys → New SSH key**, then test it:

```sh
ssh -i ~/.ssh/id_ed25519_personal -o IdentitiesOnly=yes -T git@github.com
# Hi <username>! You've successfully authenticated…
```

The first connection asks you to trust GitHub's host key; answer `yes`. In RepoDeck, enter the
**private** key path (`~/.ssh/id_ed25519_personal`, not the `.pub`). macOS hides `~/.ssh` in file
pickers; type the path or press `⌘⇧.` to show hidden folders. Keys with a passphrase must be
loaded in your ssh-agent (the `ssh-add` line above), because RepoDeck can't type a passphrase for
you.

## Cloning as an account

A plain `git clone` uses your global setup (your default SSH key or credential helper). If a repo is
only reachable by another of your accounts (say a work org repo, while your default key belongs to
your personal account), it fails with "Repository not found". **Clone…** in the toolbar solves this:

1. **Paste the URL.** RepoDeck runs `git ls-remote` as each of your accounts (read-only, four at a
   time) and marks which ones **Can read** the repo. If exactly one can, it's preselected.
2. **Pick the account and folder.** The folder must be inside one of your scan folders, so the repo
   shows up in the list. The folder name defaults to the repo's name.
3. **Clone.** RepoDeck runs `git clone -c user.name=… -c user.email=… -c core.sshCommand=… <url>`
   using that account's key, with the URL rewritten to the account's own auth method (SSH or
   HTTPS). `clone -c` writes those keys into the new repo's `.git/config` *before* fetching, the
   same keys an account switch writes. The repo is then registered, linked to the account and set
   as active, so the first commit and push already use the right identity.

Progress and **Cancel** appear in the bar at the bottom. A failed or cancelled clone removes the
folder it created (never a folder that was already there).

Without RepoDeck, the equivalent is:

```sh
GIT_SSH_COMMAND='ssh -i ~/.ssh/id_ed25519_work -o IdentitiesOnly=yes' \
  git clone git@github.com:org/repo.git ~/code/repo
```

then **Rescan** and choose the account from the repo's ▾ menu.

## Checking who can push

Knowing an email isn't enough; RepoDeck can check what an account can actually do. This needs the
network and runs only when you ask.

1. **Who is this key?** For SSH accounts, `ssh -T git@github.com` with the account's key. GitHub
   answers `Hi <username>!`. If you gave the account a GitHub username and the key belongs to
   someone else, the result is **Wrong account**.
2. **Can it push?** `git push --dry-run` of the current `HEAD` to a branch called
   `repodeck-push-check` on the repo's remote, using the account's own key or credentials (the URL
   is rewritten to SSH or HTTPS to match the account). GitHub checks write permission when the
   push starts, and a dry run stops before sending anything, so **nothing is ever created on the
   remote**. A test confirms this against a local bare remote.

Results:

| Result | Meaning |
| --- | --- |
| Can push | The account has write access |
| Can read | (Clone only) The account can read the repo |
| Read only | It can see the repo but not push |
| No access | It can't see the repo at all |
| Wrong account | The key signs in as a different GitHub user than expected |
| Sign-in failed | The key or credentials were rejected |
| Host not trusted | The host isn't in `~/.ssh/known_hosts`; connect once in a terminal |
| Offline | No network |
| Unknown | An answer RepoDeck couldn't interpret (the details show the raw message) |

The **access scan** (Accounts → Who can push where) runs this for every account × every GitHub
repo, four at a time. It can be cancelled, and **Scan changed** skips pairs checked in the last
7 days. Where an account can push to a repo it isn't linked to, RepoDeck suggests linking it
("Link all" accepts every suggestion).

## Fetch all

**Fetch all** runs `git fetch --all` in every repo with a remote, four at a time (two on machines
with two cores or in battery-saver mode), with a 60-second timeout per repo. Optional `--prune`
is in Settings. It's cancellable, ends with a summary (fetched / sign-in failed / offline / timed
out / failed) and offers **Retry failed**. Git runs with `GIT_TERMINAL_PROMPT=0` and
`GCM_INTERACTIVE=never`, so a repo needing a password fails instead of hanging.

## Keeping the list up to date

Reading every repo on every launch would be slow, so RepoDeck:

1. **Shows the saved list instantly** from its cache when the app opens.
2. **Fingerprints each repo** from a few small files: `HEAD`, `index`, `config`, `packed-refs`
   and the `refs/` tree. Only repos whose fingerprint changed are re-read with Git. A change to
   your global Git config (which affects every repo) is detected the same way.
3. **Watches for changes.** Up to 300 repos, it watches each repo's `.git` folder and `refs/`
   using the OS's file events. Bursts are debounced for 2 seconds. Above 300 repos, or if the OS
   runs out of watch handles (common on Linux), it polls every 60 seconds instead.
4. **Refreshes on focus.** Switching back to RepoDeck re-checks fingerprints (at most every 5 s).
5. **Rescans roots** for new repos every 24 hours, only while the app is idle in the background.

**Battery saver** (Settings) uses two threads, polls four times less often and skips the
automatic rescan.

## Finding your way around

- **Toolbar:** search (⌘K / Ctrl+K), **Filter ▾** (folder, account, only repos that need
  attention), **Group** (none, by GitHub owner, by account, by folder), **Clone…**, **Fetch
  all**, and **⋯** with *Look for new repos* (rescan your folders) and *Re-read every repo*
  (ignore the cache).
- **Summary line:** the number of repos, how many need attention, and how many accounts can't
  sign in. Each part is a link that filters the list or opens Accounts. Active filters show as
  chips with ✕.
- **Groups:** headers show how many repos each group has and how many need attention. Click a
  header to collapse it. The grouping choice is remembered.

## Commit graph

The Commit graph tab shows the history of all branches and tags in topological order, with each
line of development in its own coloured lane. It loads 500 commits at a time (up to 10,000 by
default) and caches the laid-out result until the repo's fingerprint changes. Click a commit to
see the files it changed. The Overview tab also shows who authored recent commits, which is handy
for spotting the wrong email.

## Opening things

- **Open in VS Code** (`O`) runs your configured editor command (`code` by default). If that isn't
  on your `PATH`, RepoDeck tries the standard install locations, then a `vscode://` link.
- **Open on GitHub** (`G`) opens the remote's web page in your browser.
- **Show in folder** reveals the repo in Finder / Explorer / your file manager.

## Security and privacy

- **Nothing leaves your machine** except the Git and SSH traffic you trigger (fetch, push
  checks). There is no telemetry or update check.
- **Encrypted cache.** RepoDeck's database is encrypted with SQLCipher. A random 256-bit key is
  created on first launch and stored in the OS keychain (macOS Keychain, Windows Credential
  Manager, or the Secret Service on Linux) under the service `com.repodeck.app`. On systems
  without a keychain, you choose a passphrase instead and the key is derived from it with
  Argon2id. If the keychain entry is lost, the cache can't be read; you can start fresh, which
  only deletes RepoDeck's own data.
- **What's stored:** repo paths, config values it displayed, account profiles (no keys or
  passwords), check results and the graph cache.
- **What it writes outside its data folder:**
  - the keys listed under [Accounts and switching](#accounts-and-switching), in a repo's
    `.git/config`, after you confirm
  - new repos you clone with **Clone…**
  - a new key pair in `~/.ssh` when you click **Generate new key** (never overwriting a file)
  - github.com's host keys in `~/.ssh/known_hosts` when you click **Trust github.com**, only
    after they match GitHub's published fingerprints

  It never edits `~/.gitconfig` or `~/.ssh/config`, and the private key never leaves the file
  `ssh-keygen` wrote: the UI only sees its path and the public key.
- **Locked-down UI.** The window has a strict Content Security Policy (no remote scripts, no
  remote connections), and the UI can only call RepoDeck's own allow-listed commands. Commands
  take repo and account IDs rather than file paths, so the UI can't point Git at arbitrary
  folders.

Data location:

| OS | Folder |
| --- | --- |
| macOS | `~/Library/Application Support/com.repodeck.app/` |
| Windows | `%APPDATA%\com.repodeck.app\` |
| Linux | `~/.local/share/com.repodeck.app/` |

To remove RepoDeck completely, delete that folder and the `com.repodeck.app` keychain entry.
