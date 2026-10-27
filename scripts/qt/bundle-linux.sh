#!/usr/bin/env bash
# Collects the Qt runtime a Debian-built avio_qt needs, so the Linux package runs on a host whose
# own Qt is a different version (qtbridge picks its signal lookup at build time, and Qt 6.12
# changed it: a binary built against an older Qt aborts there).
# Usage: bundle-linux.sh <avio_qt> <out dir>
# Writes <out>/{avio_qt,lib,plugins,qml,qt.conf}; avio_qt is the input binary with its rpath set.
set -euo pipefail

bin="$1"
out="$2"
triplet="$(dpkg-architecture -qDEB_HOST_MULTIARCH)"
libdir="/usr/lib/$triplet"
qtdir="$libdir/qt6"

mkdir -p "$out/lib" "$out/plugins" "$out/qml"
cp "$bin" "$out/avio_qt"

# Only Qt and the libraries that ship with it. Everything else (glibc, GL, X11/Wayland client
# libs, GStreamer, PulseAudio) belongs to the host.
bundled='libQt6|libicu|libdouble-conversion|libpcre2-16|libmd4c|libb2'

# Qt's libraries, then whatever the plugins link on top of the app
plugins=(platforms imageformats iconengines platforminputcontexts generic
         wayland-decoration-client wayland-graphics-integration-client wayland-shell-integration
         xcbglintegrations egldeviceintegrations)
for p in "${plugins[@]}"; do
    [[ -d "$qtdir/plugins/$p" ]] && cp -a "$qtdir/plugins/$p" "$out/plugins/"
done
cp -a "$qtdir/qml/." "$out/qml/"

collect() {
    ldd "$1" | awk '/=>/ { print $3 }' | grep -E "/($bundled)" || true
}
while read -r lib; do
    cp -L -n "$lib" "$out/lib/"
done < <(
    { collect "$bin"
      find "$out/plugins" "$out/qml" -name '*.so' | while read -r so; do collect "$so"; done
    } | sort -u
)
# Libraries the copied libraries pull in
for _ in 1 2 3; do
    while read -r lib; do
        cp -L -n "$lib" "$out/lib/"
    done < <(find "$out/lib" -name '*.so*' | while read -r so; do collect "$so"; done | sort -u)
done

# DT_RPATH, not RUNPATH: it also applies to the bundled libraries' own lookups
patchelf --force-rpath --set-rpath '$ORIGIN/lib' "$out/avio_qt"

cat > "$out/qt.conf" <<'CONF'
[Paths]
Prefix = .
Libraries = lib
Plugins = plugins
QmlImports = qml
CONF
