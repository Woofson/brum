//! Where backups read from and write to.
//!
//! The engine only talks to the `Store` trait, so local folders (including mounted NAS
//! shares) and remote locations share the same backup, restore and logging logic.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Size and last-modified time (seconds) of one file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    pub size: u64,
    pub mtime: i64,
}

impl FileMeta {
    /// Same size and a modified time within 2 seconds (FAT and some network shares round times)
    pub fn same_as(&self, other: &FileMeta) -> bool {
        self.size == other.size && (self.mtime - other.mtime).abs() <= 2
    }
}

/// Every file below a folder, keyed by its path relative to that folder (`/` between parts)
pub type Listing = BTreeMap<String, FileMeta>;

/// One entry when browsing a folder
#[derive(Debug, Clone, Serialize)]
pub struct DirItem {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
}

/// Unfinished copies end in this; they are never treated as real files
pub const PART_SUFFIX: &str = ".brum-part";

pub trait Store: Send + Sync {
    /// Human readable location, without passwords
    fn label(&self) -> String;
    fn root_exists(&self) -> bool;
    fn ensure_root(&self) -> Result<(), String>;
    /// All files below `base` ("" for the top), keyed relative to `base`.
    /// `skip(rel, is_dir)` returning true leaves a file out, or a whole folder.
    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String>;
    /// One folder level, for browsing
    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String>;
    fn stat(&self, rel: &str) -> Option<FileMeta>;
    fn is_dir(&self, rel: &str) -> bool;
    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String>;
    /// Writes `data` to a temporary file beside `rel`, sets its modified time, returns its path.
    /// `on_bytes` is told how much was written so far; returning false stops the copy.
    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String>;
    /// Moves a file, creating folders above `to`; replaces an existing `to`
    fn rename(&self, from: &str, to: &str) -> Result<(), String>;
    fn remove_file(&self, rel: &str) -> Result<(), String>;
    fn remove_dir_all(&self, rel: &str) -> Result<(), String>;
    /// Removes folders above `rel` that became empty, up to (not including) the top
    fn remove_empty_parents(&self, rel: &str);
    /// The real path for local stores, for fast local-only work (partial updates, permissions)
    fn local_path(&self, rel: &str) -> Option<PathBuf>;
}

/// Rejects paths that could leave the backup (`..`, absolute paths, drive letters)
pub fn safe_rel(rel: &str) -> Result<String, String> {
    let norm = rel.replace('\\', "/");
    let mut parts = Vec::new();
    for part in norm.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(format!("Invalid path: {rel}")),
            p if p.contains(':') && cfg!(windows) => return Err(format!("Invalid path: {rel}")),
            p => parts.push(p),
        }
    }
    Ok(parts.join("/"))
}

/// `a/b` + `c` = `a/b/c`
pub fn join_rel(base: &str, rel: &str) -> String {
    match (base.trim_matches('/'), rel.trim_matches('/')) {
        ("", r) => r.to_string(),
        (b, "") => b.to_string(),
        (b, r) => format!("{b}/{r}"),
    }
}

fn mtime_secs(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A folder on this machine; also mounted Samba/CIFS and NFS shares
pub struct LocalStore {
    root: PathBuf,
}

impl LocalStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            p.push(part);
        }
        p
    }

    fn rel_of(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        for c in rel.components() {
            parts.push(c.as_os_str().to_str()?.to_string());
        }
        Some(parts.join("/"))
    }
}

impl Store for LocalStore {
    fn label(&self) -> String {
        self.root.display().to_string()
    }

    fn root_exists(&self) -> bool {
        self.root.is_dir()
    }

