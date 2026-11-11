#!/usr/bin/env bash
# nfsh's half of the baseline (docs/BASELINE.md): builds nfsh for Windows from this
# checkout and flies the camera path (default bench/paths/speedway.path, zone 495)
# RUNS times (default 3) after one run that warms the disk cache, printing each run's
# time to the first frame and frame times, then their medians as a Markdown table row.
#
# MODE=headless (default) draws the path without a window, one frame at a time.
# MODE=window flies it in a 1280x720 window with vsync off, as a player sees it: that
# opens a window for about a minute a run, so it refuses while the PC is in use (input
# in the last NFSH_IDLE_SECONDS, default 120) unless NFSH_ALLOW_WINDOW=1, set once the
# owner has said a run may start now.
#
# Reads the store at NFSH_STORE (default D:\nfsh-store).
# Usage: [MODE=window] [RUNS=3] bench/baseline.sh [PATH_FILE [ZONE]]
set -euo pipefail
cd "$(dirname "$0")/.."
mode=${MODE:-headless}
runs=${RUNS:-3}
path_file=${1:-bench/paths/speedway.path}
zone=${2:-495}
store=${NFSH_STORE:-'D:\nfsh-store'}
case $mode in headless) flags=(--headless) ;; window) flags=() ;; *) echo "MODE is headless or window" >&2; exit 2 ;; esac

ps() { (cd /mnt/c && powershell.exe -NoProfile -NonInteractive -Command "$1" | tr -d '\r'); }

# A window takes over the screen: not while the owner is using the PC.
require_idle() {
	[ "${NFSH_ALLOW_WINDOW:-}" = 1 ] && return 0
	local need=${NFSH_IDLE_SECONDS:-120} idle
	idle=$(ps "Add-Type -Namespace W -Name U -MemberDefinition '[StructLayout(LayoutKind.Sequential)] public struct L { public uint cb; public uint t; } [DllImport(\"user32.dll\")] public static extern bool GetLastInputInfo(ref L l);'
\$l=New-Object W.U+L; \$l.cb=8; [void][W.U]::GetLastInputInfo([ref]\$l)
[int](([Environment]::TickCount - [int]\$l.t) / 1000)")
	if [ "$idle" -lt "$need" ]; then
		echo "The PC was used ${idle} s ago; not opening a window. Ask first, then rerun" >&2
		echo "with NFSH_ALLOW_WINDOW=1 (or wait until it has been idle for ${need} s)." >&2
		exit 3
	fi
}

work=$(wslpath "$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')")/.nfsh
scripts/cargo-msvc.sh build -q --release --locked -p nfsh
nfsh="$work/target-msvc/release/nfsh.exe"
# The path is copied next to the build: Windows reads it from its own drive.
mkdir -p "$work/bench"
cp "$path_file" "$work/bench/baseline.path"
path_win=$(wslpath -w "$work/bench/baseline.path")

one() { # -> "load_s mean_ms median_ms p99_ms low1_ms max_ms"
	[ "$mode" = window ] && require_idle
	local out
	out=$("$nfsh" view "$store" "$zone" --path "$path_win" "${flags[@]}" </dev/null 2>/dev/null) || {
		echo "baseline: nfsh failed" >&2
		return 1
	}
	echo "$out" | grep -E '^(load|path):' >&2
	echo "$out" | awk '
		/^load:/ { load = $2 }
		/^path:/ { for (i = 1; i <= NF; i++) { if ($i == "mean") mean = $(i+1); if ($i == "median") med = $(i+1);
			if ($i == "p99") p99 = $(i+1); if ($i == "low") low = $(i+1); if ($i == "max") max = $(i+1) } }
		END { print load, mean, med, p99, low, max }'
}

median() { sort -n | awk '{a[NR]=$1} END{print (NR%2 ? a[(NR+1)/2] : (a[NR/2]+a[NR/2+1])/2)}'; }

echo "warm-up run:" >&2
read -r first_load _ < <(one)
results=()
for i in $(seq "$runs"); do
	echo "run $i:" >&2
	results+=("$(one)")
done
col() { printf '%s\n' "${results[@]}" | awk -v c="$1" '{print $c}' | median; }
loads=$(printf '%s\n' "${results[@]}" | awk '{print $1}' | paste -sd' ')
echo "| $(basename "$path_file" .path) | $mode | $(col 2) | $(col 3) | $(col 4) | $(col 5) | $(col 6) | $(col 1) ($loads) | $first_load |"
