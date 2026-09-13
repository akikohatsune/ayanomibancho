#!/data/data/com.termux/files/usr/bin/bash
# Prevent Android from sleeping CPU and Wi-Fi radio
termux-wake-lock
ulimit -n 4096

# Read token from environment variable or local config file (~/.cloudflared/token)
TOKEN="${CLOUDFLARE_TUNNEL_TOKEN:-}"
if [ -z "$TOKEN" ] && [ -f "$HOME/.cloudflared/token" ]; then
    TOKEN=$(cat "$HOME/.cloudflared/token" | tr -d '\r\n')
fi

if [ -z "$TOKEN" ]; then
    echo "[ERROR] Cloudflare tunnel token not found! Please set CLOUDFLARE_TUNNEL_TOKEN or create ~/.cloudflared/token"
    exit 1
fi

while true; do
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Starting cloudflared tunnel (protocol: auto / QUIC)..."
    cloudflared --edge-ip-version 4 tunnel run \
        --protocol auto \
        --dns-resolver-addrs 1.1.1.1:53 \
        --proxy-connect-timeout 15s \
        --proxy-tcp-keepalive 30s \
        --proxy-keepalive-timeout 15s \
        --token "$TOKEN"
    EXIT_CODE=$?
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] cloudflared exited with code $EXIT_CODE. Restarting in 2s..."
    sleep 2
done
