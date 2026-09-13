#!/data/data/com.termux/files/usr/bin/bash
termux-wake-lock
ulimit -n 4096

# 1. Kill old loose processes
pkill -9 -f ayanomibancho 2>/dev/null
pkill -9 -f cloudflared 2>/dev/null
sleep 2

# 2. Kill old conflicting tmux sessions
tmux kill-session -t server 2>/dev/null || true
tmux kill-session -t cf 2>/dev/null || true
tmux kill-session -t f2b 2>/dev/null || true
tmux kill-session -t ayanomi 2>/dev/null || true
tmux kill-session -t cloudflared-0 2>/dev/null || true
tmux kill-session -t tunnel 2>/dev/null || true

# 3. Ensure Nginx is running
nginx -s reload 2>/dev/null || nginx

# 4. Launch tmux sessions in background with auto-recovery wrappers
tmux new-session -d -s server /data/data/com.termux/files/home/run_server.sh
tmux new-session -d -s cf /data/data/com.termux/files/home/run_cf.sh
tmux new-session -d -s f2b /data/data/com.termux/files/home/run_fail2ban.sh

sleep 5

echo "=== TMUX SESSIONS ==="
tmux ls
echo "=== SERVER PANE ==="
tmux capture-pane -pt server
echo "=== CF PANE ==="
tmux capture-pane -pt cf
echo "=== FAIL2BAN PANE ==="
tmux capture-pane -pt f2b
