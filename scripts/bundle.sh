#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
MODE="${1:-release}"
# capture-core uses ScreenCaptureKit APIs from macOS 15.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"
VERSION="$(sed -n '/^\[package\]/,/^\[/s/^version = "\([^"]*\)".*/\1/p' Cargo.toml)"
if [ "$MODE" = "release" ]; then cargo build --release --locked; else cargo build --locked; fi
# The speech helper links Metal and runs as a child of the app (D9 on the plan).
cargo build --release --locked --manifest-path speech/Cargo.toml
APP_DEST="${GLANCE_BUNDLE_DEST:-$(pwd)/target/Attn Lens.app}"
mkdir -p "$(pwd)/target" "$(dirname "$APP_DEST")"
BUILD_DIR="$(mktemp -d "$(pwd)/target/.glance-bundle.XXXXXX")"
APP="$BUILD_DIR/Attn Lens.app"
cleanup() {
    if [ ! -e "$APP_DEST" ] && [ -e "$BUILD_DIR/previous.app" ]; then
        mv "$BUILD_DIR/previous.app" "$APP_DEST"
    fi
    rm -rf "$BUILD_DIR"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cargo run --release --locked --example icon -- target/AttnLens.iconset
# A new resource name prevents macOS from reusing an earlier design's icon cache.
ICON_HASH="$(shasum -a 256 target/AttnLens.iconset/icon_512x512@2x.png | cut -c 1-12)"
ICON_NAME="AttnLens-$ICON_HASH.icns"
iconutil -c icns target/AttnLens.iconset -o "$APP/Contents/Resources/$ICON_NAME"
/usr/bin/swiftc -target "$(uname -m)-apple-macosx15.0" -O native/video_encoder.swift -o target/glance-video-encoder
/usr/bin/swiftc -target "$(uname -m)-apple-macosx15.0" -O native/video_frame.swift -o target/glance-video-frame
/usr/bin/swiftc -target "$(uname -m)-apple-macosx15.0" -O native/ocr.swift -o target/glance-ocr
cp target/glance-ocr "$APP/Contents/MacOS/glance-ocr"
cp target/glance-video-frame "$APP/Contents/MacOS/glance-video-frame"
cp target/glance-video-encoder "$APP/Contents/MacOS/glance-video-encoder"
cp "${CARGO_TARGET_DIR:-speech/target}/release/attn-speech" "$APP/Contents/MacOS/attn-speech"
mkdir -p "$APP/Contents/Resources/speech-licenses"
cp speech/LICENSES/* "$APP/Contents/Resources/speech-licenses/"
cp assets/gpui/LICENSE-APACHE "$APP/Contents/Resources/GPUI-LICENSE"
cp assets/rune/LICENSE "$APP/Contents/Resources/Rune-LICENSE"
cp assets/lucide/LICENSE "$APP/Contents/Resources/Lucide-LICENSE"
cp assets/fonts/LICENSE "$APP/Contents/Resources/Roboto-LICENSE"
cp LICENSE "$APP/Contents/Resources/LICENSE"
cp THIRD_PARTY_NOTICES.md "$APP/Contents/Resources/THIRD_PARTY_NOTICES.md"
cp "${CARGO_TARGET_DIR:-target}/$MODE/attn-lens" "$APP/Contents/MacOS/AttnLens"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>AttnLens</string>
<key>CFBundleIdentifier</key><string>dev.attn.lens</string>
<key>CFBundleIconFile</key><string>AttnLens.icns</string>
<key>CFBundleName</key><string>Attn Lens</string>
<key>CFBundleDisplayName</key><string>Attn Lens</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>CFBundleVersion</key><string>$VERSION</string>
<key>LSMinimumSystemVersion</key><string>15.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSScreenCaptureUsageDescription</key><string>Attn Lens captures your screen so you can mark it up and send it to an agent.</string>
<key>NSMicrophoneUsageDescription</key><string>Attn Lens transcribes what you say during a capture, on this Mac, and sends the words with it.</string>
</dict></plist>
PLIST
/usr/libexec/PlistBuddy -c "Set :CFBundleIconFile $ICON_NAME" "$APP/Contents/Info.plist"
./scripts/sign-app.sh "$APP"
# Do not replace the installed bundle until every build/signing step succeeds.
if [ -e "$APP_DEST" ]; then mv "$APP_DEST" "$BUILD_DIR/previous.app"; fi
mv "$APP" "$APP_DEST"
printf 'Built %s\n' "$APP_DEST"
