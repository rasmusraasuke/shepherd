#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
prefix=${PREFIX:-$HOME/.local}

install -Dm755 "$here/shepherd" "$prefix/bin/shepherd"
install -Dm644 "$here/shepherd.png" "$prefix/share/icons/hicolor/1024x1024/apps/shepherd.png"
install -Dm644 "$here/shepherd.desktop" "$prefix/share/applications/shepherd.desktop"
for font in ibm-plex-sans lilex juliamono noto-emoji; do
    install -Dm644 "$here/fonts/$font/LICENSE.txt" "$prefix/share/licenses/shepherd/fonts/$font/LICENSE.txt"
done

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
fi

echo "installed $prefix/bin/shepherd"
