# 📻 Walkie Talkie Linux

Dictée vocale vers Claude Code (ou n'importe quelle fenêtre) sous Linux X11.
Inspiré de [victorrentea/walkie-talkie](https://github.com/victorrentea/walkie-talkie), l'overlay macOS de Victor Rentea — réécrit de zéro en Rust, sans reprise de code.

## Comment ça marche

1. <kbd>Super</kbd>+<kbd>Alt</kbd>+<kbd>D</kbd> : le micro s'ouvre (notification « Écoute… »).
2. <kbd>Super</kbd>+<kbd>Alt</kbd>+<kbd>D</kbd> à nouveau : le micro se ferme, la fenêtre active est mémorisée, la transcription démarre (Whisper en local, rien ne sort de la machine).
3. Le texte s'affiche en notification et part **5 s plus tard** dans la fenêtre mémorisée, suivi d'Entrée. Le même raccourci pendant ces 5 s **annule**.
4. Le presse-papiers garde toujours la dernière phrase transcrite.

⚠️ Le texte est tapé dans la fenêtre qui avait le focus à l'arrêt du micro, quelle qu'elle soit (y compris un shell).

## Installation

```bash
sudo apt install xdotool xclip cmake clang
./install.sh
```

Le script compile et installe le binaire (`~/.cargo/bin/walkie-talkie`), télécharge le modèle `ggml-small.bin` (~466 Mo), crée le service utilisateur systemd `walkie-talkie` et le raccourci GNOME.

Avec `nvcc` présent (`nvidia-cuda-toolkit`), la compilation active CUDA.

## Réglages

| variable | défaut | rôle |
|---|---|---|
| `WALKIE_LANG` | `fr` | langue (`en`, `auto`… ; `auto` double le temps de transcription) |
| `WALKIE_HOLD` | `5` | secondes avant envoi |
| `WALKIE_MODEL` | `~/.local/share/walkie-talkie/ggml-small.bin` | modèle whisper.cpp |
| `WALKIE_SHORTCUT` (install) | `<Super><Alt>d` | raccourci GNOME |
| `WALKIE_MODEL_NAME` (install) | `small` | modèle à télécharger (`base`, `medium`…) |

Logs : `journalctl --user -u walkie-talkie -f`
