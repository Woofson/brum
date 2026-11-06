//! Git collections: a repository that keeps every saved version of the tracked files.
//! Uses the `git` program, like Brum's Git Manager.

use super::store::{DirItem, FileMeta, Listing, Store};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;

/// Written into the repository: what is tracked and from where
pub const MANIFEST: &str = "brum-collection.toml";

pub fn is_available() -> bool {
    Command::new("git").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git is not available: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("git {} failed", args.first().unwrap_or(&"")) } else { err })
    }
}

/// Makes `repo` a git repository if it is not one yet
pub fn ensure_repo(repo: &Path) -> Result<(), String> {
    std::fs::create_dir_all(repo).map_err(|e| format!("Cannot create {}: {e}", repo.display()))?;
    if repo.join(".git").exists() {
        return Ok(());
    }
    git(repo, &["init", "--quiet"])?;
    let _ = git(repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    Ok(())
}

/// The manifest: which real paths are tracked, and where they are in the repository
pub fn write_manifest(repo: &Path, name: &str, items: &[(String, String)]) -> Result<(), String> {
    let mut text = String::from("# Made by Brum Backup & Sync. Lists what this collection tracks.\n");
    text.push_str(&format!("name = {}\n\n", toml_string(name)));
    for (real, mirror) in items {
        text.push_str(&format!("[[item]]\npath = {}\nin_repository = {}\n\n", toml_string(real), toml_string(mirror)));
    }
    std::fs::write(repo.join(MANIFEST), text).map_err(|e| e.to_string())
}

/// The repository's own files, never part of what is tracked
pub fn is_own_file(rel: &str) -> bool {
    rel == ".git" || rel.starts_with(".git/") || rel == MANIFEST || rel == super::crypt::KEY_FILE
}

/// What a collection's manifest says: its name and (original path, path in the repository) per item
pub fn read_manifest(repo: &Path) -> Result<(String, Vec<(String, String)>), String> {
    #[derive(serde::Deserialize)]
    struct Item {
        #[serde(default)]
        path: String,
        in_repository: String,
    }
    #[derive(serde::Deserialize)]
    struct Manifest {
        #[serde(default)]
        name: String,
        #[serde(default)]
        item: Vec<Item>,
    }
    let text = std::fs::read_to_string(repo.join(MANIFEST)).map_err(|_| format!("This is not a Brum collection: {MANIFEST} is missing"))?;
    let m: Manifest = toml::from_str(&text).map_err(|e| format!("{MANIFEST} could not be read: {e}"))?;
    if m.item.is_empty() {
        return Err(format!("{MANIFEST} lists no files or folders"));
    }
    Ok((m.name, m.item.into_iter().map(|i| (i.path, i.in_repository)).collect()))
}

/// The address the repository was cloned from or pushes to, if any
pub fn remote_url(repo: &Path) -> Option<String> {
    git(repo, &["remote", "get-url", "origin"]).ok().map(|u| u.trim().to_string()).filter(|u| !u.is_empty())
}

/// Downloads a collection repository into `into` (which must not exist yet or be empty)
pub fn clone(url: &str, into: &Path) -> Result<(), String> {
    if url.starts_with('-') {
        return Err("This remote address is not allowed".into());
    }
    let out = Command::new("git")
        .args(["-c", "protocol.ext.allow=never", "clone", "--quiet", "--", url])
        .arg(into)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git is not available: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("git clone failed").to_string())
}

fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Saves a version if anything changed; returns the new version's short id
pub fn commit(repo: &Path, message: &str, author: &str) -> Result<Option<String>, String> {
    git(repo, &["add", "--all"])?;
    if git(repo, &["status", "--porcelain"])?.trim().is_empty() {
        return Ok(None);
    }
    // Use the machine's git identity when there is one, otherwise name the Brum user
    let has_identity = git(repo, &["config", "user.email"]).map(|e| !e.trim().is_empty()).unwrap_or(false);
    let ident_name = format!("user.name={author}");
    let ident_mail = format!("user.email={}@brum.local", author.replace(' ', "."));
    let mut args: Vec<&str> = Vec::new();
    if !has_identity {
        args.extend(["-c", ident_name.as_str(), "-c", ident_mail.as_str()]);
    }
    args.extend(["commit", "--quiet", "-m", message]);
    git(repo, &args)?;
    Ok(Some(git(repo, &["rev-parse", "--short", "HEAD"])?.trim().to_string()))
}