    fn ensure_root(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| format!("Cannot create {}: {e}", self.root.display()))
    }

    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String> {
        let start = self.path(base);
        if !start.exists() {
            return Ok(Listing::new());
        }
        if start.is_file() {
            let meta = fs::metadata(&start).map_err(|e| e.to_string())?;
            let name = start.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            let mut one = Listing::new();
            one.insert(name, FileMeta { size: meta.len(), mtime: mtime_secs(&meta) });
            return Ok(one);
        }
        let base_store = LocalStore::new(start.clone());
        let mut out = Listing::new();
        let walker = WalkDir::new(&start).follow_links(false).into_iter().filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            match base_store.rel_of(e.path()) {
                Some(rel) => !skip(&rel, e.file_type().is_dir()),
                None => true, // reported below
            }
        });
        for entry in walker {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    let what = e.path().and_then(|p| base_store.rel_of(p)).unwrap_or_default();
                    warnings.push(format!("Could not read {what}: {}", e.io_error().map(|io| io.to_string()).unwrap_or_else(|| e.to_string())));
                    continue;
                }
            };
            if entry.depth() == 0 || entry.file_type().is_dir() {
                continue;
            }
            let Some(rel) = base_store.rel_of(entry.path()) else {
                warnings.push(format!("Skipped a file whose name is not valid text: {}", entry.path().display()));
                continue;
            };
            if entry.file_type().is_symlink() {
                warnings.push(format!("Skipped link: {rel}"));
                continue;
            }
            if !entry.file_type().is_file() {
                continue; // sockets, devices, pipes
            }
            match entry.metadata() {
                Ok(meta) => {
                    out.insert(rel, FileMeta { size: meta.len(), mtime: mtime_secs(&meta) });
                }
                Err(e) => warnings.push(format!("Could not read {rel}: {e}")),
            }
        }
        Ok(out)
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        let dir = self.path(rel);
        let rd = fs::read_dir(&dir).map_err(|e| format!("Cannot open {}: {e}", dir.display()))?;
        let mut items = Vec::new();
        for ent in rd.flatten() {
            let Some(name) = ent.file_name().to_str().map(|s| s.to_string()) else { continue };
            let Ok(ft) = ent.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let meta = ent.metadata().ok();
            items.push(DirItem {
                name,
                is_dir: ft.is_dir(),
                size: if ft.is_dir() { 0 } else { meta.as_ref().map(|m| m.len()).unwrap_or(0) },
                mtime: meta.as_ref().map(mtime_secs).unwrap_or(0),
            });
        }
        items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(items)
    }

    fn stat(&self, rel: &str) -> Option<FileMeta> {
        let meta = fs::symlink_metadata(self.path(rel)).ok()?;
        meta.is_file().then(|| FileMeta { size: meta.len(), mtime: mtime_secs(&meta) })
    }

    fn is_dir(&self, rel: &str) -> bool {
        self.path(rel).is_dir()
    }

    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        File::open(self.path(rel)).map(|f| Box::new(f) as Box<dyn Read + Send>).map_err(|e| e.to_string())
    }

    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        let target = self.path(rel);
        let parent = target.parent().ok_or("Invalid path")?.to_path_buf();
        fs::create_dir_all(&parent).map_err(|e| format!("Cannot create folder: {e}"))?;
        let name = target.file_name().and_then(|n| n.to_str()).ok_or("Invalid file name")?;
        let part_name = format!(".{name}{PART_SUFFIX}");
        let part_path = parent.join(&part_name);
        let part_rel = match rel.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{part_name}"),
            None => part_name.clone(),
        };
        let result = (|| -> Result<(), String> {
            let mut out = File::create(&part_path).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 1024 * 1024];
            let mut written = 0u64;
            loop {
                let n = data.read(&mut buf).map_err(|e| format!("Read failed: {e}"))?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| format!("Write failed: {e}"))?;
                written += n as u64;
                if !on_bytes(written) {
                    return Err("Cancelled".to_string());
                }
            }
            out.flush().map_err(|e| e.to_string())?;
            drop(out);
            filetime::set_file_mtime(&part_path, filetime::FileTime::from_unix_time(mtime, 0)).map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => Ok(part_rel),
            Err(e) => {
                let _ = fs::remove_file(&part_path);
                Err(e)
            }
        }
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let (src, dst) = (self.path(from), self.path(to));
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Cannot create folder: {e}"))?;
        }
        fs::rename(&src, &dst).map_err(|e| format!("Cannot move {from}: {e}"))
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        fs::remove_file(self.path(rel)).map_err(|e| e.to_string())
    }

    fn remove_dir_all(&self, rel: &str) -> Result<(), String> {
        fs::remove_dir_all(self.path(rel)).map_err(|e| e.to_string())
    }

    fn remove_empty_parents(&self, rel: &str) {
        let mut cur = rel;
        while let Some((dir, _)) = cur.rsplit_once('/') {
            if fs::remove_dir(self.path(dir)).is_err() {
                break; // not empty (or already gone)
            }
            cur = dir;
        }
    }

    fn local_path(&self, rel: &str) -> Option<PathBuf> {
        Some(self.path(rel))
    }
}

