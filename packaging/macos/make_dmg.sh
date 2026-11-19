#!/bin/bash
# Build Pythonizer.app from a compiled binary and pack it into a DMG.
# Usage: packaging/macos/make_dmg.sh <path/to/pythonizer binary> <output.dmg>
set -euo pipefail
BIN="$1"; OUT="$2"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$HERE/../.."
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
STAGE="$(mktemp -d)"
APP="$STAGE/Pythonizer.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/pythonizer"
cp "$ROOT/assets/icon.icns" "$APP/Contents/Resources/icon.icns"
sed "s/VERSION/$VERSION/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"
# Ad-hoc signature (required on Apple Silicon). Replace "-" with a
# "Developer ID Application: …" identity to avoid the Gatekeeper warning.
codesign --force --deep --sign "${MACOS_SIGN_IDENTITY:--}" "$APP"
ln -s /Applications "$STAGE/Applications"
rm -f "$OUT"
hdiutil create -volname "Pythonizer" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
rm -rf "$STAGE"
echo "Created $OUT"
