#!/usr/bin/env bash
# Keeps this project's build output from piling up.
#
#   scripts/disk.sh            show what the project stores and where
#   scripts/disk.sh prune      shrink each Rust build folder that is over its budget:
#                              first drop incremental-compilation caches (the part
#                              that grows without limit), then `cargo clean` if it
#                              is still over. Run by .local-ci after every check.
#   scripts/disk.sh clean-all  delete every build folder and cache listed below
#
# Budget per build folder: NFSH_TARGET_BUDGET_MB (default 3072).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
budget_mb=${NFSH_TARGET_BUDGET_MB:-3072}
win_home=$(wslpath "$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')")
work="$win_home/.nfsh"

# name|path|what it is
places=(
	"Linux build|$ROOT/target|cargo output (WSL)"
	"Windows build|$work/target-msvc|cargo output (MSVC)"
	"local-ci logs|$HOME/.cache/local-ci/cmprssd/nfsh|one log per checked commit"
)

size_mb() { if [ -e "$1" ]; then du -sm "$1" | cut -f1; else echo 0; fi; }

status() {
	local total=0 entry name path what mb
	for entry in "${places[@]}"; do
		IFS='|' read -r name path what <<<"$entry"
		mb=$(size_mb "$path")
		total=$((total + mb))
		printf '%6s MB  %-17s %s\n' "$mb" "$name" "$path"
	done
	printf '%6s MB  total\n' "$total"
}

prune_target() { # name dir cargo-cmd...
	local name=$1 dir=$2; shift 2
	local mb
	mb=$(size_mb "$dir")
	[ "$mb" -le "$budget_mb" ] && return 0
	echo "$name: ${mb} MB is over the ${budget_mb} MB budget; dropping incremental caches"
	rm -rf "$dir"/*/incremental "$dir"/*/*/incremental
	mb=$(size_mb "$dir")
	if [ "$mb" -gt "$budget_mb" ]; then
		echo "$name: still ${mb} MB; cargo clean"
		"$@" clean
	fi
	echo "$name: now $(size_mb "$dir") MB"
}

case "${1:-status}" in
	status) status ;;
	prune)
		prune_target "Linux build" "$ROOT/target" cargo
		prune_target "Windows build" "$work/target-msvc" "$ROOT/scripts/cargo-msvc.sh"
		;;
	clean-all)
		(cd "$ROOT" && cargo clean)
		"$ROOT/scripts/cargo-msvc.sh" clean
		rm -rf "$HOME/.cache/local-ci/cmprssd/nfsh"
		status
		;;
	*) echo "usage: scripts/disk.sh [status | prune | clean-all]" >&2; exit 2 ;;
esac
