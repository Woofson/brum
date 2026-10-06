#!/usr/bin/env bash
# ==============================================================================
# Brum - Multi-Format Release Packaging Script
# Generates: Standalone Tarballs (.tar.gz), Debian (.deb), Alpine (.apk),
#            Windows Portable Zip (.zip), and Checksums
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

VERSION=$(grep -m1 '^version = ' Cargo.toml | cut -d '"' -f2)
ARCH=$(uname -m)
DIST_DIR="${ROOT_DIR}/dist"

echo "======================================================"
echo "Building Brum Release Packages (v${VERSION})..."
echo "======================================================"

# 1. Compile Release Binary (Linux)
echo "Compiling standalone Linux release binary..."
cargo build --release

rm -rf "${DIST_DIR}"
mkdir -p "${DIST_DIR}"

# 2. Package Generic Linux Tarball
TARBALL_NAME="brum-v${VERSION}-linux-${ARCH}"
TARBALL_DIR="/tmp/${TARBALL_NAME}"
rm -rf "${TARBALL_DIR}"
mkdir -p "${TARBALL_DIR}"

cp "./target/release/brum" "${TARBALL_DIR}/"
cp "./config.toml" "${TARBALL_DIR}/"
cp "./brum.service" "${TARBALL_DIR}/"
cp "./LICENSE" "${TARBALL_DIR}/"
cp "./README.md" "${TARBALL_DIR}/"
if [ -d "./themes" ]; then
    mkdir -p "${TARBALL_DIR}/themes"
    cp ./themes/*.toml "${TARBALL_DIR}/themes/" 2>/dev/null || true
fi
if [ -d "./plugins" ]; then
    mkdir -p "${TARBALL_DIR}/plugins"
    cp ./plugins/*.grr "${TARBALL_DIR}/plugins/" 2>/dev/null || true
fi
if [ -f "./scripts/lxc-install.sh" ]; then
    cp "./scripts/lxc-install.sh" "${TARBALL_DIR}/install.sh"
    chmod +x "${TARBALL_DIR}/install.sh"
fi

echo "📦 Creating ${TARBALL_NAME}.tar.gz..."
tar -czf "${DIST_DIR}/${TARBALL_NAME}.tar.gz" -C /tmp "${TARBALL_NAME}"
rm -rf "${TARBALL_DIR}"

# 3. Build Debian .deb Package
if command -v cargo-deb >/dev/null 2>&1 || cargo deb --version >/dev/null 2>&1; then
    echo "📦 Building Debian .deb package via cargo-deb..."
    cargo deb --no-build
    cp target/debian/*.deb "${DIST_DIR}/"
elif command -v dpkg-deb >/dev/null 2>&1; then
    echo "📦 Building Debian .deb package via dpkg-deb fallback..."
    DEB_DIR="/tmp/deb-pkg"
    rm -rf "${DEB_DIR}"
    mkdir -p "${DEB_DIR}/DEBIAN" "${DEB_DIR}/usr/bin" "${DEB_DIR}/etc/brum" "${DEB_DIR}/etc/brum/themes" "${DEB_DIR}/usr/lib/systemd/system" "${DEB_DIR}/usr/share/pixmaps" "${DEB_DIR}/usr/share/applications" "${DEB_DIR}/usr/share/doc/brum" "${DEB_DIR}/usr/share/brum/plugins"
    cat << DEBEOF > "${DEB_DIR}/DEBIAN/control"
Package: brum
Version: ${VERSION}-1
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Bolt J Woofson <bolt@boop.no>
Depends: ca-certificates, tar, bzip2, 7zip | p7zip-full
Description: Multi-Pane Web Environment (File Commander/Manager)
 Blending the orthodox speed of Total Commander / Midnight Commander
 with the modern responsiveness of Next Explorer.
DEBEOF
    cp "./target/release/brum" "${DEB_DIR}/usr/bin/"
    cp "./config.toml" "${DEB_DIR}/etc/brum/config.toml"
    if [ -d "./themes" ]; then
        cp ./themes/*.toml "${DEB_DIR}/etc/brum/themes/" 2>/dev/null || true
    fi
    cp "./brum.service" "${DEB_DIR}/usr/lib/systemd/system/"
    if [ -f "./brum.desktop" ]; then
        cp "./brum.desktop" "${DEB_DIR}/usr/share/applications/"
    fi
    if [ -f "./assets/brum.png" ]; then
        cp "./assets/brum.png" "${DEB_DIR}/usr/share/pixmaps/brum.png"
    elif [ -f "./assets/128/brum-128.webp" ]; then
        cp "./assets/128/brum-128.webp" "${DEB_DIR}/usr/share/pixmaps/brum.webp"
    fi
    if [ -d "./plugins" ]; then
        cp ./plugins/*.grr "${DEB_DIR}/usr/share/brum/plugins/" 2>/dev/null || true
    fi
    cp "./LICENSE" "${DEB_DIR}/usr/share/doc/brum/copyright"
    cp "./README.md" "${DEB_DIR}/usr/share/doc/brum/"
    if [ -d "./packaging/debian" ]; then
        cp ./packaging/debian/* "${DEB_DIR}/DEBIAN/"
        chmod 755 "${DEB_DIR}/DEBIAN"/*
    fi
    chmod 755 "${DEB_DIR}/usr/bin/brum" "${DEB_DIR}/DEBIAN"
    dpkg-deb --build "${DEB_DIR}" "${DIST_DIR}/brum_${VERSION}-1_amd64.deb"
    rm -rf "${DEB_DIR}"
fi

# 4. Build Alpine Linux (.apk) Package
APK_DIR="/tmp/apk-pkg"
rm -rf "${APK_DIR}"
mkdir -p "${APK_DIR}/usr/bin" "${APK_DIR}/etc/brum" "${APK_DIR}/etc/brum/themes" "${APK_DIR}/usr/share/pixmaps" "${APK_DIR}/usr/share/applications" "${APK_DIR}/usr/share/licenses/brum" "${APK_DIR}/usr/share/doc/brum" "${APK_DIR}/usr/share/brum/plugins"

cp "./target/release/brum" "${APK_DIR}/usr/bin/"
cp "./config.toml" "${APK_DIR}/etc/brum/config.toml"
if [ -d "./themes" ]; then
    cp ./themes/*.toml "${APK_DIR}/etc/brum/themes/" 2>/dev/null || true
fi
if [ -f "./assets/brum.png" ]; then
    cp "./assets/brum.png" "${APK_DIR}/usr/share/pixmaps/brum.png"
elif [ -f "./assets/128/brum-128.webp" ]; then
    cp "./assets/128/brum-128.webp" "${APK_DIR}/usr/share/pixmaps/brum.webp"
fi
if [ -d "./plugins" ]; then
    cp ./plugins/*.grr "${APK_DIR}/usr/share/brum/plugins/" 2>/dev/null || true
fi
if [ -f "./brum.desktop" ]; then
    cp "./brum.desktop" "${APK_DIR}/usr/share/applications/"
fi
cp "./LICENSE" "${APK_DIR}/usr/share/licenses/brum/"
cp "./README.md" "${APK_DIR}/usr/share/doc/brum/"

INSTALLED_SIZE=$(du -sb "${APK_DIR}" | cut -f1)
BUILD_DATE=$(date +%s)

cat << APKEOF > "${APK_DIR}/.PKGINFO"
# Generated by Brum release packaging
pkgname = brum
pkgver = ${VERSION}-r0
pkgdesc = Multi-Pane Web Environment (File Commander/Manager) - By Woofson
url = https://github.com/Woofson/brum
builddate = ${BUILD_DATE}
packager = Bolt J Woofson <bolt@boop.no>
size = ${INSTALLED_SIZE}
arch = ${ARCH}
origin = brum
license = MIT
depend = ca-certificates libssh2 sqlite-libs
APKEOF

echo "📦 Creating Alpine .apk package (brum-${VERSION}-r0.${ARCH}.apk)..."
tar -czf "${DIST_DIR}/brum-${VERSION}-r0.${ARCH}.apk" -C "${APK_DIR}" .PKGINFO usr etc
rm -rf "${APK_DIR}"

# 5. Build Windows Portable ZIP Package
echo "📦 Building Windows Portable Package (x86_64-pc-windows-gnu)..."
if command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    # 1. Build CLI / Server / Service Binary (brum.exe)
    cargo build --release --target x86_64-pc-windows-gnu
    # 2. Build Native Standalone Desktop Application (Brum.exe via Tauri v2 + WebView2)
    cargo build --release --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-gnu

    if [ -f "./target/x86_64-pc-windows-gnu/release/brum.exe" ]; then
        WIN_DIR="/tmp/brum-v${VERSION}-windows-x86_64"
        rm -rf "${WIN_DIR}"
        mkdir -p "${WIN_DIR}" "${WIN_DIR}/themes" "${WIN_DIR}/plugins"

        # 1. Copy Native Desktop Application (Tauri v2 + WebView2) as Brum.exe
        if [ -f "./src-tauri/target/x86_64-pc-windows-gnu/release/brum-desktop.exe" ]; then
            cp "./src-tauri/target/x86_64-pc-windows-gnu/release/brum-desktop.exe" "${WIN_DIR}/Brum.exe"
        fi

        # 2. Copy Daemon / Server / Service binary as brumd.exe
        cp "./target/x86_64-pc-windows-gnu/release/brum.exe" "${WIN_DIR}/brumd.exe"

        # 3. Copy WebView2Loader.dll for Desktop GUI
        if [ -f "./src-tauri/target/x86_64-pc-windows-gnu/release/WebView2Loader.dll" ]; then
            cp "./src-tauri/target/x86_64-pc-windows-gnu/release/WebView2Loader.dll" "${WIN_DIR}/"
        elif [ -f "./packaging/windows/WebView2Loader.dll" ]; then
            cp "./packaging/windows/WebView2Loader.dll" "${WIN_DIR}/"
        fi

        if command -v x86_64-w64-mingw32-strip >/dev/null 2>&1; then
            if [ -f "${WIN_DIR}/Brum.exe" ]; then
                x86_64-w64-mingw32-strip "${WIN_DIR}/Brum.exe" 2>/dev/null || true
            fi
            if [ -f "${WIN_DIR}/brumd.exe" ]; then
                x86_64-w64-mingw32-strip "${WIN_DIR}/brumd.exe" 2>/dev/null || true
            fi
        fi
        cp "./config.toml" "${WIN_DIR}/"
        cp "./LICENSE" "${WIN_DIR}/"
        cp "./README.md" "${WIN_DIR}/"
        if [ -d "./themes" ]; then
            cp ./themes/*.toml "${WIN_DIR}/themes/" 2>/dev/null || true
        fi
        if [ -d "./plugins" ]; then
            cp ./plugins/*.grr "${WIN_DIR}/plugins/" 2>/dev/null || true
        fi
        if [ -f "./packaging/windows/brum.ico" ]; then
            cp "./packaging/windows/brum.ico" "${WIN_DIR}/"
        fi
        if [ -f "./packaging/windows/create-shortcuts.ps1" ]; then
            cp "./packaging/windows/create-shortcuts.ps1" "${WIN_DIR}/"
        fi
        if [ -f "./packaging/windows/create-shortcuts.bat" ]; then
            cp "./packaging/windows/create-shortcuts.bat" "${WIN_DIR}/"
        fi
        if [ -f "./packaging/windows/remove-shortcuts.bat" ]; then
            cp "./packaging/windows/remove-shortcuts.bat" "${WIN_DIR}/"
        fi
        if [ -f "./packaging/windows/register-context-menu.reg" ]; then
            cp "./packaging/windows/register-context-menu.reg" "${WIN_DIR}/"
        fi
        if [ -f "./packaging/windows/unregister-context-menu.reg" ]; then
            cp "./packaging/windows/unregister-context-menu.reg" "${WIN_DIR}/"
        fi

        (
            cd /tmp
            zip -rq "${DIST_DIR}/brum-v${VERSION}-windows-x86_64.zip" "brum-v${VERSION}-windows-x86_64"
        )
        rm -rf "${WIN_DIR}"
        echo "✅ Created brum-v${VERSION}-windows-x86_64.zip"
    fi
fi

# 6. Generate SHA-256 Checksums
echo "🔒 Generating SHA-256 Checksums..."
(
    cd "${DIST_DIR}"
    rm -f SHA256SUMS SHA256SUMS.txt
    sha256sum * > SHA256SUMS
    cp SHA256SUMS SHA256SUMS.txt
)

echo "======================================================"
echo "✅ Build Complete! Release artifacts generated in ./dist/:"
ls -la "${DIST_DIR}"
echo "======================================================"
