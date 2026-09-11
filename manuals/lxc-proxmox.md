# 🐕 Brum — Proxmox VE & Linux LXC Deployment Guide

> Complete walkthrough for deploying **Brum** in Proxmox VE LXC containers or standalone Linux containers (Debian / Ubuntu / Alpine).

---

## 🌟 Why Brum in LXC?

- ⚡ **Ultra-Low Resource Footprint**: Written in Rust, idling at just **~20–30 MB of RAM** with zero runtime bloat.
- 📦 **Single Standalone Binary**: Web assets are embedded inside the executable via `rust-embed` — no Node.js or web server runtime required.
- 🔒 **Native PAM & SQLite Multi-User Auth**: Authenticate directly with Linux container users (`/etc/passwd` & `/etc/shadow`) or the built-in SQLite user database.
- 🗄️ **Direct ZFS / NFS / SMB Bind Mounts**: Seamlessly browse multi-terabyte host storage pools at raw native I/O speeds.
- 💻 **Integrated Slide-Up Linux PTY Terminal**: Run native `bash`/`sh` shell sessions inside the container directly from the browser (`Ctrl+\``).

---

## 🚀 Quick Start (Automated 1-Liner)

Inside your Debian / Ubuntu LXC container terminal, run:

```bash
curl -fsSL https://raw.githubusercontent.com/Woofson/brum/main/scripts/lxc-install.sh | bash
```

Once installed, Brum is running and enabled on boot:
- **URL**: `http://<CONTAINER_IP>:3140`
- **Default Credentials**: `admin` / `brum` (Change after first login!)

---

## 📦 Method A: Installation via Debian `.deb` Package

If you built or downloaded the `.deb` release package:

```bash
# 1. Install prerequisites
apt-get update && apt-get install -y ca-certificates tar bzip2 7zip

# 2. Install Brum
dpkg -i brum_0.8.5_amd64.deb # or dpkg -i brum_*_amd64.deb

# 3. Enable and Start Systemd Service
systemctl daemon-reload
systemctl enable --now brum.service

# 4. Check service status
systemctl status brum
```

---

## 🔧 Method B: Manual Step-by-Step Installation

```bash
# 1. Download and extract release tarball
wget https://github.com/Woofson/brum/releases/latest/download/brum-v0.8.5-linux-x86_64.tar.gz
tar -xzf brum-v0.8.5-linux-x86_64.tar.gz
cd brum-v0.8.5-linux-x86_64

# 2. Copy binary to system path
install -m 755 brum /usr/bin/brum
ln -sf /usr/bin/brum /usr/local/bin/brum

# 3. Setup configuration hierarchy
mkdir -p /etc/brum /data
cp config.toml /etc/brum/config.toml

# 4. Install systemd service
cp brum.service /usr/lib/systemd/system/brum.service 2>/dev/null || cp brum.service /etc/systemd/system/brum.service
systemctl daemon-reload
systemctl enable --now brum.service
```

---

## 🗂️ Proxmox Storage Bind-Mounting (Host ➔ LXC)

To give Brum access to host disks, ZFS pools, or NAS shares, add bind mounts in your Proxmox host configuration (`/etc/pve/lxc/<CTID>.conf`).

### 1. Privileged Container Bind Mount
Edit `/etc/pve/lxc/<CTID>.conf` on the Proxmox Host:

```ini
# Syntax: mp[index]: /host/path,mp=/container/mount/point
mp0: /tank/storage,mp=/mnt/storage
mp1: /mnt/pve/backups,mp=/mnt/backups
```

### 2. Unprivileged Container Bind Mount (Permissions / UID Mapping)
In unprivileged containers, `root` inside the container maps to `UID 100000` on the host. 

To give the container read/write access to host files:

**Option A: Set Host Permissions (Simplest)**
```bash
# On Proxmox Host:
chown -R 100000:100000 /tank/storage
chmod -R 775 /tank/storage
```

**Option B: UID/GID Passthrough Mapping in `/etc/pve/lxc/<CTID>.conf`**
```ini
mp0: /tank/storage,mp=/mnt/storage
# Map container root to host UID 0 or custom user ID
lxc.idmap: u 0 100000 1000
lxc.idmap: g 0 100000 1000
```

Restart the LXC container:
```bash
pct reboot <CTID>
```

---

## ⚙️ Reverse Proxy Setup (Nginx, Caddy, Traefik)

Brum uses WebSockets for the **Integrated Terminal (`/api/ws/terminal`)**. Ensure your reverse proxy passes `Upgrade` headers.

### Nginx Configuration

```nginx
server {
    listen 80;
    server_name files.lan.local;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl http2;
    server_name files.lan.local;

    ssl_certificate /etc/ssl/certs/brum.crt;
    ssl_certificate_key /etc/ssl/private/brum.key;

    client_max_body_size 10G;

    location / {
        proxy_pass http://192.168.1.150:3140;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # WebSocket support for slide-up terminal
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 86400;
    }
}
```

### Caddy Configuration

```caddy
files.lan.local {
    reverse_proxy 192.168.1.150:3140
}
```

---

## 👥 Multi-User Management in LXC

### Linux System Users (PAM Mode)
If PAM authentication is enabled in `/etc/brum/config.toml`, any Linux user created in the container can log in directly:

```bash
# Add Linux system user
adduser alice
usermod -aG sudo alice
```

### Built-in SQLite User Management
You can also manage independent web users directly from the **Settings (`F10`) ➔ Users Tab** inside the web interface without modifying host Linux accounts.

---

## 🔄 Service Control & Maintenance

| Command | Action |
| :--- | :--- |
| **`systemctl status brum`** | View service status and health |
| **`systemctl restart brum`** | Restart Brum daemon |
| **`systemctl stop brum`** | Stop the daemon |
| **`journalctl -u brum -f`** | Follow live application logs |

### Upgrading to New Version
```bash
# 1. Stop service
systemctl stop brum

# 2. Replace binary
cp new-brum /usr/local/bin/brum
chmod +x /usr/local/bin/brum

# 3. Start service
systemctl start brum
```

---

## 📜 License

MIT © Bolt J Woofson
