"""Builds the progress page: the numbers of scripts/progress.py, the milestones of
docs/OVERVIEW.md, and the history views as each release drew them (bench/history.sh).
Game imagery stays off main: the page is built into a folder outside the repository
and published from there to the gh-pages branch.

    uv run --with numpy --with pillow python scripts/progress_page.py <out> --work <dir>

scripts/publish-progress.sh runs it and publishes the result. <dir> is
%USERPROFILE%\\.nfsh as a WSL path; <dir>/history/<tag>/<view>.png holds the renders.
Images already converted in <out> are reused.
"""
import argparse
import json
import pathlib
import re
import shutil
import subprocess
import sys

import numpy as np
from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIRST = (0, 10, 0)  # the first release with --camera, so the first history.sh renders


def git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True, check=True).stdout.strip()


def version(tag):
    return tuple(int(n) for n in tag.lstrip("v").split("."))


def road():
    """The milestones table of docs/OVERVIEW.md ("Where it is going"), and each
    release's short label from its State cells ("textured and lit (0.10.0)")."""
    text = (ROOT / "docs/OVERVIEW.md").read_text()
    table = text.split("## Where it is going", 1)[1].split("\n## ", 1)[0]
    rows, labels = [], {}
    for line in table.splitlines():
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) == 3 and cells[0].isdigit():
            rows.append({"n": int(cells[0]), "what": cells[1], "state": cells[2]})
            for label, v in re.findall(r"(?:^|[;:] )([^;:()]+?) \((\d+\.\d+\.\d+)\)", cells[2]):
                labels[v] = label
    return rows, labels


def views():
    out = []
    for line in (ROOT / "bench/history-views.txt").read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            name, camera, time, what = line.split(None, 3)
            out.append({"name": name, "camera": camera, "time": time, "what": what})
    return out


def load(path):
    return np.asarray(Image.open(path).convert("RGB")).astype(np.int16)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=pathlib.Path)
    ap.add_argument("--work", type=pathlib.Path, required=True)
    a = ap.parse_args()
    out = a.out
    out.mkdir(parents=True, exist_ok=True)

    stats = json.loads(subprocess.run([sys.executable, str(ROOT / "scripts/progress.py"), "--json", "/dev/stdout"],
                                      capture_output=True, text=True, check=True).stdout)
    rows, labels = road()
    vs = views()

    # Each release's renders, and how much of each view changed since the release
    # before it: the share of pixels off by more than 8 in any channel.
    releases, previous = [], {}
    tags = [t for t in git("tag", "-l", "v*").split() if version(t) >= FIRST]
    for tag in sorted(tags, key=version):
        shots = {}
        for v in vs:
            png = a.work / "history" / tag / f"{v['name']}.png"
            if not png.exists():
                continue
            dest = out / "img/h" / tag / f"{v['name']}.webp"
            if not dest.exists():
                dest.parent.mkdir(parents=True, exist_ok=True)
                Image.open(png).convert("RGB").save(dest, quality=84)
            now = load(png)
            changed = None
            if v["name"] in previous and previous[v["name"]].shape == now.shape:
                changed = round(float((np.abs(now - previous[v["name"]]).max(axis=2) > 8).mean() * 100), 1)
            previous[v["name"]] = now
            shots[v["name"]] = {"changed": changed}
        v = tag.lstrip("v")
        releases.append({"tag": tag, "label": labels.get(v, git("log", "-1", "--format=%s", tag)),
                         "date": git("log", "-1", "--format=%ad", "--date=short", tag), "shots": shots})

    data = {**stats, "road": rows, "views": vs, "releases": releases}
    (out / "data.json").write_text(json.dumps(data, separators=(",", ":")))
    shutil.copy2(ROOT / "scripts/progress_page.html", out / "index.html")
    size = sum(p.stat().st_size for p in out.rglob("*") if p.is_file())
    print(f"{out}: {len(releases)} releases, {sum(1 for _ in out.rglob('*.webp'))} images, {size / 1e6:.1f} MB")


main()
