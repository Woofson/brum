# Brum — Docker & Portainer Deployment Guide

> Complete walkthrough for deploying **Brum** with **Docker**, **Docker Compose**, and **Portainer Stacks** with persistent database storage.

---

## Why Run Brum in Docker?

- **Lightweight**: Statically linked musl binary on **Alpine Linux 3.20**, using only **~20 MB of RAM** when idle.
- **Batteries Included**: The image ships the programs Brum's tools use: FFmpeg and ImageMagick (Format Converter), Git, `smbclient` (SMB shares), NFS utilities, rclone (Proton Drive), SSH, 7-Zip and Bash.
- **Isolated & Secure**: Run in rootless/isolated containers with fine-grained host volume bind-mounts and `no-new-privileges: true`.
- **Multi-Host Storage Management**: Mount `/mnt/storage`, `/mnt/nas`, USB drives, or NFS directly into Brum.
- **Zero-Loss Upgrades**: Persistent `/data` volume keeps all SQLite databases, users, backup schedules, and configurations intact across image upgrades and container refreshes.

---

## 1. Quick Start via Docker CLI

Run Brum directly with persistent data and host storage:

```bash
docker run -d \
  --name brum \
  --restart unless-stopped \
  -p 3140:3140 \
  -e TZ=Europe/Oslo \
  -e PUID=1000 -e PGID=1000 \
  -e BRUM_DATABASE_PATH=/data/brum.db \
  -v brum_data:/data \
  -v /home:/mnt/home:rw \
  -v /mnt:/mnt/storage:rw \
  ghcr.io/woofson/brum:latest
```

> **Image Details**: All official Docker images (`:latest`, `:alpine`, `:v1.17.0`) are built natively on Alpine Linux 3.20 with static musl binaries.

Open `http://<SERVER_IP>:3140` in your browser:
- **Default Username**: `admin`
- **Initial Password**: Dynamically generated on first container start. View it by running `docker logs brum`, or read `/data/initial_admin_credentials.txt` (deleted after the first admin login).

---

## 2. Deployment via Docker Compose

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
      - PUID=1000                 # owner of your files on the host (run `id`)
      - PGID=1000
      - BRUM_PORT=3140
      - BRUM_BIND=0.0.0.0
      - BRUM_DATABASE_PATH=/data/brum.db
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

## 3. Running as a Normal User (PUID / PGID)

Since **v1.15.0** the container starts Brum as an ordinary user instead of `root`, so a bug in Brum can only reach what that user can reach.

| Variable | Default | Purpose |
| :--- | :--- | :--- |
| `PUID` | `1000` | User id Brum runs as |
| `PGID` | `1000` | Group id Brum runs as |
| `BRUM_RUN_AS_ROOT` | `false` | Set to `true` to keep the old behaviour (see below) |

- **Set `PUID`/`PGID` to the owner of your files.** Run `id` on the host as that user to find the numbers. Brum can then read and write the folders you mount, like any other homelab container.
- **Upgrading from an older image:** on first start Brum gives its own `/data` folder to `PUID:PGID` once (`chown`). Your mounted host folders are **never** changed; if Brum can't open one, give `PUID`/`PGID` access to it on the host.
- **When you need root:** logging in with system (PAM) accounts and the terminal switching to each user's own account only work as root. Set `BRUM_RUN_AS_ROOT=true` if you rely on either.
- Brum's home inside the container is `/data`, so logs and per-user settings now survive container updates (logs: `/data/.local/share/brum/logs`).

---

## 4. The Terminal in Docker

Brum's terminal opens a shell **inside the container**, not on the host. It runs as the Brum user (`PUID`), with `/data` as its home folder, and it sees `/data` plus every folder you mounted. That makes it a good place to edit files and work in your folders with `bash`, `nano`, `vi`, `git` and the other programs in the image.

The host's own files, programs and services are not visible from there. To work on the host, or on any other machine, SSH out of the container:

```bash
ssh you@192.168.1.10          # the host's address on your network
```

### SSH keys that survive updates

Because the terminal's home is `/data`, SSH keys and settings you create there are stored in `/data/.ssh` and kept across container updates and re-creation.

```bash
# In Brum's terminal, once:
ssh-keygen -t ed25519            # creates /data/.ssh/id_ed25519
cat ~/.ssh/id_ed25519.pub        # add this line to ~/.ssh/authorized_keys on the host
```

A short alias saves typing. Put this in `/data/.ssh/config`:

```
Host host
    HostName 192.168.1.10
    User you
```

After that, `ssh host` is enough.

**Reaching the host by name:** add this to the Brum service in `docker-compose.yml`, then use `ssh you@host.docker.internal`:

```yaml
    extra_hosts:
      - "host.docker.internal:host-gateway"
```

**Keep in mind:**
- Anyone who may open a terminal in Brum (admins only, by default) can use these keys. So can anyone who can read your `/data` backups. Use a key made just for Brum, and give it a passphrase if you want an extra lock.
- With `BRUM_RUN_AS_ROOT=true`, the terminal's home is `/root` inside the container, which is **not** kept across updates. Mount a volume at `/root/.ssh` if you use SSH keys in that mode.

---

## 5. Database Persistence & Container Migration

Brum automatically recognizes `/data` volumes:
1. **Auto-Path Resolution**: When `/data` exists, the server automatically defaults to `/data/brum.db` and scans `/data/config.toml`.
2. **Environment Overrides**:
   | Variable | Default | Purpose |
   | :--- | :--- | :--- |
   | `BRUM_DATABASE_PATH` | `/data/brum.db` (in container) | SQLite database storage path |
   | `BRUM_PORT` | `3140` | Listening HTTP port |
   | `BRUM_BIND` | `0.0.0.0` | Listening network interface |
   | `BRUM_JWT_SECRET` | *(auto-generated)* | Signing key for auth tokens |

   The older `CD_*` names (from Brum's CommanderDog days) still work; `BRUM_*` wins if both are set.

3. **Migrating / Backing Up Existing Databases**:
```bash
# Copy an existing brum.db into your Docker volume
cp brum.db ./data/brum.db

# Recreate the container
docker compose up -d --force-recreate
```

---

## 6. Deployment via Portainer Stacks

1. Open **Portainer** $\rightarrow$ Navigate to your Environment $\rightarrow$ **Stacks** $\rightarrow$ **Add stack**.
2. Name the stack: `brum`.
3. Select **Web editor** and paste the `docker-compose.yml` above.
4. Click **Deploy the stack**.
5. Access Brum on port `3140`.

---

## 7. Reverse Proxy Integration (Traefik, Nginx, Caddy)

### Caddy
```caddy
files.example.com {
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
