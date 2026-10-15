#!/usr/bin/env bash
# Regenerates the README media: docs/demo.mp4, docs/demo.gif and
# docs/screenshots/*.png.
#
# Copywren is started in demo mode with a throw-away data directory full of
# invented clips. It drives its own popup with scripted keystrokes and
# records only its own windows; it never reads or writes your clipboard,
# registers no shortcuts and pastes nothing. tools/demo-render then places
# the frames in a drawn scene. A small Copywren window appears in the
# bottom-right corner while recording (about 35 s), and the Settings window
# opens briefly at the end.
set -euo pipefail

cd "$(dirname "$0")/.."
WORK=$(mktemp -d)/demo
trap 'rm -rf "$(dirname "$WORK")"' EXIT

cargo build -p copywren --features demo
cargo build --release -p demo-render

COPYWREN_DEMO_DIR="$WORK" target/debug/copywren
target/release/demo-render "$WORK" docs

ffmpeg -y -loglevel error -framerate 10 -i "$WORK/composite/%05d.png" \
    -vf "scale=1280:-2:flags=lanczos" -c:v libx264 -pix_fmt yuv420p -crf 24 \
    -movflags +faststart docs/demo.mp4
ffmpeg -y -loglevel error -framerate 10 -i "$WORK/composite/%05d.png" \
    -vf "scale=900:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=160:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    docs/demo.gif

echo "Wrote docs/demo.mp4, docs/demo.gif and docs/screenshots/"
ls -lh docs/demo.mp4 docs/demo.gif
