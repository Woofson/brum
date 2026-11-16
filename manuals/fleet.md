# Several Machines (Fleet)

Brum calls your machines together a **Fleet**: a desktop, a NAS, a media PC, a server in a cupboard. Each machine runs its own Brum. The one you open in the browser, your **main Brum**, reaches the others for you, so every file on every machine is a pane away.

This page covers setting one up, what you can do with it, and who can do what. All settings are in the [Configuration Guide](configuration.md#fleet-federation-defaults--user-overrides).

---

## How it fits together

* **Each machine keeps its own files and its own rules.** Nothing is copied to a central place. When you open the NAS in a pane, your main Brum asks the NAS's Brum for the list, and the NAS's Brum decides what you may see.
* **One login for all machines.** The machines share a secret that signs logins, so a login on the main Brum is accepted by the others. Pairing codes hand out that secret for you.
* **Accounts with the same name.** A login is accepted, but the account must exist on the machine too. Admins and people who use several machines need an account with the same name on each one.
* **Only your private network.** Brum is made for your home network or an overlay network (Tailscale, NetBird, WireGuard). Never put any machine of the Fleet on the open internet.

---

## Setting up: a main Brum and two machines

The example uses a main Brum and two more machines, a NAS and a media PC. It takes about ten minutes.

### 1. Install Brum on every machine

Any of the ways in the [Installation Guide](../INSTALL.md) works, and machines may mix systems: Linux, Windows, Docker.

On the machines that join, Brum must listen on the network, not only on the machine itself:

* **Linux and Windows**: Brum listens on every network by default. If you changed that, open **Admin Control Panel → Server Settings → Who can reach this Brum?** and set **Can be opened from** to **Every network this machine is on** (or one network address, such as the machine's Tailscale or NetBird address), then **Restart Brum now**. Desktop mode (`brum -s`) only listens on its own computer; run Brum as a service on machines that join.
* **Docker**: nothing to do; `-p 3140:3140` already makes it reachable.

Let port `3140` through the machine's firewall from the main Brum.

### 2. Make a code on the main Brum

Sign in as admin, click the machine name at the top left (**Fleet Control**) → **Manage Fleet** → **Add machine**. Brum shows a code such as `9WBN-C4C7`, with the commands to run. The code works **once**, for **10 minutes**.

### 3. Join from the new machine

Pick one of these on the NAS:

* **Terminal** (Linux): `sudo brum join http://main-brum:3140 9WBN-C4C7 --name nas`, then restart Brum there. Without sudo, Brum stops and shows the command to run, because the service's data needs administrator rights. To join a Brum you run yourself (not the service), add `--user`.
* **PowerShell as administrator** (Windows): `& "C:\Program Files\Brum\brum.exe" join http://main-brum:3140 9WBN-C4C7 --name nas`, then restart Brum there.
* **Brum's own page** on the NAS (any system, and the way to go for Docker): sign in as admin, **Fleet Control** → **Manage Fleet** → **Join a Fleet**, type the main Brum's address and the code. Brum restarts by itself.

`main-brum` stands for the main Brum's address as the NAS sees it, such as `http://192.168.1.10:3140` or its overlay name. `--name` is the name the NAS is listed under; without it, the machine's own name is used.

The main Brum shows straight away that the NAS joined. Both machines now list each other as Fleet machines, marked `paired`.

### 4. Repeat for the media PC

Make a new code on the main Brum and join from the media PC the same way.

### 5. Use them

* **In a pane**: the paw button in a pane's toolbar (**Places & Fleet**) lists your machines; pick one to open it in that pane. The pane shows the machine's name next to the path.
* **Everywhere at once**: **Tools → All machines**.

### Adding machines by hand instead

Without a code, a machine can also be added by its address: **Manage Fleet → Add machine → Add a machine by address**, with its name, address(es), an API token made on that machine (or none, when all machines share one secret, set with `BRUM_JWT_SECRET` or `BRUM_JWT_SECRET_FILE`), start folders, colour, groups and read only. The token is kept encrypted. Such machines are changed or removed in the same list, and take effect at once. See [Shared Cluster Secret](configuration.md#1-shared-cluster-secret-brum_jwt_secret--brum_jwt_secret_file).

Machines can also still be written in `config.toml` under `[[fleet.nodes]]`; they show in the list locked. **Move these into Brum**, under them in Manage Fleet → Add machine (or **Admin Control Panel → Server Settings → Move them into Brum**), moves them into Brum's list, together with any other settings still in the file, and keeps a copy of the old file. Paired machines, machines added by address and machines from `config.toml` can be mixed.

---

## Working across machines

### Panes on different machines

Every pane can show a different machine, with up to four panes at once. Keys, tabs, views, viewers and the right-click menu work the same as on your own disks. Each machine can have a colour and tags, set in **Manage Fleet**.

### Copying and moving

With the two panes on different machines, <kbd>F5</kbd> (copy) and <kbd>F6</kbd> (move), and drag and drop, are done by the Brum servers, not by the browser:

* **Direct**: the receiving machine fetches the files straight from the sending one. When the two cannot reach each other, your main Brum passes the files between them. The task list says which happened.
* **Keeps going**: the copy runs on the main Brum. Closing or reloading the tab does not stop it.
* **Checked**: with **Security & Integrity Checks** on (the shield in the header, on by default), each file's SHA-256 is worked out on both machines and compared before the copy replaces anything. Files are written under a temporary name first, so a broken copy never overwrites a good file. A move removes the original only after its copy is checked.
* **Pause, Resume, Stop, Retry**: in the task list. **Retry** copies only what did not make it, and a large file continues where it stopped.

The details are in [Copying and Moving Between Machines](configuration.md#5-copying-and-moving-between-machines).

### Compare and sync

**Tools → Compare → Two folders** compares folders on any two machines: pick the machine for each side, or click **Use my panels**.

* **Quick or certain**: **Compare** looks at sizes and dates; **Compare contents** also compares checksums on each machine.
* **Make one side like the other**: preview what a one-way sync would copy, replace and remove, then run it, or save it as a scheduled backup in Backup & Sync.

### All machines

**Tools → All machines** works on several machines at once. Tick the machines, or pick a group (the tags you gave them):

* **Search**: find files by name on every ticked machine; results are grouped by machine, and a click opens the hit in a pane.
* **Same file**: check one path on every machine: which have it, which differ, and compare two of them.
* **Free space**: the drives of every machine side by side.
* **Send**: copy the files selected in a pane to several machines in one go (also from the right-click menu: **Copy to…** → **Send to machines…**).

### Backups to other machines

**Backup & Sync** can back up to another Fleet machine, with schedules, old versions and restore. See [Backup & Sync](chewtoys.md).

---

## Who can do what

* **Accounts**: each machine uses its own account settings (home folder, allowed folders, read-only role) for the person signed in, so a person's limits on the NAS are the NAS's, not the main Brum's.
* **Folder rules**: on each machine, **Admin Panel → Folder Rules** says who may read a folder, write in it, or not touch it. See [Folder Rules](configuration.md#folder-rules).
* **Read-only machines**: a machine set to `read_only = true` is never written to.
* **Activity log**: each machine records who changed which files on it, also when the change came through the Fleet: **Admin Panel → Activity Log**, with a CSV download. See [Activity Log](configuration.md#activity-log).

---

## Safety

* **Trust**: a machine that holds the Fleet's secret can sign logins all others accept. Only pair machines you trust.
* **Unencrypted networks**: over plain `http`, the secret travels unencrypted while pairing. Pair over your own network or an overlay network, or put Brum behind `https`.
* **Removing a machine**: **Manage Fleet** → **Add machine** lists the paired machines with **Remove**. That takes it off the list; to shut it out completely, change the Fleet's secret and pair the others again.
* **Reports**: see [Security](https://github.com/Woofson/brum/blob/main/SECURITY.md) for how to report a problem and the published advisories.

---

## When something does not work

| What you see | What to check |
| :--- | :--- |
| The machine shows as offline | Brum runs there, listens on the network (`host = "0.0.0.0"`, not `127.0.0.1`), and the firewall lets port `3140` through from the main Brum. |
| The machine says "unauthorized" | Both machines share the same secret (pair again with a new code), and your account exists on that machine with the same name. |
| "This machine already uses a shared secret" when joining | The machine has `BRUM_JWT_SECRET`, `jwt_secret` or a secret file set. Remove it to join with a code, or give it the Fleet's secret by hand. |
| "The code is wrong or has expired" | Codes work once, for 10 minutes. Make a new one. |
| A copy goes through the browser and asks you to keep the tab open | The other machine runs Brum 1.22 or older. Update it. |
