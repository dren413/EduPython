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
# On CI machines `hdiutil create` often fails with "Resource busy" while the
# freshly mounted image is still in use. Try it a few times, then fall back to
# `makehybrid`, which writes the image without mounting it, and compress that.
created=0
for attempt in 1 2 3; do
  if hdiutil create -volname "EduPython" -srcfolder "$STAGE" -ov -format UDZO "$OUT"; then
    created=1
    break
  fi
  echo "hdiutil create failed (attempt $attempt)" >&2
  sleep $((attempt * 5))
done
if [ "$created" = 0 ]; then
  echo "Falling back to hdiutil makehybrid" >&2
  RAW="$(mktemp -d)/EduPython-raw.dmg"
  hdiutil makehybrid -hfs -hfs-volume-name "EduPython" -o "$RAW" "$STAGE"
  hdiutil convert "$RAW" -format UDZO -ov -o "$OUT"
  rm -f "$RAW"
fi
rm -rf "$STAGE"
echo "Created $OUT"
