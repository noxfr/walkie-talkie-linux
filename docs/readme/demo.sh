#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
frames=$(mktemp -d)
DEMO_FRAMES="$frames" cargo test --release -- --ignored images_de_la_demo
ffmpeg -y -loglevel error -framerate 15 -i "$frames/frame-%04d.png" \
  -vf "scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer" \
  docs/readme/demo.gif
rm -rf "$frames"
ls -la docs/readme/demo.gif
