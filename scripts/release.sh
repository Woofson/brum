#!/usr/bin/env bash
# ==============================================================================
# Brum - One-Command Cleanup, Version Bump, Build & Release Script
# Automates:
#  1. Cleanup of temporary logs, artifacts, and test files
#  2. Semver version bump across Cargo, Tauri, Web, and Packaging configs
#  3. Building release binary and distribution packages (.tar.gz, .deb, .apk)
#  4. Git commit, tag, and push to GitHub (origin main --tags)
#  5. Automatic source sha256 checksum calculation & PKGBUILD sync
#  6. Automatic AUR sync for both `brum` and `brum-bin`
#
# USAGE:
#   ./scripts/release.sh               # Auto-bumps patch (e.g. 0.8.2 -> 0.8.3)
#   ./scripts/release.sh 0.8.3         # Explicit version bump
#   ./scripts/release.sh minor         # Bumps minor (e.g. 0.8.2 -> 0.9.0)
#   ./scripts/release.sh major         # Bumps major (e.g. 0.8.2 -> 1.0.0)
#   ./scripts/release.sh --skip-aur    # Skips pushing to AUR
# ==============================================================================

set -euo pipefail

# Ensure we are in the project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

CURRENT_VERSION=$(grep -m1 '^version = ' Cargo.toml | cut -d '"' -f2)
SKIP_AUR=false
SKIP_CRATES=false
TARGET_VERSION=""

for arg in "$@"; do
    case "$arg" in
        --skip-aur)
            SKIP_AUR=true
            ;;
        --skip-crates)
            SKIP_CRATES=true
            ;;
        patch|minor|major)
            BUMP_TYPE="$arg"
            ;;
        *)
            if [[ "$arg" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
                TARGET_VERSION="$arg"
            fi
            ;;
    esac
done

# Calculate next version if not specified
if [ -z "${TARGET_VERSION}" ]; then
    IFS='.' read -r MAJOR MINOR PATCH <<< "${CURRENT_VERSION}"
    BUMP_TYPE="${BUMP_TYPE:-patch}"
    case "${BUMP_TYPE}" in
        major)
            MAJOR=$((MAJOR + 1))
            MINOR=0
            PATCH=0
            ;;
        minor)
            MINOR=$((MINOR + 1))
            PATCH=0
            ;;
        patch|*)
            PATCH=$((PATCH + 1))
            ;;
    esac
    TARGET_VERSION="${MAJOR}.${MINOR}.${PATCH}"
fi

echo "======================================================"
echo "🚀 Brum Automated Release: v${CURRENT_VERSION} -> v${TARGET_VERSION}"
echo "======================================================"

# ------------------------------------------------------------------------------
# 1. CLEANUP TEMPORARY FILES & SCRATCH ARTIFACTS
# ------------------------------------------------------------------------------
echo "🧹 Cleaning up temporary logs, scratch files, and build caches..."
rm -f *.log *.tmp *.dump *.dmp parubrum*.txt brum*.txt parucommanderdog*.txt commanderdog*.txt *.db *.db-journal *.sqlite*
rm -rf dist /tmp/aur-* /tmp/deb-pkg /tmp/apk-pkg /tmp/brum-* /tmp/commanderdog-*

# ------------------------------------------------------------------------------
# 2. PRE-FLIGHT SENSITIVE DATA & CREDENTIALS CHECK
# ------------------------------------------------------------------------------
echo "🛡️ Running pre-flight sensitive information audit..."
LEAK_DETECTED=false

# Check for sensitive file patterns staged or present in tracked workspace
SENSITIVE_FILES=$(git ls-files | grep -E '\.(pem|key|p12|pfx|token|secret|credentials|env)$|id_rsa|id_ed25519|\.db$|\.sqlite' || true)
if [ -n "${SENSITIVE_FILES}" ]; then
    echo "❌ SECURITY ALERT: Sensitive files found in git tree:"
    echo "${SENSITIVE_FILES}"
    LEAK_DETECTED=true
