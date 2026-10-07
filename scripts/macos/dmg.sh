#!/bin/bash
# Packs target/release/Murmur.app (run bundle.sh first) into murmur-vX.Y.Z-macos-<arch>.dmg in the
# repo root, with a link to Applications to drag it onto, and its .sha256 beside it.
set -euo pipefail
cd "$(dirname "$0")/../.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app=target/release/Murmur.app
[ -d "$app" ] || { echo "no $app: run scripts/macos/bundle.sh first" >&2; exit 1; }
name="murmur-v$version-macos-$(uname -m)"

stage=$(mktemp -d)
# ditto keeps the bundle's signature and extended attributes intact
ditto "$app" "$stage/Murmur.app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "Murmur $version" -srcfolder "$stage" -format UDZO -ov "$name.dmg" > /dev/null
rm -rf "$stage"
shasum -a 256 "$name.dmg" > "$name.dmg.sha256"
echo "built $name.dmg"
