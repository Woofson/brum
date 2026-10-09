# Brum v1.6.2 — Code Audit Report

**Scope:** Rust backend (`src/`, ~41k lines), config, Dockerfile/systemd, CI workflows, dependency lockfile.
**Method:** Static read-through plus targeted greps and a scripted check of every route handler. Nothing was compiled or run (sandbox has no network), so each finding is "confirmed by reading the code" unless marked **verify**.
**Not reviewed in depth:** `frontend/app.js` (2 MB bundle, XSS surface unassessed), SFTP/SMB/S3/NFS/Proton backends, disk-image parsers, sync engine, trash, Tauri shell.

---

## Executive summary

The application has **no authentication middleware**. Each handler is expected to authenticate itself, and at least 60 of ~178 routes don't. Several of the unauthenticated ones are catastrophic: remote command execution, user creation with any role, and a config dump that leaks the JWT signing secret. Because the shipped `brum.service` runs as **root** and the Docker image mounts host directories read-write, any internet- or LAN-reachable instance should be treated as fully compromised until fixed.

Fix order: **C1 → C2 → C3 → C4**, then the High items. C1 alone (a router-wide auth layer) closes most of the list.

| # | Severity | Finding |
|---|----------|---------|
| C1 | Critical | No auth middleware; many routes completely unauthenticated |
| C2 | Critical | `POST /api/actions/run` is unauthenticated remote command execution |
| C3 | Critical | `GET /api/config` leaks JWT secret, admin password, OIDC client secret |
| C4 | Critical | Hardcoded default JWT secret and default `admin`/`brum` credentials |
| H1 | High | Any authenticated user (incl. `readonly`) can run shell commands |
| H2 | High | Tar extraction path traversal (tar-slip) |
| H3 | High | Plugin installer zip-slip check is ineffective |
| H4 | High | Path sandbox bypass via `"://"` anywhere in a path; no symlink resolution |
| H5 | High | OIDC login takes over existing local accounts by username; ID token unverified |
| H6 | High | Fleet proxy SSRF via `X-Fleet-Target-Url` |
| M1–M8 | Medium | Session/cookie handling, CORS, no headers, brute force, fail-open roles, etc. |
| P1–P5 | — | Performance findings |

---

## Critical

### C1. No authentication middleware; many unauthenticated routes
`server/mod.rs` builds the router with only body-limit, CORS, trace and compression layers (lines ~247–250). Auth is per-handler via `extract_claims_or_local` / `validate_path_access`. A scripted scan of all route handlers for any auth call flagged these as having none (I manually confirmed the ones marked ✔):

- ✔ `POST /api/auth/users` (create user, caller chooses `role`) and ✔ `DELETE /api/auth/users/:username`
- ✔ `POST /api/actions/run` (see C2)
- ✔ `GET /api/config` (see C3)
- ✔ `GET /api/tools/logviewer/tail` (arbitrary file read)
- ✔ `POST /api/tools/search` (filesystem search)
- ✔ `POST /api/tools/duplicates/clean` (file deletion)
- ✔ `POST /api/system/autostart` (writes autostart entries)
- ✔ `POST /api/tools/sync/profiles/:id/run`
- Flagged by the scan, **verify** each: `/api/tools/convert`, `/api/tools/metadata/{read,update,batch}`, `/api/tools/paranoid/dry-run`, `/api/tools/sync/profiles*`, `/api/tools/syncthing/*`, all `/api/tools/notedog/*`, `/api/notes/attachments*`, `/api/tasks*` (including cancel), `/api/fs/tags*`, `/api/system/users-groups`, `/api/manuals*`.

(Intentionally public routes such as health, OIDC login/callback and share links are fine.)

**Fix:** add one `axum::middleware::from_fn_with_state` layer that validates the token and injects `Claims`, applied to everything except an explicit allow-list (`/api/health`, `/api/system/status`, `/api/auth/login`, OIDC, `/api/public/*`, static assets). Then add role checks (admin-only) to user management, config, actions, autostart, sync, plugins. Add a test that enumerates routes and asserts a 401 without a token.

