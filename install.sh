#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

SHORTCUT="${WALKIE_SHORTCUT:-<Super>q}"
SHOT_SHORTCUT="${WALKIE_SHOT_SHORTCUT:-<Super>w}"
PLAIN_SHORTCUT="${WALKIE_PLAIN_SHORTCUT:-<Super>e}"
MODEL="${WALKIE_MODEL_NAME:-small}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/walkie-talkie"

require() {
  for cmd in "$@"; do
    command -v "$cmd" >/dev/null || { echo "manquant : $cmd — voir la section Installation du README"; exit 1; }
  done
}
require curl pw-record xdotool xclip notify-send gsettings systemctl

if [ -x ./walkie-talkie ]; then
  BIN="$HOME/.local/bin/walkie-talkie"
  install -Dm755 ./walkie-talkie "$BIN"
else
  require cargo cmake
  features=()
  if command -v nvcc >/dev/null; then
    features=(--features cuda)
    export CMAKE_CUDA_ARCHITECTURES="${CMAKE_CUDA_ARCHITECTURES:-native}"
    command -v g++-12 >/dev/null && export CMAKE_CUDA_HOST_COMPILER="${CMAKE_CUDA_HOST_COMPILER:-$(command -v g++-12)}"
  fi
  cargo install --path . --locked "${features[@]}"
  BIN="$HOME/.cargo/bin/walkie-talkie"
fi

mkdir -p "$DATA_DIR"
if [ ! -f "$DATA_DIR/ggml-$MODEL.bin" ]; then
  curl -fL -o "$DATA_DIR/ggml-$MODEL.bin" "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$MODEL.bin"
fi

mkdir -p "$HOME/.config/systemd/user"
cat > "$HOME/.config/systemd/user/walkie-talkie.service" <<EOF
[Unit]
Description=Walkie Talkie (dictée vocale)
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=$BIN serve
Environment=WALKIE_MODEL=$DATA_DIR/ggml-$MODEL.bin
Restart=on-failure

[Install]
WantedBy=graphical-session.target
EOF
systemctl --user daemon-reload
systemctl --user enable --now walkie-talkie.service
systemctl --user restart walkie-talkie.service

SCHEMA=org.gnome.settings-daemon.plugins.media-keys
add_shortcut() {
  local key_path=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/$1/
  local current
  current=$(gsettings get $SCHEMA custom-keybindings)
  if [[ "$current" != *"$key_path"* ]]; then
    if [[ "$current" == "@as []" ]]; then
      gsettings set $SCHEMA custom-keybindings "['$key_path']"
    else
      gsettings set $SCHEMA custom-keybindings "${current%]*}, '$key_path']"
    fi
  fi
  local kb="$SCHEMA.custom-keybinding:$key_path"
  gsettings set "$kb" name "$2"
  gsettings set "$kb" command "$3"
  gsettings set "$kb" binding "$4"
}

DOCK=org.gnome.shell.extensions.dash-to-dock
if [[ "$(gsettings get $DOCK shortcut 2>/dev/null)" == "['$SHORTCUT']" ]]; then
  gsettings set $DOCK shortcut "[]"
fi
add_shortcut walkie-talkie "Walkie Talkie" "$BIN toggle" "$SHORTCUT"
add_shortcut walkie-talkie-shot "Walkie Talkie : capture" "$BIN shot" "$SHOT_SHORTCUT"
add_shortcut walkie-talkie-plain "Walkie Talkie : dictée simple" "$BIN plain" "$PLAIN_SHORTCUT"

echo "Installé. Dictée : $SHORTCUT — dictée simple : $PLAIN_SHORTCUT — capture : $SHOT_SHORTCUT — logs : journalctl --user -u walkie-talkie -f"