/// Updates only the parts of `dst` that differ from `src` (both local). Returns bytes written.
pub fn update_changed_blocks(src: &Path, dst: &Path, mtime: i64, cancel: &AtomicBool, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<u64, String> {
    const BLOCK: usize = 1024 * 1024;
    let mut s = File::open(src).map_err(|e| e.to_string())?;
    let mut d = OpenOptions::new().read(true).write(true).open(dst).map_err(|e| e.to_string())?;
    let src_len = s.metadata().map_err(|e| e.to_string())?.len();
    let (mut sb, mut db) = (vec![0u8; BLOCK], vec![0u8; BLOCK]);
    let (mut offset, mut written) = (0u64, 0u64);
    while offset < src_len {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }
        let n = read_full(&mut s, &mut sb).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        d.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        let m = read_full(&mut d, &mut db[..n]).map_err(|e| e.to_string())?;
        if m != n || sb[..n] != db[..n] {
            d.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
            d.write_all(&sb[..n]).map_err(|e| e.to_string())?;
            written += n as u64;
        }
        offset += n as u64;
        if !on_bytes(offset) {
            return Err("Cancelled".to_string());
        }
    }
    d.set_len(src_len).map_err(|e| e.to_string())?;
    d.flush().map_err(|e| e.to_string())?;
    drop(d);
    filetime::set_file_mtime(dst, filetime::FileTime::from_unix_time(mtime, 0)).map_err(|e| e.to_string())?;
    Ok(written)
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = f.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

/// CRC-32 of a stored file
pub fn checksum(store: &dyn Store, rel: &str) -> Result<u32, String> {
    let mut r = store.open_read(rel)?;
    let mut h = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize())
}

/// Passes data through while computing its CRC-32
pub struct CrcReader<R: Read> {
    inner: R,
    hasher: crc32fast::Hasher,
    pub bytes: u64,
}

impl<R: Read> CrcReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, hasher: crc32fast::Hasher::new(), bytes: 0 }
    }
    pub fn crc(&self) -> u32 {
        self.hasher.clone().finalize()
    }
}

impl<R: Read> Read for CrcReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_rel_rejects_escapes() {
        assert_eq!(safe_rel("a/./b//c").unwrap(), "a/b/c");
        assert_eq!(safe_rel("/a/b").unwrap(), "a/b");
        assert!(safe_rel("a/../../etc").is_err());
        assert!(safe_rel("..").is_err());
    }

    #[test]
    fn write_part_then_rename() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::new(dir.path());
        let mut data: &[u8] = b"hello";
        let part = store.write_part("x/y.txt", &mut data, 1_700_000_000, &mut |_| true).unwrap();
        assert!(part.ends_with(PART_SUFFIX));
        assert!(store.stat("x/y.txt").is_none());
        store.rename(&part, "x/y.txt").unwrap();
        let meta = store.stat("x/y.txt").unwrap();
        assert_eq!(meta, FileMeta { size: 5, mtime: 1_700_000_000 });
    }

    #[test]
    fn changed_blocks_only() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let mut big = vec![7u8; 3 * 1024 * 1024 + 10];
        fs::write(&b, &big).unwrap();
        big[2 * 1024 * 1024 + 5] = 9;
        big.truncate(3 * 1024 * 1024);
        fs::write(&a, &big).unwrap();
        let written = update_changed_blocks(&a, &b, 1_700_000_000, &AtomicBool::new(false), &mut |_| true).unwrap();
        assert_eq!(written, 1024 * 1024);
        assert_eq!(fs::read(&b).unwrap(), big);
    }
}
