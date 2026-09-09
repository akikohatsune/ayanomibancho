# AyanomiBancho - Máy chủ osu! Private Server bằng Rust 🦀

**AyanomiBancho** là một máy chủ osu! private server hiệu năng cao, tự chứa (self-contained) được xây dựng hoàn toàn bằng **Rust** theo kiến trúc **Phân tách Dịch vụ & Cô lập Sự cố (Fault Isolation)**, giúp ngăn chặn triệt để lỗi dây chuyền và chống sập tập thể.

Server hỗ trợ đầy đủ cả **Windows** và **Linux / WSL**, lưu trữ dữ liệu gọn nhẹ trong 1 file **SQLite** duy nhất (`data/ayanomi.db`), không cần cài đặt MySQL hay Redis.

---

## ✨ Các tính năng nổi bật

### 🛡️ 1. Kiến trúc Phân tách Dịch vụ (Chống sập tập thể)
- Hệ thống được chia thành 3 dịch vụ con độc lập:
  - **Gateway Layer (Port 5000)**: Reverse Proxy siêu nhẹ, điểm tiếp nhận kết nối duy nhất của client osu! (`-devserver`). Tự động phân luồng `POST /` sang Bancho và các request khác sang Web.
  - **Bancho Service (Port 5001)**: Giao thức nhị phân Little-Endian (`bcho`), heartbeat, in-memory sessions, channels (`#osu`, `#announce`), AyanomiBot.
  - **Web & API Service (Port 5002)**: Leaderboards, score submission, web dashboard, avatar và direct proxy.
- **Cô lập sự cố (Fault Isolation)**: Nếu Web Service gặp lỗi cú pháp hay bị quá tải, **Bancho Service vẫn chạy 100% bình thường**. Người chơi trong game không hề bị văng hay mất kết nối.
- **osu!Direct Circuit Breaker & Timeout 3s**: Tự động ngắt các request tìm kiếm / tải map nếu mirror bên ngoài (Nerinyan) bị nghẽn mạng, bảo vệ server không bị nghẽn luồng.

### 📊 2. Giám sát Tình trạng Máy chủ (Server Status & Telemetry)
- Theo dõi thời gian thực tình trạng từng thành phần: Gateway, Bancho, Web, SQLite Database, Beatmap Mirror.
- Thống kê tài nguyên: Mức tiêu thụ RAM (MB và %), Uptime máy chủ, dung lượng Database, số người online, tổng điểm đã ghi nhận.
- **3 kênh theo dõi**:
  - Giao diện Web Dashboard tại `http://localhost:5000` với đèn trạng thái trực quan.
  - API Health Check tại `GET /api/status`.
  - In-game Bot Command: `!status` hoặc `!uptime`.

### 🔒 3. Bảo mật: Anti-Multiaccount & Chống VPN
- **Anti-Multiaccount (Khóa phần cứng HWID)**:
  - Tự động trích xuất mã băm phần cứng (`adapters_hash`, `disk_signature`, `uninstall_id`) từ client osu!.
  - Ngăn chặn tạo nhiều nick trên cùng một máy tính. Tự động trả về mã lỗi chuẩn Bancho `-4` (*"Multiaccount detected"*).
