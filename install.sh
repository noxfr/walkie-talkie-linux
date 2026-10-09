#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

SHORTCUT="${WALKIE_SHORTCUT:-<Primary><Alt>space}"
MODEL="${WALKIE_MODEL_NAME:-small}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/walkie-talkie"

for cmd in cargo cmake pw-record xdotool xclip notify-send; do
  command -v "$cmd" >/dev/null || { echo "manquant : $cmd (sudo apt install xdotool xclip cmake clang)"; exit 1; }
done

features=()
command -v nvcc >/dev/null && features=(--features cuda)
cargo install --path . --locked "${features[@]}"
BIN="$HOME/.cargo/bin/walkie-talkie"

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
KEY_PATH=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/walkie-talkie/
current=$(gsettings get $SCHEMA custom-keybindings)
if [[ "$current" != *"$KEY_PATH"* ]]; then
  if [[ "$current" == "@as []" ]]; then
    gsettings set $SCHEMA custom-keybindings "['$KEY_PATH']"
  else
    gsettings set $SCHEMA custom-keybindings "${current%]*}, '$KEY_PATH']"
  fi
fi
KB="$SCHEMA.custom-keybinding:$KEY_PATH"
gsettings set "$KB" name "Walkie Talkie"
gsettings set "$KB" command "$BIN toggle"
gsettings set "$KB" binding "$SHORTCUT"

echo "Installé. Raccourci : $SHORTCUT — logs : journalctl --user -u walkie-talkie -f"
