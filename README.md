
# AyanomiBancho - Very low-cost osu! Private Server

**AyanomiBancho** is a high-performance, low-cost, self-contained osu! private server built entirely in **Rust**, using a **Service Separation & Fault Isolation** architecture designed to prevent cascading failures and full-system crashes.

The server fully supports both **Windows** and **Linux / WSL**, storing all data in lightweight **SQLite** databases (`data/ayanomi.db`, etc.) without requiring MySQL or Redis.

> [!IMPORTANT]
> ### Architecture Notice: Frontend & Multiplayer Service Separation
> To achieve optimal performance, modular maintainability, and total fault isolation, **AyanomiBancho** has separated its components into dedicated microservices across standalone repositories:
> - **Core Bancho & Gateway** *(This repository)*: Acts as the reverse-proxy Gateway (`:5000`) and the high-performance Bancho osu! gameplay packet engine (`:5001`).
>   - GitHub: https://github.com/akikohatsune/ayanomibancho
>   - GitLab: https://gitlab.com/luminehq/ayanomibancho
> - **Frontend Web Dashboard**: Decoupled into `ayanomi_frontend` (`:5002`). Handles the modern web UI, user profiles, custom markdown bios, badges, leaderboards, avatars, banners, and frontend API.
>   - GitHub: https://github.com/akikohatsune/ayanomibancho_frontend
>   - GitLab: https://gitlab.com/luminehq/ayanomibancho_frontend
> - **Multiplayer Microservice**: Decoupled into `roseflower` (`:5003`). Handles real-time osu! multiplayer room tracking, round completions, and match history scores.
>   - GitHub: https://github.com/akikohatsune/roseflower
>   - GitLab: https://gitlab.com/luminehq/roseflower

### Method 1: Run Directly with Cargo (Windows / Linux / WSL)

From the `ayanomibancho!` directory, run:

Generate a fresh session secret for each deployment and keep it outside the repository:

```powershell
# PowerShell
$secretBytes = New-Object byte[] 32
[Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($secretBytes)
$env:AYANOMI_SESSION_SECRET = [Convert]::ToBase64String($secretBytes)
```

```bash
# Linux / WSL
export AYANOMI_SESSION_SECRET="$(openssl rand -base64 32)"
```

Then start the core server:

```bash
cargo run
```

*This command starts the Gateway (port 5000) and the isolated Bancho engine (port 5001). To run the full suite, also run `ayanomi_frontend` (port 5002) and `roseflower` (port 5003).*

### Method 2: Run on Linux / WSL Using Shell Scripts

```bash
export AYANOMI_SESSION_SECRET="$(openssl rand -base64 32)"

# Grant execute permissions and start the server

chmod +x start.sh stop.sh

./start.sh

# View server logs

tail -f server.log

# Stop the server

./stop.sh
```

### Method 3: Run with Docker / Docker Compose (Recommended for WSL/Linux)

```bash
export AYANOMI_SESSION_SECRET="$(openssl rand -base64 32)"
docker compose up -d
```

---

## Connecting from the osu! Client

Open Command Prompt / PowerShell on Windows, or create a shortcut to `osu!.exe` with the following parameter:

```cmd
osu!.exe -devserver 127.0.0.1:5000
```

*(If the server is hosted on WSL2, you can still use `127.0.0.1:5000` because WSL2 automatically maps localhost to the Windows host. For a VPS, keep port 5000 bound to localhost and publish the configured domain through a TLS reverse proxy such as Caddy or nginx. Do not expose the plaintext backend port directly.)*

Docker Compose also publishes port 5000 on `127.0.0.1` only. A production reverse proxy should terminate HTTPS and forward to `127.0.0.1:5000`.

### ARM Support 

Does it support ARM architecture?
> **Yes**. Because this server is literally running on an old Android phone plugged in 24/7, resting on a damp paper towel. Don't ask about uptime—just pray the battery doesn't turn into a spicy pillow :P

> My old Android phone using ARMv7l, so you can try :D

### Account Registration

1. **Auto-Register**: Enter any Username and Password on the osu! login screen and the account will be created immediately.
2. **Web Dashboard**: Visit `http://localhost:5000` in your browser to select your country flag and view the Server Status dashboard.

---

## Configuration (`config.toml`)

```toml
[server]

host = "127.0.0.1" # Keep the backend private; expose it through a TLS reverse proxy.

port = 5000 # Gateway port (osu! client connects here)

bancho_port = 5001 # Internal Bancho service port

web_port = 5002 # Internal Web & API service port (proxied to ayanomi_frontend)

roseflower_port = 5003 # Internal Multiplayer service port (proxied to roseflower)

domain = "127.0.0.1:5000"

name = "AyanomiBancho"

welcome_message = "Welcome to AyanomiBancho! Enjoy your stay."

secret_key = "" # Prefer AYANOMI_SESSION_SECRET; startup fails if neither value has 32+ bytes.


[gameplay]

auto_register = true

default_country = 235 # 235: Vietnam, 224: US, 111: Japan, ...

bot_name = "AyanomiBot"

bot_id = 1


[security]

anti_multiaccount = true

max_accounts_per_hwid = 1

block_vpn = false # Set to true to block Datacenter VPNs


[database]

path = "data/ayanomi.db"

chat_path = "data/ayanomi_chat.db"

badges_path = "data/ayanomi_badges.db"

auto_backup = true


[mirrors]

direct_search_api = "https://mirror.hinamizawa.ai/api/v1/hinai/search"

download_url = "https://mirror.hinamizawa.ai/api/v1/hinai/d/{}"

beatmap_md5_api = "https://mirror.hinamizawa.ai/v3/osu/beatmaps/md5/{}"
```
