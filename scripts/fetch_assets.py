#!/usr/bin/env python3
"""Télécharge les assets CC0 de Poly Haven (textures PBR + modèles glTF) dans assets/.

Usage: python3 scripts/fetch_assets.py [resolution]   (défaut: 1k)
Tout Poly Haven est sous licence CC0 : https://polyhaven.com/license
"""
import json, os, sys, urllib.request

RES = sys.argv[1] if len(sys.argv) > 1 else "1k"
ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets")

TEXTURES = [
    "concrete_floor_worn_001",   # sol
    "concrete_wall_007",         # murs
    "rusty_metal_02",            # métal rouillé
    "metal_plate",               # plafond / panneaux
    "corrugated_iron_02",        # tôle
]
MODELS = [
    "steel_frame_shelves_01",
    "plastic_crate_01",
    "wooden_crate_01",
    "metal_office_desk",
    "hanging_industrial_lamp",
    "barrel_03",
    "old_military_crate",
    "metal_toolbox",
    "power_box_01",
    "industrial_wall_lamp",
    "metal_trash_can",
    "portable_generator",
]


def get_json(url):
    req = urllib.request.Request(url, headers={"User-Agent": "wayworld/0.1"})
    return json.load(urllib.request.urlopen(req, timeout=30))


def download(url, dest):
    if os.path.exists(dest) and os.path.getsize(dest) > 0:
        return
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    req = urllib.request.Request(url, headers={"User-Agent": "wayworld/0.1"})
    with urllib.request.urlopen(req, timeout=120) as r, open(dest, "wb") as f:
        f.write(r.read())
    print("  +", os.path.relpath(dest, ROOT))


def pick(files, keys, res):
    for k in keys:
        if k in files:
            m = files[k]
            r = res if res in m else sorted(m.keys())[0]
            fmt = "jpg" if "jpg" in m[r] else list(m[r].keys())[0]
            return m[r][fmt]["url"]
    return None


def fetch_texture(name):
    print("texture", name)
    files = get_json(f"https://api.polyhaven.com/files/{name}")
    wanted = {
        "albedo": ["Diffuse"],
        "normal": ["nor_gl"],
        "rough": ["Rough"],
        "ao": ["AO"],
    }
    for tag, keys in wanted.items():
        url = pick(files, keys, RES)
        if url:
            ext = url.rsplit(".", 1)[-1]
            download(url, os.path.join(ROOT, "textures", name, f"{tag}.{ext}"))


def fetch_model(name):
    print("model", name)
    files = get_json(f"https://api.polyhaven.com/files/{name}")
    g = files["gltf"]
    r = RES if RES in g else sorted(g.keys())[0]
    entry = g[r]["gltf"]
    base = os.path.join(ROOT, "models", name)
    download(entry["url"], os.path.join(base, os.path.basename(entry["url"])))
    for rel, inc in entry.get("include", {}).items():
        download(inc["url"], os.path.join(base, rel))


if __name__ == "__main__":
    for t in TEXTURES:
        fetch_texture(t)
    for m in MODELS:
        try:
            fetch_model(m)
        except Exception as e:
            print("  ! échec", m, e)
    with open(os.path.join(ROOT, "CREDITS.md"), "w") as f:
        f.write("# Crédits des assets\n\nTous les assets viennent de Poly Haven (https://polyhaven.com), licence CC0.\n\n")
        f.write("## Textures\n" + "".join(f"- {t}\n" for t in TEXTURES))
        f.write("\n## Modèles\n" + "".join(f"- {m}\n" for m in MODELS))
    print("OK")