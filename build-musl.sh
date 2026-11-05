#!/usr/bin/env bash
# Build the app against musl (Alpine) into ./out_musl_<arch>/ (avio_qt + gst-host).
# Usage: ./build-musl.sh [armv7|arm64|x86_64]   (default: armv7)
set -euo pipefail
cd "$(dirname "$0")"

case "${1:-armv7}" in
    armv7)  platform=linux/arm/v7 ;;
    arm64)  platform=linux/arm64/v8 ;;
    x86_64) platform=linux/amd64 ;;
    *) echo "usage: $0 [armv7|arm64|x86_64]" >&2; exit 2 ;;
esac

docker build --platform "$platform" -f Dockerfile.musl \
    --build-arg JOBS="$(( $(nproc) - 4 > 0 ? $(nproc) - 4 : 1 ))" \
    --target artifact --output "type=local,dest=out_musl_${1:-armv7}" .
