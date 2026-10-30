#!/usr/bin/env bash
# Run the Windows-host cargo from WSL with the MSVC toolchain (Visual Studio Build
# Tools must be installed), so builds and Vulkan tests run on the real Windows GPU.
# Build output goes to %USERPROFILE%\.nfsh\target-msvc, off the slow WSL share.
# Usage: scripts/cargo-msvc.sh build --release | test | clippy ...
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WIN_HOME="$(wslpath "$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')")"
export CARGO_TARGET_DIR="$(wslpath -w "$WIN_HOME")\\.nfsh\\target-msvc"
export RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
export WSLENV="${WSLENV:-}:CARGO_TARGET_DIR:RUSTUP_TOOLCHAIN"
cd "$ROOT"
exec "$WIN_HOME/.cargo/bin/cargo.exe" "$@"
