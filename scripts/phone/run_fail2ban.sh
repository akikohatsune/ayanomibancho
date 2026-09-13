#!/data/data/com.termux/files/usr/bin/bash
while true; do
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Starting Fail2ban..."
    proot-distro login ubuntu -- bash -c 'rm -f /var/run/fail2ban/fail2ban.sock 2>/dev/null && fail2ban-server -f -x -v'
    EXIT_CODE=$?
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Fail2ban exited with code $EXIT_CODE. Restarting in 2s..."
    sleep 2
done
