#!/bin/sh
# Crée une archive de release : binaire + assets + session + installateur.
# Usage : scripts/package-release.sh <binaire> <nom-archive> [dossier-sortie]
#   ex. : scripts/package-release.sh target/release/wayworld wayworld-v0.1.0-linux-x86_64 dist
set -e
cd "$(dirname "$0")/.."
BIN="$1"; NAME="$2"; OUT="${3:-dist}"
[ -f "$BIN" ] && [ -n "$NAME" ] || { echo "usage: $0 <binaire> <nom> [sortie]"; exit 1; }
[ -d assets/textures ] || python3 scripts/fetch_assets.py 1k
mkdir -p "$OUT"
TMP=$(mktemp -d)
D="$TMP/$NAME"
mkdir -p "$D"
cp "$BIN" "$D/wayworld"
cp -r assets session LICENSE README.md "$D/"
cat > "$D/install.sh" <<'EOF'
#!/bin/sh
# Installe Wayworld depuis cette archive : binaire, assets et entrée de session (nécessite cage).
set -e
cd "$(dirname "$0")"
sudo install -Dm755 wayworld /usr/local/bin/wayworld
sudo install -Dm755 session/wayworld-session /usr/local/bin/wayworld-session
sudo install -Dm644 session/wayworld.desktop /usr/share/wayland-sessions/wayworld.desktop
sudo mkdir -p /usr/local/share/wayworld
sudo rm -rf /usr/local/share/wayworld/assets
sudo cp -r assets /usr/local/share/wayworld/assets
echo "OK : lance \`wayworld\` ou choisis la session Wayworld sur l'écran de connexion."
EOF
chmod +x "$D/install.sh" "$D/wayworld"
tar -C "$TMP" -czf "$OUT/$NAME.tar.gz" "$NAME"
rm -rf "$TMP"
echo "$OUT/$NAME.tar.gz"
