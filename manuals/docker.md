# 🐳 Brum — Docker & Portainer Deployment Guide

> Complete walkthrough for deploying **Brum** with **Docker**, **Docker Compose**, and **Portainer Stacks** with persistent database storage.

---

## 🌟 Why Run Brum in Docker?

- ⚡ **Ultra-Lightweight & Minimal**: Statically linked musl binary packaged on **Alpine Linux 3.20**, resulting in a tiny **~30 MB image footprint** and using only **~20 MB of RAM**.
- 🔒 **Isolated & Secure**: Run in rootless/isolated containers with fine-grained host volume bind-mounts and `no-new-privileges: true`.
- 🗄️ **Multi-Host Storage Management**: Mount `/mnt/storage`, `/mnt/nas`, USB drives, or NFS directly into Brum.
- 🔄 **Zero-Loss Upgrades**: Persistent `/data` volume keeps all SQLite databases, users, backup schedules, and configurations intact across image upgrades and container refreshes.

---

## 🚀 1. Quick Start via Docker CLI

Run Brum directly with persistent data and host storage:

```bash
docker run -d \
  --name brum \
  --restart unless-stopped \
  -p 3140:3140 \
  -e TZ=Europe/Oslo \
  -e CD_DATABASE_PATH=/data/brum.db \
  -v brum_data:/data \
  -v /home:/mnt/home:rw \
  -v /mnt:/mnt/storage:rw \
  ghcr.io/woofson/brum:latest
```

> 💡 **Image Details**: All official Docker images (`:latest`, `:alpine`, `:v0.7.2-rc1`) are built natively on Alpine Linux 3.20 with static musl binaries. Debian images have been retired.

Open `http://<SERVER_IP>:3140` in your browser:
- **Default Username**: `admin`
- **Default Password**: `admin123` *(Change immediately in Settings > User Profile & Security!)*

---

## 📦 2. Deployment via Docker Compose

### Step 1: Create `docker-compose.yml`
```yaml
version: '3.8'

services:
  brum:
    image: ghcr.io/woofson/brum:latest
    container_name: brum
    restart: unless-stopped
    ports:
      - "3140:3140"
    environment:
      - TZ=Europe/Oslo
      - CD_PORT=3140
      - CD_BIND=0.0.0.0
      - CD_DATABASE_PATH=/data/brum.db
    volumes:
      # Data & Configuration persistence (database, custom themes, persistent profiles)
      - ./data:/data
      - ./config.toml:/etc/brum/config.toml:ro
      
      # Host Storage / Shares (Adjust to your host paths)
      - /home:/mnt/home:rw
      - /mnt:/mnt/storage:rw
      - /media:/mnt/media:rw
      
    healthcheck:
      test: ["CMD-SHELL", "curl -f http://localhost:3140/api/system/status || exit 1"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s
    
    security_opt:
      - no-new-privileges:true
```

### Step 2: Start the Service
```bash
docker compose up -d
```

### Step 3: View Logs & Update
```bash
# View live logs
docker compose logs -f brum

# Upgrade to latest release without losing database or profiles
docker compose pull
docker compose up -d --force-recreate
```

---

## 💾 3. Database Persistence & Container Migration

Brum automatically recognizes `/data` volumes:
1. **Auto-Path Resolution**: When `/data` exists, the server automatically defaults to `/data/brum.db` and scans `/data/config.toml`.
2. **Environment Overrides**:
   | Variable | Default | Purpose |
   | :--- | :--- | :--- |
   | `CD_DATABASE_PATH` | `/data/brum.db` (in container) | SQLite database storage path |
   | `CD_PORT` | `3140` | Listening HTTP port |
   | `CD_BIND` | `0.0.0.0` | Listening network interface |
   | `CD_JWT_SECRET` | *(auto-generated)* | Signing key for auth tokens |

3. **Migrating / Backing Up Existing Databases**:
```bash
# Copy an existing brum.db into your Docker volume
cp brum.db ./data/brum.db

# Recreate the container
docker compose up -d --force-recreate
```

---

## 🎛️ 4. Deployment via Portainer Stacks

1. Open **Portainer** $\rightarrow$ Navigate to your Environment $\rightarrow$ **Stacks** $\rightarrow$ **Add stack**.
2. Name the stack: `brum`.
3. Select **Web editor** and paste the `docker-compose.yml` above.
4. Click **Deploy the stack**.
5. Access Brum on port `3140`.

---

## 🔒 5. Reverse Proxy Integration (Traefik, Nginx, Caddy)

### Caddy
```caddy
brum.example.com {
    reverse_proxy brum:3140
}
```

### Nginx Proxy Manager / Nginx
```nginx
location / {
    proxy_pass http://127.0.0.1:3140;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```
