
# AyanomiBancho - Very low-cost osu! Private Server

**AyanomiBancho** is a high-performance, low-cost, self-contained osu! private server built entirely in **Rust**, using a **Service Separation & Fault Isolation** architecture designed to prevent cascading failures and full-system crashes.

The server fully supports both **Windows** and **Linux / WSL**, storing all data in lightweight **SQLite** databases (`data/ayanomi.db`, etc.) without requiring MySQL or Redis.

### Method 1: Run Directly with Cargo (Windows / Linux / WSL)

From the `ayanomibancho!` directory, run:

```bash
cargo run
```

*This command starts the Supervisor, which automatically manages all 3 services (Gateway 5000, Bancho 5001, and Web 5002).*

### Method 2: Run on Linux / WSL Using Shell Scripts

```bash
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
docker compose up -d
```

---

## Connecting from the osu! Client

Open Command Prompt / PowerShell on Windows, or create a shortcut to `osu!.exe` with the following parameter:

```cmd
osu!.exe -devserver 127.0.0.1:5000
```

*(If the server is hosted on WSL2, you can still use `127.0.0.1:5000` because WSL2 automatically maps localhost to the Windows host. If the server is hosted on a VPS, replace it with the VPS IP address or domain.)*

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

host = "0.0.0.0"

port = 5000 # Gateway port (osu! client connects here)

bancho_port = 5001 # Internal Bancho service port

web_port = 5002 # Internal Web & API service port

domain = "127.0.0.1:5000"

name = "AyanomiBancho"

welcome_message = "Welcome to AyanomiBancho! Enjoy your stay."


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

direct_search_api = "https://api.nerinyan.moe/search"

download_url = "https://api.nerinyan.moe/d/{}"
```
