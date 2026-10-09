# 📻 Walkie Talkie Linux

Dictée vocale vers Claude Code (ou n'importe quelle fenêtre) sous Linux X11.
Inspiré de [victorrentea/walkie-talkie](https://github.com/victorrentea/walkie-talkie), l'overlay macOS de Victor Rentea — réécrit de zéro en Rust, sans reprise de code.

## Utilisation

1. Mettre le focus sur la fenêtre cible (le terminal où tourne Claude Code).
2. <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd> : le micro s'ouvre (notification « 🎙️ Écoute… »). Parler.
3. <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd> à nouveau : le micro se ferme, la fenêtre active à cet instant est retenue comme cible, la transcription démarre (Whisper en local, rien ne sort de la machine).
4. Le texte s'affiche en notification et part **5 s plus tard** dans la fenêtre retenue, suivi d'Entrée. Pendant ces 5 s :
   - <kbd>Entrée</kbd> : envoyer tout de suite ;
   - <kbd>Échap</kbd> ou <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd> : annuler.
5. Le presse-papiers garde toujours la dernière phrase transcrite (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> pour la recoller dans un terminal).

⚠️ Le texte est tapé dans la fenêtre retenue, quelle qu'elle soit (y compris un shell).
⚠️ Pendant les 5 s d'attente, Entrée et Échap sont captées par Walkie Talkie et n'arrivent pas aux autres applications.

## Installation

Testé sur Ubuntu 24.04 / GNOME / X11 (pas Wayland : `xdotool` et la capture des touches en dépendent).

### 1. Dépendances

```bash
sudo apt install xdotool xclip cmake clang
```

Rust doit être installé ([rustup](https://rustup.rs)). `pw-record` (PipeWire) et `notify-send` sont présents par défaut sur Ubuntu.

### 2. GPU NVIDIA (optionnel, recommandé)

La transcription passe de ~2,5 s (CPU) à ~1,5 s (GPU) pour une phrase courte.

```bash
sudo apt install nvidia-cuda-toolkit g++-12
```

Le CUDA 12.0 d'Ubuntu refuse le gcc 13 du système : `install.sh` utilise automatiquement `g++-12` comme compilateur hôte. La compilation CUDA prend ~5 min la première fois.

### 3. Installer

```bash
./install.sh
```

Le script :
- compile et installe le binaire (`~/.cargo/bin/walkie-talkie`), avec CUDA si `nvcc` est présent ;
- télécharge le modèle `ggml-small.bin` (~466 Mo) dans `~/.local/share/walkie-talkie/` ;
- crée et démarre le service utilisateur systemd `walkie-talkie` (lancé avec la session graphique) ;
- ajoute le raccourci GNOME <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd>.

Relancer `./install.sh` après une mise à jour du code.

### 4. Micro

Un micro trop bas donne une transcription fantaisiste. Vérifier le volume d'entrée (Paramètres → Son → Entrée) ou :

```bash
wpctl get-volume @DEFAULT_AUDIO_SOURCE@
```

```bash
wpctl set-volume @DEFAULT_AUDIO_SOURCE@ 1.0
```

Couper la musique pendant la dictée : elle est captée par le micro.

## Réglages

Variables d'environnement du service (à ajouter dans `~/.config/systemd/user/walkie-talkie.service`, section `[Service]`, puis `systemctl --user daemon-reload && systemctl --user restart walkie-talkie` ; `install.sh` réécrit ce fichier, à refaire après une réinstallation) :

| variable | défaut | rôle |
|---|---|---|
| `WALKIE_LANG` | `fr` | langue (`en`, `auto`… ; `auto` double le temps de transcription) |
| `WALKIE_HOLD` | `5` | secondes avant envoi |
| `WALKIE_MODEL` | `~/.local/share/walkie-talkie/ggml-small.bin` | modèle whisper.cpp |

Variables de `install.sh` :

| variable | défaut | rôle |
|---|---|---|
| `WALKIE_SHORTCUT` | `<Primary><Alt>space` | raccourci GNOME |
| `WALKIE_MODEL_NAME` | `small` | modèle à télécharger (`base`, `medium`… ; `medium` est plus précis, raisonnable avec un GPU) |

Exemple : `WALKIE_MODEL_NAME=medium ./install.sh`

## Dépannage

```bash
journalctl --user -u walkie-talkie -f
```

- **Rien ne se passe au raccourci** : vérifier que le service tourne (`systemctl --user status walkie-talkie`) et que le raccourci inclut bien <kbd>Alt</kbd> (<kbd>Ctrl</kbd>+<kbd>Espace</kbd> seul tape un caractère nul dans un terminal).
- **« no GPU found » dans les logs** : le binaire est compilé sans CUDA ; installer `nvidia-cuda-toolkit` puis relancer `./install.sh`.
- **Transcription incompréhensible** : volume du micro, bruit de fond, ou langue (`WALKIE_LANG`).
