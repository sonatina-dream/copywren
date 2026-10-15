#!/usr/bin/env bash
# Regenerates the raster icons from assets/*.svg:
#   assets/AppIcon.icns     app bundle icon
#   assets/menubar.png      menu bar template image (18 pt @2x)
#   assets/icon-64.png      icon shown in the popup header
set -euo pipefail

cd "$(dirname "$0")/.."
cargo build -q -p icon-render
RENDER=target/debug/icon-render

ICONSET=$(mktemp -d)/AppIcon.iconset
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    "$RENDER" assets/icon.svg "$ICONSET/icon_${size}x${size}.png" "$size"
    "$RENDER" assets/icon.svg "$ICONSET/icon_${size}x${size}@2x.png" "$((size * 2))"
done
iconutil -c icns "$ICONSET" -o assets/AppIcon.icns

"$RENDER" assets/menubar.svg assets/menubar.png 36
"$RENDER" assets/icon.svg assets/icon-64.png 64
echo "Icons written to assets/"
