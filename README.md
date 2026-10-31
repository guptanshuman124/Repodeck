# RepoDeck

RepoDeck is a local-first desktop app for people who use more than one Git account. It finds every
Git repo in the folders you choose and shows, for each one, which name, email and SSH key it
commits and pushes with, and where each of those values comes from. It points out repos that will
commit as the wrong person, lets you switch a repo's account in one click, and checks which of your
accounts can actually push where.

It also gives you a commit graph per repo, "Fetch all", and one-key "Open in VS Code" / "Open on
GitHub".

- Runs entirely on your machine. No telemetry, no server, no account.
- Only ever writes to a repo's own `.git/config`, and only after showing you the exact change.
  It never edits `~/.gitconfig` or `~/.ssh/config`.
- Its cache is encrypted (SQLCipher) with a key kept in your OS keychain.

Built with Tauri 2 (Rust core, Svelte 5 UI). Runs on macOS, Windows and Linux.

## Requirements

- **Git 2.26 or newer** on your `PATH`. RepoDeck uses your own Git so identities resolve exactly
  as a real commit or push would.
- **OpenSSH** (`ssh`) on your `PATH` for SSH accounts. It ships with macOS, Linux and Windows 10+.

## Install

Download the installer for your platform from the
[releases page](../../releases) or from a CI run (see [Building](docs/BUILDING.md)):

| Platform | File |
| --- | --- |
| macOS 11+ (Apple Silicon and Intel) | `RepoDeck_<version>_universal.dmg` |
| Windows 10/11 | `RepoDeck_<version>_x64-setup.exe` or `RepoDeck_<version>_x64_en-US.msi` |
| Debian / Ubuntu | `RepoDeck_<version>_amd64.deb` / `_arm64.deb` (package name `repo-deck`) |
| Fedora / RHEL / openSUSE | `RepoDeck-<version>-1.x86_64.rpm` / `.aarch64.rpm` |
| Any Linux | `RepoDeck_<version>_amd64.AppImage` / `_aarch64.AppImage` |

The builds are not code-signed yet:

- **macOS:** the first time, right-click RepoDeck in Applications, choose **Open**, then **Open**
  again. Or run `xattr -dr com.apple.quarantine /Applications/RepoDeck.app`.
- **Windows:** SmartScreen may warn about an unknown publisher. Choose **More info → Run anyway**.

## Quick start (development)

```sh
npm install
npm run tauri dev           # run the app with hot reload
cd src-tauri && cargo test  # Rust tests
npm run check               # Svelte / TypeScript check
```

You need Node 20.19+ and Rust (stable). On Linux you also need the WebKitGTK packages listed in
[docs/BUILDING.md](docs/BUILDING.md).

## Using it

1. **Pick folders.** On first run choose the folders that hold your code (for example `~/code`).
   RepoDeck only looks inside folders you choose.
2. **Add accounts.** Accounts → Add account walks through three steps: who you commit as, how you
   sign in (pick a key already in `~/.ssh` or create one, then RepoDeck confirms which GitHub
   user it signs in as), and which GitHub owners it's for. See
   [Setting up SSH keys](docs/HOW-IT-WORKS.md#setting-up-an-ssh-key-per-account).
3. **Fix what's highlighted.** Every problem has a fix button next to it ("Use Work", "Fix key").
   When several repos belong to an account they don't use yet, a strip offers to switch them all
   with one preview. You can also select repos yourself (⌘-click, Shift-click) and switch them
   together.
4. **Clone as the right account.** **Clone…** finds which of your accounts can reach a repo and
   clones with that account's key. See
   [Cloning as an account](docs/HOW-IT-WORKS.md#cloning-as-an-account).

Keyboard: `⌘K` / `Ctrl+K` search, `↑` `↓` move, `Enter` open, `O` open in VS Code,
`G` open on GitHub, `⌘`-click / `Shift`-click / `⌘A` select several, `Esc` clear or close.

## Documentation

| Doc | What's in it |
| --- | --- |
| [docs/HOW-IT-WORKS.md](docs/HOW-IT-WORKS.md) | What RepoDeck does and why: scanning, identity sources, switching accounts, push checks, fetching, refresh, the graph, security |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | How the code is wired: processes, modules, data flow, IPC, events, database schema, threading, and how to add a command |
| [docs/BUILDING.md](docs/BUILDING.md) | Building installers for macOS, Windows and Linux, locally and in CI |

## Project layout

```
src/                      Svelte 5 UI (SvelteKit, static adapter, no SSR)
  routes/+page.svelte       Start-up gates: git missing → unlock → first run → main
  lib/api.ts                Typed wrappers for every Rust command + shared types
  lib/state.svelte.ts       App-wide reactive state and backend event listeners
  lib/components/           Screens and widgets
src-tauri/                Rust core
  src/                      Scanner, inspector, accounts, graph, database, jobs, commands
  capabilities/default.json The only permissions the UI has
  tauri.conf.json           Window, CSP and bundle settings
packaging/                Icon source and the Linux build container
scripts/build-linux.sh    Build Linux packages in Docker from any OS
.github/workflows/        CI that builds installers for all platforms
```

## Decisions on the Phase 0 open questions

- **UI framework:** Svelte.
- **HTTPS accounts:** supported from the start, through your existing credential helper
  (`credential.<host>.username`).
- **Nested repos:** scanning keeps going inside repos by default; each folder can be set to stop
  at the first repo.
- **`~/.gitconfig`:** never written. Everything RepoDeck changes stays repo-local.

## License

MIT
