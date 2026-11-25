#!/usr/bin/env python3
"""Download a standalone CPython, install the packages pupils need and zip it.

The zip is embedded into EduPython when building with
`--features bundled-python` and `EDUPYTHON_PYTHON_ZIP=<path to zip>`.

Usage:
    python3 packaging/prepare_python.py <target-triple> <output.zip> [python-minor]

    target triples: x86_64-pc-windows-msvc, aarch64-apple-darwin,
                    x86_64-apple-darwin, x86_64-unknown-linux-gnu

Python builds come from https://github.com/astral-sh/python-build-standalone
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

RELEASES = "https://api.github.com/repos/astral-sh/python-build-standalone/releases?per_page=5"
EXTRA_PACKAGES = ["pygame-ce", "numpy", "matplotlib"]  # pygame-ce is imported as `pygame`
# Not needed by pupils; removing them makes the download smaller.
REMOVE = ["test", "idlelib", "lib2to3", "ensurepip", "turtledemo", "pydoc_data"]


def request(url):
    req = urllib.request.Request(url, headers={"User-Agent": "edupython-build"})
    token = os.environ.get("GITHUB_TOKEN")
    if token and "api.github.com" in url:
        req.add_header("Authorization", f"Bearer {token}")
    return urllib.request.urlopen(req)


def find_asset(triple, minor):
    with request(RELEASES) as r:
        releases = json.load(r)
    pattern = re.compile(
        rf"cpython-3\.{minor}\.\d+\+\d+-{re.escape(triple)}-install_only(_stripped)?\.tar\.gz$"
    )
    for rel in releases:
        assets = [a for a in rel["assets"] if pattern.search(a["name"])]
        # prefer the stripped (smaller) variant
        assets.sort(key=lambda a: "_stripped" not in a["name"])
        if assets:
            return assets[0]["name"], assets[0]["browser_download_url"]
    raise SystemExit(f"No Python 3.{minor} build found for {triple}")


def main():
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    triple, out_zip = sys.argv[1], os.path.abspath(sys.argv[2])
    minor = sys.argv[3] if len(sys.argv) > 3 else "12"
    name, url = find_asset(triple, minor)
    print(f"Downloading {name}")

    work = tempfile.mkdtemp(prefix="edupython-py-")
    archive = os.path.join(work, name)
    with request(url) as r, open(archive, "wb") as f:
        shutil.copyfileobj(r, f)
    with tarfile.open(archive) as t:
        t.extractall(work)
    root = os.path.join(work, "python")

    windows = "windows" in triple
    exe = os.path.join(root, "python.exe") if windows else os.path.join(root, "bin", "python3")
    # Never take packages from the computer that builds this.
    os.environ["PYTHONNOUSERSITE"] = "1"
    for var in ("PYTHONPATH", "PYTHONHOME"):
        os.environ.pop(var, None)
    print("Installing", ", ".join(EXTRA_PACKAGES))
    subprocess.check_call([exe, "-m", "pip", "install", "--no-cache-dir", "--upgrade", *EXTRA_PACKAGES])
    subprocess.check_call([exe, "-c", "import pygame, sys; assert pygame.IS_CE; print('pygame-ce', pygame.version.ver, 'on', sys.version)"])

    lib = os.path.join(root, "Lib") if windows else None
    if not windows:
        libs = [d for d in os.listdir(os.path.join(root, "lib")) if d.startswith("python3")]
        lib = os.path.join(root, "lib", libs[0])
    for d in REMOVE:
        shutil.rmtree(os.path.join(lib, d), ignore_errors=True)
    for dirpath, dirnames, _ in os.walk(root):
        for d in list(dirnames):
            if d in ("__pycache__", "tests") and "site-packages" in dirpath:
                shutil.rmtree(os.path.join(dirpath, d), ignore_errors=True)
                dirnames.remove(d)

    # Nothing that is needed may have been removed above.
    check = "import numpy, matplotlib.pyplot, tkinter; print('numpy', numpy.__version__, 'matplotlib', matplotlib.__version__)"
    subprocess.check_call([exe, "-B", "-c", check], env={**os.environ, "MPLBACKEND": "Agg"})

    # Zip with permissions kept; symlinks are stored as plain copies.
    if os.path.exists(out_zip):
        os.remove(out_zip)
    with zipfile.ZipFile(out_zip, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for dirpath, _, files in os.walk(root):
            for fn in files:
                full = os.path.join(dirpath, fn)
                if not os.path.exists(full):
                    continue  # dangling symlink
                rel = os.path.relpath(full, root).replace(os.sep, "/")
                info = zipfile.ZipInfo.from_file(full, rel)
                info.compress_type = zipfile.ZIP_DEFLATED
                with open(full, "rb") as f:
                    z.writestr(info, f.read())
    print(f"Wrote {out_zip} ({os.path.getsize(out_zip) / 1e6:.1f} MB)")
    shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
