#!/bin/sh
# Installs Tubefast for the current user: ./install.sh, or ./install.sh --uninstall to remove it.
set -eu
cd "$(dirname "$0")"
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
desktop="$data/applications/tubefast.desktop"
icon="$data/icons/hicolor/256x256/apps/tubefast.png"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$bin/tubefast" "$desktop" "$icon"
    echo "Tubefast is removed. Your library and sign-in stay until you delete $data/tubefast."
    exit
fi

install -Dm755 tubefast "$bin/tubefast"
install -Dm644 tubefast.png "$icon"
mkdir -p "$(dirname "$desktop")"
sed "s|^Exec=.*|Exec=$bin/tubefast|" tubefast.desktop > "$desktop"
command -v update-desktop-database > /dev/null && update-desktop-database -q "$data/applications" || true
echo "Tubefast is installed. Open it from your app menu, or run $bin/tubefast."
