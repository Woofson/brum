# Cloud Storage (rclone)

> Jottacloud, Google Drive, OneDrive, Dropbox, Backblaze B2, pCloud and about 70 other services in Brum's panes and in Backup & Sync, through rclone.

Brum does not talk to each cloud service itself. It uses **[rclone](https://rclone.org)**, a free program that knows them all. Each account you connect is a **remote** with a name you choose, and opens in a pane at `rclone://name`.

Everything is set up in **Admin Control Panel → Cloud Storage**:

1. **Install** rclone (once per server), or use the one your system already has.
2. **Add a service**: Jottacloud has a guided setup; any other service is set up once with `rclone config`.
3. **Open** it in a pane, or choose **Cloud Storage** in a pane's location menu. `rclone://` lists all remotes.

---

## 1. Installing rclone

### The Install button

Admins see **Install from rclone.org**. Brum then:

* asks rclone.org which version is current (`https://downloads.rclone.org/version.txt`),
* downloads the build for this machine (Linux, Windows, macOS or FreeBSD; x86-64, ARM64 and others), about 25 MB,
* checks it against rclone's published **SHA-256 checksum** (`SHA256SUMS`) and throws it away if it does not match,
* keeps it in Brum's data folder: `rclone/bin/rclone`, next to the database.

**Check for updates** compares your version with rclone.org. When a newer one exists, **Update** installs it the same way. Nothing is installed without someone clicking.

### Using the rclone you already have

Brum finds rclone in this order:

1. `[rclone] cli_path` in `config.toml`
2. Brum's own copy (`rclone/bin/` in the data folder)
3. The system: `PATH`, `/usr/local/bin`, `/usr/bin`

The easiest way on Debian, Ubuntu and Arch Linux is the system package: `sudo apt install rclone` or `sudo pacman -S rclone`. The **Docker image** already includes rclone. The setup page shows which copy Brum uses and its version. System packages can be older than rclone.org's version. That is fine for many services, but not for all: Jottacloud needs a recent rclone (see below).

### Turning downloads off

On locked-down servers, stop admins from downloading rclone:

```toml
[rclone]
allow_download = false
cli_path = "/usr/bin/rclone"   # optional: a program you installed yourself
```

---

## 2. Adding services

### Jottacloud (guided)

1. Sign in on [jottacloud.com](https://www.jottacloud.com/web/secure) and open **Settings → Security**.
2. Make a **personal login token** and copy it.
3. In **Cloud Storage → Add a service**, choose a name (for example `jotta`), paste the token and click **Add Jottacloud**.

Brum hands the token to rclone, which exchanges it for a lasting login, and checks that the account can be read before keeping it. A token works **once** and only for a short time: if adding fails, make a new one. The token never appears in Brum's logs.

`rclone://jotta` opens your **Archive** in Jottacloud.

**Jottacloud needs a recent rclone.** rclone 1.60 (Debian 13's package) cannot sign in to Jottacloud any more: every token is refused with *invalid_grant*. rclone 1.76 works. Use the Install button, or a newer package.

### Every other service

Set them up once with rclone's own setup in a terminal on the server. It asks questions and opens a browser sign-in for services such as Google Drive, OneDrive and Dropbox:

* **Linux service, Docker, Windows service:** Brum keeps its own settings file (see below), so point rclone at it. The Settings file box on the setup page shows the exact command, for example:

  ```bash
  sudo rclone config --config /var/lib/brum/rclone/rclone.conf
  ```

* **Brum started as your own user:** just run `rclone config`.

New remotes show up in Brum by themselves; no restart is needed. See [rclone's list of services](https://rclone.org/overview/) and each service's page for what to answer. On a server without a browser, rclone explains how to finish the sign-in on another computer (`rclone authorize`).

### Testing and removing

Each remote has **Test** (shows storage used and free, where the service tells) and **Remove**. Removing forgets the login in rclone; the files stay with the service. Backups that use that remote stop working until it is added again.

---

## 3. Where logins are kept

rclone keeps each remote's login in its **settings file**. Anyone who can read that file can use the account, so:

| Brum runs as | Settings file |
| :--- | :--- |
| The Linux system service | Brum's own file, `/var/lib/brum/rclone/rclone.conf`, readable only by Brum (root) |
| Docker | Brum's own file in the data volume, `/data/rclone/rclone.conf` |
| The Windows service | Brum's own file in its data folder, `rclone\rclone.conf` |
| Your own user (`brum`, `brum service install`, desktop) | rclone's normal file for that user, shared with `rclone config` in a terminal |

Brum keeps its own file private (only Brum's user may read it) and puts rclone's cache next to it, never in the service account's home folder.

**Upgrading from 1.24 or older:** backups to `rclone://` places used the remotes of the account Brum runs as. On its first start with its own file, Brum copies those remotes into it once, so these backups keep working. Later changes in the old file are not copied again.

---

## 4. Using cloud storage in panes

* **Browse:** `rclone://` lists the remotes; `rclone://jotta/Archive/Photos` opens a folder. Backspace or `..` goes up, back to the list of remotes.
* **Open and view:** Brum downloads a private copy for viewing, kept up to an hour while the file is unchanged. Pictures in the icon grid show icons instead of previews, because every preview would download the whole file.
* **Upload:** drop files or folders on a cloud pane.
* **New folder, rename, delete** work as on disks. Deleting follows the service's own rules: Jottacloud, for example, moves deleted files to its trash, where they still count toward your storage until the trash is emptied on jottacloud.com.
* **Copy and move (<kbd>F5</kbd>/<kbd>F6</kbd>):** between cloud storage and this machine's disks, and between remotes. rclone moves the data itself, and inside one service the service copies or moves without downloading. Copies show in the task list, where they can be stopped.
* **Fleet machines:** a pane showing another Fleet machine shows that machine's remotes, set up on that machine.

**Not yet:** copying straight between cloud storage and SFTP, Samba, Proton Drive or another Fleet machine. Copy through a folder on this machine for now.

---

## 5. Backups to and from cloud storage

In **Backup & Sync**, write a cloud place as `rclone://remote/folder`, or pick a remote from the list. rclone copies the files itself, straight from place to place. See [Remote places in the Chewtoys manual](chewtoys.md) for the backup options.

---

## 6. Who may use cloud storage

A remote is **one account shared by the whole server**, so access is limited:

* **Administrators** may use every remote.
* **Other users** need an allowed folder (Admin → Users) that grants it:
  * `remote` or `rclone`: every remote,
  * `rclone://jotta`: only that remote,
  * `rclone://jotta/Shared`: only that folder in it.

Users without such an entry do not see Cloud Storage at all. Only administrators can add, test and remove remotes. Read-only users can browse but not change anything.

---

## 7. Troubleshooting

| Message | What to do |
| :--- | :--- |
| *rclone is not installed* | Install it in Cloud Storage, or install the system package. |
| *rclone has no remote by that name* | The remote was removed or renamed. Check Cloud Storage → Services. |
| *The service did not accept the saved login* | The login expired or was revoked at the service. Remove the remote and add it again. |
| *invalid_grant* while adding Jottacloud | The token was already used or has expired: make a new one and add it straight away. If fresh tokens are refused too, your rclone is too old for Jottacloud (1.60 is; 1.76 works): install or update it in Cloud Storage. |
| *Could not reach the service* | The server has no internet connection, or the service is down. |
