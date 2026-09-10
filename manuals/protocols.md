# <img src="../assets/brum_commanderdog_legacy.webp" alt="Brum Logo" height="36" style="vertical-align: -6px; margin-right: 8px;" /> Remote Protocols & VFS Guide

Brum includes a zero-leakage, multi-protocol Virtual Filesystem (VFS) client engine directly integrated into the panel manager.

---

## Supported Protocols & Endpoints

| Protocol | URI Scheme | Description |
| :--- | :--- | :--- |
| **Local Filesystem** | `file://` or `/path` | Direct Linux/Windows native filesystem with PAM login |
| **SFTP / SSH** | `sftp://user@host:port/path` | Secure file transfer with key / password authentication |
| **Samba / Windows Shares** | `smb://user@host/share` | Native SMB/CIFS network share mounting (ports 445 / 139) |
| **NFS** | `nfs://host/export` | NFSv3 / NFSv4 automated export mounting and discovery |
| **Hetzner Storage Box** | `sftp://...` / `https://...` | Pre-configured fast preset for SFTP (port 23) and WebDAV |
| **WebDAV** | `webdav://` / `https://...` | Nextcloud, ownCloud, Synology, Apache/Nginx WebDAV |
| **S3 Cloud Object Storage** | `s3://bucket/path` | AWS S3, MinIO, Cloudflare R2, Backblaze B2, Hetzner S3 (SigV4) |
| **Proton Drive** | `proton://` | End-to-end encrypted cloud storage via CLI bridge |
| **Encrypted Vaults** | `vault://path.cdvault` | RAM-only Argon2id + AES-256-GCM zero-leakage containers |
| **Virtual Archives** | `archive://file.zip` | Direct browse inside `.zip`, `.tar.gz`, `.tar.bz2`, `.7z` |

---

## Zero-Leakage Credential Architecture

* **In-Memory Volatile Credentials**: Passwords, private keys, and API tokens are never saved into browser `localStorage`, session history, search bars, or DOM attributes.
* **Global Mounts**: Administrators can configure global shares in the Admin Panel and assign access permissions to specific users. Assigned shares automatically populate in the user's **Favorites / Bookmarks** menu.
