@echo off
setlocal
cd /d "%~dp0"
echo ======================================================
echo   Brum - Create Windows Shortcuts
echo ======================================================
echo.

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0create-shortcuts.ps1" %*
if %ERRORLEVEL% equ 0 (
    echo.
    echo Shortcuts created successfully!
    echo To also create a Desktop shortcut, run: create-shortcuts.bat -Desktop
) else (
    echo.
    echo Failed to create shortcuts.
)
echo.
pause