fi

if [ "${LEAK_DETECTED}" = true ]; then
    echo "🚫 Release aborted: Remove sensitive files before releasing."
    exit 1
fi
echo "✅ Security check passed: No private keys, credentials, or databases found."

# ------------------------------------------------------------------------------
# 3. VERSION BUMP ACROSS CONFIGS
# ------------------------------------------------------------------------------
echo "📝 Updating version to ${TARGET_VERSION} across all project files..."

# Cargo.toml (root)
sed -i "0,/^version = .*/s//version = \"${TARGET_VERSION}\"/" Cargo.toml

# src-tauri/Cargo.toml
if [ -f "src-tauri/Cargo.toml" ]; then
    sed -i "0,/^version = .*/s//version = \"${TARGET_VERSION}\"/" src-tauri/Cargo.toml
fi

# src-tauri/tauri.conf.json
if [ -f "src-tauri/tauri.conf.json" ]; then
    sed -i "s/\"version\": \".*\"/\"version\": \"${TARGET_VERSION}\"/" src-tauri/tauri.conf.json
fi

# frontend/index.html badge
if [ -f "frontend/index.html" ]; then
    sed -i "s/v[0-9]\+\.[0-9]\+\.[0-9]\+ (Desktop & Web)/v${TARGET_VERSION} (Desktop \& Web)/g" frontend/index.html
fi

# packaging/PKGBUILD
if [ -f "packaging/PKGBUILD" ]; then
    sed -i "s/^pkgver=.*/pkgver=${TARGET_VERSION}/" packaging/PKGBUILD
    sed -i "s/^pkgrel=.*/pkgrel=1/" packaging/PKGBUILD
fi

# packaging/brum-bin.PKGBUILD
if [ -f "packaging/brum-bin.PKGBUILD" ]; then
    sed -i "s/^pkgver=.*/pkgver=${TARGET_VERSION}/" packaging/brum-bin.PKGBUILD
    sed -i "s/^pkgrel=.*/pkgrel=1/" packaging/brum-bin.PKGBUILD
fi

# packaging/APKBUILD
if [ -f "packaging/APKBUILD" ]; then
    sed -i "s/^pkgver=.*/pkgver=${TARGET_VERSION}/" packaging/APKBUILD
    sed -i "s/^pkgrel=.*/pkgrel=0/" packaging/APKBUILD
fi

# packaging/windows/scoop/brum.json
if [ -f "packaging/windows/scoop/brum.json" ]; then
    sed -i "s/\"version\": \".*\"/\"version\": \"${TARGET_VERSION}\"/" packaging/windows/scoop/brum.json
    sed -i "s/download\/v[^\/]*\//download\/v${TARGET_VERSION}\//g" packaging/windows/scoop/brum.json
    sed -i "s/v[0-9]\+\.[0-9]\+\.[0-9]\+.*\.zip/v${TARGET_VERSION}.zip/g" packaging/windows/scoop/brum.json
fi

# packaging/windows/winget
if [ -f "packaging/windows/winget/Woofson.Brum.yaml" ]; then
    sed -i "s/^PackageVersion: .*/PackageVersion: ${TARGET_VERSION}/" packaging/windows/winget/Woofson.Brum.yaml
fi
if [ -f "packaging/windows/winget/Woofson.Brum.installer.yaml" ]; then
    sed -i "s/^PackageVersion: .*/PackageVersion: ${TARGET_VERSION}/" packaging/windows/winget/Woofson.Brum.installer.yaml
    sed -i "s/download\/v[^\/]*\//download\/v${TARGET_VERSION}\//g" packaging/windows/winget/Woofson.Brum.installer.yaml
    sed -i "s/Brum_[0-9]\+\.[0-9]\+\.[0-9]\+[^_]*_/Brum_${TARGET_VERSION}_/g" packaging/windows/winget/Woofson.Brum.installer.yaml
