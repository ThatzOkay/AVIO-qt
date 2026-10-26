# Builds for Surface RT (Tegra 3, 32-bit ARMv7 hard-float) running Linux.
# Pass `--platform linux/arm/v7` (build-surface-rt.sh does) for the Surface; on an x86_64 host
# that runs under QEMU user emulation (needs binfmt: e.g.
# `docker run --privileged --rm tonistiigi/binfmt --install arm`). Without it the image
# is built natively, which is much faster for shaking out build errors.
FROM debian:forky-slim AS build

ARG JOBS=1
ARG TARGETARCH
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:$PATH \
    QMAKE=/usr/lib/qt6/bin/qmake6 \
    CARGO_BUILD_JOBS=${JOBS}

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl pkg-config \
        qt6-base-dev qt6-base-private-dev qt6-declarative-dev qt6-declarative-private-dev qt6-base-dev-tools \
        qml6-module-qtquick qml6-module-qtquick-controls \
        qml6-module-qtquick-layouts qml6-module-qtquick-templates \
        qml6-module-qtquick-window \
        qml6-module-qtquick-dialogs qml6-module-qt-labs-folderlistmodel \
    && rm -rf /var/lib/apt/lists/*

# Native libs the path crates link against (cpal, gst-video, rtl-sdr, welle-io-sys,
# libpulse-binding), protoc for aa-proto and perl for the vendored openssl build.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libasound2-dev libpulse-dev \
        libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
        librtlsdr-dev libfftw3-dev libbz2-dev \
        protobuf-compiler perl \
    && rm -rf /var/lib/apt/lists/*

# qt-build-utils (qtbridge's build script) runs libexec/qtpaths, but Debian either leaves that
# as a dangling symlink (trixie) or doesn't ship it there at all (forky armhf); only
# bin/qtpaths6 exists.
RUN mkdir -p /usr/lib/qt6/libexec \
    && ln -sf /usr/lib/qt6/bin/qtpaths6 /usr/lib/qt6/libexec/qtpaths

RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable

WORKDIR /app
RUN mkdir /out
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Dependencies (and the path crates, which sit under crates/) first, against a stub main.rs, so
# this expensive layer is cached until Cargo.toml, Cargo.lock or crates/ change. The target dir
# is deliberately not a cache mount: mounts aren't part of the layer, so CI (buildx's GHA cache
# exports layers only) could never reuse it. Matters most for armv7, which builds under QEMU.
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN --mount=type=cache,target=/opt/cargo/registry \
    cargo build --release --locked \
    && cargo build --release --locked -p gst-video --bin gst-host

COPY src ./src
# The stub's mtime can be newer than the copied sources', which would make cargo skip the rebuild
RUN --mount=type=cache,target=/opt/cargo/registry \
    touch src/main.rs \
    && cargo build --release --locked \
    && cp target/release/avio_qt target/release/gst-host /out/

# `docker build --target artifact --output out .` writes just the binaries: the app and its
# gst-host sidecar (the app wants it as gst-host-<arch>-unknown-linux-gnu next to itself).
FROM scratch AS artifact
COPY --from=build /out/ /
