#!/bin/sh
# Packages a built binary for Linux: linux/package.sh [binary] [output folder]
# Makes tubefast-linux-x64.tar.gz, and Tubefast-x86_64.AppImage too when appimagetool is on the PATH.
set -eu
cd "$(dirname "$0")/.."
binary="${1:-target/release/tubefast}"
out="${2:-target}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

folder="$work/tubefast"
mkdir -p "$folder/licenses"
cp "$binary" "$folder/tubefast"
cp linux/tubefast.desktop linux/tubefast.png linux/install.sh README.md LICENSE "$folder/"
cp assets/fonts/OFL-*.txt "$folder/licenses/"
chmod 755 "$folder/tubefast" "$folder/install.sh"
tar -czf "$out/tubefast-linux-x64.tar.gz" -C "$work" tubefast
echo "$out/tubefast-linux-x64.tar.gz"

if command -v appimagetool > /dev/null; then
    app="$work/Tubefast.AppDir"
    install -Dm755 "$binary" "$app/usr/bin/tubefast"
    install -Dm644 linux/tubefast.desktop "$app/usr/share/applications/tubefast.desktop"
    install -Dm644 linux/tubefast.png "$app/usr/share/icons/hicolor/256x256/apps/tubefast.png"
    cp linux/tubefast.desktop linux/tubefast.png "$app/"
    ln -s usr/bin/tubefast "$app/AppRun"
    ARCH=x86_64 appimagetool --no-appstream "$app" "$out/Tubefast-x86_64.AppImage" > /dev/null
    echo "$out/Tubefast-x86_64.AppImage"
fi
