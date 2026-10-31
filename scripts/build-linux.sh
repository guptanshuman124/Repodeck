#!/usr/bin/env bash
# Build RepoDeck's Linux bundles inside Docker. Usage: scripts/build-linux.sh [amd64|arm64]
# Output: dist/linux-<arch>/ (.deb, .rpm, .AppImage)
set -euo pipefail
arch="${1:-amd64}"
root="$(cd "$(dirname "$0")/.." && pwd)"
image="repodeck-linux-builder:$arch"
out="$root/dist/linux-$arch"
mkdir -p "$out"

docker build --platform "linux/$arch" -t "$image" "$root/packaging/linux"

# The source is copied into the container (minus macOS node_modules and build output);
# cargo's registry and target dir live in named volumes so rebuilds are incremental.
docker run --rm --platform "linux/$arch" \
  -v "$root":/src:ro \
  -v "$out":/out \
  -v "repodeck-cargo-$arch":/usr/local/cargo/registry \
  -v "repodeck-target-$arch":/work/src-tauri/target \
  "$image" bash -euo pipefail -c '
    rsync -a --delete --exclude node_modules --exclude /build --exclude /dist --exclude .svelte-kit \
      --exclude src-tauri/target --exclude .git /src/ /work/
    npm ci --no-audit --no-fund
    npm run tauri build -- --bundles deb,rpm,appimage
    b=src-tauri/target/release/bundle
    cp -f $b/deb/*.deb $b/rpm/*.rpm $b/appimage/*.AppImage /out/
    ls -lh /out'
