#!/bin/bash
# Builds target/release/Murmur.app: the release binary, the sherpa-onnx dylibs beside it, an
# Info.plist that makes Murmur a menu bar app and explains the microphone prompt, and the icon.
#
# macOS ties the Accessibility and microphone permissions to the app's code signature. Set
# MURMUR_SIGN_IDENTITY to a signing identity in your keychain (`security find-identity -v -p
# codesigning` lists them) so a rebuilt app keeps its permissions; otherwise the first Apple
# Development identity is used, and with none the app is signed ad hoc, which macOS treats as a
# new app on every build.
set -euo pipefail
cd "$(dirname "$0")/../.."

cargo build --release --locked

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app=target/release/Murmur.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/murmur target/release/*.dylib "$app/Contents/MacOS/"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Murmur</string>
  <key>CFBundleDisplayName</key><string>Murmur</string>
  <key>CFBundleIdentifier</key><string>dev.jshowah.murmur</string>
  <key>CFBundleExecutable</key><string>murmur</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Murmur listens while you hold the push-to-talk key and turns your speech into text on this Mac. Audio never leaves it.</string>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" > /dev/null

iconset=$(mktemp -d)/AppIcon.iconset
mkdir -p "$iconset"
for size in 16 32 128 256; do
  sips -z "$size" "$size" assets/murmur.png --out "$iconset/icon_${size}x${size}.png" > /dev/null
  double=$((size * 2))
  if [ "$double" -le 256 ]; then
    sips -z "$double" "$double" assets/murmur.png --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
  fi
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

identity=${MURMUR_SIGN_IDENTITY:-}
if [ -z "$identity" ]; then
  identity=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Apple Development: .*\)"/\1/p' | head -n 1)
fi
if [ -z "$identity" ]; then
  identity=-
  echo "warning: no signing identity found; signing ad hoc, so macOS asks for permissions again after every build" >&2
fi
for lib in "$app"/Contents/MacOS/*.dylib; do
  codesign --force --sign "$identity" "$lib"
done
codesign --force --sign "$identity" --identifier dev.jshowah.murmur "$app"
codesign --verify --strict "$app"
echo "built $app (v$version, signed by ${identity/#-/ad hoc})"
