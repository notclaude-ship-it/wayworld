#!/bin/sh
# Installe Wayworld : binaire, assets et entrée de session (GDM/SDDM/greetd).
set -e
cd "$(dirname "$0")"
[ -d assets/textures ] || python3 scripts/fetch_assets.py 1k
cargo build --release
sudo install -Dm755 target/release/wayworld /usr/local/bin/wayworld
sudo install -Dm755 session/wayworld-session /usr/local/bin/wayworld-session
sudo install -Dm644 session/wayworld.desktop /usr/share/wayland-sessions/wayworld.desktop
sudo mkdir -p /usr/local/share/wayworld
sudo rm -rf /usr/local/share/wayworld/assets
sudo cp -r assets /usr/local/share/wayworld/assets
echo "OK — choisis la session « Wayworld » sur l'écran de connexion (nécessite le paquet cage)."
