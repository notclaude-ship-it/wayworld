#!/bin/sh
# Cross-compile Wayworld pour arm64 (aarch64-unknown-linux-gnu) depuis une machine x86_64.
# Prérequis : gcc-aarch64-linux-gnu, rustup, python3. Les libs arm64 (alsa, udev, xkbcommon)
# sont récupérées dans un sysroot local, sans toucher à apt.
# Usage : scripts/build-arm64.sh [options cargo]   ->  target/aarch64-unknown-linux-gnu/release/wayworld
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
TARGET=aarch64-unknown-linux-gnu
SYSROOT="${ARM64_SYSROOT:-$PWD/target/arm64-sysroot}"

command -v aarch64-linux-gnu-gcc >/dev/null || { echo "Installe d'abord : sudo apt install gcc-aarch64-linux-gnu"; exit 1; }
rustup target add "$TARGET" >/dev/null 2>&1 || true
python3 scripts/arm64_sysroot.py "$SYSROOT"

LIBDIR="$SYSROOT/usr/lib/aarch64-linux-gnu"
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_SYSROOT_DIR="$SYSROOT"
export PKG_CONFIG_LIBDIR="$LIBDIR/pkgconfig:$SYSROOT/usr/share/pkgconfig"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export RUSTFLAGS="$RUSTFLAGS -C link-arg=-Wl,-rpath-link,$LIBDIR"

cargo build --release --target "$TARGET" "$@"
echo "OK : target/$TARGET/release/wayworld"