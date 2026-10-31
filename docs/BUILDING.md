# Building RepoDeck

RepoDeck is a Tauri 2 app. A build compiles the Svelte UI to static files (`build/`), compiles
the Rust core with those files embedded, and packages the result as native installers.

| Target | Built on | Output |
| --- | --- | --- |
| macOS (universal: Apple Silicon + Intel) | macOS | `.app`, `.dmg` |
| Windows x64 | Windows, or CI | `.msi`, NSIS `-setup.exe` |
| Linux x64 / arm64 | Linux, Docker on any OS, or CI | `.deb`, `.rpm`, `.AppImage` |

Tauri can't cross-compile installers between operating systems in general, so each OS is built
on that OS. The two exceptions here: **Linux packages can be built from macOS or Windows with
Docker** (`scripts/build-linux.sh`), and **CI builds all of them** (`.github/workflows/build.yml`).

## Prerequisites (all platforms)

- **Node.js 20.19+** (22 recommended) and npm
- **Rust stable** via [rustup](https://rustup.rs)
- **Git** (also needed at runtime)

Then, once:

```sh
npm install
```

The vendored SQLCipher/OpenSSL build also needs **Perl** and a C compiler. macOS and most Linux
systems already have Perl; on Windows see below.

## Development

```sh
npm run tauri dev            # app with hot reload (UI) and auto-rebuild (Rust)
npm run check                # svelte-check + TypeScript
cd src-tauri && cargo test   # Rust tests
cd src-tauri && cargo clippy # lints (expected to be clean)
cd src-tauri && cargo fmt    # formatting
```

The dev build uses the same encrypted database and keychain entry as the installed app.

## macOS

Install the Xcode command line tools (`xcode-select --install`) and both Rust targets:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri build -- --target universal-apple-darwin --bundles app,dmg
```

Output in `src-tauri/target/universal-apple-darwin/release/bundle/`:

- `macos/RepoDeck.app`
- `dmg/RepoDeck_<version>_universal.dmg`

For a faster single-architecture build, drop `--target …` (output goes to
`src-tauri/target/release/bundle/`).

**Signing and notarisation** are not set up. Unsigned apps open after right-click → Open. To sign,
set `APPLE_SIGNING_IDENTITY` (and `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` for notarising)
before `tauri build`; see the [Tauri macOS signing guide](https://v2.tauri.app/distribute/sign/macos/).

## Windows

On a Windows 10/11 machine:

1. Install **Visual Studio 2022 Build Tools** with the "Desktop development with C++" workload.
2. Install **Strawberry Perl** and **NASM** and make sure both are on `PATH` (needed to build the
   vendored OpenSSL that SQLCipher uses).
3. WebView2 is preinstalled on Windows 10 (1803+) and 11.
4. Build from a "Developer PowerShell for VS 2022" (so `nmake` and `cl` are on `PATH`):

```powershell
npm install
npm run tauri build -- --bundles msi,nsis
```

Output in `src-tauri\target\release\bundle\`:

- `msi\RepoDeck_<version>_x64_en-US.msi`
- `nsis\RepoDeck_<version>_x64-setup.exe`

**Why not from macOS?** Tauri's experimental Windows cross-build (`cargo-xwin`) can't build the
vendored OpenSSL, which needs the MSVC `nmake` toolchain. Use a Windows machine or CI.

**Signing:** not set up; see the [Tauri Windows signing guide](https://v2.tauri.app/distribute/sign/windows/).

## Linux

### With Docker (from any OS)

```sh
scripts/build-linux.sh amd64   # x86_64
scripts/build-linux.sh arm64   # aarch64
```

Output goes to `dist/linux-<arch>/`. The script builds a Debian Bookworm image from
`packaging/linux/Dockerfile` (Node 22, Rust, WebKitGTK 4.1 and friends), copies the source into
the container (leaving out your host's `node_modules` and build output), and runs the Tauri build.
Cargo's registry and target folder live in Docker volumes (`repodeck-cargo-<arch>`,
`repodeck-target-<arch>`), so the second build is much faster.

On Apple Silicon, `arm64` builds natively; `amd64` runs under emulation and is several times
slower. Remove the cache volumes with `docker volume rm repodeck-cargo-amd64 repodeck-target-amd64`
(and the same for `arm64`).

Building on Bookworm (glibc 2.36) means the packages run on Debian 12+, Ubuntu 22.10+, Fedora 37+
and similar. For older distributions build on an older base (CI uses Ubuntu 22.04, glibc 2.35).

### Natively

On Debian / Ubuntu:

```sh
sudo apt install build-essential curl wget file perl pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev
npm install
npm run tauri build -- --bundles deb,rpm,appimage
```

On Fedora: `sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file perl
libxdo-devel libappindicator-gtk3-devel librsvg2-devel dbus-devel` and `@development-tools`.

Output in `src-tauri/target/release/bundle/`: `deb/*.deb`, `rpm/*.rpm`, `appimage/*.AppImage`.

The `.deb` and `.rpm` declare a dependency on `git`. The AppImage expects `git` to be installed.

**Keychain on Linux:** the database key is stored through the Secret Service API (GNOME Keyring,
KWallet 5.97+, KeePassXC). Without one, RepoDeck asks for a passphrase instead.

## CI (all platforms)

`.github/workflows/build.yml` builds on GitHub-hosted runners:

| Job | Runner | Output |
| --- | --- | --- |
| `macos-universal` | `macos-latest` | `.dmg` |
| `windows-x64` | `windows-latest` | `.msi`, `-setup.exe` |
| `linux-x64` | `ubuntu-22.04` | `.deb`, `.rpm`, `.AppImage` |
| `linux-arm64` | `ubuntu-22.04-arm` | `.deb`, `.rpm`, `.AppImage` |

Each job runs `npm run check` and `cargo test` before building. It runs:

- **On demand:** GitHub → Actions → build → Run workflow. The installers are attached to the run
  as artifacts.
- **On a version tag:** `git tag v0.1.0 && git push --tags` also creates a **draft release** with
  every installer attached. Review it and publish.

## Releasing a new version

1. Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`
   (they must match).
2. Commit, tag `v<version>`, push the tag.
3. Publish the draft release CI creates.

## Changing the icon

The icon source is `packaging/icon/icon.svg`. Regenerate every size with:

```sh
npx tauri icon packaging/icon/icon.svg
rm -rf src-tauri/icons/android src-tauri/icons/ios   # mobile icons aren't used
cp src-tauri/icons/32x32.png static/favicon.png
```

## Troubleshooting

| Problem | Fix |
| --- | --- |
| `failed to run custom build command for openssl-sys` / `openssl-src` | Perl (and on Windows NASM + `nmake`) missing from `PATH` |
| Linux: `webkit2gtk-4.1 not found` | Install `libwebkit2gtk-4.1-dev` (Ubuntu 22.04+ / Debian 12+) |
| AppImage step fails in a container | Set `APPIMAGE_EXTRACT_AND_RUN=1` (the Docker image does) |
| App starts but says Git is missing | Git isn't on the `PATH` of GUI apps. On macOS RepoDeck reads your login shell's `PATH`; check `git --version` works in a new terminal |
| `not allowed` error when calling a new command | Add it to `src-tauri/build.rs` and `capabilities/default.json` (see [ARCHITECTURE.md](ARCHITECTURE.md#adding-a-command)) |
