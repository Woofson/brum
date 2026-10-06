@echo off
setlocal
cd /d "%~dp0"
echo ======================================================
echo   Brum - Remove Windows Shortcuts
echo ======================================================
echo.

powershell -NoProfile -ExecutionPolicy Bypass -Command "$p = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::Programs); $d = Join-Path $p 'Brum'; if (Test-Path $d) { Remove-Item -Recurse -Force $d; Write-Host 'Removed Start Menu shortcut.' -ForegroundColor Green }; $desk = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::Desktop); $deskLink = Join-Path $desk 'Brum.lnk'; if (Test-Path $deskLink) { Remove-Item -Force $deskLink; Write-Host 'Removed Desktop shortcut.' -ForegroundColor Green }"

echo.
echo Shortcut cleanup complete.
echo.
pause
