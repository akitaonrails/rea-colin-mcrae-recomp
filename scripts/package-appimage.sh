#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 VERSION APPIMAGETOOL RUNTIME OUTPUT" >&2
  exit 2
fi

version=$1
appimagetool=$2
runtime=$3
output=$4

[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "invalid release version: $version" >&2
  exit 2
}
[[ -x target/release/cmr-app && -x target/release/cmr-assets ]] || {
  echo "release binaries are missing; build cmr-app and cmr-assets first" >&2
  exit 1
}
[[ -x $appimagetool && -f $runtime ]] || {
  echo "appimagetool or its pinned runtime is unavailable" >&2
  exit 1
}

mkdir -p .work
stage_root=$(mktemp -d .work/appimage-stage.XXXXXX)
trap 'rm -rf -- "$stage_root"' EXIT
appdir="$stage_root/CMR-Modern.AppDir"
mkdir -p \
  "$appdir/usr/bin" \
  "$appdir/usr/lib" \
  "$appdir/usr/share/cmr/profiles" \
  "$appdir/usr/share/cmr/docs" \
  "$appdir/usr/share/icons/hicolor/scalable/apps"

install -m 0755 target/release/cmr-app "$appdir/usr/bin/cmr-app"
install -m 0755 target/release/cmr-assets "$appdir/usr/bin/cmr-assets"
install -m 0755 packaging/appimage/AppRun "$appdir/AppRun"
install -m 0644 packaging/appimage/cmr-modern.desktop "$appdir/cmr-modern.desktop"
install -m 0644 packaging/appimage/cmr-modern.svg "$appdir/cmr-modern.svg"
install -m 0644 packaging/appimage/cmr-modern.svg \
  "$appdir/usr/share/icons/hicolor/scalable/apps/cmr-modern.svg"
install -m 0644 profiles/*.json "$appdir/usr/share/cmr/profiles/"
install -m 0644 README.md CONTRIBUTING.md CHANGELOG.md packaging/RELEASE.md \
  "$appdir/usr/share/cmr/docs/"
python scripts/collect-third-party-licenses.py \
  "$appdir/usr/share/cmr/docs/third-party-licenses"

sdl_library=$(find target/release/build -type f -path '*/out/lib/libSDL3.so.0*' -print -quit)
[[ -n $sdl_library ]] || {
  echo "bundled libSDL3.so.0 was not produced" >&2
  exit 1
}
install -m 0755 "$sdl_library" "$appdir/usr/lib/libSDL3.so.0"

mkdir -p "$(dirname "$output")"
VERSION=$version ARCH=x86_64 "$appimagetool" --appimage-extract-and-run \
  --runtime-file "$runtime" --no-appstream "$appdir" "$output"
chmod 0755 "$output"
