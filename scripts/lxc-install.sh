#!/usr/bin/env bash
# ==============================================================================
# Brum - LXC / Bare-Metal Automated Installer
# Supported: Debian 11/12, Ubuntu 22.04/24.04, Proxmox LXC Containers
# ==============================================================================

set -euo pipefail

echo "======================================================"
echo "Installing Brum on Linux / Proxmox LXC..."
echo "======================================================"

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

# 2. Setup directory hierarchy
echo "📁 Configuring /etc/brum and /data..."
mkdir -p /etc/brum /data /var/log/brum

# 3. Copy binary and configs
if [ -f "./target/release/brum" ]; then
    cp ./target/release/brum /usr/bin/brum
    chmod +x /usr/bin/brum
    # Fallback symlink for legacy /usr/local/bin paths
    ln -sf /usr/bin/brum /usr/local/bin/brum 2>/dev/null || true
    cp ./config.toml /etc/brum/config.toml
    if [ -f "./brum.service" ]; then
        cp ./brum.service /usr/lib/systemd/system/brum.service 2>/dev/null || cp ./brum.service /etc/systemd/system/brum.service
    fi
fi

# 4. Enable and start systemd service
if command -v systemctl >/dev/null 2>&1; then
    echo "⚙️ Enabling and starting systemd service..."
    systemctl daemon-reload
    systemctl enable --now brum.service
    echo "✅ Brum is running and enabled on boot!"
    echo "🌐 Access via: http://$(hostname -I | awk '{print $1}'):3140"
fi

