# 🔒 Brum Transparent Encrypted Vaults Manual

> **Zero-Knowledge, RAM-Only Authenticated Encryption (AES-256-GCM & Argon2id)**

Brum Transparent Encrypted Vaults (`.cdvault` / `.cdv`) provide self-contained, password-protected virtual filesystem containers. Vaults decrypt files **on-the-fly entirely in volatile system RAM** without ever writing unencrypted temporary files or plaintext data to the physical disk.

---

## 📑 Table of Contents
1. [Core Security Architecture](#1-core-security-architecture)
2. [Creating an Encrypted Vault](#2-creating-an-encrypted-vault)
3. [Unlocking & Accessing a Vault](#3-unlocking--accessing-a-vault)
4. [In-Vault Operations & Transparent Editing](#4-in-vault-operations--transparent-editing)
5. [Auto-Lock Timers & Memory Purging](#5-auto-lock-timers--memory-purging)
6. [Portable Cloud & Backup Workflows](#6-portable-cloud--backup-workflows)
7. [REST API Reference](#7-rest-api-reference)

---

## 1. Core Security Architecture

```mermaid
graph TD
    UserPass["User Master Passphrase"] -->|Argon2id (16-byte Salt)| UserKey["256-bit Derived Key"]
    UserKey -->|AES-256-GCM Decrypt| MasterKey["256-bit Vault Master Key (In RAM Only)"]
    MasterKey -->|AES-256-GCM Decrypt Blocks| Plaintext["RAM-Only Decrypted Files (Zero Disk Leakage)"]
    MasterKey -->|AES-256-GCM Encrypt Blocks| EncryptedContainer["Single-File .cdvault Container on Disk"]
```

* **Authenticated Symmetric Cipher**: **AES-256-GCM** (Galois/Counter Mode) with a unique 96-bit random IV (nonce) and 128-bit authentication tag per data block.
* **Key Derivation Function (KDF)**: **Argon2id** (memory-hard, resistant to GPU/ASIC brute-force and side-channel timing attacks) with a cryptographically secure 16-byte random salt.
* **Two-Tier Key Hierarchy**:
  * Each vault container contains an independent 256-bit random Master Key.
  * The Master Key is encrypted using the user's derived Argon2id key.
  * When modifying passwords or re-wrapping keys, data blocks do not need full re-encryption.
* **Zero Disk Artifacts**:
  * Files read or edited inside a vault are decrypted directly in memory buffers.
  * No temporary files, unencrypted swap artifacts, or cleartext remnants are written to the host filesystem.

---

## 2. Creating an Encrypted Vault

You can create an encrypted vault anywhere in Brum (local storage, external drives, or remote mounts):

### Method A: Via Right-Click Context Menu
1. Navigate to the desired parent folder in any directory pane.
2. Right-click on empty space (or tap the **Actions** button on mobile).
3. Select **🔒 Create Encrypted Vault (.cdvault)...**.
4. In the modal:
   * Enter the destination file path (e.g. `/home/bolt/Documents/Confidential_2026.cdvault`).
   * Enter and confirm your master passphrase.
5. Click **Create Encrypted Vault**.

### Method B: Via Spotlight Quick-Switcher (<kbd>Ctrl+K</kbd>)
1. Press <kbd>Ctrl+K</kbd> anywhere in Brum.
2. Type `vault` or `create vault`.
3. Press <kbd>Enter</kbd> to open the creation dialog.

---

## 3. Unlocking & Accessing a Vault

Unlocked vaults are mounted seamlessly as a virtual filesystem under the `vault://` protocol scheme.

1. Locate any `.cdvault`, `.cdv`, or `.vault` file in the file browser (marked with the amber 🔒 **`shield-check`** badge).
2. **Double-click** or press <kbd>Enter</kbd> on the vault container.
3. In the **Unlock Encrypted Vault** dialog:
   * Enter the vault's master password.
   * Select an **Auto-Lock Inactivity Timer**:
     * `5 Minutes Inactivity`
     * `15 Minutes Inactivity` *(Default & Recommended)*
     * `30 Minutes Inactivity`
     * `1 Hour Inactivity`
     * `4 Hours Inactivity`
     * `Until Session Ends / Manual Lock`
4. Click **Unlock Vault** (or press <kbd>Enter</kbd>).
5. Brum derives the key using Argon2id, verifies the authentication tag, and navigates immediately into `vault:///path/to/my_vault.cdvault#`.

---

## 4. In-Vault Operations & Transparent Editing

Inside the vault, you have full access to Brum's suite of power tools:

* **Navigating Subfolders**: Browse virtual directories transparently (e.g. `vault:///path/to/my.cdvault#finance/taxes/`).
* **Live In-Memory Editing**: Open text files, Markdown, JSON, YAML, code, and config files with **EditorDog**. When pressing <kbd>Ctrl+S</kbd>, changes are encrypted in RAM with AES-256-GCM and written back atomically.
* **Creating Folders & Files**: Use <kbd>F7</kbd> to create virtual folders or right-click to create new documents inside the encrypted container.
* **Uploading Files**: Drag & drop files from your desktop or use the **Upload** button to encrypt external files directly into the vault.
* **Downloading & Streaming**: Download decrypted files or stream media directly through the browser without saving temporary copies on the server.
* **Deleting**: Remove files or folders inside the vault with <kbd>F8</kbd> or right-click $\rightarrow$ Delete.

---

## 5. Auto-Lock Timers & Memory Purging

Brum implements strict memory hygiene for open vault sessions:

1. **Inactivity Auto-Lock**:
   * Every access (read, write, list) updates the session's `last_accessed` timestamp.
   * If no requests occur within the configured duration (e.g. 15 minutes), the vault locks automatically and purges the 256-bit Master Key from RAM.
2. **1-Click Breadcrumb Lock**:
   * While browsing inside a vault, a red **[ 🔒 Lock ]** button appears in the breadcrumb bar next to the vault name.
   * Clicking **[ 🔒 Lock ]** purges the Master Key immediately and returns the pane to the parent directory.
3. **Session Lock Integration**:
   * Locking your overall Brum session (<kbd>Ctrl+Alt+L</kbd>) immediately invalidates all active vault keys in memory.

---

## 6. Portable Cloud & Backup Workflows

Because Brum vaults are self-contained container files (`.cdvault`), they are 100% portable:

* **USB & External Drives**: Copy your `.cdvault` container to any FAT32/exFAT/NTFS/ext4 drive.
* **Cloud Storage & Offsite Backup**: Upload `.cdvault` containers to AWS S3, Cloudflare R2, MinIO, SFTP, SMB, or Proton Drive without revealing folder names or file contents to third-party cloud providers.
* **Bit-for-Bit Verification**: Use Brum's built-in SHA-256 hash calculator (<kbd>Shift+F7</kbd>) to verify container integrity before and after transfers.

---

## 7. REST API Reference

For custom automation, CI/CD scripts, or headless integrations:

### 1. Create Vault
```http
POST /api/vault/create
Content-Type: application/json
Authorization: Bearer <TOKEN>

{
  "path": "/home/bolt/Documents/Secrets.cdvault",
  "password": "MyStrongMasterPassphrase123!"
}
```

### 2. Unlock Vault
```http
POST /api/vault/unlock
Content-Type: application/json
Authorization: Bearer <TOKEN>

{
  "path": "/home/bolt/Documents/Secrets.cdvault",
  "password": "MyStrongMasterPassphrase123!",
  "auto_lock_secs": 900
}
```

### 3. Check Vault Status
```http
GET /api/vault/status?path=/home/bolt/Documents/Secrets.cdvault
Authorization: Bearer <TOKEN>

Response:
{
  "unlocked": true,
  "path": "/home/bolt/Documents/Secrets.cdvault"
}
```

### 4. Lock Vault
```http
POST /api/vault/lock
Content-Type: application/json
Authorization: Bearer <TOKEN>

{
  "path": "/home/bolt/Documents/Secrets.cdvault"
}
```

### 5. File Operations inside Vault
* **List contents**: `GET /api/fs/list?path=vault:///home/bolt/Documents/Secrets.cdvault#finance`
* **Read file**: `GET /api/fs/read?path=vault:///home/bolt/Documents/Secrets.cdvault#passwords.txt`
* **Write file**: `POST /api/fs/write` with `{ "path": "vault:///home/bolt/Documents/Secrets.cdvault#passwords.txt", "content": "..." }`
* **Download file**: `GET /api/fs/download?path=vault:///home/bolt/Documents/Secrets.cdvault#tax_return.pdf`
* **Delete file**: `POST /api/fs/delete` with `{ "paths": ["vault:///home/bolt/Documents/Secrets.cdvault#old.txt"] }`

---

*Brum Transparent Encrypted Vaults — Engineered for Maximum Privacy & Simplicity.* 🐕🔒