fi

# ------------------------------------------------------------------------------
# 4. BUILD RELEASE PACKAGES
# ------------------------------------------------------------------------------
echo "📦 Compiling and building distribution packages..."
bash "${SCRIPT_DIR}/build-packages.sh"

# Extract binary sha256 for -bin package
BIN_TARBALL="brum-v${TARGET_VERSION}-linux-x86_64.tar.gz"
BIN_SHA256=$(grep "${BIN_TARBALL}" dist/SHA256SUMS | awk '{print $1}')
if [ -n "${BIN_SHA256}" ] && [ -f "packaging/brum-bin.PKGBUILD" ]; then
    sed -i "s/^sha256sums=('.*')/sha256sums=('${BIN_SHA256}')/" packaging/brum-bin.PKGBUILD
fi

# ------------------------------------------------------------------------------
# 5. GIT COMMIT, TAG & PUSH TO GITHUB
# ------------------------------------------------------------------------------
echo "🐙 Committing, tagging, and pushing v${TARGET_VERSION} to GitHub..."
git add -A
git commit -m "release: v${TARGET_VERSION}" || echo "No changes to commit"
git tag -fa "v${TARGET_VERSION}" -m "Release v${TARGET_VERSION}"
git push origin main --tags -f

# ------------------------------------------------------------------------------
# 5. FETCH SOURCE TARBALL SHA256 & SYNC PKGBUILD
# ------------------------------------------------------------------------------
echo "🔒 Calculating GitHub source tarball SHA-256 for AUR..."
sleep 2
SOURCE_URL="https://github.com/Woofson/brum/archive/refs/tags/v${TARGET_VERSION}.tar.gz"
SOURCE_SHA256=""
for attempt in {1..5}; do
    SOURCE_SHA256=$(curl -sL "${SOURCE_URL}" | sha256sum | awk '{print $1}')
    if [ -n "${SOURCE_SHA256}" ] && [ "${#SOURCE_SHA256}" -eq 64 ]; then
        break
    fi
    echo "Retrying tarball checksum fetch (${attempt}/5)..."
    sleep 2
done

if [ -n "${SOURCE_SHA256}" ] && [ "${#SOURCE_SHA256}" -eq 64 ]; then
    echo "Source SHA-256: ${SOURCE_SHA256}"
    sed -i "s/^sha256sums=('.*')/sha256sums=('${SOURCE_SHA256}')/" packaging/PKGBUILD
    git add packaging/PKGBUILD packaging/brum-bin.PKGBUILD
    git commit -m "chore(pkg): update source checksum for v${TARGET_VERSION}" || true
    git push origin main
else
    echo "⚠️ Warning: Could not verify remote source tarball SHA256. Using local fallback."
fi

# ------------------------------------------------------------------------------
# 6. AUR AUTOMATIC PUBLISHING
# ------------------------------------------------------------------------------
if [ "${SKIP_AUR}" = false ]; then
    echo "🏔️ Syncing to Arch User Repository (AUR)..."

    # 6.1 brum (source)
    AUR_SRC_DIR="/tmp/aur-brum-${TARGET_VERSION}"
    rm -rf "${AUR_SRC_DIR}"
    if git clone aur@aur.archlinux.org:brum.git "${AUR_SRC_DIR}"; then
        cp packaging/PKGBUILD "${AUR_SRC_DIR}/PKGBUILD"
        cat << SRCINFO_EOF > "${AUR_SRC_DIR}/.SRCINFO"
