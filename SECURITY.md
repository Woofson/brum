# Security

Brum reaches the files of every machine in your Fleet, so its security matters. This page says how to report a problem, what happens next, and every problem found so far with the version that fixed it.

## Reporting a problem

Report security problems **privately**, never in a public issue, discussion or pull request:

* On GitHub, open the repository's **Security** tab and click **Report a vulnerability**. Only the maintainer sees the report.

Helpful to include: the Brum version (shown in **About**, or `brum --version`), the system (Linux, Windows, Docker), what someone could do, and the steps to show it. A short report is fine; questions come back through the same private report.

## What happens next

Brum has one maintainer, so these are aims rather than promises:

1. **Answer**: within a few days, saying whether the problem is confirmed.
2. **Fix**: in a new release, as soon as it is ready. Serious problems get their own release instead of waiting for the next planned one.
3. **Publish**: when the fixed release is out, a security advisory goes up on GitHub with what was wrong, who was affected, and the version that fixed it. Until then nothing about the problem is made public, also not in commit messages.
4. **Credit**: if you would like to be named in the advisory, say so in your report.

Only the latest release gets fixes, so keep Brum updated. New releases are listed on the [releases page](https://github.com/Woofson/brum/releases); on GitHub, **Watch → Custom → Releases** sends you a notice for each one.

## Published advisories

| Severity | Problem | Affected | Fixed in | Published |
| :--- | :--- | :--- | :--- | :--- |
| Low | [Unauthenticated configuration endpoint reveals Fleet machine names and addresses](https://github.com/Woofson/brum/security/advisories/GHSA-8rgf-8pgm-v444) | 1.22.0 and older | 1.23.0 | 2026-11-15 |
| Low | [Unauthenticated system status reveals the service account name and home folder](https://github.com/Woofson/brum/security/advisories/GHSA-6p5c-gr9f-gxx7) | 1.21.0 and older | 1.22.0 | 2026-11-14 |
| Medium | [Windows all-users install keeps the service database readable to local users](https://github.com/Woofson/brum/security/advisories/GHSA-9qgm-hfjj-hxq4) | 1.21.0 | 1.22.0 | 2026-11-14 |
| High | [Windows all-users install lets any local user run code as SYSTEM and read Brum's secrets](https://github.com/Woofson/brum/security/advisories/GHSA-385p-4c6h-62cr) | 1.6.2 to 1.20.0 | 1.21.0 | 2026-11-12 |
| High | [Stored credentials and the node-local login signing secret readable from the database file](https://github.com/Woofson/brum/security/advisories/GHSA-f63j-9gj5-cxcc) | 1.17.0 and older | 1.18.0 | 2026-11-05 |
| Medium | [Notes export writes files outside the notes folder](https://github.com/Woofson/brum/security/advisories/GHSA-rc6h-m6rf-x5j7) | 0.8.8 to 1.16.0 | 1.17.0 | 2026-11-04 |
| High | [Unauthenticated file read when Brum runs next to a frontend folder](https://github.com/Woofson/brum/security/advisories/GHSA-qvqw-wx9j-7fcr) | 0.8.8 to 1.16.0 | 1.17.0 | 2026-11-04 |
| High | [Authenticated arbitrary file read through Chewtoy asset paths](https://github.com/Woofson/brum/security/advisories/GHSA-pv23-5739-f2h3) | 0.8.8 to 1.16.0 | 1.17.0 | 2026-11-04 |
| High | [Shell command injection through file names in Custom Actions and Open With](https://github.com/Woofson/brum/security/advisories/GHSA-vqc6-335r-r2m4) | before 1.16.0 | 1.16.0 | 2026-11-03 |
| Critical | [Access control and injection weaknesses in Brum before 1.15.0](https://github.com/Woofson/brum/security/advisories/GHSA-jjmj-g7wj-5q2c) | before 1.15.0 | 1.15.0 | 2026-11-02 |

The full list, always current: [github.com/Woofson/brum/security/advisories](https://github.com/Woofson/brum/security/advisories).

## Running Brum safely

* **Never on the open internet.** Run Brum on your own network, or reach it through an overlay network such as Tailscale, NetBird or WireGuard. Brum includes a terminal and full file access; it is not built to face the internet directly. See the [Reverse Proxy & Mesh VPN Guide](manuals/reverse-proxy.md).
* **Use `https` across networks you do not control**, for example with a reverse proxy, so logins and pairing codes do not travel unencrypted.
* **Keep every machine updated.** In a Fleet, update all machines, not only the main Brum.
* **Give people only what they need**: their own folders, read-only accounts where that is enough, and [folder rules](manuals/configuration.md#folder-rules) for shared places. The [activity log](manuals/configuration.md#activity-log) shows who changed which files.
* **Pair only machines you trust.** A machine in the Fleet can sign logins the other machines accept. See [Several machines (Fleet)](manuals/fleet.md#safety).
* **Protect the files next to the database**: `brum-secrets.key` unlocks saved passwords, and `fleet-secret.key` signs Fleet logins. Brum makes them readable to itself only; keep it that way, and keep them out of shared backups.
