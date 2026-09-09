#!/usr/bin/env bash
# AyanomiBancho Linux / WSL Stop Script

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"

echo "Stopping AyanomiBancho..."

if [ -f server.pid ]; then
    PID=$(cat server.pid)
    if kill -0 "$PID" 2>/dev/null; then
        kill "$PID"
        echo "[SUCCESS] Stopped AyanomiBancho (PID $PID)."
    else
        echo "[INFO] Process $PID is not running."
    fi
    rm -f server.pid
fi

# Also check for docker compose
if command -v docker &> /dev/null && docker compose ps --services 2>/dev/null | grep -q "ayanomibancho"; then
    docker compose down
    echo "[SUCCESS] Stopped Docker container."
fi

# Kill any stray processes
pkill -f ayanomibancho || true
echo "Done."