### C2. Unauthenticated remote command execution
`handle_run_action` (`server/mod.rs:6642`) takes only a JSON body — no `State`, no headers — and passes it to `ActionRunner::execute` (`tools/actions.rs`), which substitutes placeholders into `command` and runs `$SHELL -c <command>` in a client-supplied `working_dir`. Route: `POST /api/actions/run`.

Anyone who can reach port 3140 can run arbitrary commands as the server user (root under `brum.service`).

**Fix:** remove this endpoint; actions are already defined server-side in `config.toml` (`[[custom_actions]]`), so the client should send an action **id** and the server should look up the command. Never accept a command string from the client. Pass file names as separate argv entries rather than interpolating into a shell string.

### C3. `GET /api/config` returns the full config including secrets
`handle_get_config` returns `Json((*state.config).clone())`. `AppConfig` derives `Serialize` and no secret field has `skip_serializing`: `server.jwt_secret`, `auth.default_admin_pass`, `auth.oidc.client_secret`, plus Syncthing `api_key` and fleet node tokens. No auth required.

Combined with C4's token design, this gives an attacker a forged admin token in two requests, even if the operator changed every default.

**Fix:** return a dedicated, public-safe DTO; mark secret fields `#[serde(skip_serializing)]`.

### C4. Hardcoded JWT secret and default credentials
- `config/mod.rs:133`: default `jwt_secret` is the literal `"brum-super-secret-jwt-key-2026"`. The shipped `config.toml` doesn't override it, there is no startup warning, and nothing generates a random secret. Docker users who don't set `BRUM_JWT_SECRET` all share one key.
- Session JWTs are fully stateless: `verify_token` trusts `role`, `home_dir`, `allowed_roots` from the claims and only hits the DB for API tokens. A forged token with `role: admin` is accepted, and a deleted/disabled/demoted user's token stays valid until expiry (72 h, or the cookie's 30 days).
- Default `admin` / `brum` is created on first run and printed in the README. `allow_entire_system = true` ships by default, so that account has the whole filesystem.

**Fix:** on first start generate a random 256-bit secret and persist it (e.g. in the DB or a 0600 file); refuse to start if the secret equals the known default. Generate a random admin password on first run and print it once (or force change on first login). Look up the user in the DB on each request (or keep a token-version/revocation list).

---

## High

### H1. Any logged-in user can execute shell commands
`handle_open_with` (`/api/system/open-with`) and `handle_run_custom_action` (`/api/system/run-custom-action`) run `sh -c` on a client-supplied command string. They only call `validate_path_access(..., false)`, i.e. no admin check, and `is_write=false` means even `readonly` users pass. Quote-wrapping the substituted path (`"…"`) doesn't stop injection: file names can contain `"`, `$()` and backticks.

**Fix:** admin-only at minimum; preferably the same id-based, argv-style design as C2.

### H2. Tar extraction path traversal
`vfs/archive.rs::unpack_tar_with_strip` strips only leading `/`, does `target.join(entry_path)` and calls `entry.unpack(&out_path)`. `tar::Entry::unpack` does **not** validate the destination (`unpack_in` does). An entry named `../../etc/cron.d/x` is written outside the target; symlink entries are also materialised, enabling symlink-then-write escapes. Affects `.tar`, `.tar.gz`, `.tgz`, `.tar.bz2`. The zip path correctly uses `enclosed_name()`.
Also: no size/entry-count limits (decompression bombs), and zip extraction restores `unix_mode` verbatim (setuid/setgid bits survive when running as root). Fallback to `tar -xf` / `7z x` has the same no-limit behaviour.

**Fix:** use `entry.unpack_in(target)` (and reject symlinks/hardlinks escaping the root); mask mode with `& 0o777`; cap total bytes, entries and ratio.

