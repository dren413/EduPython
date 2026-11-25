#!/usr/bin/env bash
# Package the Linux build as EduPython-linux-x64.tar.gz:
#   EduPython/EduPython          the program (Python + packages built in)
#   EduPython/install.sh         optional: adds EduPython to the app menu
#   EduPython/edupython.png      icon
#   EduPython/README.txt
# Usage: packaging/linux/make_tarball.sh <binary> <output.tar.gz>
set -euo pipefail
bin="$1"
out="$2"
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
work="$(mktemp -d)"
dir="$work/EduPython"
mkdir -p "$dir"
install -m 755 "$bin" "$dir/EduPython"
install -m 644 "$root/assets/icon.png" "$dir/edupython.png"
install -m 755 "$here/install.sh" "$dir/install.sh"
cat > "$dir/README.txt" <<'TXT'
EduPython for Linux (64-bit)

Start:    double-click EduPython, or run  ./EduPython  in a terminal.
Menu:     run  ./install.sh  once to add EduPython to the app menu.

Python 3.12, pygame-ce, numpy and matplotlib are built in. On first start they are unpacked
once into ~/.local/share/EduPython (a few seconds).
School settings: put edupython.ini next to the EduPython program, or in
/etc/edupython/. See https://edupython.lu
TXT
tar -C "$work" -czf "$out" EduPython
rm -rf "$work"
echo "Wrote $out"
