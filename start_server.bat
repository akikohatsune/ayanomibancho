@echo off
title AyanomiBancho Server and HTTPS Proxy
echo ======================================================================
echo   AYANOMIBANCHO SERVER AND HTTPS GATEWAY
echo ======================================================================
echo.
echo 1. Dang khoi dong Bancho va Web Service...
start "AyanomiBancho Core" "target\debug\ayanomibancho.exe"

timeout /t 2 /nobreak >nul

echo 2. Dang khoi dong HTTPS Proxy (Port 443 va 80)...
start "AyanomiBancho SSL Gateway" python -u "data\https_proxy.py"

echo.
echo [HOAN TAT] May chu va HTTPS Proxy dang hoat dong!
echo - Web Dashboard: http://127.0.0.1:5000
echo - Bancho HTTPS:  https://127.0.0.1.nip.io
echo.
pause
