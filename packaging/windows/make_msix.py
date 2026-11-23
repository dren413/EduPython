"""Build the Microsoft Store package (MSIX) from edupython.exe.

    python packaging/windows/make_msix.py <edupython.exe> <out.msix> <version>

Runs on Windows with the Windows SDK (present on GitHub's windows runners).
The package is not signed: the Store signs it when it is published.
The identity comes from Partner Center (Apps and games > EduPython >
Product identity); the environment variables override the defaults below.
"""
import glob
import os
import shutil
import subprocess
import sys
from pathlib import Path

IDENTITY = {
    "identity_name": os.environ.get("MSIX_IDENTITY_NAME") or "DrenGashi.EduPython",
    "publisher": os.environ.get("MSIX_PUBLISHER") or "CN=F72C4EEF-416D-4EBA-A377-86A65C9CFAE2",
    "publisher_display_name": os.environ.get("MSIX_PUBLISHER_DISPLAY_NAME") or "Dren Gashi",
}


def sdk_tool(name: str) -> str:
    found = sorted(glob.glob(rf"C:\Program Files (x86)\Windows Kits\10\bin\10.*\x64\{name}"))
    if not found:
        sys.exit(f"{name} not found: install the Windows SDK")
    return found[-1]


def store_version(v: str) -> str:
    # The Store wants four numbers and the last one must be 0: 2.0.4 -> 2.0.4.0
    nums = [int("".join(c for c in p if c.isdigit()) or 0) for p in v.lstrip("v").split(".")[:3]]
    nums += [0] * (3 - len(nums))
    return ".".join(map(str, nums + [0]))


def main() -> None:
    exe, out, version = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    here = Path(__file__).parent
    stage = out.parent / "msix-stage"
    shutil.rmtree(stage, ignore_errors=True)
    shutil.copytree(here / "Assets", stage / "Assets")
    shutil.copy2(exe, stage / "edupython.exe")
    manifest = (here / "AppxManifest.xml").read_text(encoding="utf-8")
    for key, value in {**IDENTITY, "version": store_version(version)}.items():
        manifest = manifest.replace("{" + key + "}", value)
    (stage / "AppxManifest.xml").write_text(manifest, encoding="utf-8")

    makepri = sdk_tool("makepri.exe")
    pri_config = out.parent / "priconfig.xml"
    subprocess.run([makepri, "createconfig", "/cf", str(pri_config), "/dq", "en-US", "/pv", "10.0.0", "/o"], check=True)
    subprocess.run([makepri, "new", "/pr", str(stage), "/cf", str(pri_config), "/of", str(stage / "resources.pri"), "/o"], check=True)
    subprocess.run([sdk_tool("makeappx.exe"), "pack", "/d", str(stage), "/p", str(out), "/o"], check=True)
    print(f"{out}: version {store_version(version)}")


if __name__ == "__main__":
    main()
