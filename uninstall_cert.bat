@echo off
title Xoa Chung chi SSL - AyanomiBancho
echo ======================================================================
echo   XOA BO CHUNG CHI TAM THOI KHOI HE THONG
echo ======================================================================
echo.
powershell -NoProfile -Command "Get-ChildItem Cert:\CurrentUser\Root | Where-Object { $_.Subject -like '*127.0.0.1*' -or $_.Subject -like '*Ayanomi*' } | Remove-Item"
powershell -NoProfile -Command "Get-ChildItem Cert:\CurrentUser\TrustedPeople | Where-Object { $_.Subject -like '*127.0.0.1*' -or $_.Subject -like '*Ayanomi*' } | Remove-Item"
echo.
echo [HOAN TAT] Da xoa sach chung chi tam khoi he thong cua ban!
echo.
pause
