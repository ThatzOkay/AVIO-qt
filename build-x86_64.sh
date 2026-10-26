#!/usr/bin/env bash
# Build the app for x86 (linux/amd64) into ./out_*/ (avio_qt + gst-host)
set -euo pipefail
cd "$(dirname "$0")"
docker build --platform linux/amd64 \
    --build-arg JOBS="$(( $(nproc) - 4 > 0 ? $(nproc) - 4 : 1 ))" \
    --target artifact --output type=local,dest=out_x86_64 .
