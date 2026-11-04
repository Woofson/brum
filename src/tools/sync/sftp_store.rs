//! Backups to and from an SFTP server (for example a Hetzner Storage Box), over one
//! SSH connection for the whole run. Login uses Brum's usual order: password, key file,
//! SSH agent, then the keys in `~/.ssh`; host keys are checked as configured for SFTP.

use super::store::{DirItem, FileMeta, Listing, Store};
use crate::vfs::sftp::{SftpClient, SftpParams};
use ssh2::{FileStat, RenameFlags, Session, Sftp};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct SftpStore {
    params: SftpParams,
    /// Shown in logs and messages; never contains a password
    label: String,
    conn: Mutex<Option<Conn>>,
}

struct Conn {
    // The session must outlive the SFTP channel
    _session: Session,
    sftp: Sftp,
    root: String,
}

impl SftpStore {
    /// `uri` is `sftp://user@host:port/path` (or `ssh://`); a password in the address is used but never shown
    pub fn new(uri: &str, password: Option<&str>, config: &crate::config::SftpConfig) -> Result<Self, String> {
        let mut params = SftpClient::parse_uri_with_config(uri, None, password, Some(config))?;
        if params.host.trim().is_empty() {
            return Err("The SFTP address has no server name".into());
        }
        // "sftp://host/~/backups" means a folder in the login's home
        if params.remote_path.starts_with("/~") {
            params.remote_path = params.remote_path[1..].to_string();
        }
        let label = crate::vfs::sanitize_uri(uri);
        Ok(Self { params, label, conn: Mutex::new(None) })
    }

    /// Runs `f` with a connected SFTP channel, connecting on first use
    fn with<T>(&self, f: impl FnOnce(&Sftp, &str) -> Result<T, String>) -> Result<T, String> {
        let mut guard = self.conn.lock().map_err(|_| "SFTP connection is unusable".to_string())?;
        if guard.is_none() {
            let session = SftpClient::connect(&self.params).map_err(|e| format!("Cannot connect to {}: {e}", self.label))?;
            let sftp = session.sftp().map_err(|e| format!("Cannot start SFTP on {}: {e}", self.label))?;
            let root = SftpClient::resolve_remote_path(&sftp, &self.params.remote_path).trim_end_matches('/').to_string();
            let root = if root.is_empty() { "/".to_string() } else { root };
            *guard = Some(Conn { _session: session, sftp, root });
        }
        let conn = guard.as_ref().expect("connected above");
        f(&conn.sftp, &conn.root)
    }

    fn full(root: &str, rel: &str) -> PathBuf {
        let rel = rel.trim_matches('/');
        if rel.is_empty() {
            PathBuf::from(root)
        } else if root == "/" {
            PathBuf::from(format!("/{rel}"))
        } else {
            PathBuf::from(format!("{root}/{rel}"))
        }
    }

    /// Creates `dir` and the folders above it
    fn mkdirs(sftp: &Sftp, dir: &Path) -> Result<(), String> {
        if sftp.lstat(dir).map(|s| s.is_dir()).unwrap_or(false) {
            return Ok(());
        }
        if let Some(parent) = dir.parent() {
            if parent != dir && !parent.as_os_str().is_empty() {
                Self::mkdirs(sftp, parent)?;
            }
        }
        match sftp.mkdir(dir, 0o755) {
            Ok(()) => Ok(()),
            // Someone else made it in the meantime
            Err(_) if sftp.lstat(dir).map(|s| s.is_dir()).unwrap_or(false) => Ok(()),
            Err(e) => Err(format!("Cannot create folder {}: {e}", dir.display())),
        }
    }

    fn remove_tree(sftp: &Sftp, dir: &Path) -> Result<(), String> {
        for (path, stat) in sftp.readdir(dir).map_err(|e| e.to_string())? {
            if stat.is_dir() {
                Self::remove_tree(sftp, &path)?;
            } else {
                sftp.unlink(&path).map_err(|e| e.to_string())?;
            }
        }
        sftp.rmdir(dir).map_err(|e| e.to_string())
    }
}

fn is_link(stat: &FileStat) -> bool {
    stat.perm.is_some_and(|p| p & 0o170000 == 0o120000)
}

fn meta(stat: &FileStat) -> FileMeta {
    FileMeta { size: stat.size.unwrap_or(0), mtime: stat.mtime.unwrap_or(0) as i64 }
}

fn name_of(path: &Path) -> Option<String> {
    path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string())
}

/// Reading a remote file while keeping the store's connection lock free
struct RemoteFile(ssh2::File);

impl Read for RemoteFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Store for SftpStore {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn root_exists(&self) -> bool {
        self.with(|sftp, root| Ok(sftp.stat(Path::new(root)).map(|s| s.is_dir()).unwrap_or(false))).unwrap_or(false)
    }

