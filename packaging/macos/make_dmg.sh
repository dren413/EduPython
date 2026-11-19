#!/bin/bash
# Build EduPython.app from a compiled binary and pack it into a DMG.
# Usage: packaging/macos/make_dmg.sh <path/to/edupython binary> <output.dmg>
set -euo pipefail
BIN="$1"; OUT="$2"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$HERE/../.."
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
STAGE="$(mktemp -d)"
APP="$STAGE/EduPython.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/edupython"
cp "$ROOT/assets/icon.icns" "$APP/Contents/Resources/icon.icns"
sed "s/VERSION/$VERSION/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"
# Ad-hoc signature (required on Apple Silicon). Replace "-" with a
# "Developer ID Application: …" identity to avoid the Gatekeeper warning.
codesign --force --deep --sign "${MACOS_SIGN_IDENTITY:--}" "$APP"
ln -s /Applications "$STAGE/Applications"
rm -f "$OUT"
# hdiutil sometimes fails on CI machines ("Resource busy"): retry a few times.
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "EduPython" -srcfolder "$STAGE" -ov -format UDZO "$OUT"; then
    break
  fi
  if [ "$attempt" = 5 ]; then
    echo "hdiutil failed 5 times" >&2
    exit 1
  fi
  echo "hdiutil failed (attempt $attempt), retrying..." >&2
  sleep $((attempt * 5))
done
rm -rf "$STAGE"
echo "Created $OUT"
