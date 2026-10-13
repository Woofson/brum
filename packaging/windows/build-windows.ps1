# Brum Windows Native Build & Packaging Script
# Author: Bolt J Woofson <bolt@boop.no>
# Usage: powershell -ExecutionPolicy Bypass -File packaging\windows\build-windows.ps1 [-Release]

param(
    [switch]$Release = $true,
    [string]$Version = "0.8.2"
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  🐕 Brum v$Version Windows Build & Packager              " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$RootDir = (Get-Item $PSScriptRoot).Parent.Parent.FullName
Set-Location $RootDir

# 1. Build Brum Binary
Write-Host "[1/3] Building Brum Windows Binary..." -ForegroundColor Yellow
Set-Location $RootDir
if ($Release) {
    cargo build --release --target x86_64-pc-windows-msvc
} else {
    cargo build --target x86_64-pc-windows-msvc
}

# 2. Create Standalone Portable Zip Distribution
Write-Host "[2/3] Creating Portable Standalone ZIP distribution..." -ForegroundColor Yellow
$DistDir = "$RootDir\dist\windows"
if (Test-Path $DistDir) { Remove-Item -Recurse -Force $DistDir }
New-Item -ItemType Directory -Path "$DistDir\brum" | Out-Null

$ExeSource = if (Test-Path "$RootDir\target\x86_64-pc-windows-msvc\release\brum.exe") {
    "$RootDir\target\x86_64-pc-windows-msvc\release\brum.exe"
} elseif (Test-Path "$RootDir\target\x86_64-pc-windows-gnu\release\brum.exe") {
    "$RootDir\target\x86_64-pc-windows-gnu\release\brum.exe"
} else {
    "$RootDir\target\release\brum.exe"
}
Copy-Item $ExeSource "$DistDir\brum\brum.exe"
Copy-Item "$RootDir\config.toml" "$DistDir\brum\"
Copy-Item "$RootDir\LICENSE" "$DistDir\brum\"
Copy-Item "$RootDir\README.md" "$DistDir\brum\"
Copy-Item "$RootDir\packaging\windows\register-context-menu.reg" "$DistDir\brum\"
Copy-Item "$RootDir\packaging\windows\unregister-context-menu.reg" "$DistDir\brum\"
Copy-Item "$RootDir\packaging\windows\create-shortcuts.ps1" "$DistDir\brum\" -ErrorAction SilentlyContinue
Copy-Item "$RootDir\packaging\windows\remove-shortcuts.bat" "$DistDir\brum\" -ErrorAction SilentlyContinue
Copy-Item "$RootDir\assets\brum.ico" "$DistDir\brum\" -ErrorAction SilentlyContinue

$ZipPath = "$DistDir\brum-windows-x86_64-v$Version.zip"
Compress-Archive -Path "$DistDir\brum\*" -DestinationPath $ZipPath -Force

# 3. Generate SHA256 Checksums for Portable ZIP
Write-Host "[3/4] Generating SHA-256 Checksums..." -ForegroundColor Yellow
$Hash = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLower()
"$Hash  $([System.IO.Path]::GetFileName($ZipPath))" | Out-File -FilePath "$DistDir\SHA256SUMS.txt" -Encoding ascii

# 4. Build Windows Setup Installer (Inno Setup)
Write-Host "[4/4] Compiling Inno Setup Windows Installer..." -ForegroundColor Yellow
$IsccPaths = @(
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe",
    "ISCC.exe"
)
$IsccExe = $null
foreach ($p in $IsccPaths) {
    if (Get-Command $p -ErrorAction SilentlyContinue) {
        $IsccExe = $p
        break
    }
}

if ($IsccExe) {
    Write-Host "Found Inno Setup Compiler at: $IsccExe" -ForegroundColor Cyan
    & $IsccExe "/DMyAppVersion=$Version" "$RootDir\packaging\windows\setup.iss"
    $InstallerExe = "$DistDir\Brum-Setup-v$Version.exe"
    if (Test-Path $InstallerExe) {
        $InstHash = (Get-FileHash -Path $InstallerExe -Algorithm SHA256).Hash.ToLower()
        "$InstHash  $([System.IO.Path]::GetFileName($InstallerExe))" | Out-File -FilePath "$DistDir\SHA256SUMS.txt" -Append -Encoding ascii
        Write-Host "📦 Setup Installer: $InstallerExe" -ForegroundColor Green
        Write-Host "🔑 Installer Hash:  $InstHash" -ForegroundColor Green
    }
} else {
    Write-Host "Inno Setup Compiler (ISCC.exe) not found. Skipping installer exe creation (portable ZIP created)." -ForegroundColor DarkGray
}

Write-Host "`nBuild and packaging complete!" -ForegroundColor Green
Write-Host "📦 Portable ZIP: $ZipPath" -ForegroundColor Green
Write-Host "🔑 SHA-256:      $Hash" -ForegroundColor Green

