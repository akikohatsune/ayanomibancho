#!/usr/bin/env bash
# AyanomiBancho Linux / WSL Startup Script

set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"

echo "=========================================================="
echo "    Starting AyanomiBancho on Linux / WSL...              "
echo "=========================================================="

# Check if Docker is preferred
if [ "$1" == "--docker" ]; then
    echo "[INFO] Starting with Docker Compose..."
    docker compose up -d
    echo "[SUCCESS] AyanomiBancho is running in Docker!"
    exit 0
fi

# Check if cargo is available
if command -v cargo &> /dev/null; then
    echo "[INFO] Found cargo. Starting server in release mode..."
    mkdir -p data data/avatars
    nohup cargo run --release > server.log 2>&1 &
    echo $! > server.pid
    echo "[SUCCESS] AyanomiBancho started with PID $(cat server.pid)!"
    echo "[INFO] Logs available at: server.log"
    echo "[INFO] Web Dashboard: http://localhost:5000"
    exit 0
elif [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
    mkdir -p data data/avatars
    nohup cargo run --release > server.log 2>&1 &
    echo $! > server.pid
    echo "[SUCCESS] AyanomiBancho started with PID $(cat server.pid)!"
    exit 0
fi

# If prebuilt binary exists
if [ -f "./target/release/ayanomibancho" ]; then
    mkdir -p data data/avatars
    nohup ./target/release/ayanomibancho > server.log 2>&1 &
    echo $! > server.pid
    echo "[SUCCESS] AyanomiBancho started from prebuilt binary (PID: $(cat server.pid))!"
    exit 0
fi

echo "[ERROR] Could not find 'cargo' or Docker."
echo "Install Rust with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
echo "Or run with Docker: ./start.sh --docker"
exit 1
