# 📻 Walkie Talkie Linux

Dictée vocale sous Linux X11 : ce que tu dis est tapé dans la fenêtre qui a le focus — terminal (Claude Code…), éditeur, navigateur, messagerie.
Inspiré de [victorrentea/walkie-talkie](https://github.com/victorrentea/walkie-talkie), l'overlay macOS de Victor Rentea — réécrit de zéro en Rust, sans reprise de code.

## Utilisation

1. Mettre le focus sur la fenêtre où écrire (n'importe laquelle).
2. <kbd>Super</kbd>+<kbd>Q</kbd> : le micro s'ouvre (notification « 🎙️ Écoute… »). Parler.
3. <kbd>Super</kbd>+<kbd>Q</kbd> à nouveau : le micro se ferme, la fenêtre active à cet instant est retenue comme cible, la transcription démarre (Whisper en local, rien ne sort de la machine).
4. Le texte s'affiche en notification et part **5 s plus tard** dans la fenêtre retenue, suivi d'Entrée. Pendant ces 5 s :
   - <kbd>Entrée</kbd> : envoyer tout de suite ;
   - <kbd>Échap</kbd> ou <kbd>Super</kbd>+<kbd>Q</kbd> : annuler.
5. Les annotations de bruit que Whisper ajoute (`*Bruit de la porte*`, `[Musique]`, `(rires)`) sont retirées du texte.
6. Le presse-papiers garde toujours la dernière phrase transcrite (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> pour la recoller dans un terminal).

⚠️ Le texte est tapé dans la fenêtre retenue, quelle qu'elle soit (y compris un shell), puis Entrée est pressée : dans une messagerie ou un formulaire, ça envoie.
⚠️ Pendant les 5 s d'attente, Entrée et Échap sont captées par Walkie Talkie et n'arrivent pas aux autres applications.

## Installation

Testé sur Ubuntu 22.04 / 24.04, GNOME, X11 (pas Wayland : `xdotool` et la capture des touches en dépendent).

### Option A — binaire précompilé (recommandé)

Binaire x86_64, transcription sur CPU (processeur avec AVX2 : Intel depuis 2013, AMD depuis 2015), glibc ≥ 2.35.

```bash
sudo apt install xdotool xclip
```

```bash
curl -fsSL https://github.com/NoxFr/walkie-talkie-linux/releases/latest/download/walkie-talkie-linux-x86_64.tar.gz | tar xz
```

```bash
./walkie-talkie-linux-x86_64/install.sh
```

Le binaire est installé dans `~/.local/bin/walkie-talkie`. Pour mettre à jour : refaire les deux dernières commandes.

### Option B — depuis les sources (pour le GPU NVIDIA)

```bash
sudo apt install xdotool xclip cmake clang
```

Rust doit être installé ([rustup](https://rustup.rs)). Pour le GPU NVIDIA (optionnel) :

```bash
sudo apt install nvidia-cuda-toolkit g++-12
```

Gain modeste avec le modèle `small` sur un petit GPU (voir [Performances](#performances)) ; plus utile pour `medium` ou pour laisser le CPU libre. Le CUDA 12.0 d'Ubuntu refuse le gcc 13 du système : `install.sh` utilise automatiquement `g++-12` comme compilateur hôte. La compilation CUDA prend ~5 min la première fois.

```bash
git clone https://github.com/NoxFr/walkie-talkie-linux && cd walkie-talkie-linux && ./install.sh
```

Le binaire est compilé et installé dans `~/.cargo/bin/walkie-talkie`, avec CUDA si `nvcc` est présent. Relancer `./install.sh` après une mise à jour du code.

### Ce que fait `install.sh` (les deux options)

- télécharge le modèle `ggml-small.bin` (~466 Mo) dans `~/.local/share/walkie-talkie/` ;
- crée et démarre le service utilisateur systemd `walkie-talkie` (lancé avec la session graphique) ;
- ajoute le raccourci GNOME <kbd>Super</kbd>+<kbd>Q</kbd> (<kbd>Super</kbd> = touche Windows) et le retire du Dock Ubuntu, qui l'utilise pour afficher ses numéros d'applications (<kbd>Super</kbd>+<kbd>1</kbd>…<kbd>9</kbd> restent actifs). Pour rendre la touche au Dock : `gsettings set org.gnome.shell.extensions.dash-to-dock shortcut "['<Super>q']"`.

`pw-record` (PipeWire), `notify-send` et `curl` sont présents par défaut sur Ubuntu.

### Micro

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
| `WALKIE_SHORTCUT` | `<Super>q` | raccourci GNOME |
| `WALKIE_MODEL_NAME` | `small` | modèle à télécharger (`base`, `medium`… ; `medium` est plus précis, raisonnable avec un GPU) |

Exemple : `WALKIE_MODEL_NAME=medium ./install.sh`

## Performances

Temps de transcription d'une phrase de 4,5 s, langue `fr`, 4 passes :

| matériel | `small` (466 Mo) | `medium` (1,5 Go) |
|---|---|---|
| CPU Intel i7-12700H (10 threads) | 2,6 – 2,8 s | 7,4 – 8,1 s |
| GPU NVIDIA T600 Laptop 4 Go (CUDA 12.0) | 1,8 – 2,8 s | 4,7 – 4,8 s |

Whisper traite toujours une fenêtre de 30 s : le temps varie peu avec la longueur de la phrase. `WALKIE_LANG=auto` ajoute une détection de langue qui double à peu près le temps.

## Tests et CI

```bash
cargo test
```

Tester la transcription d'un fichier WAV (16 kHz mono 16 bits), utile pour vérifier le micro ou un modèle :

```bash
walkie-talkie transcribe fichier.wav
```

La CI GitHub Actions (`.github/workflows/ci.yml`) lance les tests, compile un binaire portable et vérifie une transcription réelle (phrase synthétisée par `espeak-ng`, modèle `base`).

## Publier une release

```bash
git tag v0.2.0 && git push origin v0.2.0
```

Le workflow `release.yml` rejoue la CI puis publie `walkie-talkie-linux-x86_64.tar.gz` (binaire + `install.sh` + README) et sa somme SHA-256 sur la page Releases.

## Dépannage

```bash
journalctl --user -u walkie-talkie -f
```

- **Rien ne se passe au raccourci** : vérifier que le service tourne (`systemctl --user status walkie-talkie`) et qu'aucune autre application ne capte <kbd>Super</kbd>+<kbd>Q</kbd> (`gsettings list-recursively | grep "<Super>q"`).
- **« no GPU found » dans les logs** : le binaire est compilé sans CUDA ; installer `nvidia-cuda-toolkit` puis relancer `./install.sh`.
- **Transcription incompréhensible** : volume du micro, bruit de fond, ou langue (`WALKIE_LANG`).
