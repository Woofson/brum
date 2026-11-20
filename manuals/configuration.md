# Settings and Configuration

You do not need to edit any file to set Brum up. Almost everything is set inside Brum, in plain words. This page explains where each kind of setting lives, how to change it, and what is still in `config.toml` for those who want it.

---

## Where Settings Live

| What | Where | Who |
| :--- | :--- | :--- |
| How Brum works for everyone: who can reach it, who can log in, which folders people see, safety, the Fleet and more | **Admin → Server Settings**, saved in Brum's database | Admins |
| Your own look and feel: theme, layout, icons, keys | **Settings** (the gear), per person | Everyone |
| What Brum needs before it can start: where its database is, and a few settings that are deliberately kept out of the page | **`config.toml`** | Whoever runs the machine |
| Settings for containers and services | **Environment variables** (`BRUM_PORT`, `BRUM_JWT_SECRET`, …) | Whoever runs the machine |

When the same setting is set in more than one place, this order decides (the last one wins):

1. Brum's built-in default
2. Server Settings
3. `config.toml`
4. An environment variable

So a setting written in `config.toml` always wins over Server Settings, and the page shows it locked, with the reason. Setups managed by hand, or by tools such as Ansible, keep working exactly as before.

### Where Are My Settings Kept?

Your own settings (theme, layout, column widths, pane names and colours, keys and the rest of **Settings**) are kept in two places:

1. **In your browser.** Each browser you use Brum in keeps a copy, so Brum opens with your look straight away, before it has heard from the machine.
2. **On the Brum machine**, in its database, under your account. This copy is what makes your settings follow you to every phone, tablet and computer you sign in on.

When you open Brum, the browser shows its own copy first and then takes the machine's copy. When you change a setting, only that one setting is sent to the machine. Other browsers pick it up the next time they open Brum or refresh.

A few things to know:

- **A brand-new Brum, or a new account, starts from the browser's copy.** The first time a browser signs in and the machine has nothing saved for you yet, the browser's copy becomes the machine's copy. That is why a theme or name from an older Brum at the same address can come back after a fresh install.
- **Each Fleet machine keeps its own copy.** Settings made on one machine do not follow you to another machine in the Fleet.
- **The name badge, the app title and the login page title are saved as your own settings for now**, not as the machine's. They will move to Server Settings later.

#### Starting over with default settings

Removing only one of the two copies is not enough: the other one fills it again. To start clean everywhere:

1. In every browser you use Brum in **except one**, delete Brum's site data. In most browsers: the padlock or site icon in the address bar, then **Cookies and site data** (or **Site settings**), then **Delete**. This signs you out there.
2. In the last browser, open **Settings → Account → Sync Between Devices** and click **Reset Server Defaults**. Then delete Brum's site data in this browser too.
3. Sign in again. Brum starts with its default settings, and they become your new copy on the machine.

Server Settings, Fleet machines and everything else an admin sets are not affected.

---

## Server Settings

Open the **Admin Control Panel** (avatar menu) → **Server Settings**.

* **Groups**: the settings are grouped as *Network & Access*, *Sign-in*, *Single Sign-On*, *Folders*, *Fleet*, *Terminal*, *Chewtoys*, *Defaults for New Users*, *Other Services* and *Logs & Records*. Every setting says what it does. **Find a setting…** searches all of them.
* **Changing**: switches, choices in words, a folder picker for folders, and a list editor for shared folders. Changes collect in a bar at the bottom with **Undo** and **Save**. Values are checked before anything is saved: a folder that does not exist, a port that cannot work or a badly written address is refused with a plain message, and nothing is saved.
* **When changes apply**: when Brum restarts. Saved changes are marked **After restart**, and **Restart Brum now** restarts it from the page (copies and other tasks in progress are interrupted).
* **Badges**:
  * **In config.toml** (lock): set in the file, which wins. Change it there, or remove it from the file to change it here.
  * **BRUM_…** (lock): set by that environment variable, which wins.
  * **Default**: changed here; click it to go back to Brum's default.
* **Passwords and keys** (single sign-on secret, Syncthing API key) are stored encrypted and never shown again; the field says **Saved. Type to replace it**.
* **Admins only**. Every change is written to the **Activity Log**, with who made it.

### Changing the Address or Port

*Can be opened from* and *Port* decide how Brum is reached, so a mistake could lock everyone out. Brum protects against that:

1. Before saving, Brum checks that the address belongs to this machine and that the port is free.
2. After **Restart Brum now**, the page follows Brum to the new port (when you open Brum directly at its port; behind a reverse proxy the page simply reloads).
3. Brum asks **Keep the new address?** Press **Keep it** within **two minutes**. If nobody does, for example because Brum can no longer be reached, Brum goes back to the previous address and port by itself and restarts.

A change saved without restarting at once is simply used the next time Brum starts.

### Settings That Stay in `config.toml`

Some settings cannot be changed in the page, on purpose:

