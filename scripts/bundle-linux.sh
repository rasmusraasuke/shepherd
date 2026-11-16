#!/bin/sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

version=$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)
target=${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
case "$target" in
    *-linux-*) ;;
    *)
        echo "bundle-linux.sh builds a Linux binary; use bundle.sh on macOS" >&2
        exit 1
        ;;
esac

arch=${target%%-*}
cargo build --release --target "$target"

name=Shepherd-$version-linux-$arch
staging=target/linux/$name
rm -rf "$staging"
mkdir -p "$staging"
cp "target/$target/release/shepherd" "$staging/shepherd"
mkdir -p "$staging/fonts/ibm-plex-sans" "$staging/fonts/lilex"
cp assets/fonts/DejaVuSans-LICENSE.txt "$staging/fonts/"
cp assets/fonts/ibm-plex-sans/LICENSE.txt "$staging/fonts/ibm-plex-sans/"
cp assets/fonts/lilex/LICENSE.txt "$staging/fonts/lilex/"
cp packaging/linux/shepherd.desktop "$staging/shepherd.desktop"
cp packaging/linux/install.sh "$staging/install.sh"
cp assets/icon.png "$staging/shepherd.png"
chmod +x "$staging/shepherd" "$staging/install.sh"

if command -v strip >/dev/null 2>&1; then
    strip "$staging/shepherd"
fi

mkdir -p dist
tar -C target/linux -czf "dist/$name.tar.gz" "$name"
echo "archive: $root/dist/$name.tar.gz"
