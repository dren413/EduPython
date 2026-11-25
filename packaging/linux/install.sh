#!/usr/bin/env bash
# Add EduPython to the app menu of the current user (no root needed).
# Run it again after moving the EduPython folder.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
mkdir -p "$apps" "$icons"
cp "$here/edupython.png" "$icons/edupython.png"
cat > "$apps/edupython.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=EduPython
Comment=Python editor for school
Exec="$here/EduPython" %F
Icon=edupython
Terminal=false
Categories=Development;Education;IDE;
MimeType=text/x-python;
StartupWMClass=edupython
DESKTOP
chmod 644 "$apps/edupython.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$apps" >/dev/null 2>&1 || true
echo "EduPython was added to the app menu."
