#!/bin/sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

name=Shepherd
version=$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)
host=$(rustc -vV | sed -n 's/^host: //p')
universal=false
install=false
link=false
zip=false

for arg in "$@"; do
    case "$arg" in
        --universal) universal=true ;;
        --install) install=true ;;
        --link) link=true ;;
        --zip) zip=true ;;
        *)
            echo "unknown option: $arg" >&2
            exit 1
            ;;
    esac
done

targets=$host
arch=${host%%-*}
if $universal; then
    targets="aarch64-apple-darwin x86_64-apple-darwin"
    arch=universal
fi

for target in $targets; do
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "missing target $target: run rustup target add $target" >&2
        exit 1
    fi
    cargo build --release --target "$target"
done

if $universal; then
    mkdir -p target/universal
    lipo -create -output target/universal/shepherd \
        target/aarch64-apple-darwin/release/shepherd \
        target/x86_64-apple-darwin/release/shepherd
    binary=target/universal/shepherd
else
    binary="target/$host/release/shepherd"
fi

app=target/$name.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/shepherd"
for font in ibm-plex-sans lilex juliamono noto-emoji; do
    mkdir -p "$app/Contents/Resources/fonts/$font"
    cp "assets/fonts/$font/LICENSE.txt" "$app/Contents/Resources/fonts/$font/"
done

if [ -f assets/icon.png ]; then
    iconset=target/$name.iconset
    rm -rf "$iconset"
    mkdir -p "$iconset"
    icon() { sips -z "$1" "$1" assets/icon.png --out "$iconset/$2" >/dev/null; }
    icon 16 icon_16x16.png
    icon 32 icon_16x16@2x.png
    icon 32 icon_32x32.png
    icon 64 icon_32x32@2x.png
    icon 128 icon_128x128.png
    icon 256 icon_128x128@2x.png
    icon 256 icon_256x256.png
    icon 512 icon_256x256@2x.png
    icon 512 icon_512x512.png
    icon 1024 icon_512x512@2x.png
    iconutil -c icns "$iconset" -o "$app/Contents/Resources/$name.icns"
else
    echo "no assets/icon.png: the app is built without an icon" >&2
fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>$name</string>
    <key>CFBundleExecutable</key>
    <string>shepherd</string>
    <key>CFBundleIconFile</key>
    <string>$name</string>
    <key>CFBundleIdentifier</key>
    <string>com.rasmusraasuke.shepherd</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>$name</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

codesign --force --sign - "$app"
codesign --verify --strict "$app"

if $zip; then
    mkdir -p dist
    ditto -c -k --keepParent "$app" "dist/$name-$version-macos-$arch.zip"
    echo "zip: $root/dist/$name-$version-macos-$arch.zip"
fi

if $install; then
    rm -rf "/Applications/$name.app"
    ditto "$app" "/Applications/$name.app"
    echo "app: /Applications/$name.app"
else
    echo "app: $root/$app"
fi

if $link; then
    if $install; then
        binary=/Applications/$name.app/Contents/MacOS/shepherd
    else
        binary="$root/$app/Contents/MacOS/shepherd"
    fi
    mkdir -p "$HOME/.local/bin"
    ln -sf "$binary" "$HOME/.local/bin/shepherd"
    echo "command: $HOME/.local/bin/shepherd"
fi