    fn ensure_root(&self) -> Result<(), String> {
        self.with(|sftp, root| Self::mkdirs(sftp, Path::new(root)))
    }

    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String> {
        self.with(|sftp, root| {
            let start = Self::full(root, base);
            let mut out = Listing::new();
            match sftp.stat(&start) {
                Err(_) => return Ok(out),
                Ok(s) if !s.is_dir() => {
                    if let Some(n) = name_of(&start) {
                        out.insert(n, meta(&s));
                    }
                    return Ok(out);
                }
                Ok(_) => {}
            }
            // Folders still to read, each with its path relative to `base`
            let mut todo = vec![(start, String::new())];
            while let Some((dir, rel_dir)) = todo.pop() {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                let entries = match sftp.readdir(&dir) {
                    Ok(e) => e,
                    Err(e) => {
                        warnings.push(format!("Could not read {}: {e}", if rel_dir.is_empty() { "the top folder" } else { &rel_dir }));
                        continue;
                    }
                };
                for (path, stat) in entries {
                    let Some(name) = name_of(&path) else {
                        warnings.push(format!("Skipped a file whose name is not valid text: {}", path.display()));
                        continue;
                    };
                    if name == "." || name == ".." {
                        continue;
                    }
                    let rel = if rel_dir.is_empty() { name } else { format!("{rel_dir}/{name}") };
                    if is_link(&stat) {
                        warnings.push(format!("Skipped link: {rel}"));
                        continue;
                    }
                    let is_dir = stat.is_dir();
                    if skip(&rel, is_dir) {
                        continue;
                    }
                    if is_dir {
                        todo.push((path, rel));
                    } else if stat.is_file() {
                        out.insert(rel, meta(&stat));
                    }
                }
            }
            Ok(out)
        })
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        self.with(|sftp, root| {
            let dir = Self::full(root, rel);
            let entries = sftp.readdir(&dir).map_err(|e| format!("Cannot open {}: {e}", dir.display()))?;
            let mut items: Vec<DirItem> = entries
                .into_iter()
                .filter(|(_, s)| !is_link(s))
                .filter_map(|(p, s)| {
                    let name = name_of(&p)?;
                    (name != "." && name != "..").then(|| DirItem { name, is_dir: s.is_dir(), size: if s.is_dir() { 0 } else { s.size.unwrap_or(0) }, mtime: s.mtime.unwrap_or(0) as i64 })
                })
                .collect();
            items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            Ok(items)
        })
    }

    fn stat(&self, rel: &str) -> Option<FileMeta> {
        self.with(|sftp, root| Ok(sftp.lstat(&Self::full(root, rel)).ok().filter(|s| s.is_file()).map(|s| meta(&s)))).ok().flatten()
    }

    fn is_dir(&self, rel: &str) -> bool {
        self.with(|sftp, root| Ok(sftp.stat(&Self::full(root, rel)).map(|s| s.is_dir()).unwrap_or(false))).unwrap_or(false)
    }

    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        self.with(|sftp, root| {
            let f = sftp.open(&Self::full(root, rel)).map_err(|e| format!("Cannot open {rel}: {e}"))?;
            Ok(Box::new(RemoteFile(f)) as Box<dyn Read + Send>)
        })
    }

    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        self.with(|sftp, root| {
            let target = Self::full(root, rel);
            let parent = target.parent().ok_or("Invalid path")?.to_path_buf();
            Self::mkdirs(sftp, &parent)?;
            let part_rel = super::store::part_name(rel);
            let part_path = Self::full(root, &part_rel);
            let result = (|| -> Result<(), String> {
                let mut out = sftp.create(&part_path).map_err(|e| format!("Cannot write {rel}: {e}"))?;
                let mut buf = vec![0u8; 256 * 1024];
                let mut written = 0u64;
                loop {
                    let n = data.read(&mut buf).map_err(|e| format!("Read failed: {e}"))?;
                    if n == 0 {
                        break;
                    }
                    out.write_all(&buf[..n]).map_err(|e| format!("Write failed: {e}"))?;
                    written += n as u64;
                    if !on_bytes(written) {
                        return Err("Cancelled".into());
                    }
                }
                out.flush().map_err(|e| e.to_string())?;
                drop(out);
                let times = FileStat { size: None, uid: None, gid: None, perm: None, atime: Some(mtime.max(0) as u64), mtime: Some(mtime.max(0) as u64) };
                sftp.setstat(&part_path, times).map_err(|e| format!("Cannot set the file's time: {e}"))
            })();
            match result {
                Ok(()) => Ok(part_rel),
                Err(e) => {
                    let _ = sftp.unlink(&part_path);
                    Err(e)
                }
            }
        })
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.with(|sftp, root| {
            let (src, dst) = (Self::full(root, from), Self::full(root, to));
            if let Some(parent) = dst.parent() {
                Self::mkdirs(sftp, parent)?;
            }
            let flags = Some(RenameFlags::OVERWRITE | RenameFlags::ATOMIC | RenameFlags::NATIVE);
            if sftp.rename(&src, &dst, flags).is_ok() {
                return Ok(());
            }
            // Many servers (SFTP version 3) refuse to rename over an existing file
            if sftp.lstat(&dst).map(|s| s.is_file()).unwrap_or(false) {
                sftp.unlink(&dst).map_err(|e| format!("Cannot replace {to}: {e}"))?;
            }
            sftp.rename(&src, &dst, None).map_err(|e| format!("Cannot move {from}: {e}"))
        })
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        self.with(|sftp, root| sftp.unlink(&Self::full(root, rel)).map_err(|e| e.to_string()))
    }

    fn remove_dir_all(&self, rel: &str) -> Result<(), String> {
        self.with(|sftp, root| Self::remove_tree(sftp, &Self::full(root, rel)))
    }

    fn remove_empty_parents(&self, rel: &str) {
        let _ = self.with(|sftp, root| {
            let mut cur = rel.to_string();
            while let Some((dir, _)) = cur.rsplit_once('/') {
                if sftp.rmdir(&Self::full(root, dir)).is_err() {
                    break;
                }
                cur = dir.to_string();
            }
            Ok(())
        });
    }

    fn local_path(&self, _rel: &str) -> Option<PathBuf> {
        None
    }
}