### H3. Plugin installer zip-slip check does nothing
`plugins/mod.rs` ~line 350: `let outpath = dest_dir.join(rel_name); if !outpath.starts_with(&dest_dir)`. `Path::starts_with` compares components lexically and does not collapse `..`, so `dest_dir.join("../../x")` still "starts with" `dest_dir`. The code also uses `file.name()` (raw) rather than `enclosed_name()`. A crafted `.grr` can write anywhere the server user can. Gated by plugin-install permission (admin, `can_install_plugins`, or `allow_user_installs`).
Note also that installed plugins run JS in the main app origin, so a malicious plugin is effectively full account takeover for any user who opens it; the permission model should say so.

**Fix:** `file.enclosed_name()`, and canonicalise parent before writing.

### H4. Path sandbox bypasses
`validate_path_access` (`server/mod.rs:937`):
1. `if raw_path.contains("://") { return Ok(raw_path) }` returns the **un-normalised, unchecked** string for any path containing `://` anywhere, not just a leading scheme. Dispatch elsewhere uses `starts_with("sftp://")`, so a path like `/home/me/x://../../../etc/shadow` falls through to the local backend unvalidated. **verify** (it is only exploitable when the user isn't already unrestricted, but that is exactly the case the sandbox exists for). Remote schemes (`sftp://`, `smb://`, `nfs://`, `http(s)://` via the WebDAV branch) are also open to every user, which is an internal-network pivot/SSRF (see H6).
2. Checks are lexical only. Apart from the share handler, nothing canonicalises, so a symlink inside an allowed root pointing outside it is followed.
3. **Fail-open roles:** `allowed_roots` that fails JSON parsing defaults to `["*"]`; NULL in the DB defaults to `["*"]`; OIDC auto-provision and `handle_create_user` default to `["*"]`. With the shipped `allow_entire_system = true`, every non-admin user therefore gets the whole filesystem by default.
4. `{username}` is substituted into the home template unchecked in `resolve_effective_home`; a username `..` (which the OIDC sanitizer permits) resolves to the parent directory.

**Fix:** parse scheme with `starts_with` only, then validate; canonicalise (resolve symlinks) before the prefix check; default to least privilege (deny on parse failure, default non-admin roots to home only); validate usernames.

### H5. OIDC problems
- Identity is mapped to a local account purely by sanitised `preferred_username`. An SSO user named `admin` **logs in as the existing local admin**, and the code then rewrites that account's role from IdP claims. `email_verified` is never enforced.
- The ID token payload is base64-decoded with **no signature, `iss`, `aud` or `exp` check**, and the stored `nonce` is never compared (it's only used as a username fallback). Receiving it directly from the token endpoint over TLS mitigates much of this, but the checks are cheap and expected.
- Admin is granted if `is_superuser`, an `admin` role, the admin group, **or** `admin_group` is empty and `default_user_role = "admin"`.
- After SSO, the token is placed in the redirect URL (`/?token=…`) and a cookie (see M1).

**Fix:** key SSO users on `(issuer, sub)` in a separate column and refuse to link to pre-existing local users; validate the ID token with the JWKS and check `iss/aud/exp/nonce`.

### H6. Fleet proxy SSRF
`fleet.rs::resolve_target_node` accepts `X-Fleet-Target-Url` (and token) from the request when the node id isn't configured, so any authenticated user can make the server issue HTTP/WS requests to arbitrary URLs (cloud metadata `169.254.169.254`, localhost admin ports, internal hosts) and read the response.

**Fix:** only allow configured nodes; if ad-hoc nodes are required, restrict to admin and block loopback/link-local/private ranges after DNS resolution.

---

## Medium

- **M1. Token handling.** The SSO cookie `cd_token` has `SameSite=Lax`, no `HttpOnly`, no `Secure`, and `Max-Age` 30 days vs a 72 h JWT. The token is also in the redirect URL (history, logs, `Referer`), and accepted as `?token=` on WebSockets (logged by proxies). Use `HttpOnly; Secure; SameSite=Strict`, and a one-time code exchange instead of URL tokens.
- **M2. CORS is `allow_origin(Any)`, `allow_methods(Any)`, `allow_headers(Any)`.** Bearer-token auth limits the damage, but with cookie auth accepted as well and the unauthenticated endpoints above, restrict to the configured origin.
- **M3. No security headers anywhere** (no CSP, `X-Frame-Options`/`frame-ancestors`, `nosniff`, HSTS, `Referrer-Policy`). This matters for a file manager that renders user files, Markdown/Mermaid, plugin HTML and shared files. Serve user content with `Content-Disposition: attachment` or from a sandboxed origin.
- **M4. No rate limiting or lockout** anywhere (grep for rate/lockout/attempts is empty). Login, session-unlock, share-password and share-email verification are all brute-forceable, and each Argon2 verification is CPU/memory-heavy, so unauthenticated floods are also a DoS. Share passwords are also accepted as a URL query parameter (`?password=`) and so end up in logs.
- **M5. Session-unlock endpoint** (`~line 1640`): works with an expired token and, on failure for user X, silently retries the supplied password against the **default admin** account (an extra guessing path against admin). Error strings differ per failure (account disabled, bad hash format, etc.), enabling account enumeration. In standalone mode it issues an admin token with no password at all; make sure standalone binds to loopback only.
- **M6. Share tokens** are the first 16 hex chars of a UUIDv4 (~60 random bits; fine against guessing at low rates, weak without rate limiting). Use 128 bits from `OsRng`. Download counters increment before the transfer completes and aren't atomic with the limit check.
- **M7. Terminal.** Role gate and privilege drop exist and are reasonable, but there is no `Origin` check on the WebSocket upgrade (cross-site WebSocket hijacking is possible because cookies are accepted), and tokens in the query string get logged. `virtual users` default to denied, good.
- **M8. Deployment defaults.** `brum.service` runs as `User=root` with no sandboxing directives (`NoNewPrivileges`, `ProtectSystem`, `CapabilityBoundingSet`, …); `config.toml` binds `0.0.0.0` with `root_path="/"` and `allow_entire_system=true`; the Docker image runs as root with `/home` mounted `rw`; the image's `HEALTHCHECK` hits an unauthenticated status endpoint (fine). Ship safe defaults (bind `127.0.0.1`, non-root user, narrow roots) and make the exposed configuration opt-in.

## Low / hardening

- `smbclient -c <string>` is built from file names in `vfs/smb.rs`; names containing `;` or quotes may inject smbclient commands. **verify** escaping.
- `7z x <path>` / `tar` get user paths as arguments; prefix with `--` to prevent option injection.
- Vault KDF is `Argon2::default()` (19 MiB, t=2, p=1), the OWASP minimum; raise for at-rest containers. Random 96-bit AES-GCM nonces per block are acceptable but note the ~2^32-message safe limit per key for very large vaults. Notes use ChaCha20-Poly1305 + Argon2id with the same parameters (fine).
- `tools/actions.rs` honours `$SHELL`; use a fixed shell.
- 518 non-test `unwrap()/expect()` and 48 `unsafe` sites; a panicking handler on bad input is a cheap DoS. Audit the `unsafe` blocks (libc `getpwnam`, PAM, etc.).
- `jwt_secret` and other secrets live in a plain-text `config.toml`; support `*_FILE` env vars / secrets.
- CI: actions are pinned to major tags (`@v4`, `@stable`, `@v2`), not commit SHAs; `release.yml` publishes with a token, so pin and scope `permissions` per job. No `cargo audit`/`cargo deny` step, no clippy/test gate visible in the publish workflow.
- Dependencies looked current in `Cargo.lock` (tar 0.4.46, zip 2.4.2, openssl 0.10.81, tokio 1.53, rustls 0.23.43, axum 0.7.9, rusqlite 0.32.1). I couldn't query advisory databases offline, so run `cargo audit`. Two `bzip2` majors (0.4 and 0.5) are pulled in; `lopdf` 0.34 and the filesystem-image crates (`ntfs`, `ext4`, `backhand`, `fatfs`, `hadris-udf`) parse untrusted input and deserve fuzzing.

---

## Performance

- **P1. Blocking I/O on the async runtime.** `server/mod.rs` has ~46 `std::fs` calls but only 9 `spawn_blocking` uses. `std::fs::read`, directory walks, `search`, `duplicates`, hashing and `disk_usage` run directly in handlers (e.g. lines 5437, 5669, 6066, 7096, 7155) and will stall other requests on busy servers. Wrap in `spawn_blocking` or use `tokio::fs`.
- **P2. Whole-file reads into memory** (`tokio::fs::read` at 2695, 5323, 5350; `std::fs::read` at 5437, 6066, 5669) for downloads/previews/uploads. With `upload_max_size_mb` at 10–20 GB and `DefaultBodyLimit` set to match, a few concurrent requests can OOM the host. Stream with `ReaderStream`/`Body::from_stream` and cap preview sizes.
- **P3. One `std::sync::Mutex<rusqlite::Connection>`** shared by auth, notes, tags, backups and shares. Every DB call serialises and blocks a runtime worker; WAL and `busy_timeout` are set (good), but a connection pool (r2d2 / deadpool-sqlite) or a dedicated DB thread would remove the contention. Argon2 verification (CPU-heavy) should also run in `spawn_blocking`.
- **P4. Monolithic files hurt build time and review.** `server/mod.rs` is 8,086 lines / 330 KB and `auth/mod.rs` 2,758 lines; incremental compile and `cargo check` times suffer. Split by domain (auth, fs, shares, tools, plugins) and add the router-level layers once.
- **P5. Frontend payload.** `frontend/app.js` is 1.95 MB and `index.html` 605 KB, `app.css` 387 KB, embedded in the binary. Gzip/br/zstd compression is on, but there is no code splitting or `Cache-Control`/ETag strategy visible; minify, split per tool and add long-lived hashed asset caching. Also, `CompressionLayer` over already-compressed media/archives wastes CPU; exclude those content types.
- **P6. Directory listings** read and `stat` entire folders; `max_limit`/`is_truncated` exist, so ensure the limit is enforced server-side for huge folders and consider streaming/paged listings.

## Architecture / maintainability

- Centralise authz (middleware + a `Principal` extractor with `require_admin()` / `require_write()`), instead of 150+ handlers each remembering to check. This is the root cause of C1, C2, H1.
- Add API tests that fail for any route reachable without a token (the repo has 95 Rust unit tests and a Python API test file, but none caught this).
- Replace the pile of per-module "extract claims" copies (`extract_claims_or_local`, `extract_terminal_claims`, `extract_fleet_claims`, plugin installer's inline version) with one implementation; they already differ (the plugin installer only reads the `Authorization` header).
- Introduce a typed command/action model instead of shell strings; typed `Path` newtype for validated paths so unvalidated strings can't reach VFS calls.
- `GEMINI.md` and `.agents/` suggest AI-assisted development; the pattern of copy-pasted handlers is typical. Add `clippy -D warnings`, `cargo deny`, and a security review checklist to PRs.

---

## Suggested remediation order

1. **Today:** put the service behind a firewall/VPN, change the admin password, set a random `BRUM_JWT_SECRET`, remove or block `/api/actions/run` and `/api/config`.
2. **This week:** global auth middleware + admin-only routes (C1, H1), config DTO (C3), generated secrets/credentials (C4).
3. **Next:** tar-slip, plugin zip-slip, path validation/symlinks, OIDC account linking and ID-token validation, SSRF restrictions (H2–H6).
4. **Then:** cookie/CORS/CSP hardening, rate limiting, deployment defaults, performance items, CI hardening.
