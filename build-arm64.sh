#!/usr/bin/env bash
# Build the app for ARM64 (linux/arm64/v8) into ./out_*/ (avio_qt + gst-host)
set -euo pipefail
cd "$(dirname "$0")"
docker build --platform linux/arm64/v8 \
    --build-arg JOBS="$(( $(nproc) - 4 > 0 ? $(nproc) - 4 : 1 ))" \
    --target artifact --output type=local,dest=out_arm64 .
