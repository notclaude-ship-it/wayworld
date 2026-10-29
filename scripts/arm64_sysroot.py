#!/usr/bin/env python3
"""Prépare un sysroot arm64 local (alsa, udev, xkbcommon) pour cross-compiler Wayworld.

Télécharge les .deb arm64 depuis ports.ubuntu.com et les extrait dans un dossier :
rien n'est installé sur le système, apt n'est pas modifié.

Usage : arm64_sysroot.py [dossier]   (défaut : target/arm64-sysroot)
"""
import gzip
import os
import subprocess
import sys
import urllib.request

BASE = "http://ports.ubuntu.com/ubuntu-ports"
RELEASE = os.environ.get("UBUNTU_RELEASE", "noble")
SUITES = [f"{RELEASE}-updates", f"{RELEASE}-security", RELEASE]
WANT = [
    "libasound2-dev", "libasound2t64",
    "libudev-dev", "libudev1",
    "libxkbcommon-dev", "libxkbcommon0",
    "libwayland-dev", "libwayland-client0", "libwayland-server0",
    "libwayland-cursor0", "libwayland-egl1",
    "libffi-dev", "libffi8",
]


def packages(suite):
    url = f"{BASE}/dists/{suite}/main/binary-arm64/Packages.gz"
    with urllib.request.urlopen(url) as r:
        text = gzip.decompress(r.read()).decode()
    out = {}
    for block in text.split("\n\n"):
        fields = {}
        for line in block.splitlines():
            if ": " in line and not line.startswith(" "):
                k, v = line.split(": ", 1)
                fields[k] = v
        if "Package" in fields and "Filename" in fields:
            out[fields["Package"]] = fields["Filename"]
    return out


def main():
    root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/arm64-sysroot")
    stamp = os.path.join(root, ".ok")
    if os.path.exists(stamp):
        return
    os.makedirs(root, exist_ok=True)
    index = {}
    for suite in reversed(SUITES):  # le plus prioritaire écrase les autres
        index.update(packages(suite))
    for name in WANT:
        if name not in index:
            sys.exit(f"paquet arm64 introuvable : {name}")
        deb = os.path.join(root, os.path.basename(index[name]))
        if not os.path.exists(deb):
            print(f"[sysroot] {name}")
            urllib.request.urlretrieve(f"{BASE}/{index[name]}", deb)
        subprocess.run(["dpkg", "-x", deb, root], check=True)
    open(stamp, "w").close()


main()