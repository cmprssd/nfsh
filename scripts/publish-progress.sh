#!/usr/bin/env bash
# Builds the progress page (scripts/progress_page.py) and publishes it to GitHub Pages:
# the gh-pages branch holds the page alone, as one commit replaced on every publish, so
# main stays free of the game's imagery and the repository doesn't grow with each
# refresh. Run it from the updated main after each merge, once bench/history.sh has
# rendered any new release.
# Usage: scripts/publish-progress.sh
set -euo pipefail
cd "$(dirname "$0")/.."
git fetch -q origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] ||
	{ echo "publish from main as merged (HEAD is not origin/main)" >&2; exit 1; }
work=$(wslpath "$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')")/.nfsh
out="$work/progress-page"  # kept between runs: images already converted are reused
rm -f "$out/data.json" "$out/index.html"
uv run -q --with numpy --with pillow python scripts/progress_page.py "$out" --work "$work"
site=$(mktemp -d)
trap 'rm -rf "$site"' EXIT
cp -r "$out"/. "$site"
touch "$site/.nojekyll"
remote=$(git remote get-url origin)
source=$(git rev-parse --short HEAD)
(
	cd "$site"
	git init -q -b gh-pages
	git add -A
	git commit -q -m "The progress page, built from main at $source"
	git push -q -f "$remote" gh-pages
)
echo "published: https://cmprssd.github.io/nfsh/ (built from $source)"
