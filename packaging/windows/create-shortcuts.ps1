# ==============================================================================
# 🐕 Brum - Create Windows Shortcuts (Start Menu & Optional Desktop)
# ==============================================================================

param(
    [switch]$Desktop = $false
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$BrumExe = Join-Path $ScriptDir "brum.exe"
$BrumIco = Join-Path $ScriptDir "brum.ico"

if (-not (Test-Path $BrumExe)) {
    Write-Error "brum.exe was not found in $ScriptDir"
    exit 1
}

$WshShell = New-Object -ComObject WScript.Shell

# 1. Start Menu Shortcut (Always created)
$ProgramsDir = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::Programs)
$StartMenuDir = Join-Path $ProgramsDir "Brum"
if (-not (Test-Path $StartMenuDir)) {
    New-Item -ItemType Directory -Path $StartMenuDir -Force | Out-Null
}
$StartShortcutPath = Join-Path $StartMenuDir "Brum.lnk"
$StartShortcut = $WshShell.CreateShortcut($StartShortcutPath)
$StartShortcut.TargetPath = $BrumExe
$StartShortcut.Arguments = "-s"
$StartShortcut.WorkingDirectory = $ScriptDir
if (Test-Path $BrumIco) {
    $StartShortcut.IconLocation = "$BrumIco,0"
}
$StartShortcut.Description = "Brum - File Commander & Manager (Standalone)"
$StartShortcut.Save()
Write-Host "✓ Created Start Menu shortcut: $StartShortcutPath" -ForegroundColor Green

# 2. Desktop Shortcut (Optional, default off unless -Desktop is specified)
if ($Desktop) {
    $DesktopDir = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::Desktop)
    $DeskShortcutPath = Join-Path $DesktopDir "Brum.lnk"
    $DeskShortcut = $WshShell.CreateShortcut($DeskShortcutPath)
    $DeskShortcut.TargetPath = $BrumExe
    $DeskShortcut.Arguments = "-s"
    $DeskShortcut.WorkingDirectory = $ScriptDir
    if (Test-Path $BrumIco) {
        $DeskShortcut.IconLocation = "$BrumIco,0"
    }
    $DeskShortcut.Description = "Brum - File Commander & Manager (Standalone)"
    $DeskShortcut.Save()
    Write-Host "✓ Created Desktop shortcut: $DeskShortcutPath" -ForegroundColor Green
}