| Setting | Why |
| :--- | :--- |
| `[server]` `database_path` | Brum needs it to find its database, where the other settings are kept |
| `[server]` `allow_web_config_edit` | It decides whether settings may be changed in the page at all (see below) |
| `[server]` `jwt_secret`, `jwt_secret_file` | The shared Fleet signing key; kept out of the database |
| `[server]` `enable_auth` | Switching logins off from a web page is too easy to do by mistake |
| `[auth]` `default_admin_user`, `default_admin_pass` | They only matter the very first time Brum starts |
| `[terminal]` `default_shell`, `user_shells`; `[proton]` `cli_path`; `[rclone]` `cli_path` | They name programs to run |
| `[plugins]` `directory`, `user_directory` | They decide where Chewtoys are loaded from |
| `[logging]` `directory` | Brum needs it before it starts |

**Settings only in the file**: `allow_web_config_edit = false` (or `BRUM_ALLOW_WEB_CONFIG_EDIT=false`) locks Server Settings and the web editor for `config.toml`. The page then shows the settings but cannot change them, for machines managed only over SSH or by configuration tools.

---

## Moving Settings from an Older `config.toml`

Brum 1.23 and older shipped a long `config.toml` with nearly every setting in it. Those settings all show as locked in Server Settings, because the file wins. To manage them in the page instead:

1. Open **Server Settings**. The note at the top says how many settings are written in the file.
2. Click **Move them into Brum**.

What happens:

* The settings the page can manage move into Brum's database, exactly as they were written.
* Fleet machines written in the file (`[[fleet.nodes]]`) move into Brum's list of machines, with their API tokens encrypted. They then show in **Fleet → Manage Fleet → Add machine** with **Edit** and **Remove**.
* `config.toml` keeps only what Brum needs before it starts, plus anything that differs from Brum's defaults that the page does not offer (for example Folder rules). Lines that only repeat Brum's defaults are left out.
* A copy of the file as it was is kept next to it: `config.toml.before-move-<date>`.
* Brum works exactly as before, and nothing needs a restart. Brum checks this before changing anything.

Nothing is changed when:

* Brum is not allowed to change the file: it may run as a different account than the file's owner, or the file may be mounted read-only (Docker). Make the file writable for Brum, or edit it by hand.
* A setting's value cannot be used as it is written, for example a shared folder that does not exist. That setting stays in the file; Brum lists it with the reason, and moves the others.

In Docker, the bundled file lives inside the container; the moved settings are kept in the database on your `/data` volume, so they survive updates of the container.

---

## The `config.toml` File

### Where Brum Looks

Brum uses the first file it finds:

