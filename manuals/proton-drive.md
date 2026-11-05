# Proton Drive

> Use your end-to-end encrypted Proton Drive in Brum, through Proton's official command-line program.

Brum does not talk to Proton by itself. It uses **Proton's own program** (`proton-drive`), which does the encryption on your server, so your files stay end-to-end encrypted on the way to and from Proton.

Everything is set up in **Settings (<kbd>F10</kbd>) → Proton Drive**:

1. **Install** the program (admins, once per server).
2. **Sign in** with your Proton account (each Brum user, once).
3. Open `proton:///my-files` in a panel, or use **Open My files** in the setup.

---

## 1. Installing the program

### The Install button (recommended)

Admins see **Install from proton.me**. Brum then:

* reads Proton's official release list (`https://proton.me/download/drive/cli/version.json`),
* picks the build for this machine (Linux, Windows, macOS or FreeBSD; x64 or ARM64; the musl build on Alpine and in Brum's Docker image),
* downloads it from proton.me (about 95 MB) with a progress bar,
* checks it against Proton's **SHA-512 fingerprint** and throws it away if it does not match,
* keeps it in Brum's data folder: `proton/bin/proton-drive`, next to the database.

**Check for updates** compares your version with Proton's list. When a newer one exists, **Update** installs it the same way. Nothing is ever installed without someone clicking.

### Installing it yourself

Brum also finds a program you installed yourself, in this order:

1. `[proton] cli_path` in `config.toml`
2. Brum's own copy (`proton/bin/` in the data folder)
3. The system: `PATH`, `~/.local/bin`, `/usr/local/bin`, `/usr/bin`, `/opt/proton-drive`

Where to get it:

* **All systems**: [Proton's download page](https://proton.me/download/drive/cli/index.html) lists every build with its SHA-512 fingerprint. Make the file executable (`chmod +x proton-drive`).
* **Arch Linux**: the AUR package `proton-drive-cli-bin`.

The setup page shows which copy Brum uses and its version.

### Turning downloads off

On locked-down servers, stop admins from downloading the program:

```toml
[proton]
allow_download = false
cli_path = "/usr/local/bin/proton-drive"   # optional: a program you installed yourself
```

---

## 2. Signing in

Each Brum user signs in to **their own** Proton account; two users can link two different accounts.

1. Click **Sign in to Proton Drive**.
2. Brum shows Proton's sign-in link, a **Copy link** button and a **QR code**. Open the link on this computer, or scan the code with your phone. You sign in on Proton's own page; **Brum never sees your password**.
3. Brum notices by itself when you are done and shows **Signed in as …**.

An unfinished sign-in stops after 15 minutes. **Cancel** stops it at once.

**Sign out** ends the login with Proton and removes it from Brum. You can also end it in your Proton account settings (Security → Sessions).

### Where the login is kept

Brum keeps each user's Proton login in a private folder inside its data folder, `proton/users/<name>/`, readable only by the user Brum runs as. It uses the program's file store, so it works for a background service and in Docker, where there is no desktop keyring.

That file **is** the login, just like an rclone configuration file: anyone who can read Brum's data folder (root, or Brum's own user) could use it. Keep the data folder private, and include it in your backups only if you are happy for the backup to carry the login too. Without it you simply sign in again.

---

## 3. Using Proton Drive in panels

Proton Drive addresses start with `proton://`:

| Address | What it is |
| :--- | :--- |
| `proton:///` | The top level: My files, Devices, Shared with me, Shared by me, Photos, Albums, Trash |
| `proton:///my-files` | Your own files |
| `proton:///my-files/Projects` | A folder inside them |

Open one by typing it in a panel's path bar, from **Open My files** in the setup, or with **Connect → Proton Drive**.

### What you can do

| Action | In Proton Drive |
| :--- | :--- |
| **Copy / Move** (<kbd>F5</kbd> / <kbd>F6</kbd>) | Between a Proton panel and a local panel in either direction, or between Proton folders (Proton copies and moves those itself). Uploads and downloads run in **Tasks** and can be cancelled. |
| **New folder** (<kbd>F7</kbd>), **Rename** (<kbd>F2</kbd>) | As usual. Renaming to a name that is already taken is refused. |
| **Delete** (<kbd>F8</kbd>) | Moves items to **Proton's trash** (`proton:///trash`), where they can be restored on drive.proton.me. Deleting inside the trash removes items for good. |
| **View / open** (<kbd>F3</kbd>, double-click) | Brum downloads a private copy for viewing, kept for an hour while the file is unchanged. |
| **Download / upload in the browser** | Works like local folders; dropped folders are uploaded whole. |
| **Save** (editor) | Saves a new version; the previous one stays in Proton's version history. |

**When a file already exists**, *Overwrite* uploads a **new version** (the old one stays in Proton's version history), *Skip* leaves it, and *Rename* keeps both. Proton skips files whose content is already identical by itself.

**DeltaCopy** works between local folders only; for Proton Drive (and other remote storage) Brum uses the standard copy and says so in the task log.

### Limits

* Only **My files**, **Devices**, folders **shared with you** and the **trash** can be changed. The top level, Photos, Albums and *Shared by me* are for looking only.
* **Proton Docs and Sheets** cannot be downloaded or copied out; open them on drive.proton.me.
* Copying directly between Proton Drive and other remote storage (SFTP, SMB…) is not possible; copy through a local folder.
* **Grid view shows icons instead of picture thumbnails** in Proton folders: each thumbnail would mean downloading the whole picture.
* Each request goes through Proton's program and takes one to four seconds. Brum remembers each folder for 30 seconds and refreshes it at once after its own changes; changes made elsewhere (phone, web) show up within 30 seconds, or at once with the panel's refresh button.

**Who may use it:** admins, and users an admin allows `remote` or `proton` in their allowed places (Admin panel → Users).

---

## 4. Docker

Brum's image includes what Proton's Alpine build needs (`libstdc++`). Then:

1. Open **Settings → Proton Drive** and click **Install**. The program goes into `/data/proton/bin/`, so it survives container updates.
2. Click **Sign in**, and open the link or scan the code with your phone.

No extra volumes or environment variables are needed. To use a program you downloaded yourself, put it in the data volume (for example `./data/proton/bin/proton-drive`) and make it executable.

---

## Troubleshooting

| Problem | What to do |
| :--- | :--- |
| *The program was downloaded but does not start* | On Alpine without Brum's image, install `libstdc++` (`apk add libstdc++`). |
| *Could not reach proton.me* | The server needs internet access to `proton.me` (download) and `drive-api.proton.me` (files). |
| *Not signed in to Proton Drive* | The login ended (signed out elsewhere, or removed in Proton's session list). Sign in again in Settings → Proton Drive. |
| *Access denied: Proton Drive is restricted…* | An admin must allow `remote` or `proton` for your user. |
