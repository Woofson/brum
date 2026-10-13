# Reverse Proxy & Mesh VPN Guide (Tailscale, NetBird, Caddy, Nginx, Traefik)

Brum is built from the ground up to run seamlessly behind **any reverse proxy**, **mesh VPN**, or **Cloudflare Tunnel** with zero configuration required.

It supports:
- **Automatic WebSockets over HTTPS (`wss://`)** for the interactive PTY terminal.
- **Dynamic Viewport & Safe Area (`100dvh`)** for mobile browsers (Vivaldi, Chrome, Firefox, Safari).
- **Permissive Cross-Origin & Private IP Routing** (Tailscale `100.x.y.z`, NetBird `100.x.y.z`, LAN `192.168.x.x`, Localhost).
- **Large File Uploads** up to 10GB+ with streaming chunk processing.

---

## 1. Tailscale

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

## 2. NetBird

NetBird provides fast peer-to-peer overlay networking over WireGuard.

### Direct Access
Brum listens on `0.0.0.0:3140` by default. Once your host is joined to NetBird, access Brum directly from any peer machine:
`http://100.x.y.z:3140`

### With NetBird Routing Peer
If you have configured a NetBird Routing Peer with domain resolution (e.g. `commander.netbird.internal`), simply point your reverse proxy (Caddy/Nginx) to `localhost:3140`.

---

## 3. Caddy 2 (Recommended)

Caddy provides **automatic HTTPS certificates**, HTTP/2, HTTP/3, and built-in WebSocket proxying with zero configuration.

### `/etc/caddy/Caddyfile`
```caddyfile
files.yourdomain.com {
    reverse_proxy localhost:3140
}
```

Reload Caddy:
```bash
sudo systemctl reload caddy
```

---

## 4. Nginx

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

## 5. Nginx Proxy Manager (NPM)

In the Nginx Proxy Manager Web UI:
1. **Details Tab**:
   - Domain Names: `files.yourdomain.com`
   - Scheme: `http`
   - Forward Hostname / IP: `127.0.0.1` (or container IP)
   - Forward Port: `3140`
   - Check: **Cache Assets**: OFF
   - Check: **Block Common Exploits**: ON
   - Check: **Websockets Support**: **ON** *(Crucial for Terminal)*
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

## 6. Traefik (v2 / v3)

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

## 7. Cloudflare Tunnel (`cloudflared`)

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

> **Note**: In your Cloudflare Dashboard, ensure **Network ➔ WebSockets** is toggled **ON** so terminal sessions connect cleanly.

---

## 8. Apache (httpd)

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

## 9. Brum Fleet Gateway Hub (Reverse Proxy for LAN Nodes)

When managing a fleet of distributed Brum nodes across local subnets or private mesh networks, Brum includes a native **Fleet Gateway Hub Reverse Proxy**.

### Why use Fleet Gateway Hub?
* **Mixed-Content Prevention**: If you access your main Brum instance via HTTPS (`https://files.yourdomain.com`), modern browsers block direct HTTP requests to LAN nodes (`http://192.168.1.50:3140`). Fleet Gateway Hub transparently relays API calls and WebSockets through the main server.
* **Unified Single Ingress**: Access all nodes in your home lab or office fleet from outside the network through a single public domain or VPN IP without opening separate firewall ports for every node.
* **WebSocket Relaying**: Proxies PTY terminal sessions (`/api/fleet/proxy/:node_id/api/terminal/ws`) seamlessly.
* **Zero-CORS Client Simplification**: The client browser communicates strictly with the hub server. The hub's Rust backend connects to remote nodes as a server-side client, completely eliminating browser CORS cross-origin restrictions on remote nodes.

### Gateway Proxy Routing Modes
In Fleet Manager (**Fleet Control ➔ Manage Fleet Nodes**), each node supports three routing modes:
1. **Auto (Recommended)**: Automatically routes via Gateway Hub (`/api/fleet/proxy/:node_id`) if the current page was loaded from a public domain, HTTPS origin, or non-localhost address; falls back to direct connection when browsing locally on `localhost`.
2. **Gateway Proxy (`proxy`)**: Forces all REST API traffic and WebSocket terminal sessions through `/api/fleet/proxy/<node-id>`.
3. **Direct (`direct`)**: Directly connects the client browser to the remote node endpoint URL (requires remote node to whitelist the origin in `allowed_origins`).

### Cluster Federation via Shared JWT Secret (`BRUM_JWT_SECRET`)
To manage an entire fleet without generating individual API tokens for each node:
1. Generate a high-entropy secret (e.g. `openssl rand -hex 32`).
2. Deploy this secret across all fleet nodes in `/etc/brum/brum.env`:
   ```bash
   BRUM_JWT_SECRET="your-high-entropy-cluster-secret"
   ```
