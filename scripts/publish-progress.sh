#!/usr/bin/env bash
# Builds the progress page and publishes it to GitHub Pages: the page is
# scripts/progress_page.html plus scripts/progress.py's numbers (data.json). The
# gh-pages branch holds the page alone, as one commit replaced on every publish, so main
# stays free of generated files and the repository doesn't grow with each refresh. Run
# it from the updated main after each merge.
# Usage: scripts/publish-progress.sh
set -euo pipefail
cd "$(dirname "$0")/.."
git fetch -q origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] ||
	{ echo "publish from main as merged (HEAD is not origin/main)" >&2; exit 1; }
site=$(mktemp -d)
trap 'rm -rf "$site"' EXIT
python3 scripts/progress.py --json "$site/data.json"
cp scripts/progress_page.html "$site/index.html"
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
