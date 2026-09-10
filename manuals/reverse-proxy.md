# 🌐 Reverse Proxy & Mesh VPN Guide (Tailscale, NetBird, Caddy, Nginx, Traefik)

Brum is built from the ground up to run seamlessly behind **any reverse proxy**, **mesh VPN**, or **Cloudflare Tunnel** with zero configuration required.

It supports:
- ✅ **Automatic WebSockets over HTTPS (`wss://`)** for the interactive PTY terminal.
- ✅ **Dynamic Viewport & Safe Area (`100dvh`)** for mobile browsers (Vivaldi, Chrome, Firefox, Safari).
- ✅ **Permissive Cross-Origin & Private IP Routing** (Tailscale `100.x.y.z`, NetBird `100.x.y.z`, LAN `192.168.x.x`, Localhost).
- ✅ **Large File Uploads** up to 10GB+ with streaming chunk processing.

---

## 🦎 1. Tailscale

Tailscale provides encrypted peer-to-peer WireGuard connections and automatic TLS certificates via MagicDNS.

### Option A: Tailscale Serve (Private to Tailnet with Automatic HTTPS)
Serve Brum securely to devices on your private Tailscale network with a signed Let's Encrypt HTTPS certificate:
```bash
# Run Brum in the background on port 3140
brum &

# Expose to your Tailnet over HTTPS
tailscale serve --bg 3140
```
Your Brum instance is now available at:
`https://<node-name>.<tailnet-name>.ts.net`

### Option B: Tailscale Funnel (Public Internet Access with HTTPS)
Expose Brum to the public Internet with full TLS and WebSocket proxying:
```bash
tailscale funnel --bg 3140
```

### Option C: Direct Tailscale IP Access
If you prefer direct IP connections over your Tailnet:
`http://100.x.y.z:3140`

---

## 🦅 2. NetBird

NetBird provides fast peer-to-peer overlay networking over WireGuard.

### Direct Access
Brum listens on `0.0.0.0:3140` by default. Once your host is joined to NetBird, access Brum directly from any peer machine:
`http://100.x.y.z:3140`

### With NetBird Routing Peer
If you have configured a NetBird Routing Peer with domain resolution (e.g. `commander.netbird.internal`), simply point your reverse proxy (Caddy/Nginx) to `localhost:3140`.

---

## 🔒 3. Caddy 2 (Recommended)

Caddy provides **automatic HTTPS certificates**, HTTP/2, HTTP/3, and built-in WebSocket proxying with zero configuration.

### `/etc/caddy/Caddyfile`
```caddyfile
brum.yourdomain.com {
    reverse_proxy localhost:3140
}
```

Reload Caddy:
```bash
sudo systemctl reload caddy
```

---

## 🌐 4. Nginx

When using Nginx, you **must** configure WebSocket upgrade headers and disable request buffering for large file uploads and real-time terminal streaming.

### `/etc/nginx/sites-available/brum`
```nginx
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

server {
    listen 80;
    listen [::]:80;
    server_name files.yourdomain.com;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name files.yourdomain.com;

    # SSL Certificates (Let's Encrypt / Certbot)
    ssl_certificate /etc/letsencrypt/live/files.yourdomain.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/files.yourdomain.com/privkey.pem;

    # Allow large uploads (10GB)
    client_max_body_size 10240M;

    location / {
        proxy_pass http://127.0.0.1:3140;
        proxy_http_version 1.1;

        # WebSocket Support (Required for Terminal)
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;

        # Standard Proxy Headers
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # Timeouts for Long-Running Terminal Sessions & Uploads
        proxy_read_timeout 86400s;
        proxy_send_timeout 86400s;
        proxy_buffering off;
    }
}
```

