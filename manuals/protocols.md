# Remote Protocols & Virtual Filesystem (VFS) Guide

Brum includes a zero-leakage, multi-protocol Virtual Filesystem (VFS) client engine directly integrated into the panel manager.

---

## Supported Protocols & Endpoints

| Protocol | URI Scheme | Description |
| :--- | :--- | :--- |
| **Local Filesystem** | `file://` or `/path` | Direct Linux/Windows native filesystem with PAM or SQLite authentication |
| **Client Local VFS** | `client://` | Direct browser-side folder mounting via File System Access API with zero server data transfer |
| **SFTP / SSH** | `sftp://user@host:port/path` | Secure file transfer with SSH key or password authentication |
| **Samba / Windows Shares** | `smb://user@host/share` | Native SMB/CIFS network share mounting (ports 445 / 139) |
| **NFS** | `nfs://host/export` | NFSv3 / NFSv4 automated export mounting and discovery |
| **Hetzner Storage Box** | `sftp://...` / `https://...` | Pre-configured fast preset for SFTP (port 23) and WebDAV |
| **WebDAV** | `webdav://` / `https://...` | Nextcloud, ownCloud, Synology, Apache/Nginx WebDAV |
| **S3 Cloud Object Storage** | `s3://bucket/path` | AWS S3, MinIO, Cloudflare R2, Backblaze B2, Hetzner S3 (SigV4) |
| **Proton Drive** | `proton://` | End-to-end encrypted cloud storage through Proton's official program; see the [Proton Drive guide](proton-drive.md) |
| **Encrypted Vaults** | `vault://path.cdvault` | RAM-only Argon2id + AES-256-GCM zero-leakage containers |
| **Virtual Archives** | `archive://file.zip` | Direct browsing inside `.zip`, `.tar.gz`, `.tar.bz2`, `.7z`, `.iso`, `.sqsh` |
| **Virtual Disks & FOG Images** | `archive://disk.img` / `archive://fog_dir` | Native browsing & extraction for raw `.img`, VMDK, QCOW2, MBR/GPT partitions, FAT/Ext4/NTFS, and FOG Project Partclone (`.zst`/`.gz`) multi-partition directories |
| **Compressed Disk & CD Images** | `archive://old.iso.zst` | `.iso`, `.img`, `.raw`, `.dd` and `.bin` images compressed with zstd, gzip, xz or bzip2 (`.iso.zst`, `.img.gz`, `.img.xz`, `.iso.bz2`, …) |

### Compressed disk and CD images

Brum unpacks a compressed image once, with progress and a cancel button in **Tasks**, then looks at what is inside: a CD/DVD (ISO 9660 or UDF), a SquashFS image, a disk with partitions or a bare filesystem, or a partclone/FOG backup. Addresses keep the original file name, for example `old.iso.zst/docs/README.TXT`.

* **Space**: the unpacked copy needs about as much free space as the full image. It is kept in Brum's cache folder (`~/.cache/brum/partclone_cache` on Linux), so the image opens instantly next time. It is refreshed when the original file changes.
* **Cleanup**: copies nobody has opened for 7 days are removed automatically. Admins can change the number of days, see how much space is used, or clear everything under **Admin panel → Remote Storage & Mounts → Unpacked Image Cache**. 0 days keeps copies until they are cleared there.
* **Old zstd files**: zstd files made with the trial versions from before zstd 1.0 (2015–16) are not read. Images compressed with any later zstd open normally.
* A plain `.gz` file (for example `notes.txt.gz`) is not treated as an image; `.tar.gz` and the other archives open as before.

---

## Zero-Leakage Credential Architecture

* **In-Memory Volatile Credentials**: Passwords, private keys, and API tokens are never saved into browser `localStorage`, session history, search bars, or DOM attributes.
* **Global Mounts**: Administrators can configure global shares in the Admin Panel and assign access permissions to specific users. Assigned shares automatically populate in the user's **Favorites / Bookmarks** menu.
* **Streamed Cross-Node Pipelines**: Transfers between two remote nodes stream directly through memory pipes without storing intermediate temporary files on the host disk.
