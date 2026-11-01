# RepoDeck 0.1.0

First public release of RepoDeck, a local-first desktop app for people who use more than one Git
or GitHub account. It shows which name, email and SSH key every repo on your machine commits and
pushes with, and fixes the ones that are wrong.

## Highlights

- **See every repo's identity at a glance.** RepoDeck scans the folders you choose and shows,
  for each repo, the name, email and key it commits and pushes with, and where each value comes
  from: local config, global config, `includeIf`, or the SSH command.
- **One-click fixes.** Every problem has a fix button next to it ("Use Work", "Fix key", "Trust
  GitHub", "Add account"). Changes go only into the repo's own `.git/config`, and RepoDeck shows
  you the exact change before it applies it.
- **Switch many repos at once.** Select repos with ⌘/Ctrl-click, Shift-click or ⌘A, or accept
  a suggestion strip ("12 repos under acme-corp could use Work"). You see a preview before
  anything is written.
- **Account wizard.** Set up an account in three steps: who you commit as, how you sign in
  (pick a key in `~/.ssh` or create a new ed25519 key), and which GitHub owners it's for. The
  wizard confirms which GitHub user the key signs in as.
- **Account health.** Account cards warn when a key is missing, is a `.pub` file, or signs in as
  the wrong GitHub user, and offer a fix.
- **Clone as an account.** **Clone…** checks which of your accounts can reach a repo and clones
  it with that account's key and identity already set.
- **Push checks.** RepoDeck checks which accounts can actually push to each remote.
- **Repo tools.** A commit graph for each repo, "Fetch all", one-key "Open in VS Code" (`O`) and
  "Open on GitHub" (`G`), search (`⌘K`), filters, and grouping by owner, account or folder.

## Privacy and safety

- Runs entirely on your machine: no telemetry, no server, no sign-up.
- Never edits `~/.gitconfig` or `~/.ssh/config`.
- Never overwrites an existing SSH key.
- Trusts GitHub's host keys only after checking them against GitHub's published fingerprints.
- The local cache is encrypted (SQLCipher), with its key in your OS keychain.

## Downloads

| Platform | File |
| --- | --- |
| macOS 11+ (Apple Silicon and Intel) | `RepoDeck_0.1.0_universal.dmg` |
| Windows 10/11 | `RepoDeck_0.1.0_x64-setup.exe` or `RepoDeck_0.1.0_x64_en-US.msi` |
| Debian / Ubuntu | `RepoDeck_0.1.0_amd64.deb` / `RepoDeck_0.1.0_arm64.deb` |
| Fedora / RHEL / openSUSE | `RepoDeck-0.1.0-1.x86_64.rpm` / `RepoDeck-0.1.0-1.aarch64.rpm` |
| Any Linux | `RepoDeck_0.1.0_amd64.AppImage` / `RepoDeck_0.1.0_aarch64.AppImage` |

**Requires** Git 2.26 or newer and OpenSSH on your `PATH`.

## Known limitations

- **Builds are not code-signed yet.**
  - macOS: right-click RepoDeck in Applications → **Open** → **Open**, or run
    `xattr -dr com.apple.quarantine /Applications/RepoDeck.app`.
  - Windows: SmartScreen may warn about an unknown publisher; choose **More info → Run anyway**.
- **No auto-update.** Download new versions from this page.
- **Hosts.** Account access checks and key setup are built for github.com.