Enable and reload Nginx:
```bash
sudo ln -s /etc/nginx/sites-available/brum /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

---

## 🎛️ 5. Nginx Proxy Manager (NPM)

In the Nginx Proxy Manager Web UI:
1. **Details Tab**:
   - Domain Names: `files.yourdomain.com`
   - Scheme: `http`
   - Forward Hostname / IP: `127.0.0.1` (or container IP)
   - Forward Port: `3140`
   - Check: **Cache Assets**: OFF
   - Check: **Block Common Exploits**: ON
   - Check: **Websockets Support**: **ON** ⚠️ *(Crucial for Terminal)*
2. **SSL Tab**:
   - Select SSL Certificate (Let's Encrypt)
   - Check: **Force SSL**: ON
   - Check: **HTTP/2 Support**: ON
3. **Advanced Tab** (Custom Nginx Configuration):
   ```nginx
   client_max_body_size 10240M;
   proxy_read_timeout 86400s;
   proxy_send_timeout 86400s;
   proxy_buffering off;
   ```

---

## 🚦 6. Traefik (v2 / v3)

### Docker Compose Example
```yaml
version: '3.8'

services:
  brum:
    image: ghcr.io/woofson/brum:latest
    container_name: brum
    restart: unless-stopped
    volumes:
      - /:/host-root
      - ./data:/etc/brum
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.brum.rule=Host(`files.yourdomain.com`)"
      - "traefik.http.routers.brum.entrypoints=websecure"
      - "traefik.http.routers.brum.tls.certresolver=letsencrypt"
      - "traefik.http.services.brum.loadbalancer.server.port=3140"
      - "traefik.http.middlewares.cd-buffering.buffering.maxRequestBodyBytes=10485760000"
      - "traefik.http.routers.brum.middlewares=cd-buffering"
```

---

## ☁️ 7. Cloudflare Tunnel (`cloudflared`)

### `~/.cloudflared/config.yml`
```yaml
tunnel: <TUNNEL_ID>
credentials-file: /root/.cloudflared/<TUNNEL_ID>.json

ingress:
  - hostname: files.yourdomain.com
    service: http://localhost:3140
    originRequest:
      noTLSVerify: true
  - service: http_status:404
```

> **Note**: In your Cloudflare Dashboard, make sure **Network ➔ WebSockets** is toggled **ON** so terminal sessions connect cleanly.

---

## 🐘 8. Apache (httpd)

Ensure `mod_proxy`, `mod_proxy_http`, and `mod_proxy_wstunnel` are enabled:
```bash
sudo a2enmod proxy proxy_http proxy_wstunnel ssl
```

### `/etc/apache2/sites-available/brum.conf`
```apache
<VirtualHost *:443>
    ServerName files.yourdomain.com

    SSLEngine on
    SSLCertificateFile /etc/letsencrypt/live/files.yourdomain.com/fullchain.pem
    SSLCertificateKeyFile /etc/letsencrypt/live/files.yourdomain.com/privkey.pem

    # WebSocket Proxy for Terminal
    RewriteEngine on
    RewriteCond %{HTTP:Upgrade} websocket [NC]
    RewriteCond %{HTTP:Connection} upgrade [NC]
    RewriteRule ^/?(.*) "ws://127.0.0.1:3140/$1" [P,L]

    # HTTP Proxy
    ProxyPass / http://127.0.0.1:3140/
    ProxyPassReverse / http://127.0.0.1:3140/

    # Allow 10GB Uploads
    LimitRequestBody 10737418240
</VirtualHost>
```

---

## 🔍 Verification Checklist

Once deployed behind your reverse proxy or VPN, test:
1. **Web Dashboard**: Navigate to your domain or Tailscale/NetBird IP in browser.
2. **Interactive Terminal (`F4` or Slide-up)**: Verify that the bash/sh shell connects immediately over WebSocket (`wss://`).
3. **File Uploads**: Drag and drop large files to ensure the proxy's `client_max_body_size` is not rejecting uploads.
4. **Mobile Navigation**: Test on mobile browsers (Vivaldi, Chrome, Firefox) to confirm the dynamic viewport and Total Commander bottom bar sit comfortably above any bottom browser toolbars.