pkgbase = brum
	pkgdesc = Multi-Pane Web Environment (File Commander/Manager) - By Woofson
	pkgver = ${TARGET_VERSION}
	pkgrel = 1
	url = https://github.com/Woofson/brum
	arch = x86_64
	arch = aarch64
	license = MIT
	makedepends = cargo
	makedepends = rust
	makedepends = pkgconf
	makedepends = gtk3
	makedepends = webkit2gtk-4.1
	depends = glibc
	depends = sqlite
	depends = libssh2
	depends = openssl
	depends = ca-certificates
	depends = gtk3
	depends = webkit2gtk-4.1
	options = !lto
	source = brum-${TARGET_VERSION}.tar.gz::https://github.com/Woofson/brum/archive/refs/tags/v${TARGET_VERSION}.tar.gz
	sha256sums = ${SOURCE_SHA256}

pkgname = brum
SRCINFO_EOF
        (cd "${AUR_SRC_DIR}" && git add PKGBUILD .SRCINFO && git commit -m "release: v${TARGET_VERSION}" && git push origin HEAD:master)
        rm -rf "${AUR_SRC_DIR}"
        echo "✅ AUR 'brum' updated successfully!"
    else
        echo "⚠️ Skipping AUR 'brum' (SSH access not configured or clone failed)"
    fi

    # 6.2 brum-bin (pre-compiled binary)
    AUR_BIN_DIR="/tmp/aur-brum-bin-${TARGET_VERSION}"
    rm -rf "${AUR_BIN_DIR}"
    if git clone aur@aur.archlinux.org:brum-bin.git "${AUR_BIN_DIR}"; then
        cp packaging/brum-bin.PKGBUILD "${AUR_BIN_DIR}/PKGBUILD"
        cat << SRCINFO_BIN_EOF > "${AUR_BIN_DIR}/.SRCINFO"
pkgbase = brum-bin
	pkgdesc = Multi-Pane Web Environment (File Commander/Manager) - By Woofson (Pre-compiled standalone binary)
	pkgver = ${TARGET_VERSION}
	pkgrel = 1
	url = https://github.com/Woofson/brum
	arch = x86_64
	license = MIT
	provides = brum
	conflicts = brum
	depends = glibc
	depends = sqlite
	depends = libssh2
	depends = openssl
	depends = ca-certificates
	source = brum-v${TARGET_VERSION}-linux-x86_64.tar.gz::https://github.com/Woofson/brum/releases/download/v${TARGET_VERSION}/brum-v${TARGET_VERSION}-linux-x86_64.tar.gz
	sha256sums = ${BIN_SHA256}

pkgname = brum-bin
SRCINFO_BIN_EOF
        (cd "${AUR_BIN_DIR}" && git add PKGBUILD .SRCINFO && git commit -m "release: v${TARGET_VERSION}" && git push origin HEAD:master)
        rm -rf "${AUR_BIN_DIR}"
        echo "✅ AUR 'brum-bin' updated successfully!"
    else
        echo "⚠️ Skipping AUR 'brum-bin' (SSH access not configured or clone failed)"
    fi
fi

# ------------------------------------------------------------------------------
# 7. CRATES.IO AUTOMATIC PUBLISHING
# ------------------------------------------------------------------------------
if [ "${SKIP_CRATES}" = false ]; then
    echo "📦 Publishing to Crates.io..."
    if cargo publish; then
        echo "✅ Crates.io package published successfully!"
    else
        echo "⚠️ Crates.io publish returned a non-zero exit (check if version already exists or token needed)."
    fi
fi

# Final Cleanup
rm -f parubrum*.txt parucommanderdog*.txt
rm -rf /tmp/aur-* /tmp/deb-pkg /tmp/apk-pkg /tmp/brum-* /tmp/commanderdog-*

echo "======================================================"
echo "🎉 SUCCESS: Brum v${TARGET_VERSION} is released & published!"
echo "   - GitHub: https://github.com/Woofson/brum"
echo "   - AUR Source: https://aur.archlinux.org/packages/brum"
echo "   - AUR Bin:    https://aur.archlinux.org/packages/brum-bin"
echo "   - Crates.io:  https://crates.io/crates/brum"
echo "   - Local Packages in: ./dist/"
echo "======================================================"