1. The file named by the environment variable `BRUM_CONFIG` (also `CD_CONFIG`, `CONFIG_PATH`, `CONFIG_FILE`)
2. Your settings folder: `~/.config/brum/config.toml` (Linux), `%APPDATA%\brum\config.toml` (Windows)
3. Your local data folder: `~/.local/share/brum/config.toml` (Linux), `%LOCALAPPDATA%\brum\config.toml` (Windows)
4. Next to the program: `config.toml` or `brum.toml` (portable use, and the Windows installer's `C:\Program Files\Brum\config.toml`)
5. The current folder: `./brum.toml`, `./config.toml`
6. `/data/config.toml`, then `/etc/brum/config.toml` (packages and Docker)

Without a file, Brum uses its built-in defaults and Server Settings. The log line at start-up says which file is used, and how many settings saved in Brum are in use.

Custom themes are read from `themes` folders next to these (`~/.config/brum/themes/*.toml`, `/etc/brum/themes/*.toml`, `%LOCALAPPDATA%\brum\themes\*.toml`).

### The Short File New Installs Get

```toml
[server]
database_path = "brum.db"        # Where Brum keeps its database
allow_web_config_edit = true     # false: settings only in this file
# jwt_secret_file = "/etc/brum/jwt.secret"
# host = "0.0.0.0"               # Also possible here; a value here wins over Server Settings
# port = 3140

[plugins]
directory = "/etc/brum/plugins"
user_directory = "~/.config/brum/plugins"
```

Any setting from the tables below can be added, with the name in the *In config.toml* column; it then wins over Server Settings. Windows paths are easiest in single quotes: `path = 'D:\Media'`.

### Editing the File in Brum

**Admin → config.toml (Advanced)** shows the file in an editor. Passwords and keys (`jwt_secret`, `default_admin_pass`, `client_secret`, …) are shown as `********`; leaving `********` in place keeps the stored value when you save. Brum checks the file before saving, repairs Windows paths written with single backslashes, and reminds you that most changes take effect after a restart. `allow_web_config_edit = false` makes it read-only.

---

## All Server Settings

Each setting as it is called in Server Settings, its name in `config.toml`, and Brum's default.

### Network & Access

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Can be opened from | `[server]` `host` | Every network this machine is on (`0.0.0.0`) |
| Port | `[server]` `port` | `3140` |
| Name of this machine | `[server]` `server_name` | empty |
| Stay signed in for (hours) | `[server]` `session_duration_hours` | `72` |
| Largest upload (MB) | `[server]` `upload_max_size_mb` | `20480` |
| Extra web addresses Brum is opened at | `[server]` `allowed_origins` | none |
| Reverse proxies to trust | `[server]` `trusted_proxies` | none |

### Sign-in

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Accounts that can sign in | `[auth]` `mode` | Brum accounts and this computer's accounts (`mixed`) |
| PAM service | `[auth]` `pam_service` | `login` |
| Let guests look around without signing in | `[auth]` `allow_guest` | off |

### Single sign-on

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Use single sign-on | `[auth.oidc]` `enabled` | off |
| Name on the button | `[auth.oidc]` `provider_name` | `Authentik` |
| Issuer address | `[auth.oidc]` `issuer_url` | empty |
| Client ID | `[auth.oidc]` `client_id` | empty |
| Client secret | `[auth.oidc]` `client_secret` | empty |
| Return address | `[auth.oidc]` `redirect_url` | empty |
| What Brum asks for | `[auth.oidc]` `scopes` | `openid`, `profile`, `email`, `groups` |
| Make a Brum account at first sign-in | `[auth.oidc]` `auto_provision` | on |
| Group that makes someone an admin | `[auth.oidc]` `admin_group` | `brum-admins` |
| Everyone else becomes | `[auth.oidc]` `default_user_role` | A normal user (`user`) |
| Their own folder | `[auth.oidc]` `default_home_template` | as above |
| Only allow single sign-on | `[auth.oidc]` `force_sso_only` | off |
| Icon on the button | `[auth.oidc]` `button_icon` | `shield-check` |

### Folders

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Shared folders | `[storage]` `roots` | none |
| Admins can browse the whole machine | `[storage]` `allow_entire_system` | on |
| Top folder of the machine | `[server]` `root_path` | / (C:\ on Windows) |
| Personal folder for new Brum accounts | `[storage]` `default_user_home_template` | C:/Users/{username} on Windows, /data/users/{username} in Docker, otherwise /home/{username} |
| Make personal folders when missing | `[storage]` `auto_create_home_dirs` | on |
| When a personal folder cannot be used | `[storage]` `home_fallback_scheme` | Find the next best folder (`auto`) |
| Allow remote connections to this machine itself | `[storage]` `allow_remote_loopback` | off |
| Checking SFTP servers | `[storage.sftp]` `host_key_checking` | Remember each server the first time (recommended) (`tofu`) |

### Safety

The safety settings for everyone are on their own page, **Admin Control Panel → Security & Safety**, next to auto-lock. They are saved the same way as the other Server Settings.

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Deleted files go to the bin | `[paranoid]` `trash_enabled` | on |
| Bin folder | `[paranoid]` `custom_trash_dir` | empty (the system's bin) |
| Ask before deleting | `[paranoid]` `confirm_delete` | on |
| Ask before replacing files | `[paranoid]` `confirm_overwrite` | on |
| Check copies | `[paranoid]` `enabled` | on |
| Check after copying between machines | `[paranoid]` `verify_after_transfer` | on |
| How copies are checked | `[paranoid]` `checksum_algorithm` | SHA-256 (recommended) (`sha256`) |
| Save files safely | `[paranoid]` `atomic_writes` | on |
| Use Windows' own bin and copying | `[paranoid]` `windows_native_file_ops` | on |
| Say which program is using a file | `[paranoid]` `detect_locking_processes` | on |

### Fleet

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Work with other Brum machines | `[fleet]` `enabled` | on |
| Copies between machines at the same time | `[fleet]` `transfer_jobs` | `2` |

### Terminal

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Allow the terminal | `[terminal]` `enabled` | on |
| Roles that may use it | `[terminal]` `allow_roles` | `admin`, `root` |
| Brum accounts may use it too | `[terminal]` `allow_virtual_users` | off |
| Run as the signed-in person | `[terminal]` `drop_privileges` | on |

On Linux, `sudo` does not work in the terminal of the Brum service, on purpose. See [Admin Rights in the Terminal](#admin-rights-in-the-terminal-linux) for how to allow it.

### Chewtoys

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Allow Chewtoys | `[plugins]` `enabled` | on |
| People can install Chewtoys themselves | `[plugins]` `allow_user_installs` | off |
| Which Chewtoys are allowed | `[plugins]` `default_policy` | All of them (`allow_all`) |
| Allowed Chewtoys | `[plugins]` `global_whitelist` | `*` |
| Blocked Chewtoys | `[plugins]` `global_blacklist` | none |

### Defaults for New Users

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Panel layout | `[ui]` `default_layout` | Two panels side by side (`dual-vertical`) |
| Number of panels | `[ui]` `default_pane_count` | `2` |
| How files are listed | `[ui]` `default_view_mode` | Details (`details`) |
| Show hidden files | `[ui]` `show_hidden_files` | on |
| Browser tab title | `[ui]` `window_title` | empty |
| Login page title | `[ui]` `login_title` | empty (Brum's own) |
| Icon next to the clock | `[desktop]` `enable_tray` | on |

### Other services

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Use Syncthing | `[syncthing]` `enabled` | on when Brum finds Syncthing on the machine |
| Syncthing address | `[syncthing]` `url` | `http://127.0.0.1:8384` |
| Syncthing API key | `[syncthing]` `api_key` | found from Syncthing's own settings when possible |
| Admins can install the Proton Drive program | `[proton]` `allow_download` | on |
| Admins can install rclone (cloud storage) | `[rclone]` `allow_download` | on |
| Notes folder | `[notes]` `notes_folder` | `~/Notes` |

### Logs & Records

| Setting | In config.toml | Default |
| :--- | :--- | :--- |
| Keep an Activity Log | `[activity]` `enabled` | on |
| Keep entries for (days) | `[activity]` `retention_days` | `90` |
| How much Brum writes in its log | `[logging]` `level` | Normal (recommended) (`info`) |
| Write the log to a file | `[logging]` `file_enabled` | on |
| Show the log in the terminal window | `[logging]` `console_enabled` | on |
| Start a new log file | `[logging]` `rotation` | Every day (`daily`) |
| Old log files to keep | `[logging]` `max_files` | `14` |

> Configs written before Brum 1.17 use `[notedog]` instead of `[notes]`. It is still read.

---

## Settings Only in `config.toml`

Besides the settings listed under [Settings That Stay in `config.toml`](#settings-that-stay-in-configtoml), these are set in the file (or elsewhere in Brum, as noted):

```toml
[storage.sftp]
# known_hosts_file = "~/.ssh/known_hosts"   # Where SFTP server keys are remembered

# Fleet machines written by hand. Easier: Fleet → Manage Fleet → Add machine, with a pairing
# code or "Add a machine by address"; those are kept in Brum and can be changed there.
# [[fleet.nodes]]
# id = "homelab-nas"
# name = "Homelab NAS"
# url = "http://100.64.0.15:3140"   # or urls = ["http://a:3140", "http://b:3140"]
# token = ""                        # empty: the shared signing secret; or an API token from that machine
# start_path = "/mnt/storage"
# read_only = false
# color_accent = "#f59e0b"
# tags = ["homelab", "nas"]
# [fleet.nodes.start_paths]         # Start folder per system the machine reports
# linux = "/home/bolt"
# windows = 'C:\Users\bolt'

# Folder rules (also in Admin → Folder Rules)
# [[access.rules]]
# path = "/srv/media"
# access = "read"

[themes]
default_theme = "amber-charcoal"    # Theme for people who have not chosen one

[ui]
show_hostname_badge = true          # The machine name in the header
hostname_badge = ""                 # Its text (empty: the machine's name)
hostname_color = "slate"            # amber, emerald, sky, purple, rose, orange, cyan, slate
hostname_style = "text"             # text, subtle, solid, outline, pill, glow
hostname_icon = "none"              # server, hard-drive, cpu, terminal, cloud, shield, box, home, globe, radio, none
hostname_size = "md"                # sm, md, lg
show_global_refresh = false

# Right-click actions, "Open with" rules and global bookmarks have built-in defaults;
# add [[custom_actions]], [[open_with]] or [[bookmarks]] entries to replace them.
```

---

## Admin Rights in the Terminal (Linux)

Brum's Linux service is locked down by design. Its start-up file (`brum.service`) contains two protections, and everything Brum starts inherits them, the terminal included:

* **`NoNewPrivileges=true`**: no program started by Brum can gain more rights than it was started with. `sudo`, `su` and other programs that switch to another account stop with *"The 'no new privileges' flag is set"*. This also stops other programs that need such rights, for example `fusermount` (mounting with `sshfs` or `rclone mount`), `passwd` and `pkexec`.
* **`ProtectSystem=true`**: the system folders `/usr`, `/boot` and `/efi` are read-only for Brum and everything it starts, even for root. Installing or updating software from a Brum terminal fails, even with `sudo`.

With the default settings the terminal runs as the signed-in person's own Linux account (`drop_privileges`), so it can do what that account can, no more. That is the safe default, and it stays the default. If you want to do admin work from a Brum terminal, you can lift these protections yourself on the machines where you want that.

### Allowing `sudo` in the terminal

In a normal terminal or SSH session on that machine (not inside Brum):

```bash
sudo systemctl edit brum
```

An editor opens. Add these three lines in the space it marks, between the two comment lines near the top, then save and close. The first line, `[Service]`, is needed: without it the other two are silently ignored.

```ini
[Service]
NoNewPrivileges=false
ProtectSystem=false
```

Then restart Brum and open a new Brum terminal (terminals that are already open keep the old rules):

```bash
sudo systemctl restart brum
```

Check that the change took:

```bash
systemctl show brum -p NoNewPrivileges -p ProtectSystem
```

It should say `NoNewPrivileges=no` and `ProtectSystem=no`. If it still says `yes`, run `sudo systemctl edit brum` again and check that `[Service]` is there and that the lines are not below the comment *"Lines below this comment will be discarded"*.

`sudo` now works in the Brum terminal as it does over SSH: for accounts allowed to use it, and with that account's password. Your change is kept in its own file (`/etc/systemd/system/brum.service.d/override.conf`), so Brum updates do not undo it.

**To undo it:** `sudo systemctl revert brum`, then `sudo systemctl restart brum`.

### What you give up

* Anyone who can open a Brum terminal with an account that may use `sudo` can then do **anything** on the machine. A stolen or left-open Brum login becomes as powerful as an SSH login of that account.
* Brum itself runs as root. Without `ProtectSystem`, a flaw in Brum could change programs in `/usr`, not only Brum's own files.

On a home network, behind your own sign-in and never on the open internet, many people find that a fair trade. Decide per machine.

### Opening up less

You do not have to lift both protections:

* **Only `sudo`, system folders stay read-only:** add only `NoNewPrivileges=false`. `sudo` works, but installing software still fails.
* **Only some system folders writable:** keep `ProtectSystem=true` and name the folders you need, for example `ReadWritePaths=/usr/local`.
* **Only mounting with `sshfs` or `rclone mount`:** this also needs `NoNewPrivileges=false`, because `fusermount` has to gain rights for a moment.

Who may open a terminal at all is set under [Terminal](#terminal) in Server Settings: by default only administrators, and only with Linux accounts. Turning off "Run as the signed-in person" (`drop_privileges`) gives every allowed person a **root** shell, without a password; that is rarely what you want.

### Brum you run as your own user

A Brum installed with `brum service install` runs as your own account, from `~/.config/systemd/user/brum.service`. It has `NoNewPrivileges=true` as well, but no read-only system folders. To allow `sudo` there:

```bash
systemctl --user edit brum          # add: [Service] and NoNewPrivileges=false
systemctl --user restart brum
```

Undo with `systemctl --user revert brum`. A Brum you start by hand in a terminal has neither protection.

### `/tmp` is the real `/tmp`

Brum's service shares `/tmp` with the rest of the machine on purpose. Its terminal and panels see the same `/tmp` as an SSH login, so:

* files you save in `/tmp` from Brum stay there when Brum restarts, and other programs can see them;
* `tmux` and `screen` sessions started in a Brum terminal keep working after a Brum restart, and you can attach to them from SSH.

Brum's own temporary files get random names that cannot be guessed, so sharing `/tmp` does not put them at risk.

**Upgrading from 1.24.1 or older:** the Debian and Arch Linux packages replace the start-up file for you; restart Brum afterwards. The LXC install script (`install.sh`) replaces it and restarts Brum itself. Two cases need one extra step:

* **You copied `brum.service` by hand** (from the download archive): copy the new one over it, then run `sudo systemctl daemon-reload` and `sudo systemctl restart brum`.
* **You used `brum service install`**: run it again, then `systemctl --user daemon-reload` and `systemctl --user restart brum`.

Until then Brum keeps its old private `/tmp`, which is emptied on every restart. Check with `systemctl show brum -p PrivateTmp` (add `--user` for the second case): it should say `PrivateTmp=no`.

**To give Brum its own `/tmp` again** on a machine, run `sudo systemctl edit brum` and add:

```ini
[Service]
PrivateTmp=true
```

Then run `sudo systemctl restart brum` (for `brum service install`, use `systemctl --user edit brum` and `systemctl --user restart brum`). Undo it with `sudo systemctl revert brum`. Doing this costs you three things:

* every Brum restart empties Brum's `/tmp`;
* `/tmp` in Brum's panels is not the `/tmp` other programs see;
* `tmux` and `screen` sessions started in a Brum terminal are cut off on every restart. They keep running, but nothing can reach them any more.

### Docker and Windows

* **Docker:** the terminal is a shell inside the container, not on the host, and the image has no `sudo`. The `no-new-privileges` line in `docker-compose.yml` is the same protection for the container. See [The Terminal in Docker](docker.md#4-the-terminal-in-docker).
* **Windows:** none of this applies. The Windows service has its own rules; see the [Windows manual](windows.md).

---

## Stored Passwords & Tokens

Passwords and tokens Brum has to use again (bookmark and shared-mount logins, SFTP and Fleet logins kept with backups, Fleet tokens in user settings) are stored **encrypted** in the database. The key is in **`brum-secrets.key`**, next to the database, readable only by Brum's user; Brum creates it on first start.

* A copy of the database on its own does not reveal them. Keep the key file with the database, but leave it out of copies of the database you share or upload.
* **Keep the key file.** Without it, stored passwords cannot be read: bookmarks and backups to remote places then ask for their password again.
* Containers and setups that keep secrets outside the data folder can set `BRUM_SECRETS_KEY` (64 hex characters) instead; the key file is then not used.
* Passwords stored by older versions are encrypted automatically on the first start of this version.
* Without a configured signing secret (`BRUM_JWT_SECRET`, `BRUM_JWT_SECRET_FILE`, `jwt_secret` or `jwt_secret_file`), Brum signs logins with a secret derived from this key, so it is not kept in the database either. After updating from an older version, everyone on such an installation signs in again once, and API tokens made there must be created again. Installations with a configured secret are not affected.

## Initial Administrative Credentials & Seeding Protocol

`default_admin_pass` in `config.toml` operates under specific security rules:

1. **One-Time Database Seeding Only**:
   The `default_admin_pass` setting is evaluated **only** on the very first startup when the SQLite database (`brum.db`) contains zero users. Once seeded, user credentials are stored as Argon2 hashes in the database. Editing `default_admin_pass` in `config.toml` afterwards has **no effect** on existing accounts. Passwords are changed in Brum (avatar menu → **User Profile & Account Settings**), or the built-in admin gets a new one with `brum reset-admin-password`.
2. **Insecure Default Guard (`"brum"` vs Custom Password)**:
   * **If left as `"brum"` (or blank)**: Brum deliberately rejects `"brum"` as an insecure default and generates a cryptographically random 20-character password on startup. It prints the credentials to stdout (so `docker logs` shows them) and writes them to `initial_admin_credentials.txt` in the same folder as the database (`brum.db`). The password is never written to the log files. The note stays until the `admin` password is changed. Logging in with `admin` / `brum` will fail.
   * **If set to a custom password before first boot**: The configured custom password is used verbatim for the initial account.

## Browser Login Cookie

When you log in from the web page, Brum keeps the login only in a protected cookie (`HttpOnly`, `SameSite=Lax`, and `Secure` over HTTPS). Scripts on the page cannot read it, so it cannot be copied out of the browser even if something unwanted ever ran on the page.

* **One cookie per Brum**: the cookie is named after the port the browser uses (for example `brum_session_3140`), so two Brums on the same computer keep separate logins.
* **Scripts and tools are unchanged**: `POST /api/auth/login` still returns the login in its reply, and API tokens work as before with `Authorization: Bearer …`.
* **Upgrading**: a browser that still holds a login stored by an older Brum swaps it once for the protected cookie and then forgets the old copy. Nobody has to log in again.

---

## Fleet Federation, Defaults & User Overrides

When connecting multiple Brum instances across local subnets or overlay mesh networks (Tailscale, NetBird, WireGuard):

### 1. Shared Cluster Secret (`BRUM_JWT_SECRET` / `BRUM_JWT_SECRET_FILE`)
To federate nodes without managing static per-node API tokens:
* Give every node the identical signing secret (make one with `openssl rand -hex 32`). Brum takes the first of these it finds:
  1. `BRUM_JWT_SECRET`: the secret itself, in an environment variable (for example in `/etc/brum/brum.env`, `chmod 600`).
  2. `BRUM_JWT_SECRET_FILE`: the path of a file holding the secret. Suits Docker secrets (`/run/secrets/brum_jwt`) and keeps the secret out of process listings and `docker inspect`.
  3. `jwt_secret` in `config.toml`: the secret itself.
  4. `jwt_secret_file` in `config.toml`: the path of a file holding the secret. The config file, and the web config editor, then only show the path.
  5. `fleet-secret.key` next to the database: written when this machine joined a Fleet with a pairing code (see below).
  6. None of these: a node-local secret derived from `brum-secrets.key`. Do not copy that key file between machines to share a secret; it also unlocks that machine's saved passwords.
* **Secret files**: spaces and line breaks around the secret are ignored. If a named file is missing, unreadable or empty, Brum refuses to start instead of quietly using a node-local secret, which would split the Fleet's logins.
* **Checks at start-up**: the log says where the secret came from, never the secret. Brum warns when it is shorter than 32 characters. On Linux and macOS it warns when other users can read the file (use `chmod 600`, or `640` with a group for Brum). On Windows it limits the file to Brum's own account, SYSTEM and Administrators.
* In `[[fleet.nodes]]`, set `token = ""` (or omit `token`).
* When `token` is empty, Brum's Gateway Hub automatically forwards the caller's active session JWT. Because both nodes share the master signing secret, the remote node validates the signature and authenticates the user instantly.
* **Static API Tokens**: If a remote node does *not* share the same secret key, create a static API Token on that node (**Admin Control Panel → API & Fleet Tokens**) and set `token = "<your_token>"` in `config.toml`.

### 2. Joining with a Pairing Code
The quickest way to add a machine, without copying secrets by hand:
1. On your main Brum (as admin): **Fleet → Add machine**. Brum shows an 8-character code (for example `9WBN-C4C7`) that works **once**, for **10 minutes**, with the commands to run.
2. On the new machine, after installing Brum, either:
   * in a terminal: `sudo brum join http://main-brum:3140 9WBN-C4C7` (Linux/macOS), or in PowerShell opened as administrator: `& "C:\Program Files\Brum\brum.exe" join http://main-brum:3140 9WBN-C4C7`. Add `--name nas` to choose the name it is listed under. Restart Brum there afterwards;
   * or in Brum on the new machine (as admin): **Fleet → Join a Fleet**, with the main Brum's address and the code. Brum restarts by itself.
3. The new machine receives the secret your main Brum signs logins with and keeps it in `fleet-secret.key` next to its database (readable only by Brum). Both machines list each other as Fleet machines, marked `paired`; nothing is written into `config.toml`.

What to know:
* **Trust**: a paired machine can sign logins that every machine sharing the secret accepts. Only pair machines you trust, as with a shared secret set by hand.
* **Accounts**: people need an account with the same name on each machine they use; the login is accepted, the account must exist.
* **Sessions**: nobody signs in again on the main Brum; on the new machine everyone signs in again once.
* **Codes**: a code is used up when a machine joins with it, and after 10 wrong codes in a short time every open code is cancelled. Making codes and joining are recorded in the activity log.
* **Unencrypted networks**: over plain `http`, the secret travels unencrypted. Pair over your own network or an overlay network (NetBird, Tailscale, WireGuard), or put Brum behind `https`.
* **A machine that already has a secret** (`BRUM_JWT_SECRET`, `jwt_secret`, ...) cannot join with a code; remove that setting first, or give it the Fleet's secret by hand.
* **Removing**: **Fleet → Add machine** lists the paired machines with **Remove**. That removes the machine from the list; it keeps the secret, so to shut a machine out completely, change the Fleet's secret and pair the others again.
* **Addresses**: the new machine tells the main Brum its name and the network address it uses towards it. A Brum that can only be opened from its own computer cannot be reached by others: in Server Settings, set *Can be opened from* to *Every network this machine is on*.

### 3. Config Defaults vs Fleet Manager Overrides
* **Automatic Ingestion**: Statically configured nodes in `config.toml` are automatically synced into the browser's UI (appearing in **Fleet Control** and the **Paw Menu**).
* **Auto-Syncing Updates**: Modifying a node in `config.toml` (e.g. changing `start_path` or `url`) automatically updates the browser on next load, avoiding stale cached paths.
* **User Overrides**: Users can customize a static node in the **Fleet Manager** (changing its start directory, display name, color accent, or routing mode) and click **Save Profile**. This tags the node with `user_override: true` and preserves personal preferences over `config.toml`.
* **One-Click Reset**: To discard custom overrides and revert to `config.toml` settings, click the amber **Reset** button (`rotate-ccw`) on the node card in Fleet Manager.

### 4. One Machine, Several Systems or Addresses
A machine that boots more than one system (for example Linux as `griffon` and Windows as `wingriff`) needs only one Fleet entry:
* **Addresses**: list them in `urls`. The existing `url` still works and counts as the first address. The hub uses the address that answered last; when it stops answering, the others are tried again in order. The answer is kept for a minute, so not every request tries every address.
* **Start folder per system**: under `[fleet.nodes.start_paths]`, give a folder for `linux`, `windows` or `macos`. Brum picks the one for the system the machine reports when it is pinged; `start_path` is used when none matches.
* **Folders from the other system**: a folder remembered while the machine ran one system (such as `C:\Users\bolt`) is not tried while it runs the other. The panel opens that system's start folder instead.
* **Fleet list and manager**: the entry shows a badge for the running system and which address is in use ("address 2 of 2"). In the Fleet manager, type several addresses separated by commas, and a start folder per system. Test Connection tries them in order and says which one answered.
* **Older saved entries**: if you had one entry per hostname, both are merged into the one entry from `config.toml` the next time the page loads.
* **Token**: one token (or the shared signing secret) for all addresses of the machine.

### 5. Copying and Moving Between Machines
With the two panes on different Fleet machines, **F5** (copy) and **F6** (move) are done by the Brum servers, not by the browser:
* **Direct**: when both panes show other machines, the receiving machine fetches the files straight from the sending one. If the two cannot reach each other (for example on different networks), the Brum you are connected to passes the files between them. The task list says which happened. Either way nothing goes through the browser.
* **Keeps going**: the copy runs in the task list of the Brum you are connected to. Closing or reloading the tab does not stop it; open Brum again to see it.
* **Pause, Resume, Stop, Retry**: from the task list. **Retry** copies again only the files that did not make it, and a large file that broke off continues where it stopped.
* **Checked**: with **Security & Integrity Checks** on (the shield in the header, on by default), every file's SHA-256 is worked out on each machine and compared before the copy replaces anything. Each file is written to a temporary `.name.brum-part` file first, so a broken copy never overwrites a good file. A move removes the original only after its copy is checked.
* **Folders**: whole folders are copied, empty ones included. A same-named item at the target is replaced, or kept with a new name such as `photo (1).jpg` when you choose Keep both.
* **How many at once**: *Copies between machines at the same time* in Server Settings → Fleet (`[fleet] transfer_jobs`, default `2`). Further copies wait their turn.
* **Logins**: each machine uses its usual Fleet login (its `token`, or the shared signing secret). For a direct copy, the receiving machine is given the login for the sending machine for that copy. Read-only machines (`read_only = true`) are never written to.
* **Older machines**: a machine still on Brum 1.22 or older does not understand these copy jobs. Brum then says so and copies through the browser as before; keep the tab open until it finishes. The same happens for places such as SFTP or Samba opened inside a pane.

### 6. Zero CORS Requirements on Gateway Proxy
When routed through the Fleet Gateway Hub (`/api/fleet/proxy/:node_id`), all communication is proxied server-to-server. Remote nodes **do not** require the coordinator's domain in their `allowed_origins` whitelist.

---

## Folder Rules

On each machine, admins can say who may read a folder, write in it, or not touch it: **Admin Panel → Folder Rules**, or `[[access.rules]]` in `config.toml`.

```toml
[[access.rules]]
path = "/srv/media"
access = "read"          # "read", "write" or "none"
roles = ["user"]         # or users = ["bob", "ann"]; neither means everyone

[[access.rules]]
path = "/srv/media/private"
access = "none"
```

* **Most specific wins**: for a file, the rule for the deepest folder above it that applies to the person decides; at the same folder, the stricter rule. Without a matching rule, the account's own folders decide as before.
* **Limit and open**: a rule can make a folder read only or closed for someone, and it can also open a folder outside their own folders (read, or read and write). A storage root marked read-only stays read-only.
* **Whole folders**: a folder that holds something closed to a person cannot be copied, downloaded, packed or backed up by them as a whole, and a folder holding something they may not change cannot be deleted, moved or renamed by them as a whole. Search and Compare leave such files out.
* **Admins** are not limited by rules (they can change them anyway).
* **Changes apply at once**, and adding or removing a rule is recorded in the activity log. Rules from `config.toml` show in the Admin Panel with a `config.toml` badge and are changed there.
* **Fleet**: someone signing in from another Fleet machine gets this machine's rules, and this machine's own settings for their account (home folder and allowed folders), not those of the machine where they signed in. In the Fleet manager, **Folder rules** shows a machine's rules (for admins there).

---

## Activity Log

Each Brum keeps a record of who changed which files on it: **Admin Panel → Activity Log** (admins only).

* **What is recorded**: saving, new folders, renaming, deleting (to the trash or for good), copying and moving (also between Fleet machines), uploads, packing and unpacking archives, permissions and owners, shares, the trash, user accounts and API tokens, server settings (changes, moves from `config.toml`, and new addresses kept or undone), and saving `config.toml` from the web editor. Each entry has the time, the person, what happened, the path and more details; failures are marked with their reason, refusals included.
* **Fleet**: a change made through the Fleet is recorded on the machine where it happens, under the person's name, so each machine has its own complete record. Removals sent by another Brum (backups, syncing) are recorded one by one; files arriving from another Brum are recorded as the copy or backup on the Brum that runs it.
* **Search and download**: search paths, actions and details, filter by person, days or failures only, and download what is shown as a CSV file for a spreadsheet (cells that a spreadsheet would run as formulas are made harmless).
* **Settings**: *Keep an Activity Log* and *Keep entries for (days)* in Server Settings → Logs & Records (default 90; `0` keeps everything). Entries are stored in Brum's database; recording never slows down the operation itself.
* **Not recorded**: reading, viewing and downloading files, and passwords (addresses with passwords are masked).

---

## Environment Variable Overrides Matrix

Environment variables win over Server Settings and `config.toml`. A setting set this way shows in Server Settings with the variable's name and a lock.

| Environment Variable | Overrides Key | Description | Example |
| :--- | :--- | :--- | :--- |
| `BRUM_PORT` / `PORT` | `server.port` | HTTP listening port | `3140` |
| `BRUM_BIND` / `BRUM_HOST` / `HOST` | `server.host` | Network interface binding | `0.0.0.0` or `127.0.0.1` |
| `BRUM_DATABASE_PATH` / `DATABASE_PATH` | `server.database_path` | SQLite database file location | `/var/lib/brum/brum.db` |
| `BRUM_HOSTNAME` | *(none)* | Server name shown in Brum (defaults to the machine's hostname) | `nas` |
| `BRUM_JWT_SECRET` / `JWT_SECRET` | `server.jwt_secret` | Master cluster JWT signing key | `64-hex-char-secret` |
| `BRUM_JWT_SECRET_FILE` | `server.jwt_secret_file` | File holding the signing key (used when `BRUM_JWT_SECRET` is not set) | `/run/secrets/brum_jwt` |
| `BRUM_SECRETS_KEY` | *(none)* | Key for stored passwords and tokens, instead of the `brum-secrets.key` file | `64-hex-char-key` |
| `BRUM_ALLOW_WEB_CONFIG_EDIT` | `server.allow_web_config_edit` | `false`: settings only in `config.toml` (Server Settings and the file editor are read-only) | `true` or `false` |
| `BRUM_CONFIG` | *(none)* | The `config.toml` to use | `/etc/brum/config.toml` |
| `BRUM_OIDC_ENABLED` | `auth.oidc.enabled` | Enable OpenID Connect SSO | `true` or `false` |
| `BRUM_OIDC_ISSUER_URL` | `auth.oidc.issuer_url` | OIDC IdP Discovery Issuer URL | `https://auth.domain.com/` |
| `BRUM_OIDC_CLIENT_ID` | `auth.oidc.client_id` | OAuth2 Client ID | `brum-app` |
| `BRUM_OIDC_CLIENT_SECRET` | `auth.oidc.client_secret` | OAuth2 Client Secret | `secret-string` |
| `BRUM_OIDC_REDIRECT_URL` | `auth.oidc.redirect_url` | OAuth2 Callback Redirect URL | `https://domain.com/api/auth/oidc/callback` |
| `BRUM_OIDC_ADMIN_GROUP` | `auth.oidc.admin_group` | IdP claim group mapped to admin | `brum-admins` |
| `BRUM_OIDC_FORCE_SSO` | `auth.oidc.force_sso_only` | Disable local password login form | `true` or `false` |
