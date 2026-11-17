#!/bin/sh
# Instala o pdrop para o usuário atual: binário em ~/.cargo/bin, ícone e atalho no menu.
set -e
cd "$(dirname "$0")"
cargo install --path . --locked
install -Dm644 assets/pdrop.png "$HOME/.local/share/icons/hicolor/256x256/apps/pdrop.png"
# caminho absoluto: o menu gráfico nem sempre tem ~/.cargo/bin no PATH
sed "s|^Exec=.*|Exec=$HOME/.cargo/bin/pdrop|" assets/pdrop.desktop > /tmp/pdrop.desktop
install -Dm644 /tmp/pdrop.desktop "$HOME/.local/share/applications/pdrop.desktop"
rm /tmp/pdrop.desktop
echo "pronto: procure 'pdrop' no menu de aplicativos"
