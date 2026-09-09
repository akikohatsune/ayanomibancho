@echo off
echo ======================================================================
echo   CAI DAT CHUNG CHI SSL CHO 127.0.0.1 - AYANOMIBANCHO
echo ======================================================================
echo.
echo He thong dang them chung chi 127.0.0.1 vao Windows...
echo Khi bang Security Warning hien len, hay bam [Yes] hoac [Co].
echo.
certutil -user -f -addstore Root "%~dp0data\certs\server.crt"
echo.
if %ERRORLEVEL% EQU 0 (
    echo [THANH CONG] Da cai dat chung chi SSL thanh cong!
) else (
    echo [LOI] Cai dat chung chi chua hoan tat.
)
echo.
pause
