# PowerShell helper to build ARM targets using Cross or Docker
Write-Host "=== AyanomiBancho ARM Multi-Arch Cross Compiler ===" -ForegroundColor Cyan

if (-not (Get-Command "cross" -ErrorAction SilentlyContinue)) {
    Write-Host "Installing 'cross' tool for multi-arch compilation..." -ForegroundColor Yellow
    cargo install cross --git https://github.com/cross-rs/cross
}

Write-Host "1. Building for ARMv8 / AArch64..." -ForegroundColor Green
cross build --release --target aarch64-unknown-linux-gnu --bin ayanomibancho

Write-Host "2. Building for ARMv7..." -ForegroundColor Green
cross build --release --target armv7-unknown-linux-gnueabihf --bin ayanomibancho

Write-Host "=== Finished ARM builds ===" -ForegroundColor Cyan
