@echo off
title Khoi dong osu! - AyanomiBancho Private Server
echo ======================================================================
echo   KHOI DONG OSU! CLIENT KET NOI VAO AYANOMIBANCHO
echo ======================================================================
echo.

set "OSU_DIR=C:\Users\midnightblue\AppData\Local\osu!test"
set "OSU_PATH=%OSU_DIR%\osu!.exe"

if not exist "%OSU_PATH%" (
    echo Khong tim thay osu! tai: %OSU_PATH%
    pause
    exit /b 1
)

echo 1. Dong cac tien trinh osu! cu neu co...
taskkill /f /im osu!.exe >nul 2>&1
ping 127.0.0.1 -n 2 >nul

if exist "%OSU_DIR%\.require_update" (
    del /f /q "%OSU_DIR%\.require_update" >nul 2>&1
)

echo 2. Dang khoi dong osu! voi devserver: 127.0.0.1.nip.io ...
cd /d "%OSU_DIR%"
start "" "%OSU_PATH%" -devserver 127.0.0.1.nip.io

echo [THANH CONG] Da mo game osu! ket noi may chu AyanomiBancho!
echo Ban co the tao tai khoan hoac dang nhap truc tiep trong game.
echo.