/// Sets (or removes) the remote the collection is pushed to
pub fn set_remote(repo: &Path, url: Option<&str>) -> Result<(), String> {
    let current = git(repo, &["remote", "get-url", "origin"]).ok().map(|u| u.trim().to_string());
    match (url.map(str::trim).filter(|u| !u.is_empty()), current) {
        (Some(u), Some(c)) if u == c => Ok(()),
        (Some(u), Some(_)) => git(repo, &["remote", "set-url", "origin", u]).map(|_| ()),
        (Some(u), None) => git(repo, &["remote", "add", "origin", u]).map(|_| ()),
        (None, Some(_)) => git(repo, &["remote", "remove", "origin"]).map(|_| ()),
        (None, None) => Ok(()),
    }
}

/// Pushes the saved versions; logs in with the SSH keys or credential helper of the user Brum runs as
pub fn push(repo: &Path) -> Result<String, String> {
    if git(repo, &["rev-parse", "--verify", "HEAD"]).is_err() {
        return Err("Nothing has been saved yet".into());
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "protocol.ext.allow=never", "push", "--porcelain", "-u", "origin", "HEAD"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git is not available: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(if text.contains("[up to date]") { "Already up to date on the remote".into() } else { "Pushed to the remote".into() })
    } else {
        let last = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("To ")).last().unwrap_or("git push failed").to_string();
        Err(last)
    }
}

#[derive(Debug, Clone)]
pub struct Version {
    pub id: String,
    pub time: i64,
    pub message: String,
}

/// Saved versions, newest first
pub fn versions(repo: &Path, limit: usize) -> Vec<Version> {
    let limit_arg = format!("-{}", limit.max(1));
    git(repo, &["log", &limit_arg, "--format=%H%x09%ct%x09%s"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, '\t');
            Some(Version { id: p.next()?.to_string(), time: p.next()?.parse().ok()?, message: p.next().unwrap_or("").to_string() })
        })
        .collect()
}