- **Chống VPN & Datacenter Proxy (Anti-VPN)**:
  - Tự động phát hiện và chặn các dải IP Datacenter/Proxy (AWS, Cloudflare WARP, DigitalOcean, Hetzner,...).
  - Tự động Whitelist IP mạng nội bộ LAN (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`) và Localhost (`127.0.0.1`, `::1`).

### 🌐 4. Bảng mã Quốc gia Phong phú (Country Codes & Flags)
- Hỗ trợ đầy đủ bảng mã quốc gia chuẩn ISO (Việt Nam 🇻🇳, Nhật Bản 🇯🇵, Mỹ 🇺🇸, Hàn Quốc 🇰🇷, Đài Loan 🇹🇼, Anh 🇬🇧, Đức 🇩🇪, Pháp 🇫🇷, Nga 🇷🇺, Canada 🇨🇦, Úc 🇦🇺,...).
- Cho phép người chơi tự chọn cờ quốc gia khi đăng ký tài khoản trên Web Dashboard.

### 🐧 5. Hỗ trợ Toàn diện Linux & WSL
- Hỗ trợ chạy trực tiếp trên Windows và Linux/WSL2.
- Cung cấp script tự động `start.sh` và `stop.sh`.
- File cấu hình `Dockerfile` (multi-stage build siêu nhỏ gọn) và `docker-compose.yml` chạy 1-lệnh không cần cài đặt Rust toolchain thủ công.
- Tệp `ayanomi.service` chạy nền dưới dạng dịch vụ Linux systemd.

### 🗄️ 6. Kiến trúc 3 Database SQLite Cô lập (Fault-Isolated Multi-DB)
- **Tách riêng hoàn toàn 3 Database SQLite độc lập**:
  - `data/ayanomi.db`: Dữ liệu người chơi (`users`), bảng xếp hạng (`stats`), lịch sử trận đấu (`matches`, `match_scores`).
  - `data/ayanomi_chat.db`: Toàn bộ tin nhắn trò chuyện công khai và tin nhắn riêng tư (`chat_messages`). Ghi bất đồng bộ (`tokio::spawn`), không gây chậm gói tin Bancho.
  - `data/ayanomi_badges.db`: Danh mục huy hiệu (`badges`) và danh sách cấp phát cho người chơi (`user_badges`).
- **Chống nghẽn & Khóa dữ liệu (`SQLITE_BUSY`)**: Cả 3 DB đều chạy ở chế độ **WAL (`journal_mode = WAL`)**, `busy_timeout = 5000ms`, `synchronous = Normal`. Tác vụ chat tần suất cao không bao giờ tranh chấp khóa với nộp điểm gameplay.
- **Tự động sao lưu định kỳ (`auto_backup`)**: Sao lưu an toàn bằng `VACUUM INTO` định kỳ mà không cần ngắt máy chủ.

### 🛡️ 7. Chống Raid & Multi-Tier Rate Limiting (Token Bucket)
- Phân tầng hạn mức request theo giây/phút: `Sensitive` (10 RPM), `Direct` (40 RPM), `Bancho` (180 RPM), `General` (120 RPM).
- Chặn đứng spam flood packet, spam tạo nick và spam tải map, trả về `HTTP 429 Too Many Requests`.

### 🎖️ 8. Hệ thống Huy hiệu (Badges) & Tùy biến Menu Background
- **Huy hiệu người chơi (Badges)**: Tích hợp 5 danh hiệu mặc định (👑 Server Admin, 🏆 Tournament Champion, 🚀 Early Pioneer, 💖 Supporter, 🎯 Pro Clicker). Hỗ trợ tạo và trao huy hiệu trực tiếp trên Web Dashboard qua `/api/badges/award` và `/api/badges/create`.
- **Background Menu osu! (Seasonal Backgrounds)**: Cho phép tải ảnh `.jpg`, `.png` lên qua Web Dashboard, osu! client sẽ tự động đồng bộ và hiển thị ở Menu chính.

---

## 🚀 Hướng dẫn Cài đặt & Khởi chạy

### Cách 1: Chạy trực tiếp qua Cargo (Windows / Linux / WSL)
Trong thư mục `ayanomibancho!`, chạy lệnh:
```bash
cargo run
```
*Lệnh này sẽ khởi động bộ Supervisor tự động quản lý cả 3 dịch vụ (Gateway 5000, Bancho 5001, Web 5002).*

### Cách 2: Chạy trên Linux / WSL bằng Shell Scripts
```bash
# Cấp quyền thực thi và khởi động
chmod +x start.sh stop.sh
./start.sh

# Xem log hoạt động
tail -f server.log

# Dừng server
./stop.sh
```

### Cách 3: Chạy bằng Docker / Docker Compose (Tiện lợi nhất trên WSL/Linux)
```bash
docker compose up -d
```

---

## 🎮 Cách kết nối từ osu! Client

Mở Command Prompt / PowerShell trên Windows hoặc tạo một Shortcut đến `osu!.exe` với tham số:
```cmd
osu!.exe -devserver 127.0.0.1:5000
```
*(Nếu server host trên WSL2, bạn vẫn dùng `127.0.0.1:5000` vì WSL2 tự động ánh xạ localhost sang Windows host. Nếu host trên VPS, thay bằng IP hoặc domain của VPS).*

### Đăng ký tài khoản:
1. **Auto-Register**: Nhập bất kỳ Username và Password nào trong màn hình đăng nhập osu!, tài khoản sẽ được tạo ngay lập tức.
2. **Web Dashboard**: Truy cập `http://localhost:5000` trên trình duyệt để chọn cờ quốc gia và xem bảng Server Status.

---

## ⚙️ Tùy chỉnh Cấu hình (`config.toml`)

```toml
[server]
host = "0.0.0.0"
port = 5000               # Gateway port (osu! client kết nối vào đây)
bancho_port = 5001        # Cổng dịch vụ Bancho nội bộ
web_port = 5002           # Cổng dịch vụ Web & API nội bộ
domain = "127.0.0.1:5000"
name = "AyanomiBancho"
welcome_message = "Welcome to AyanomiBancho! Enjoy your stay."

[gameplay]
auto_register = true
default_country = 235     # 235: Vietnam, 224: US, 111: Japan, ...
bot_name = "AyanomiBot"
bot_id = 1

[security]
anti_multiaccount = true
max_accounts_per_hwid = 1
block_vpn = false         # Đổi thành true nếu muốn chặn VPN Datacenter

[database]
path = "data/ayanomi.db"
chat_path = "data/ayanomi_chat.db"
badges_path = "data/ayanomi_badges.db"
auto_backup = true

[mirrors]
direct_search_api = "https://api.nerinyan.moe/search"
download_url = "https://api.nerinyan.moe/d/{}"
```
