# Build environment for RepoDeck's Linux bundles (.deb, .rpm, .AppImage).
# Used by scripts/build-linux.sh; works for linux/amd64 and linux/arm64.
FROM node:22-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential curl wget file perl pkg-config rsync xdg-utils \
      libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev \
      librsvg2-dev libdbus-1-dev \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable

# linuxdeploy (used for the AppImage) is itself an AppImage; FUSE is not available in containers.
ENV APPIMAGE_EXTRACT_AND_RUN=1
WORKDIR /work