/// True for full or short commit ids only, so nothing else can reach a git command
pub fn is_version_id(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The files of one saved version, read straight from git (for browsing and restoring)
pub struct GitVersionStore {
    repo: PathBuf,
    version: String,
    time: i64,
}

impl GitVersionStore {
    pub fn new(repo: &Path, version: &str) -> Result<Self, String> {
        if !is_version_id(version) {
            return Err("Unknown version".into());
        }
        let time = git(repo, &["show", "-s", "--format=%ct", version])?.trim().parse().unwrap_or(0);
        Ok(Self { repo: repo.to_path_buf(), version: version.to_string(), time })
    }

    /// `git ls-tree` lines below `rel`: (path, is_dir, size)
    fn tree(&self, rel: &str, recursive: bool) -> Result<Vec<(String, bool, u64)>, String> {
        let rel = rel.trim_matches('/');
        let spec = if rel.is_empty() { self.version.clone() } else { format!("{}:{}", self.version, rel) };
        let mut args = vec!["ls-tree", "-l", "-z"];
        if recursive {
            args.push("-r");
        }
        args.push(&spec);
        let out = git(&self.repo, &args)?;
        Ok(out
            .split('\0')
            .filter_map(|line| {
                let (info, path) = line.split_once('\t')?;
                let mut f = info.split_whitespace();
                let _mode = f.next()?;
                let kind = f.next()?;
                let _hash = f.next()?;
                let size = f.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                (kind == "blob" || kind == "tree").then(|| (path.to_string(), kind == "tree", size))
            })
            .collect())
    }
}

impl Store for GitVersionStore {
    fn label(&self) -> String {
        format!("{} at version {}", self.repo.display(), &self.version[..self.version.len().min(8)])
    }
    fn root_exists(&self) -> bool {
        true
    }
    fn ensure_root(&self) -> Result<(), String> {
        Err("Saved versions cannot be changed".into())
    }
    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, _cancel: &AtomicBool, _w: &mut Vec<String>) -> Result<Listing, String> {
        let mut out = Listing::new();
        if let Some(m) = self.stat(base) {
            out.insert(base.rsplit('/').next().unwrap_or(base).to_string(), m);
            return Ok(out);
        }
        for (path, is_dir, size) in self.tree(base, true)? {
            if is_dir || path == super::git_repo::MANIFEST || skip(&path, false) {
                continue;
            }
            out.insert(path, FileMeta { size, mtime: self.time });
        }
        Ok(out)
    }
    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        let mut items: Vec<DirItem> = self
            .tree(rel, false)?
            .into_iter()
            .filter(|(p, _, _)| !(rel.trim_matches('/').is_empty() && p == MANIFEST))
            .map(|(p, is_dir, size)| DirItem { name: p.rsplit('/').next().unwrap_or(&p).to_string(), is_dir, size, mtime: self.time })
            .collect();
        items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(items)
    }
    fn stat(&self, rel: &str) -> Option<FileMeta> {
        let rel = rel.trim_matches('/');
        if rel.is_empty() {
            return None;
        }
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        self.tree(dir, false).ok()?.into_iter().find(|(p, is_dir, _)| !is_dir && p.rsplit('/').next() == Some(name)).map(|(_, _, size)| FileMeta { size, mtime: self.time })
    }
    fn is_dir(&self, rel: &str) -> bool {
        rel.trim_matches('/').is_empty() || self.tree(rel, false).map(|t| !t.is_empty()).unwrap_or(false) && self.stat(rel).is_none()
    }
    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        let spec = format!("{}:{}", self.version, rel.trim_matches('/'));
        let out = Command::new("git").arg("-C").arg(&self.repo).args(["show", &spec]).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(Box::new(Cursor::new(out.stdout)))
    }
    fn write_part(&self, _rel: &str, _data: &mut dyn Read, _mtime: i64, _on: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        Err("Saved versions cannot be changed".into())
    }
    fn rename(&self, _from: &str, _to: &str) -> Result<(), String> {
        Err("Saved versions cannot be changed".into())
    }
    fn remove_file(&self, _rel: &str) -> Result<(), String> {
        Err("Saved versions cannot be changed".into())
    }
    fn remove_dir_all(&self, _rel: &str) -> Result<(), String> {
        Err("Saved versions cannot be changed".into())
    }
    fn remove_empty_parents(&self, _rel: &str) {}
    fn local_path(&self, _rel: &str) -> Option<PathBuf> {
        None
    }
}

/// Names that usually hold passwords or keys; they get a warning before they go into a pushed repository
pub fn looks_sensitive(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    let exact = ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa", ".env", ".netrc", ".pgpass", "credentials", "shadow", ".git-credentials", ".npmrc", ".pypirc"];
    let ends = [".pem", ".key", ".p12", ".pfx", ".kdbx", ".gpg", ".asc", ".ovpn"];
    exact.contains(&name.as_str()) || ends.iter().any(|e| name.ends_with(e)) || name.contains("secret") || name.contains("password") || (name.starts_with(".env.") && !name.ends_with(".example"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_names() {
        assert!(looks_sensitive("home/.ssh/id_ed25519"));
        assert!(looks_sensitive("home/.config/app/.env"));
        assert!(looks_sensitive("root/etc/ssl/server.key"));
        assert!(!looks_sensitive("home/.ssh/id_ed25519.pub"));
        assert!(!looks_sensitive("home/.config/nvim/init.lua"));
        assert!(!looks_sensitive("home/proj/.env.example"));
    }

    #[test]
    fn version_ids_only() {
        assert!(is_version_id("1a2b3c4"));
        assert!(!is_version_id("HEAD"));
        assert!(!is_version_id("--all"));
        assert!(!is_version_id("abc"));
    }
}
