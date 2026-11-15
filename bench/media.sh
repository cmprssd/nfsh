#!/usr/bin/env bash
# Renders the progress page's gallery (bench/gallery.txt) and fly-throughs
# (bench/films.txt) with nfsh built from this checkout, headless on the Windows GPU:
# no window opens. Run it from a release's tag after bench/history.sh; the page shows
# the newest release that has media. Shots already rendered are skipped (FORCE=1
# renders again). A film's frames (`nfsh view --path --headless --frames`, about 2 GB
# for 40 s) are encoded to H.264 by imageio-ffmpeg's static ffmpeg, then deleted.
# Output: %USERPROFILE%\.nfsh\media\<tag>\<name>.png, .mp4 and -poster.png (game imagery: never
# committed). Reads the store at NFSH_STORE (default D:\nfsh-store).
# Usage: bench/media.sh   (DEV=1 renders an untagged checkout into media/dev)
set -euo pipefail
cd "$(dirname "$0")/.."
store=${NFSH_STORE:-'D:\nfsh-store'}
tag=$(git diff --quiet HEAD && git describe --tags --exact-match HEAD 2>/dev/null) || {
	[ "${DEV:-}" = 1 ] || { echo "media.sh renders a release: check out its tag (or DEV=1)" >&2; exit 2; }
	tag=dev
}
work=$(wslpath "$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')")/.nfsh
out="$work/media/$tag"
mkdir -p "$out" "$work/bench"
scripts/cargo-msvc.sh build -q --release --locked -p nfsh
nfsh="$work/target-msvc/release/nfsh.exe"
fresh() { [ "${FORCE:-}" = 1 ] || [ ! -s "$1" ]; }

grep -v '^#' bench/gallery.txt | awk 'NF' | while read -r name zone camera time _; do
	fresh "$out/$name.png" || continue
	args=(); [ "$camera" = - ] || args=(--camera "$camera")
	[ "$time" = tealnight ] && args+=(--night)
	echo "== $name"
	timeout 1200 "$nfsh" view "$store" "$zone" --shot "$(wslpath -w "$out")\\$name.png" "${args[@]}" \
		</dev/null >/dev/null 2>&1 || echo "   $name: not drawn"
done

ffmpeg=$(uv run -q --with imageio-ffmpeg python -c 'import imageio_ffmpeg; print(imageio_ffmpeg.get_ffmpeg_exe())')
grep -v '^#' bench/films.txt | awk 'NF' | while read -r name zone path time _; do
	fresh "$out/$name.mp4" || continue
	frames="$work/media/frames-$name"
	rm -rf "$frames"
	cp "bench/paths/$path" "$work/bench/film.path"
	night=(); [ "$time" = tealnight ] && night=(--night)
	echo "== $name"
	timeout 3600 "$nfsh" view "$store" "$zone" --path "$(wslpath -w "$work/bench/film.path")" --headless \
		--frames "$(wslpath -w "$frames")" "${night[@]}" </dev/null >/dev/null 2>&1 || { echo "   $name: not drawn"; continue; }
	"$ffmpeg" -loglevel error -y -framerate 30 -i "$frames/%05d.png" -c:v libx264 -preset slow -crf 22 \
		-pix_fmt yuv420p -movflags +faststart "$out/$name.mp4"
	cp "$frames/00030.png" "$out/$name-poster.png"  # one second in: the video's still
	rm -rf "$frames"
done
ls -la "$out"