3. In `config.toml` on your fleet nodes (or in the Fleet Manager UI), leave `token = ""` for these nodes.
4. When you authenticate to the hub node, the JWT token generated by the hub is cryptographically valid across all nodes sharing the secret. When requests route through the Gateway Proxy, the hub forwards your authorization header, allowing seamless single sign-on access across the entire fleet.
5. **No `allowed_origins` Needed on Remote Nodes**: Because all communication passes server-to-server through `/api/fleet/proxy/:node_id`, remote nodes never see browser cross-origin requests. You do not need to configure `allowed_origins` on the downstream nodes when using Gateway Hub proxying.

---

## 10. Trusted Proxies & Rate Limiting (`trusted_proxies`)

When Brum runs behind a reverse proxy (e.g. Caddy, Nginx, Traefik, Apache), client requests arrive with the reverse proxy's socket address as the peer IP. To accurately attribute client IP addresses for brute-force rate limiting and share access logs, configure `trusted_proxies` in `config.toml`:

```toml
[server]
# Canonical server hostname used for origin matching (e.g. "files.yourdomain.com")
server_name = "files.yourdomain.com"

# List of trusted reverse proxy IP addresses or CIDR subnets allowed to forward client IPs
trusted_proxies = ["127.0.0.1", "::1", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"]

# Optional CORS and WebSocket origin whitelist (supports exact domains, subdomain wildcards, and overlay networks)
allowed_origins = [
    "https://files.yourdomain.com",
    "https://*.yourdomain.com",
    "https://node.tailscale.ts.net",
    "http://100.64.0.12:3140",
]
```

### Security Considerations:
- **Default Safe State**: When `trusted_proxies` is empty (the default), Brum completely ignores incoming `X-Forwarded-For`, `X-Real-IP`, and `X-Forwarded-Proto` headers, keying rate limits and cookie flags strictly by direct peer socket state. This prevents external clients from bypassing rate limits with spoofed headers.
- **Subnet & CIDR Support**: Both individual IP addresses (`"127.0.0.1"`, `"::1"`) and CIDR subnet blocks (`"172.17.0.0/16"`, `"10.42.0.0/16"`) are fully supported, simplifying Docker, Podman, and Kubernetes ingress deployments where proxy IPs dynamically allocate within container subnets.
- **Right-to-Left Traversal**: When connected from a configured trusted proxy or trusted subnet, Brum parses `X-Forwarded-For` from right to left, skipping trusted proxy hops and selecting the first untrusted IP address. This guarantees that client-forged header prefixes are discarded.
- **Origin Validation & Subdomain Wildcards**: When `allowed_origins` is specified, CORS and WebSocket connections are strictly constrained to matching origins. `allowed_origins` natively supports exact matches, wildcard ports (e.g. `https://*.domain.com:*`), and subdomain wildcards (e.g. `https://*.yourdomain.com`) with strict label boundary checking (rejecting lookalike domains like `evilyourdomain.com`). Local loopback (`localhost`, `127.0.0.1`, `[::1]`) and native desktop webview origins are automatically retained.
- **`server_name` Matching**: Setting `server_name = "files.yourdomain.com"` treats incoming Host and Origin headers for `http(s)://files.yourdomain.com` as internal same-origin traffic.
- **Overlay Networks (Tailscale & NetBird)**: RFC 6598 Shared Address Space (`100.64.0.0/10`) and IPv6 ULA (`fc00::/7`) overlay mesh IPs are recognized as safe local network addresses during host-matching origin checks.

### Diagnostic Connection Logging
When troubleshooting reverse proxy headers or origin rejections, Brum logs client IP resolution and trust decisions:
```bash
# Monitor live connection and trust validation logs on systemd
journalctl -u brum -f

# Or check the Brum log file directly
tail -f /var/log/brum/brum.log
```
If an untrusted proxy forwards headers, Brum logs a notice indicating that `X-Forwarded-For` was ignored because the connecting peer is not in `trusted_proxies`.

---

## Verification Checklist

Once deployed behind your reverse proxy or VPN, test:
1. **Web Dashboard**: Navigate to your domain or Tailscale/NetBird IP in browser.
2. **Interactive Terminal (Backtick or Slide-up)**: Verify that the bash/sh shell connects immediately over WebSocket (`wss://`).
3. **File Uploads**: Drag and drop large files to ensure the proxy's `client_max_body_size` is not rejecting uploads.
4. **Fleet Node Probing**: Probe remote fleet nodes to confirm Gateway Hub proxy routes resolve latency and status accurately.
5. **Mobile Navigation**: Test on mobile browsers (Vivaldi, Chrome, Firefox) to confirm the dynamic viewport sits comfortably above mobile bottom bars.
