#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
target_dir="$PWD/target"
version="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"
build="${BUILD_NUMBER:-1}"
triple="${TARGET:-}"

if [ -n "$triple" ]; then
    cargo build --release --locked --target "$triple" --target-dir "$target_dir"
    binary="$target_dir/$triple/release/openmusic"
    bundle="$target_dir/$triple/OpenMusic.app"
else
    cargo build --release --locked --target-dir "$target_dir"
    binary="$target_dir/release/openmusic"
    bundle="$target_dir/OpenMusic.app"
fi

mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp assets/icon/OpenMusic.icns "$bundle/Contents/Resources/OpenMusic.icns"
cp "$binary" "$bundle/Contents/MacOS/OpenMusic"
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>OpenMusic</string>
    <key>CFBundleIdentifier</key>
    <string>io.openmusic.player</string>
    <key>CFBundleIconFile</key>
    <string>OpenMusic</string>
    <key>CFBundleName</key>
    <string>OpenMusic</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$build</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

plutil -lint "$bundle/Contents/Info.plist"
identity="${CODESIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Apple Development|Developer ID Application/ { print $2; exit }')}"
case "$identity" in
    "Developer ID Application"*) set -- --options runtime --timestamp ;;
    *) set -- ;;
esac
codesign --force "$@" --sign "${identity:--}" "$bundle"
printf '%s\n' "$bundle"
