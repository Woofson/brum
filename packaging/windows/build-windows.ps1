# CommanderDog Windows Native Build & Packaging Script
# Author: Bolt J Woofson <bolt@boop.no>
# Usage: powershell -ExecutionPolicy Bypass -File packaging\windows\build-windows.ps1 [-Release]

param(
    [switch]$Release = $true,
    [string]$Version = "0.8.0"
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  🐕 CommanderDog v$Version Windows Build & Packager      " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$RootDir = (Get-Item $PSScriptRoot).Parent.Parent.FullName
Set-Location $RootDir

# 1. Build Desktop Tauri Application
Write-Host "[1/4] Building Tauri Windows Native Desktop App..." -ForegroundColor Yellow
Set-Location "$RootDir\src-tauri"
if ($Release) {
    cargo tauri build --target x86_64-pc-windows-msvc
} else {
    cargo tauri build --debug --target x86_64-pc-windows-msvc
}

# 2. Build Core CLI Binary
Write-Host "[2/4] Building CommanderDog CLI / Server Binary..." -ForegroundColor Yellow
Set-Location $RootDir
if ($Release) {
    cargo build --release --target x86_64-pc-windows-msvc
} else {
    cargo build --target x86_64-pc-windows-msvc
}

# 3. Create Standalone Portable Zip Distribution
Write-Host "[3/4] Creating Portable Standalone ZIP distribution..." -ForegroundColor Yellow
$DistDir = "$RootDir\dist\windows"
if (Test-Path $DistDir) { Remove-Item -Recurse -Force $DistDir }
New-Item -ItemType Directory -Path "$DistDir\commanderdog" | Out-Null

Copy-Item "$RootDir\target\x86_64-pc-windows-msvc\release\commanderdog.exe" "$DistDir\commanderdog\"
if (Test-Path "$RootDir\src-tauri\target\x86_64-pc-windows-msvc\release\commanderdog-desktop.exe") {
    Copy-Item "$RootDir\src-tauri\target\x86_64-pc-windows-msvc\release\commanderdog-desktop.exe" "$DistDir\commanderdog\CommanderDog.exe"
}
Copy-Item "$RootDir\config.toml" "$DistDir\commanderdog\"
Copy-Item "$RootDir\LICENSE" "$DistDir\commanderdog\"
Copy-Item "$RootDir\README.md" "$DistDir\commanderdog\"
Copy-Item "$RootDir\packaging\windows\register-context-menu.reg" "$DistDir\commanderdog\"
Copy-Item "$RootDir\packaging\windows\unregister-context-menu.reg" "$DistDir\commanderdog\"
Copy-Item "$RootDir\assets\commanderdog.ico" "$DistDir\commanderdog\" -ErrorAction SilentlyContinue

$ZipPath = "$DistDir\commanderdog-windows-x86_64-v$Version.zip"
Compress-Archive -Path "$DistDir\commanderdog\*" -DestinationPath $ZipPath -Force

# 4. Generate SHA256 Checksums
Write-Host "[4/4] Generating SHA-256 Checksums..." -ForegroundColor Yellow
$Hash = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLower()
"$Hash  $([System.IO.Path]::GetFileName($ZipPath))" | Out-File -FilePath "$DistDir\SHA256SUMS.txt" -Encoding ascii

Write-Host "`n✅ Build and packaging complete!" -ForegroundColor Green
Write-Host "📦 Portable ZIP: $ZipPath" -ForegroundColor Green
Write-Host "🔑 SHA-256:      $Hash" -ForegroundColor Green
