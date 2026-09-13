#!/data/data/com.termux/files/usr/bin/bash
# Keep CPU & network awake
termux-wake-lock
ulimit -n 4096

while true; do
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Starting AyanomiBancho Server..."
    proot-distro login ubuntu -- bash -c "cd /root/osu && ulimit -n 4096 && ./ayanomibancho"
    EXIT_CODE=$?
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] AyanomiBancho exited with code $EXIT_CODE. Restarting in 2s..."
    sleep 2
done
