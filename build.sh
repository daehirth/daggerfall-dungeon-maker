#!/usr/bin/env bash
# Builds Daggerfall Dungeon Maker as a release executable on Linux.
#
# Requirements:
#   * Rust (https://rustup.rs).
#   * A C compiler, pkg-config and the GTK 3, X11/Wayland and Vulkan development
#     packages. On Debian or Ubuntu:
#       sudo apt install build-essential pkg-config libgtk-3-dev libxkbcommon-dev \
#         libwayland-dev libx11-dev mesa-vulkan-drivers
#     On Fedora:
#       sudo dnf install gcc pkgconf-pkg-config gtk3-devel libxkbcommon-devel \
#         wayland-devel libX11-devel mesa-vulkan-drivers
#
# The executable is written to target/release/daggerfall-dungeon-maker.
set -euo pipefail
cd "$(dirname "$0")"

# Shells opened before Rust was installed may not have it on the PATH yet.
if ! command -v cargo >/dev/null && [ -x "$HOME/.cargo/bin/cargo" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

cargo build --release
echo "Built $(pwd)/target/release/daggerfall-dungeon-maker"
