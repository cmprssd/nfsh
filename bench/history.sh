#!/usr/bin/env bash
# Renders the history views (bench/history-views.txt) with nfsh as each release since
# 0.10.0 (the first with --camera) built it, for the progress page
# (scripts/progress_page.py). Each tag is checked out in its own worktree and built
# with the Windows toolchain in a target folder of its own, and every shot is drawn on
# the Windows GPU, headless: no window opens. Shots already rendered are skipped, so a
# new release costs one build; the build folder and worktree are removed at the end.
# PNGs go to %USERPROFILE%\.nfsh\history\<tag>\<view>.png (game imagery: never
# committed). Reads the store at NFSH_STORE (default D:\nfsh-store).
# Usage: bench/history.sh
set -uo pipefail
cd "$(dirname "$0")"
repo=$(git rev-parse --show-toplevel)
store=${NFSH_STORE:-'D:\nfsh-store'}
work_win="$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')\\.nfsh"
work=$(wslpath "$work_win")
wt=${HOME}/.cache/claude-worktrees/nfsh-history
export CARGO_TARGET_DIR="$work_win\\target-history"
export RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
export WSLENV="${WSLENV:-}:CARGO_TARGET_DIR:RUSTUP_TOOLCHAIN"
cargo="$(dirname "$work")/.cargo/bin/cargo.exe"
nfsh="$work/target-history/release/nfsh.exe"
views=$(grep -v '^#' history-views.txt | awk 'NF')

git -C "$repo" fetch -q --tags origin
[ -d "$wt" ] || git -C "$repo" worktree add -q --detach "$wt" HEAD
for tag in $(git -C "$repo" tag -l 'v*' --sort=v:refname); do
	minor=$(echo "$tag" | cut -d. -f2)
	[ "${tag%%.*}" = v0 ] && [ "$minor" -lt 10 ] && continue
	out="$work/history/$tag"
	missing=$(echo "$views" | while read -r name _; do [ -s "$out/$name.png" ] || echo "$name"; done)
	[ -n "$missing" ] || continue
	mkdir -p "$out"
	echo "== $tag"
	git -C "$wt" checkout -q --detach "$tag"
	(cd "$wt" && "$cargo" build -q --release --locked -p nfsh </dev/null) || { echo "   build failed"; continue; }
	echo "$views" | while read -r name camera time _; do
		echo "$missing" | grep -qx "$name" || continue
		night=(); [ "$time" = tealnight ] && night=(--night)
		timeout 1200 "$nfsh" view "$store" 495 --shot "$(wslpath -w "$out")\\$name.png" --camera "$camera" "${night[@]}" \
			</dev/null >/dev/null 2>&1 || echo "   $name: not drawn by this build"
	done
done
# The builds are only needed once; the renders stay.
rm -rf "$work/target-history"
git -C "$repo" worktree remove --force "$wt"
