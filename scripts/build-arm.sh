#!/usr/bin/env bash
set -e

echo "=== AyanomiBancho ARM Multi-Arch Cross Compiler ==="

# Check if cross is installed
if ! command -v cross &> /dev/null; then
    echo "Installing 'cross' tool for multi-arch compilation..."
    cargo install cross --git https://github.com/cross-rs/cross
fi

echo "1. Building for ARMv8 / AArch64 (Raspberry Pi 4/5, Oracle Cloud ARM)..."
cross build --release --target aarch64-unknown-linux-gnu --bin ayanomibancho
echo "-> Output: target/aarch64-unknown-linux-gnu/release/ayanomibancho"

echo "2. Building for ARMv7 (Raspberry Pi 2/3, Orange Pi)..."
cross build --release --target armv7-unknown-linux-gnueabihf --bin ayanomibancho
echo "-> Output: target/armv7-unknown-linux-gnueabihf/release/ayanomibancho"

echo "=== All ARM builds completed successfully! ==="
