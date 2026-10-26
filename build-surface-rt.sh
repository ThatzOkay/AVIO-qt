#!/usr/bin/env bash
# Build the app for Surface RT (linux/arm/v7) into ./out_*/ (avio_qt + gst-host)
set -euo pipefail
cd "$(dirname "$0")"
docker build --platform linux/arm/v7 \
    --build-arg JOBS="$(( $(nproc) - 4 > 0 ? $(nproc) - 4 : 1 ))" \
    --target artifact --output type=local,dest=out_armv7 .
