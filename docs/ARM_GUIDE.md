# Hướng Dẫn Chạy AyanomiBancho trên ARMv7 & ARMv8 (AArch64)

AyanomiBancho hỗ trợ đầy đủ các kiến trúc vi xử lý ARM:
- **ARMv8 (64-bit / AArch64)**: Raspberry Pi 4/5 (64-bit OS), Oracle Cloud Ampere A1, AWS Graviton, Apple Silicon Linux, Orange Pi 5, VPS Linux ARM64.
- **ARMv7 (32-bit / armv7hf)**: Raspberry Pi 2/3 (Raspberry Pi OS 32-bit), Orange Pi One/PC, NanoPi, Asus Tinker Board.

## Cách 1: Tự động biên dịch qua GitLab CI/CD (.gitlab-ci.yml)
Dự án đã được cấu hình sẵn pipeline **GitLab CI/CD** chuẩn (`.gitlab-ci.yml`):
- Mỗi khi bạn `git push` lên GitLab:
  - Job `build:armv8-aarch64`: Tự động cross-compile cho **ARMv8 (64-bit)** bằng `gcc-aarch64-linux-gnu`.
  - Job `build:armv7`: Tự động cross-compile cho **ARMv7 (32-bit)** bằng `gcc-arm-linux-gnueabihf`.
  - Job `build:x86_64`: Tự động compile cho **Linux x86_64**.
  - Job `docker-build-multiarch`: Build Docker image đa kiến trúc đẩy thẳng vào GitLab Container Registry (`$CI_REGISTRY_IMAGE`).
- File nhị phân được tự động đóng gói thành `.tar.gz` đính kèm trong mục **Artifacts** của từng Job hoặc trang **Deploy -> Releases** trên GitLab.
- Bạn chỉ cần tải về thiết bị ARM, giải nén và chạy:
  ```bash
  tar -xzvf ayanomibancho-linux-armv8-aarch64.tar.gz
  chmod +x ayanomibancho
  ./ayanomibancho
  ```

---

## Cách 2: Tự động qua GitHub Actions (.github/workflows/build-arm.yml)
Nếu bạn có mirror repo sang GitHub, GitHub Actions cũng đã được thiết lập sẵn sàng để build song song.

---

## Cách 3: Chạy qua Docker / Docker Compose (Tiện lợi nhất)
Dự án đã tích hợp sẵn `Dockerfile` đa kiến trúc (Multi-arch) và `docker-compose.yml`:

```bash
# Khởi động server trong nền
docker compose up -d --build

# Xem log
docker compose logs -f
```
Docker sẽ tự động nhận diện thiết bị của bạn là ARMv7 hay ARMv8 và biên dịch/tối ưu phù hợp.

---

## Cách 4: Biên dịch trực tiếp trên máy ARM (Native Build)
Nếu bạn đang SSH vào Raspberry Pi hoặc VPS Linux ARM:

```bash
# 1. Cài đặt Rust và thư viện C cần thiết
sudo apt update && sudo apt install -y curl build-essential pkg-config libssl-dev

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# 2. Biên dịch bản phát hành tối ưu
cargo build --release --bin ayanomibancho

# 3. File chạy tạo ra tại:
./target/release/ayanomibancho
```

---

## Cách 5: Cross-compile từ máy tính cá nhân bằng Cross
Nếu bạn dùng Windows hoặc Linux x86 và muốn biên dịch ra file nhị phân cho ARM:

```bash
# Cài đặt công cụ cross
cargo install cross --git https://github.com/cross-rs/cross

# Build cho ARMv8 (64-bit)
cross build --release --target aarch64-unknown-linux-gnu --bin ayanomibancho

# Build cho ARMv7 (32-bit)
cross build --release --target armv7-unknown-linux-gnueabihf --bin ayanomibancho
```

---

## Cấu hình tự động thích ứng (Zero-Config Path)
- Hệ thống đã được tích hợp bộ chuẩn hoá đường dẫn đa nền tảng: Nếu bạn mang file `config.toml` từ Windows sang ARM Linux, các đường dẫn ổ đĩa Windows (như `C:/...`) sẽ tự động được chuyển đổi sang thư mục `data/` an toàn trên Linux mà không gây crash server.
