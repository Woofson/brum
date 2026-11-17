#!/usr/bin/env bash
# ==============================================================================
# Brum - LXC / Bare-Metal Automated Installer
# Supported: Debian 11/12/13, Ubuntu 22.04/24.04, Proxmox LXC Containers
#
# Works three ways:
#   - from the unpacked release archive:   ./install.sh
#   - from a built source checkout:        cargo build --release && sudo ./scripts/lxc-install.sh
#   - on its own (the one-liner):          curl -fsSL .../scripts/lxc-install.sh | bash
#     It then downloads the latest release archive from GitHub and checks its checksum.
# Running it again upgrades Brum and keeps your /etc/brum/config.toml.
# ==============================================================================

set -euo pipefail

REPO="Woofson/brum"

echo "======================================================"
echo "Installing Brum on Linux / Proxmox LXC..."
echo "======================================================"

if [ "$(id -u)" -ne 0 ]; then
    echo "❌ Please run this as root (for example with sudo)." >&2
    exit 1
fi

# 1. Install prerequisites
echo "📦 Installing system dependencies..."
apt-get update -qq
apt-get install -y -qq \
    curl \
    ca-certificates \
    tar \
    gzip \
    bzip2 \
    openssh-client

# Install 7zip (modern) or p7zip-full (legacy fallback)
if ! apt-get install -y -qq 7zip 2>/dev/null; then
    apt-get install -y -qq p7zip-full || true
fi

# 2. Find the files to install: source checkout, unpacked archive, or download
SCRIPT_DIR=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

WORK_DIR=""
cleanup() { if [ -n "${WORK_DIR}" ]; then rm -rf "${WORK_DIR}"; fi; }
trap cleanup EXIT

if [ -f "./target/release/brum" ] && [ -f "./brum.service" ]; then
    SRC="$(pwd)"
    BIN="${SRC}/target/release/brum"
    echo "📂 Installing from the source checkout in ${SRC}"
elif [ -n "${SCRIPT_DIR}" ] && [ -f "${SCRIPT_DIR}/brum" ] && [ -f "${SCRIPT_DIR}/brum.service" ]; then
    SRC="${SCRIPT_DIR}"
    BIN="${SRC}/brum"
    echo "📂 Installing from the release archive in ${SRC}"
elif [ -f "./brum" ] && [ -f "./brum.service" ]; then
    SRC="$(pwd)"
    BIN="${SRC}/brum"
    echo "📂 Installing from the release archive in ${SRC}"
else
    ARCH="$(uname -m)"
    if [ "${ARCH}" != "x86_64" ]; then
        echo "❌ Release downloads exist for x86_64 only (this machine is ${ARCH})." >&2
        echo "   Build from source instead: https://github.com/${REPO}" >&2
        exit 1
    fi
    TAG="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")"
    TAG="${TAG##*/}"
    case "${TAG}" in
        v[0-9]*) ;;
        *) echo "❌ Could not find the latest Brum release on GitHub." >&2; exit 1 ;;
    esac
    NAME="brum-${TAG}-linux-${ARCH}"
    BASE="https://github.com/${REPO}/releases/download/${TAG}"
    WORK_DIR="$(mktemp -d)"
    echo "⬇️  Downloading Brum ${TAG}..."
    curl -fsSL -o "${WORK_DIR}/${NAME}.tar.gz" "${BASE}/${NAME}.tar.gz"
    curl -fsSL -o "${WORK_DIR}/SHA256SUMS.txt" "${BASE}/SHA256SUMS.txt"
    echo "🔒 Checking the download..."
    (cd "${WORK_DIR}" && grep " ${NAME}.tar.gz\$" SHA256SUMS.txt | sha256sum -c --quiet -) || {
        echo "❌ The downloaded archive does not match its checksum. Nothing was installed." >&2
        exit 1
    }
    tar -xzf "${WORK_DIR}/${NAME}.tar.gz" -C "${WORK_DIR}"
    SRC="${WORK_DIR}/${NAME}"
    BIN="${SRC}/brum"
fi

# Installs a settings file only if it is not there yet. An existing file is kept, and a
# different shipped version is put beside it as <name>.new.
install_config() {
    local from="$1" to="$2"
    if [ ! -e "${to}" ]; then
        install -m 644 "${from}" "${to}"
    elif ! cmp -s "${from}" "${to}"; then
        install -m 644 "${from}" "${to}.new"
        echo "   Kept your ${to}; the shipped version is in ${to}.new"
    fi
}

# 3. Copy program, settings, themes and service
echo "📁 Installing Brum..."
install -m 755 "${BIN}" /usr/bin/brum
# Fallback symlink for legacy /usr/local/bin paths
ln -sf /usr/bin/brum /usr/local/bin/brum 2>/dev/null || true

mkdir -p /etc/brum
install_config "${SRC}/config.toml" /etc/brum/config.toml
if [ -d "${SRC}/themes" ]; then
    mkdir -p /etc/brum/themes
    for theme in "${SRC}"/themes/*.toml; do
        if [ -f "${theme}" ]; then
            install_config "${theme}" "/etc/brum/themes/$(basename "${theme}")"
        fi
    done
fi

install -m 644 "${SRC}/brum.service" /usr/lib/systemd/system/brum.service 2>/dev/null \
    || install -m 644 "${SRC}/brum.service" /etc/systemd/system/brum.service
# Data (/var/lib/brum), logs (/var/log/brum) and cache (/var/cache/brum) are created by
# systemd when the service starts.

# Configure PAM service for brum if /etc/pam.d exists and /etc/pam.d/brum is missing
if [ -d /etc/pam.d ] && [ ! -f /etc/pam.d/brum ]; then
    if [ -f /etc/pam.d/common-auth ]; then
        cat << 'PAMEOF' > /etc/pam.d/brum
# PAM configuration for Brum (Debian/Ubuntu)
@include common-auth
@include common-account
@include common-session
PAMEOF
    elif [ -f /etc/pam.d/login ]; then
        cp /etc/pam.d/login /etc/pam.d/brum 2>/dev/null || true
    elif [ -f /etc/pam.d/passwd ]; then
        cp /etc/pam.d/passwd /etc/pam.d/brum 2>/dev/null || true
    fi
fi

# 4. Enable and start (or restart, on an upgrade) the systemd service
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    if systemctl is-active --quiet brum.service; then
        echo "⚙️ Restarting Brum with the new version..."
        systemctl restart brum.service
    else
        echo "⚙️ Enabling and starting systemd service..."
        systemctl enable --now brum.service
    fi
    echo "✅ Brum is running and enabled on boot!"
    echo "🌐 Access via: http://$(hostname -I | awk '{print $1}'):3140"
fi
